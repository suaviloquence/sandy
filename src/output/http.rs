use std::{
	collections::{HashMap, VecDeque},
	convert::Infallible,
	fmt::Write,
	net::SocketAddr,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc, Mutex,
	},
	time::Duration,
};

use futures::{future, StreamExt};
use hyper::{
	body::{Buf, Bytes},
	header,
	server::conn::AddrStream,
	service::{make_service_fn, service_fn},
	Body, Request, Response,
};
use tokio::sync::{mpsc, RwLock};

use crate::{
	playlist::{Playlist, SongMetadata},
	song::{
		mp3::{Frame, Mp3},
		Song,
	},
};

use super::{Message, Receiver, Sender};

#[derive(Debug)]
struct BodyStream(hyper::body::Sender);

impl BodyStream {
	#[inline]
	fn ok(res: hyper::Result<()>) -> bool {
		match res {
			Ok(_) => true,
			Err(e) => {
				if !e.is_closed() {
					log::warn!("Error writing to body stream: {:?}", e);
				}
				false
			}
		}
	}

	fn frame_to_bytes(frames: &[Frame]) -> Bytes {
		frames
			.iter()
			.flat_map(|frame| {
				frame
					.header
					.iter()
					.copied()
					.chain(frame.data.iter().copied())
			})
			.collect()
	}

	fn metadata_to_bytes(song: &SongMetadata) -> Bytes {
		let mut data = Vec::with_capacity(song.title.len() + 2 + song.artist.len() + 2);
		data.extend((song.title.len() as u16).to_be_bytes());
		data.extend(song.title.as_bytes());
		data.extend((song.artist.len() as u16).to_be_bytes());
		data.extend(song.artist.as_bytes());

		Bytes::from(data)
	}

	fn message_to_bytes(msg: &Message) -> Bytes {
		match msg {
			Message::Frames(f) => Self::frame_to_bytes(f),
			Message::Next(n) => Self::metadata_to_bytes(n),
		}
	}

	async fn send(&mut self, bytes: Bytes) -> bool {
		Self::ok(self.0.send_data(bytes).await)
	}
}

#[derive(Debug, Clone)]
struct State {
	playlist: Arc<Mutex<Playlist>>,
	curr: Arc<RwLock<SongMetadata>>,
	curr_chunk: Arc<RwLock<Option<Vec<Frame>>>>,
	conns: Arc<tokio::sync::Mutex<HashMap<usize, Sender>>>,
	conn_idx: Arc<AtomicUsize>,
}

impl State {
	async fn route(self, req: Request<Body>) -> hyper::http::Result<Response<Body>> {
		match req.uri().path() {
			"/" => self.app().await,
			"/queue" => self.queue().await,
			"/stream" => self.stream().await,
			"/skip" => self.skip().await,
			"/now" => self.now().await,
			path => Self::not_found(path).await,
		}
	}

	async fn app(self) -> hyper::http::Result<Response<Body>> {
		let contents = tokio::fs::read("app/index.html")
			.await
			.expect("Error loading file contents");

		Response::builder()
			.header(header::CONTENT_TYPE, "text/html")
			.body(Body::from(contents))
	}

	async fn queue(self) -> hyper::http::Result<Response<Body>> {
		let mut writer = String::new();
		let playlist_guard = self
			.playlist
			.lock()
			.expect("Error locking playlist to read");

		for song in playlist_guard.iter().take(5) {
			write!(
				&mut writer,
				"{}\n{}\n",
				song.metadata.artist, song.metadata.title
			)
			.expect("Error writing to buffer");
		}

		Response::builder()
			.header(header::CONTENT_TYPE, "text/plain;charset=utf-8")
			.body(Body::from(writer))
	}

	async fn skip(self) -> hyper::http::Result<Response<Body>> {
		let skipped = self
			.playlist
			.lock()
			.expect("Error locking mutex to skip song")
			.pop_front()
			.is_some();

		let text = if skipped { "OK" } else { "Empty" };

		Response::builder()
			.header(header::CONTENT_TYPE, "text/plain")
			.body(Body::from(text))
	}

	async fn now(self) -> hyper::http::Result<Response<Body>> {
		let guard = self.curr.read().await;

		Response::builder()
			.header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
			.body(Body::from(format!("{}\n{}", guard.artist, guard.title)))
	}

	async fn stream(self) -> hyper::http::Result<Response<Body>> {
		let (sx, mut rx) = mpsc::channel(8);

		self.conns
			.lock()
			.await
			.insert(self.conn_idx.fetch_add(1, Ordering::Relaxed), sx);

		let (sx, body) = Body::channel();
		let mut sx = BodyStream(sx);

		let mut data = BodyStream::metadata_to_bytes(&*self.curr.read().await);

		dbg!(self.curr.try_read().is_err());

		let chunk = self
			.curr_chunk
			.read()
			.await
			.as_deref()
			.map(BodyStream::frame_to_bytes);

		if let Some(next) = chunk {
			let len = data.len() + next.len();
			data = data.chain(next).copy_to_bytes(len);
		}

		if !sx.send(data).await {
			rx.close();
		}

		dbg!(self.curr_chunk.try_read().is_err());

		tokio::spawn(async move {
			while let Some(msg) = rx.recv().await {
				let data = BodyStream::message_to_bytes(msg.as_ref());
				drop(msg);

				if !sx.send(data).await {
					rx.close();
					break;
				}
			}
		});

		Response::builder()
			.header(header::CONTENT_TYPE, "application/x-mp3+info")
			.body(body)
	}

	async fn not_found(_path: &str) -> hyper::http::Result<Response<Body>> {
		Response::builder()
			.header(header::CONTENT_TYPE, "text/plain")
			.body(Body::from("Invalid path"))
	}
}

#[derive(Debug)]
pub struct Server {
	rx: Receiver,
	state: State,
}

impl Server {
	pub fn new(playlist: Arc<Mutex<Playlist>>, rx: Receiver) -> Self {
		Self {
			rx,
			state: State {
				curr: Arc::new(RwLock::const_new({
					let guard = playlist
						.lock()
						.expect("Error unlocking playlist in http::Server::new()");
					let song = guard.front().expect("Playlist is empty").metadata.clone();

					drop(guard);
					song
				})),
				curr_chunk: Default::default(),
				conns: Default::default(),
				conn_idx: Default::default(),
				playlist,
			},
		}
	}

	async fn worker(&mut self) {
		while let Some(msg) = self.rx.recv().await {
			match msg.as_ref() {
				Message::Next(n) => *self.state.curr.write().await = n.clone(),
				Message::Frames(f) => {
					let mut guard = self.state.curr_chunk.write().await;
					*guard = Some(f.clone());
					drop(guard)
				}
			}

			let mut conns_guard = self.state.conns.lock().await;

			let failures: Vec<_> = futures::stream::iter(conns_guard.iter().map(|(idx, sx)| {
				let msg = Arc::clone(&msg);

				async move { sx.send(msg).await.err().map(|_| *idx) }
			}))
			.filter_map(|x| async { x.await })
			.collect()
			.await;

			for failure in failures {
				conns_guard.remove(&failure);
			}
		}
	}

	pub async fn run_loop(mut self) {
		let addr = SocketAddr::from(([0, 0, 0, 0], 6912));

		let state = self.state.clone();
		let make_service = make_service_fn(|_: &AddrStream| {
			let state = state.clone();

			let service = service_fn(move |req| state.clone().route(req));

			async move { Ok::<_, Infallible>(service) }
		});

		let server = hyper::Server::bind(&addr).serve(make_service);

		if let (_, Err(e)) = tokio::join!(self.worker(), server) {
			log::error!("Error running http: {:?}", e);
		}
	}
}

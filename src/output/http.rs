use std::{
	convert::Infallible,
	fmt::Write,
	net::SocketAddr,
	sync::{
		Arc, Mutex, RwLock,
	},
};

use hyper::{
	body::Bytes,
	header,
	server::conn::AddrStream,
	service::{make_service_fn, service_fn},
	Body, Request, Response,
};

use crate::{
	playlist::{Playlist, SongMetadata},
	runner::{Control, ControlSender, Current},
	song::mp3::Frame,
};

use super::Message;

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
	rx: Arc<RwLock<lighthouse::Receiver<Message>>>,
	current: Arc<Current>,
	control: ControlSender,
}

impl State {
	async fn route(self, req: Request<Body>) -> hyper::http::Result<Response<Body>> {
		match req.uri().path() {
			"/" => self.app().await,
			"/queue" => self.queue().await,
			"/stream" => self.stream().await,
			"/skip/next" => self.skip_next().await,
			"/skip/curr" => self.skip_curr().await,
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
				song.metadata.title, song.metadata.artist
			)
			.expect("Error writing to buffer");
		}

		Response::builder()
			.header(header::CONTENT_TYPE, "text/plain;charset=utf-8")
			.body(Body::from(writer))
	}

	async fn skip_next(self) -> hyper::http::Result<Response<Body>> {
		let skipped = self
			.playlist
			.lock()
			.expect("Error locking mutex to skip song")
			.pop_front()
			.is_some();

		let (status, text) = if skipped { (200, "OK") } else { (400, "Empty") };

		Response::builder()
			.status(status)
			.header(header::CONTENT_TYPE, "text/plain")
			.body(Body::from(text))
	}

	async fn skip_curr(self) -> hyper::http::Result<Response<Body>> {
		self.control
			.send(Control::SkipCurr)
			.await
			.expect("Error sending skip curr signal");

		Response::builder()
			.header(header::CONTENT_TYPE, "text/plain")
			.body(Body::from("OK"))
	}

	async fn now(self) -> hyper::http::Result<Response<Body>> {
		let guard = self.current.song.read().await;

		if let Some(song) = guard.as_ref() {
			Response::builder()
				.header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
				.body(Body::from(format!("{}\n{}", song.title, song.artist)))
		} else {
			Response::builder()
				.status(400)
				.header(header::CONTENT_TYPE, "text/plain")
				.body(Body::from("not playing"))
		}
	}

	async fn stream(self) -> hyper::http::Result<Response<Body>> {
		let mut rx = self.rx.read().expect("RwLock poisoned").clone();

		let (sx, body) = Body::channel();
		let mut sx = BodyStream(sx);

		let metadata = self
			.current
			.song
			.read()
			.await
			.as_ref()
			.map(BodyStream::metadata_to_bytes);

		let chunk = self
			.current
			.chunk
			.read()
			.await
			.as_deref()
			.map(BodyStream::frame_to_bytes);

		match (metadata, chunk) {
			(Some(metadata), Some(chunk)) => {
				sx.send(metadata).await;
				sx.send(chunk).await;
			}
			(Some(data), None) | (None, Some(data)) => {
				sx.send(data).await;
			}
			(None, None) => (),
		};

		tokio::spawn(async move {
			while let Ok(msg) = rx.recv().await {
				let data = BodyStream::message_to_bytes(msg.as_ref());
				drop(msg);

				if !sx.send(data).await {
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
	state: State,
}

impl Server {
	pub fn new(
		rx: lighthouse::Receiver<Message>,
		playlist: Arc<Mutex<Playlist>>,
		current: Arc<Current>,
		control: ControlSender,
	) -> Self {
		Self {
			state: State {
				current,
				rx: Arc::new(RwLock::new(rx)),
				playlist,
				control,
			},
		}
	}
	
	pub async fn run_loop(self) {
		let addr = SocketAddr::from(([0, 0, 0, 0], 6912));

		let state = self.state.clone();
		let make_service = make_service_fn(|_: &AddrStream| {
			let state = state.clone();

			let service = service_fn(move |req| state.clone().route(req));

			async move { Ok::<_, Infallible>(service) }
		});

		let server = hyper::Server::bind(&addr).serve(make_service);

		if let Err(e) = server.await {
			log::error!("Error running http: {:?}", e);
		}
	}
}

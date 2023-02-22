use std::{
	collections::HashMap,
	io,
	net::SocketAddr,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	},
};

use futures::{future, StreamExt};
use tokio::{
	io::AsyncWriteExt,
	net::{tcp::OwnedWriteHalf, TcpListener},
	sync::{mpsc, Mutex, RwLock},
};

use crate::{
	playlist::SongMetadata,
	song::{id3::Id3, mp3::Frame},
};

use super::Message;

#[derive(Debug)]
pub struct Tcp {
	rx: mpsc::Receiver<Arc<Message>>,
	conns: Arc<Mutex<HashMap<usize, mpsc::Sender<Arc<Message>>>>>,
	curr_id: Arc<RwLock<Vec<u8>>>,
	idx: AtomicUsize,
}

impl Tcp {
	pub fn new(rx: mpsc::Receiver<Arc<Message>>, curr: &SongMetadata) -> Self {
		Self {
			rx,
			curr_id: Arc::new(RwLock::new(Id3::from_song(curr).as_bytes())),
			conns: Default::default(),
			idx: AtomicUsize::new(0),
		}
	}

	pub async fn run_loop(self) -> io::Result<()> {
		let queue = {
			let mut rx = self.rx;
			let conns = Arc::clone(&self.conns);
			let curr = Arc::clone(&self.curr_id);

			async move {
				while let Some(msg) = rx.recv().await {
					match msg.as_ref() {
						Message::Next(next) => {
							*curr.write().await = Id3::from_song(next).as_bytes();
						}
						Message::Frames(_) => {
							let mut conns_guard = conns.lock().await;

							let failures: Vec<usize> =
								futures::stream::iter(conns_guard.iter().map(|(id, sx)| {
									let msg = msg.clone();
									async move { sx.send(msg).await.err().map(|_| id) }
								}))
								.filter_map(|x| x)
								.collect()
								.await;

							for failure in &failures {
								conns_guard.remove(failure);
							}
						}
					}
				}
			}
		};

		let server = {
			let conns = Arc::clone(&self.conns);
			let curr = Arc::clone(&self.curr_id);

			async move {
				let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 3615))).await?;

				loop {
					match listener.accept().await {
						Ok((stream, _addr)) => {
							let (_reader, writer) = stream.into_split();
							let (sx, rx) = mpsc::channel(4);
							conns
								.lock()
								.await
								.insert(self.idx.fetch_add(1, Ordering::SeqCst), sx);

							let curr = Arc::clone(&curr);

							tokio::spawn(async move {
								if let Err(e) = writer_loop(rx, writer, curr).await {
									log::error!("Write to stream error: {:?}", e);
								}
							});

							// TODO: reader
						}
						Err(e) => log::error!("Error accepting connection: {:?}", e),
					}
				}
			}
		};

		future::join(queue, server).await.1
	}
}

async fn writer_loop(
	mut rx: mpsc::Receiver<Arc<Message>>,
	mut writer: OwnedWriteHalf,
	curr: Arc<RwLock<Vec<u8>>>,
) -> io::Result<()> {
	writer.write(&curr.read().await).await?;

	while let Some(msg) = rx.recv().await {
		if writer.writable().await.is_err() {
			rx.close();
			// TODO: break vs continue
			break;
		}
		match msg.as_ref() {
			Message::Next(_) => {
				// writer.write(&id3).await?;
				// jk dont do anything - no metadata allowed in the middle of a stream
			}
			Message::Frames(frames) => {
				for frame in frames.iter() {
					writer.write(&*frame.header).await?;
					writer.write(&frame.data).await?;
				}

				writer.flush().await?;
			}
		}
	}

	Ok(())
}

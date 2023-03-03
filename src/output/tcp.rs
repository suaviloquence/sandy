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
	sync::{mpsc, Mutex},
};

use crate::runner::Current;

use super::Message;

#[derive(Debug)]
pub struct Tcp {
	rx: mpsc::Receiver<Arc<Message>>,
	conns: Arc<Mutex<HashMap<usize, mpsc::Sender<Arc<Message>>>>>,
	idx: AtomicUsize,
	current: Current,
}

impl Tcp {
	pub fn new(rx: mpsc::Receiver<Arc<Message>>, current: Current) -> Self {
		Self {
			rx,
			conns: Default::default(),
			idx: AtomicUsize::new(0),
			current,
		}
	}

	pub async fn run_loop(self) -> io::Result<()> {
		let queue = {
			let mut rx = self.rx;
			let conns = Arc::clone(&self.conns);

			async move {
				while let Some(msg) = rx.recv().await {
					match msg.as_ref() {
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
						_ => (),
					}
				}
			}
		};

		let server = {
			let conns = Arc::clone(&self.conns);
			let current = self.current.clone();

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

							let current = current.clone();

							tokio::spawn(async move {
								if let Err(e) = writer_loop(rx, writer, current).await {
									log::info!("Write to stream error: {:?}", e);
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
	current: Current,
) -> io::Result<()> {
	let guard = current.chunk.read().await;
	if let Some(frames) = guard.as_ref() {
		for frame in frames {
			frame.write(&mut writer).await?;
		}
		writer.flush().await?;
	}

	drop(guard);

	while let Some(msg) = rx.recv().await {
		match msg.as_ref() {
			Message::Next(_) => {
				// writer.write(&id3).await?;
				// jk dont do anything - no metadata allowed in the middle of a stream
			}
			Message::Frames(frames) => {
				for frame in frames.iter() {
					frame.write(&mut writer).await?;
				}

				writer.flush().await?;
			}
		}
	}

	Ok(())
}

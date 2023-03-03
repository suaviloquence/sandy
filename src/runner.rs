use std::{
	io,
	sync::{Arc, Mutex},
	time::Duration,
};

use futures::future;
use tokio::{
	sync::{mpsc, RwLock},
	time::{self, Instant},
};

use crate::{
	output::{Message, Sender},
	playlist::{Playlist, SongMetadata},
	song::mp3::Frame,
};

#[derive(Debug, Clone, Copy)]
pub enum Control {
	SkipCurr,
}

pub type ControlSender = mpsc::Sender<Control>;

#[derive(Debug, Clone, Default)]
pub struct Current {
	pub song: Arc<RwLock<Option<SongMetadata>>>,
	pub chunk: Arc<RwLock<Option<Vec<Frame>>>>,
}

#[derive(Debug)]
pub struct Runner<const N: usize> {
	pub receiver: mpsc::Receiver<Control>,
	pub services: [Sender; N],
	pub playlist: Arc<Mutex<Playlist>>,
	pub current: Current,
}

async fn control_sleep(rx: &mut mpsc::Receiver<Control>, until: Instant) -> Result<(), Control> {
	while let Some(dur) = until.checked_duration_since(Instant::now()) {
		if let Ok(Some(msg)) = tokio::time::timeout(dur, rx.recv()).await {
			match msg {
				Control::SkipCurr => {
					// flush queue
					while let Ok(_) = rx.try_recv() {}
					// with this, we lose the ~3 second buffer from each client
					// time::sleep_until(until.into()).await;
					return Err(msg);
				}
			}
		}
	}
	Ok(())
}

impl<const N: usize> Runner<N> {
	// don't take &self so we can do a partial borrow
	async fn send(services: &[Sender; N], msg: Message) {
		let msg = Arc::new(msg);
		future::try_join_all(services.iter().map(|x| x.send(Arc::clone(&msg))))
			.await
			.expect("Channel closed");
	}

	async fn send_frame(&mut self, buffer: Vec<Frame>, duration: Duration) -> Result<(), Control> {
		let copy = buffer.clone();
		let until = Instant::now() + duration;

		let sender = Self::send(&self.services, Message::Frames(buffer));
		let writer = async {
			*self.current.chunk.write().await = Some(copy);
		};

		let controller_sleeper = control_sleep(&mut self.receiver, until);

		tokio::join!(sender, writer, controller_sleeper).2
	}

	pub async fn run_loop(mut self) -> io::Result<()> {
		while let Some(song) = {
			let mut guard = self.playlist.lock().expect("Error locking playlist mutex");
			let song = guard.pop_front();
			drop(guard);

			song
		} {
			log::info!(
				"Now playing: {} - {}",
				song.metadata.title,
				song.metadata.artist,
			);

			tokio::join!(
				async { *self.current.song.write().await = Some(song.metadata.clone()) },
				Self::send(&self.services, Message::Next(song.metadata.clone()))
			);

			const BUFFER_SIZE: usize = 128;

			let mut buffer = Vec::with_capacity(BUFFER_SIZE);
			let mut duration = 0.;

			// TODO: add skipping mid-song with recv_until and Instant

			for frame in song.frames() {
				duration += frame.header.duration();
				buffer.push(frame);
				if buffer.len() == BUFFER_SIZE {
					if let Err(Control::SkipCurr) = self
						.send_frame(buffer, Duration::from_secs_f64(duration))
						.await
					{
						buffer = Vec::with_capacity(0);
						break;
					}
					buffer = Vec::with_capacity(BUFFER_SIZE);
					duration = 0.;
				}
			}

			if !buffer.is_empty() {
				if let Err(Control::SkipCurr) = self
					.send_frame(buffer, Duration::from_secs_f64(duration))
					.await
				{
					// song is already skipped, do nothing
				}
			}

			// loop song at the end
			self.playlist
				.lock()
				.expect("Error locking playlist mutex to loop")
				.push_back(song);
		}

		Ok(())
	}
}

use std::{
	collections::VecDeque,
	io,
	sync::{Arc, Mutex},
	time::Duration,
};

use futures::future;

use crate::{
	output::{Message, Sender},
	song::{mp3::Mp3, Song},
};

#[derive(Debug)]
pub struct Runner<const N: usize> {
	pub services: [Sender; N],
	pub playlist: Arc<Mutex<VecDeque<Song<Mp3>>>>,
}

impl<const N: usize> Runner<N> {
	async fn send(&self, msg: Message) {
		let msg = Arc::new(msg);
		future::try_join_all(self.services.iter().map(|x| x.send(Arc::clone(&msg))))
			.await
			.expect("Channel closed");
	}

	pub async fn run_loop(self) -> io::Result<()> {
		while let Some(song) = {
			let mut guard = self.playlist.lock().expect("Error locking playlist mutex");
			let song = guard.pop_front();
			drop(guard);

			song
		} {
			self.send(Message::Next(song.metadata.clone())).await;

			const BUFFER_SIZE: usize = 128;

			let mut buffer = Vec::with_capacity(BUFFER_SIZE);
			let mut duration = (0., 0.);

			for frame in song.frames() {
				duration.1 += frame.header.duration();
				buffer.push(frame);
				if buffer.len() == BUFFER_SIZE {
					self.send(Message::Frames(buffer)).await;
					tokio::time::sleep(Duration::from_secs_f64(duration.0)).await;

					buffer = Vec::with_capacity(BUFFER_SIZE);
					duration = (duration.1, 0.);
				}
			}

			if !buffer.is_empty() {
				self.send(Message::Frames(buffer)).await;
				// tokio::time::sleep(Duration::from_secs_f64(duration.1)).await;
			}

			// loop song at the end
			self.playlist
				.lock()
				.expect("Error locking playlist mutex to loop")
				.push_back(song);
			log::info!("{}", line!());
		}

		Ok(())
	}
}

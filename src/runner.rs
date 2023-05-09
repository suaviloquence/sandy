use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    sync::{mpsc, RwLock},
    time::Instant,
};

use crate::{
    output::Message,
    playlist::{Playlist, SongMetadata},
    song::mp3::Frame,
};

#[derive(Debug, Clone, Copy)]
pub enum Control {
    SkipCurr,
}

pub type ControlSender = mpsc::Sender<Control>;

#[derive(Debug)]
pub struct Current {
    pub song: RwLock<Option<SongMetadata>>,
    pub chunk: RwLock<Option<Vec<Frame>>>,
    pub tail: RwLock<lighthouse::Receiver<Message>>,
}

impl Current {
    pub fn new(tail: lighthouse::Receiver<Message>) -> Self {
        Self {
            song: Default::default(),
            chunk: Default::default(),
            tail: RwLock::new(tail),
        }
    }
}

#[derive(Debug)]
pub struct Runner {
    pub receiver: mpsc::Receiver<Control>,
    pub sender: lighthouse::Sender<Message>,
    pub playlist: Arc<Mutex<Playlist>>,
    pub current: Arc<Current>,
}

async fn control_sleep(rx: &mut mpsc::Receiver<Control>, until: Instant) -> Result<(), Control> {
    while let Some(dur) = until.checked_duration_since(Instant::now()) {
        if let Ok(Some(msg)) = tokio::time::timeout(dur, rx.recv()).await {
            match msg {
                Control::SkipCurr => {
                    // flush queue
                    while rx.try_recv().is_ok() {}
                    // with this, we lose the ~3 second buffer from each client
                    // time::sleep_until(until.into()).await;
                    return Err(msg);
                }
            }
        }
    }
    Ok(())
}

async fn send(
    sx: &mut lighthouse::Sender<Message>,
    msg: Message,
    current: &Current,
) -> Result<(), lighthouse::SendError> {
    sx.send(msg)?;
    current.tail.write().await.try_recv().expect("Error advancing current");
    Ok(())
}

impl Runner {
    async fn send_frame(&mut self, buffer: Vec<Frame>, duration: Duration) -> Result<(), Control> {
        let until = Instant::now() + duration;

        send(
            &mut self.sender,
            Message::Frames(buffer.clone()),
            &self.current,
        )
        .await
        .expect("Error sending");

        let writer = async {
            *self.current.chunk.write().await = Some(buffer);
        };

        let controller_sleeper = control_sleep(&mut self.receiver, until);

        tokio::join!(writer, controller_sleeper).1
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

            send(
                &mut self.sender,
                Message::Next(song.metadata.clone()),
                &self.current,
            )
            .await
            .expect("Error sending");
            *self.current.song.write().await = Some(song.metadata.clone());
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

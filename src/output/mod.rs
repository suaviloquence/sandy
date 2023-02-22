use std::sync::Arc;

use tokio::sync::mpsc;

use crate::{playlist::SongMetadata, song::mp3::Frame};

pub mod http;
pub mod m3u;
pub mod tcp;

#[derive(Debug)]
pub enum Message {
	Next(SongMetadata),
	Frames(Vec<Frame>),
}

pub type Sender = mpsc::Sender<Arc<Message>>;
pub type Receiver = mpsc::Receiver<Arc<Message>>;

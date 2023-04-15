use crate::{playlist::SongMetadata, song::mp3::Frame};

pub mod http;
pub mod m3u;
pub mod tcp;

#[derive(Debug)]
pub enum Message {
    Next(SongMetadata),
    Frames(Vec<Frame>),
}

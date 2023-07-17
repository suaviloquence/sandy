use std::io::{self, Seek, Read};

use futures::AsyncWrite;

use crate::playlist::SongMetadata;

use self::mp3::Mp3;

pub mod mp3;
pub mod flac;

#[derive(Debug)]
pub struct Song<C = Mp3> {
    pub metadata: SongMetadata,
    pub data: Vec<u8>,
    pub duration: f64,
    pub codec: C,
}

pub trait Codec: Sized {
	const MIME_TYPE: &'static str;
    
    type Frame: Frame;
    type Iterator<'a>: Iterator<Item = Self::Frame> + 'a where Self: 'a;

    fn load(metadata: SongMetadata, data: impl Read + Seek) -> io::Result<Song<Self>>;
    fn frames(song: &Song<Self>) -> Self::Iterator<'_>;
}

pub trait Frame: std::fmt::Debug + Clone {
    type Future: core::future::Future<Output = io::Result<usize>>;
    fn into_bytes(self) -> io::Result<Vec<u8>>;
    fn write<W: AsyncWrite + Unpin + Send>(&self, w: W) -> Self::Future;
}
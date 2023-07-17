use std::io::{self, Read, Seek, Cursor};

use futures::{AsyncWrite, future::BoxFuture};

use crate::playlist::SongMetadata;

#[derive(Debug)]
pub struct Flac {

}

#[derive(Debug, Clone)]
pub struct Frame {}

#[derive(Debug)]
pub struct FrameIterator<R> {
    cursor: R,
}

impl <R: Read> Iterator for FrameIterator<R> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl super::Frame for Frame {
    type Future = BoxFuture<'static, io::Result<usize>>;

    fn into_bytes(self) -> io::Result<Vec<u8>> {
        todo!()
    }

    fn write<W: AsyncWrite + Unpin + Send>(&self, w: W) -> Self::Future {
        todo!()
    }
}

impl super::Codec for Flac {
    const MIME_TYPE: &'static str = "audio/flac";
    type Frame = Frame;
    type Iterator<'a> = FrameIterator<Cursor<&'a Vec<u8>>>;
    
    fn load(metadata: SongMetadata, data: impl Read + Seek) -> io::Result<super::Song<Self>> {
        todo!()
    }

    fn frames(song: &super::Song<Self>) -> Self::Iterator<'_> {
        todo!()
    }
}
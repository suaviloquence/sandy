use std::io;

use futures::Future;

use crate::playlist::SongMetadata;

pub mod fs;
pub mod youtube_dl;

#[derive(Debug)]
pub enum Source {
	File(std::fs::File),
	Buffer(io::Cursor<Vec<u8>>),
}

impl io::Read for Source {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		match self {
			Self::File(fp) => fp.read(buf),
			Self::Buffer(cur) => cur.read(buf),
		}
	}
}

impl io::Seek for Source {
	fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
		match self {
			Self::File(fp) => fp.seek(pos),
			Self::Buffer(cur) => cur.seek(pos),
		}
	}
}

pub trait Getter {
	type Error: std::error::Error;
	type Future: Future<Output = Result<Source, Self::Error>>;

	fn can_get(&self, _: &SongMetadata) -> Option<bool> {
		None
	}

	fn get(&self, song: &SongMetadata) -> Self::Future;
}

macro_rules! multi {
		($($getter: ident $(,)?)+) => {
			(
				|song| async {
					let mut src = None;

					$(
						if src.is_none() && $getter.can_get(&song).unwrap_or(true) {
							match $getter.get(&song).await {
								Ok(s) => src = Some(s),
								Err(e) => log::error!("{} error: {:?}", stringify!($getter), e),
							}
						}
					)+

					src.and_then(|source| match Song::load(song, source) {
						Ok(song) => Some(song),
						Err(e) => {
							log::error!("Error reading song: {:?}", e);
							None
						}
					})
				}
			)
		};
}

pub(crate) use multi;

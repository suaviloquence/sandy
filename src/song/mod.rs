use std::io::{self, Read};

use crate::playlist::SongMetadata;

use self::mp3::Mp3;

pub mod id3;
pub mod mp3;

#[derive(Debug)]
pub struct Song<C = Mp3> {
	pub metadata: SongMetadata,
	pub data: Vec<u8>,
	pub duration: f64,
	pub codec: C,
}

/// Helper trait to read a number (big/network endian) from a [`Read`]
trait ReadBe: Sized {
	fn read_be(read: impl Read) -> io::Result<Self>;
}

macro_rules! impl_read_be {
	($($t: ty)+) => {
		$(
			impl ReadBe for $t {
				#[inline]
				fn read_be(mut read: impl Read) -> io::Result<Self> {
					let mut buf = [0; (<$t>::BITS / 8) as usize];
					read.read_exact(&mut buf)?;
					Ok(<$t>::from_be_bytes(buf))
				}
			}
		)+
	};
}

impl_read_be!(u32 u16 u8);

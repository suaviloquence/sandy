use crate::playlist::SongMetadata;

use self::mp3::Mp3;

pub mod mp3;

#[derive(Debug)]
pub struct Song<C = Mp3> {
    pub metadata: SongMetadata,
    pub data: Vec<u8>,
    pub duration: f64,
    pub codec: C,
}

pub trait Codec {
	const MIME_TYPE: &'static str;
}
use std::{fmt::Write, io};

use crate::song::{mp3::Mp3, Song};

pub fn generate_m3u8(
	list: &[Song<Mp3>],
	writer: impl Fn(&mut String, &Song<Mp3>) -> std::fmt::Result,
) -> io::Result<String> {
	let mut m3u8 = String::from("#EXTM3U\r\n");
	for song in list {
		write!(
			&mut m3u8,
			"#EXTINF:{},{} - {}\r\n",
			song.duration.ceil() as u64,
			song.metadata.artist,
			song.metadata.title,
		)
		.expect("Error writing to string!");

		writer(&mut m3u8, song).expect("Error writing to string in writer.");
		m3u8.push_str("\r\n");
	}

	Ok(m3u8)
}

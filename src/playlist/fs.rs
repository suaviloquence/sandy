use std::{collections::VecDeque, io, path::Path};

use super::SongMetadata;

pub async fn glob(
	playlist: &mut VecDeque<SongMetadata>,
	dir: impl AsRef<Path>,
	mut ok: impl FnMut(&Path) -> bool,
) -> io::Result<()> {
	let dir = dir.as_ref();
	if !dir.is_dir() {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			"Not valid directory",
		));
	}

	let mut i = 0;

	for entry in dir.read_dir()? {
		let entry = entry?;
		if !entry.file_type()?.is_dir() {
			continue;
		}
		let artist = entry.file_name().to_string_lossy().into_owned();

		for entry in entry.path().read_dir()? {
			let entry = entry?;

			if !entry.file_type()?.is_file() {
				continue;
			}

			let path = entry.path();
			if ok(&path) {
				let title = path
					.file_stem()
					.ok_or_else(|| {
						io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name")
					})?
					.to_string_lossy()
					.into_owned();

				let song = SongMetadata {
					artist: artist.clone(),
					title,
					youtube_url: None,
				};

				// "random"
				if ((((i % 11) % 7) % 5) % 3) % 2 == 0 {
					playlist.push_back(song);
				} else {
					playlist.push_front(song);
				}

				i += 1;
			}
		}
	}

	Ok(())
}

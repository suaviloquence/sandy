use std::{fs, io, path::PathBuf};

use crate::playlist::Song;

#[derive(Debug)]
pub struct Fs {
	dir: PathBuf,
	ext: &'static str,
}

impl Fs {
	pub fn new(dir: impl Into<PathBuf>, ext: &'static str) -> Self {
		Self {
			dir: dir.into(),
			ext,
		}
	}

	pub fn path(&self, song: &Song) -> PathBuf {
		let mut dir = self.dir.join(&song.artist);

		dir.push(&song.title);
		dir.set_extension(self.ext);

		dir
	}
}

impl super::Getter for Fs {
	type Source = fs::File;
	type Error = io::Error;

	fn can_get(&self, song: &Song) -> Option<bool> {
		Some(self.path(song).exists())
	}

	fn get(&self, song: &Song) -> Result<Self::Source, Self::Error> {
		fs::File::options().read(true).open(self.path(song))
	}
}

use std::{
    ffi::OsStr,
    fs,
    future::{ready, Ready},
    io,
    path::PathBuf,
};

use crate::playlist::SongMetadata;

use super::Source;

#[derive(Debug, Clone, Copy)]
pub enum Ext {
    Mp3,
}

impl AsRef<str> for Ext {
    fn as_ref(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
        }
    }
}

impl AsRef<OsStr> for Ext {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        str::as_ref(self.as_ref())
    }
}

#[derive(Debug)]
pub struct Fs {
    dir: PathBuf,
    ext: Ext,
}

impl Fs {
    pub fn new(dir: impl Into<PathBuf>, ext: Ext) -> Self {
        Self {
            dir: dir.into(),
            ext,
        }
    }

    pub fn path(&self, song: &SongMetadata) -> PathBuf {
        let mut dir = self.dir.join(&song.artist);

        dir.push(&song.title);
        dir.set_extension(self.ext);

        dir
    }
}

impl super::Getter for Fs {
    type Error = io::Error;
    type Future = Ready<io::Result<Source>>;

    fn can_get(&self, song: &SongMetadata) -> Option<bool> {
        Some(self.path(song).exists())
    }

    fn get(&self, song: &SongMetadata) -> Self::Future {
        ready(
            fs::File::options()
                .read(true)
                .open(self.path(song))
                .map(Source::File),
        )
    }
}

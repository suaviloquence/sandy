use std::collections::VecDeque;

use crate::song::{mp3::Mp3, Song};

pub mod fs;
pub mod lastfm;

#[derive(Debug, Clone, PartialEq)]
pub struct SongMetadata {
    pub title: String,
    pub artist: String,
    pub youtube_url: Option<String>,
}

pub type Playlist = VecDeque<Song<Mp3>>;

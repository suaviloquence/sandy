pub mod lastfm;

#[derive(Debug, Clone, PartialEq)]
pub struct Song {
	pub title: String,
	pub artist: String,
	pub youtube_url: Option<String>,
}

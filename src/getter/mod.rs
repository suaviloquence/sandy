use crate::playlist::Song;

mod fs;
pub use fs::Fs;

// TODO make async
pub trait Getter {
	type Source: symphonia::core::io::MediaSource;
	type Error: std::error::Error;

	fn can_get(&self, _: &Song) -> Option<bool> {
		None
	}

	fn get(&self, song: &Song) -> Result<Self::Source, Self::Error>;
}

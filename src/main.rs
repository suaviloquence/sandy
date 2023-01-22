use std::env;

use getter::Getter;
use playlist::lastfm;

mod getter;
mod playlist;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let lastfm = lastfm::Client::new(env::var("SID").expect("neeed env var SID"));

	let songs = lastfm.scrape_recommendations().await?;

	let fs = getter::Fs::new("./media", "mp3");

	for song in songs {
		if let Some(true) = fs.can_get(&song) {
			let src = fs.get(&song)?;

			todo!()
		}
	}

	Ok(())
}

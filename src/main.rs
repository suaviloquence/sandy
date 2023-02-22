use std::{
	collections::VecDeque,
	env,
	io::{BufReader, Read},
	sync::Arc,
};

use futures::StreamExt;
use getter::Getter;
use output::http;
use playlist::{lastfm, SongMetadata};
use runner::Runner;
use song::Song;
use tokio::sync::mpsc;

mod getter;
mod output;
mod playlist;
mod runner;
mod song;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	env_logger::init();

	let mut playlist = VecDeque::new();
	playlist::fs::glob(&mut playlist, "./media", |x| {
		x.extension() == Some("mp3".as_ref())
	})
	.await?;

	// let mut lastfm = lastfm::Client::new(env::var("SID").expect("need env var SID"));

	// let songs = lastfm.scrape_recommendations().await?;

	let fs = Arc::new(getter::fs::Fs::new("./media", getter::fs::Ext::Mp3));
	let ytdl = getter::youtube_dl::YoutubeDl {
		executable: "/usr/bin/yt-dlp".into(),
		ffmpeg: "/usr/bin/ffmpeg".into(),
		fs: Some(Arc::clone(&fs)),
	};

	let multi = getter::multi!(fs, ytdl);

	let playlist = futures::stream::iter(playlist.into_iter().map(multi))
		.buffered(3)
		.filter_map(|x| async { x })
		.collect::<VecDeque<_>>()
		.await;

	let (tcp_sx, rx) = mpsc::channel(8);
	let tcp = output::tcp::Tcp::new(rx, &playlist[0].metadata);
	tokio::spawn(tcp.run_loop());

	let playlist = Arc::new(std::sync::Mutex::new(playlist));

	let (http_sx, rx) = mpsc::channel(8);
	let http = output::http::Server::new(Arc::clone(&playlist), rx);
	tokio::spawn(http.run_loop());

	Runner {
		playlist,
		services: [tcp_sx, http_sx],
	}
	.run_loop()
	.await?;

	Ok(())
}

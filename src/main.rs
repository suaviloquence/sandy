use std::{collections::VecDeque, env, sync::Arc};

use futures::StreamExt;
use getter::Getter;
use playlist::lastfm;
use runner::{Current, Runner};
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
	// playlist::fs::glob(&mut playlist, "./media", |x| {
	// 	x.extension() == Some("mp3".as_ref())
	// })
	// .await?;

	let mut lastfm = lastfm::Client::new(env::var("SID").expect("need env var SID"));

	lastfm.scrape_recommendations(&mut playlist).await?;

	let fs = Arc::new(getter::fs::Fs::new("./media", getter::fs::Ext::Mp3));
	let ytdl = getter::youtube_dl::YoutubeDl {
		executable: "/usr/bin/yt-dlp".into(),
		ffmpeg: "/usr/bin/ffmpeg".into(),
		fs: Some(Arc::clone(&fs)),
	};

	let multi = getter::multi!(fs, ytdl);

	let (control_sx, control_rx) = mpsc::channel(8);

	let playlist = futures::stream::iter(playlist.into_iter().map(multi))
		.buffered(3)
		.filter_map(|x| async { x })
		.collect::<VecDeque<_>>()
		.await;

	let current = Current::default();

	let (tcp_sx, rx) = mpsc::channel(8);
	let tcp = output::tcp::Tcp::new(rx, current.clone());
	tokio::spawn(tcp.run_loop());

	let playlist = Arc::new(std::sync::Mutex::new(playlist));

	let (http_sx, rx) = mpsc::channel(8);
	let http = output::http::Server::new(
		rx,
		Arc::clone(&playlist),
		current.clone(),
		control_sx.clone(),
	);
	tokio::spawn(http.run_loop());

	let runner = Runner {
		receiver: control_rx,
		services: [tcp_sx, http_sx],
		playlist,
		current,
	};

	runner.run_loop().await?;

	Ok(())
}

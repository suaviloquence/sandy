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
    
	let sender = lighthouse::Sender::new();

	let (control_sx, control_rx) = mpsc::channel(8);

	let playlist = futures::stream::iter(playlist.into_iter().map(multi))
		.buffered(3)
		.filter_map(|x| async { x })
		.collect::<VecDeque<_>>()
		.await;

	let current = Arc::new(Current::new(sender.subscribe()));

	let tcp = output::tcp::Tcp::new(Arc::clone(&current));
	tokio::spawn(tcp.run_loop());

	let playlist = Arc::new(std::sync::Mutex::new(playlist));

	let http = output::http::Server::new(
		sender.subscribe(),
		Arc::clone(&playlist),
		Arc::clone(&current),
		control_sx.clone(),
	);
	tokio::spawn(http.run_loop());

	let runner = Runner {
		receiver: control_rx,
		sender,
		playlist,
		current,
	};

	runner.run_loop().await?;

	Ok(())
}

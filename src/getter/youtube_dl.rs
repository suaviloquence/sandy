use std::{
	fmt,
	io::{self, Cursor},
	path::PathBuf,
	process::ExitStatus,
	sync::Arc,
};

use tokio::process::Command;

use crate::playlist::SongMetadata;

use super::{fs::Fs, Getter, Source};

#[derive(Debug, Clone)]
pub struct YoutubeDl {
	pub executable: PathBuf,
	pub ffmpeg: PathBuf,
	pub fs: Option<Arc<Fs>>,
}

#[derive(Debug)]
pub enum Error {
	Io(io::Error),
	Download,
	Transcode,
}

impl Error {
	#[inline]
	fn success(res: Result<ExitStatus, io::Error>, err: Error) -> Result<(), Self> {
		if res?.success() {
			Ok(())
		} else {
			Err(err)
		}
	}

	#[inline]
	async fn try_run(cmd: &mut Command, err: Error) -> Result<(), Self> {
		Self::success(cmd.status().await, err)
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Error::Io(e) => write!(f, "IO Error: {e}"),
			Error::Download => write!(f, "Download Error"),
			Error::Transcode => write!(f, "Transcode Error"),
		}
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Self {
		Self::Io(e)
	}
}

impl std::error::Error for Error {}

impl YoutubeDl {
	async fn get(self, song: SongMetadata) -> Result<Source, Error> {
		let mut cmd = Command::new(&self.executable);
		cmd.arg(song.youtube_url.as_ref().expect("youtube_url is None"))
			.arg("-f")
			.arg("bestaudio")
			.arg("-o");

		if let Some(fs) = self.fs.as_deref() {
			let path = fs.path(&song);

			let parent = path.parent().expect("No parent directory");
			if !parent.exists() {
				std::fs::create_dir_all(parent)?
			}

			let dl = path.with_extension("dl");
			cmd.arg(&dl);

			Error::try_run(&mut cmd, Error::Download).await?;

			Error::try_run(
				Command::new(&self.ffmpeg).arg("-i").arg(&dl).arg(&path),
				Error::Transcode,
			)
			.await?;

			std::fs::remove_file(dl)?;

			fs.get(&song).await.map_err(Error::from)
		} else {
			let out = cmd.arg("-").arg("-f").arg("bestaudio").output().await?;
			if !out.status.success() {
				return Err(Error::Download);
			}

			Ok(Source::Buffer(Cursor::new(out.stdout)))
		}
	}
}

impl Getter for YoutubeDl {
	type Error = Error;
	type Future = futures::future::BoxFuture<'static, Result<Source, Self::Error>>;

	fn get(&self, song: &SongMetadata) -> Self::Future {
		Box::pin(self.clone().get(song.clone()))
	}

	fn can_get(&self, song: &SongMetadata) -> Option<bool> {
		Some(song.youtube_url.is_some())
	}
}

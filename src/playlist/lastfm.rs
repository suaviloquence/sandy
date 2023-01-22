use std::sync::Arc;

use reqwest::cookie;
use scraper::{Html, Selector};

use super::Song;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), " v", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
/// last.fm scraping client
pub struct Client {
	http: reqwest::Client,
}

impl Client {
	/// Constructs a new `Client` with the given last.fm session ID.
	pub fn new(sid: impl AsRef<str>) -> Self {
		let cookies = cookie::Jar::default();
		cookies.add_cookie_str(
			&format!("sessionid={}", sid.as_ref()),
			&reqwest::Url::parse("https://www.last.fm").unwrap(),
		);
		Self {
			http: reqwest::Client::builder()
				.user_agent(USER_AGENT)
				.https_only(true)
				.cookie_store(true)
				.cookie_provider(Arc::new(cookies))
				.build()
				.expect("Error creating reqwest::Client in lastfm::Client::new()"),
		}
	}

	pub async fn scrape_recommendations(&self) -> Result<Vec<Song>, Box<dyn std::error::Error>> {
		let mut songs = Vec::with_capacity(30);

		// should be consts or lazy_statics
		let song_selector = Selector::parse(".recommended-tracks-item").unwrap();
		let title_selector = Selector::parse(r#"[itemprop="name"]"#).unwrap();
		let artist_selector = Selector::parse(r#"[itemprop="byArtist"]"#).unwrap();
		let link_selector = Selector::parse(r#".desktop-playlink"#).unwrap();

		for i in 1..=3 {
			let res = self
				.http
				.get("https://last.fm/music/+recommended/tracks/")
				.query(&[("page", i)])
				.send()
				.await?
				.text()
				.await?;

			let html = Html::parse_document(&res);

			println!(
				"{}",
				html.select(&Selector::parse("title").unwrap())
					.next()
					.unwrap()
					.text()
					.collect::<String>()
			);

			for song in html.select(&song_selector) {
				let title = song
					.select(&title_selector)
					.next()
					.expect("No title elem!")
					.text()
					.map(&str::trim)
					.collect();

				let artist = song
					.select(&artist_selector)
					.next()
					.expect("No artist elem!")
					.text()
					.map(&str::trim)
					.collect();

				let youtube_url = song
					.select(&link_selector)
					.next()
					.and_then(|elem| elem.value().attr("href"))
					.map(|s| s.trim().to_owned());

				songs.push(Song {
					title,
					artist,
					youtube_url,
				})
			}
		}

		Ok(songs)
	}
}

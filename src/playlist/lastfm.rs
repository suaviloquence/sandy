use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fmt::Write,
};

use hyper::{
    body::HttpBody, client::HttpConnector, header, http::uri::PathAndQuery, Body, HeaderMap,
    Request, Uri,
};
use hyper_tls::HttpsConnector;
use scraper::{Html, Selector};

use super::SongMetadata;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), " v", env!("CARGO_PKG_VERSION"));

// avoid allocating extra buffers
const URLS: [&str; 3] = [
    "https://www.last.fm/music/+recommended/tracks?page=1",
    "https://www.last.fm/music/+recommended/tracks?page=2",
    "https://www.last.fm/music/+recommended/tracks?page=3",
];

#[derive(Debug, Clone)]
/// last.fm scraping client
pub struct Client {
    http: hyper::Client<HttpsConnector<HttpConnector>>,
    // no key should be duplicated
    cookies: HashMap<Cow<'static, str>, String>,
}

impl Client {
    /// Constructs a new `Client` with the given last.fm session ID.
    pub fn new(sid: String) -> Self {
        Self {
            http: hyper::Client::builder().build(HttpsConnector::new()),
            cookies: HashMap::from([(Cow::Borrowed("sessionid"), sid)]),
        }
    }

    fn cookies_header(&self) -> String {
        let mut buf = String::new();

        let mut iter = self.cookies.iter();

        let mut cur = iter.next();

        while let Some((k, v)) = cur {
            let _ = write!(&mut buf, "{}={}", k, v);

            cur = iter.next();
            if cur.is_some() {
                buf.push_str("; ");
            }
        }

        buf
    }

    fn update_cookies(&mut self, headers: &HeaderMap) {
        for val in headers.get_all(header::SET_COOKIE) {
            let (k, v) = match val
                .to_str()
                .ok()
                .and_then(|val| val.split(';').next())
                .and_then(|val| val.split_once('='))
            {
                Some(t) => t,
                None => {
                    log::error!("Invalid set-cookie header {:?}", val);
                    continue;
                }
            };
            self.cookies.insert(Cow::Owned(k.to_owned()), v.to_owned());
        }
    }

    async fn login(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut uri = Uri::from_static("https://www.last.fm/login");

        loop {
            let req = Request::builder()
                .uri(&uri)
                .header(header::COOKIE, self.cookies_header())
                .body(Body::empty())?;

            let res = self.http.request(req).await?;

            self.update_cookies(res.headers());

            if res.status().is_redirection() {
                let tmp_uri = Uri::try_from(
                    res.headers()
                        .get(header::LOCATION)
                        .expect("No location for redirect.")
                        .to_str()?,
                )?;

                uri = Uri::builder()
                    .scheme(
                        tmp_uri
                            .scheme()
                            .cloned()
                            .unwrap_or(uri.scheme().unwrap().clone()),
                    )
                    .authority(
                        tmp_uri
                            .authority()
                            .cloned()
                            .unwrap_or(uri.authority().unwrap().clone()),
                    )
                    .path_and_query(
                        tmp_uri
                            .path_and_query()
                            .cloned()
                            .unwrap_or(PathAndQuery::from_static("")),
                    )
                    .build()?;
            } else if res.status().is_success() {
                break Ok(());
            }
        }
    }

    pub async fn scrape_recommendations(
        &mut self,
        playlist: &mut VecDeque<SongMetadata>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // should be consts or lazy_statics
        let song_selector = Selector::parse(".recommended-tracks-item").unwrap();
        let title_selector = Selector::parse(r#"[itemprop="name"]"#).unwrap();
        let artist_selector = Selector::parse(r#"[itemprop="byArtist"]"#).unwrap();
        let link_selector = Selector::parse(r#".desktop-playlink"#).unwrap();

        self.login().await?;

        for url in URLS {
            let req = Request::builder()
                .uri(Uri::from_static(url))
                .method("GET")
                .header(header::USER_AGENT, USER_AGENT)
                .header(header::COOKIE, self.cookies_header())
                .body(Body::empty())?;

            let mut res = self.http.request(req).await?;

            let mut buf = Vec::with_capacity(
                res.headers()
                    .get(header::CONTENT_LENGTH)
                    .and_then(|x| x.to_str().ok())
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(1 << 16),
            );

            while let Some(chunk) = res.body_mut().data().await {
                buf.extend(chunk?);
            }

            let text = String::from_utf8(buf)?;

            let html = Html::parse_document(&text);

            log::debug!(
                "Scraping page {} (url {})",
                html.select(&Selector::parse("title").unwrap())
                    .next()
                    .unwrap()
                    .text()
                    .collect::<String>(),
                url
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

                playlist.push_back(SongMetadata {
                    title,
                    artist,
                    youtube_url,
                })
            }
        }

        Ok(())
    }
}

//! Anonymous public Apple Search/Lookup metadata; no credential or preview transport.
use crate::{
    artwork,
    cache::Cache,
    config::Limits,
    model::{Page, Track},
};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;
const MAX_BODY: usize = 512 * 1024;
const MAX_RESULTS: usize = 200;
const PERSONAL_UNAVAILABLE: &str = "Personal library/playlists authorization is not implemented. A locally saved token has not been checked against Apple. Public search works without that token.";
struct State {
    cache: Cache<String, Page>,
    calls: VecDeque<Instant>,
    blocked_until: Option<Instant>,
}
pub struct Catalog {
    client: reqwest::Client,
    origin: String,
    state: Mutex<State>,
    capacity: AtomicUsize,
    entries: AtomicUsize,
}
impl Catalog {
    pub fn new() -> Result<Self> {
        Self::create("https://itunes.apple.com")
    }
    fn create(origin: &str) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!(
                    "Rustune/",
                    env!("CARGO_PKG_VERSION"),
                    " (public catalog metadata)"
                ))
                .build()
                .map_err(|_| anyhow::anyhow!("Could not initialize the public catalog client"))?,
            origin: origin.into(),
            state: Mutex::new(State {
                cache: Cache::new(8),
                calls: VecDeque::new(),
                blocked_until: None,
            }),
            capacity: AtomicUsize::new(8),
            entries: AtomicUsize::new(0),
        })
    }
    #[cfg(test)]
    fn with_origin(origin: &str) -> Self {
        Self::create(origin).unwrap()
    }
    pub fn cache_entries(&self) -> usize {
        self.entries.load(Ordering::Relaxed)
    }
    pub fn set_capacity(&self, capacity: usize) {
        let capacity = capacity.min(32);
        if self.capacity.swap(capacity, Ordering::Relaxed) != capacity
            && let Ok(mut state) = self.state.try_lock()
        {
            state.cache = Cache::new(capacity);
            self.entries.store(0, Ordering::Relaxed);
        }
    }
    pub async fn browse(
        &self,
        kind: &str,
        term: &str,
        next: Option<&str>,
        country: &str,
        limits: Limits,
    ) -> Result<Page> {
        ensure!(
            country.len() == 2 && country.bytes().all(|b| b.is_ascii_alphabetic()),
            "Catalog country must be a two-letter code"
        );
        ensure!(
            term.len() <= 512 && !term.chars().any(char::is_control),
            "Search text is invalid or exceeds 512 bytes"
        );
        ensure!(kind.len() <= 64, "Invalid public catalog category");
        let offset = match next {
            None => 0,
            Some(s) if !s.is_empty() && s.len() <= 3 && s.bytes().all(|b| b.is_ascii_digit()) => {
                s.parse::<usize>().unwrap_or(MAX_RESULTS)
            }
            _ => anyhow::bail!("Invalid public catalog page cursor"),
        };
        ensure!(
            offset < MAX_RESULTS,
            "Public catalog is limited to 200 results per query"
        );
        let size = limits.page_size.clamp(1, 50);
        let (path, entity, wanted, id) = match kind {
            "search:songs" => ("search", "song", "songs", None),
            "search:albums" => ("search", "album", "albums", None),
            "search:artists" => ("search", "musicArtist", "artists", None),
            _ if kind.starts_with("open:albums:") || kind.starts_with("open:artists:") => {
                let (prefix, id) = kind
                    .rsplit_once(':')
                    .ok_or_else(|| anyhow::anyhow!("Invalid catalog lookup"))?;
                ensure!(
                    matches!(prefix, "open:albums" | "open:artists"),
                    "Invalid catalog lookup"
                );
                ensure!(
                    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()),
                    "Invalid catalog ID"
                );
                if prefix == "open:albums" {
                    ("lookup", "song", "songs", Some(id))
                } else {
                    ("lookup", "album", "albums", Some(id))
                }
            }
            "library" | "albums" | "artists" | "playlists" | "recent" | "search:playlists" => {
                anyhow::bail!(PERSONAL_UNAVAILABLE)
            }
            _ => anyhow::bail!("Unsupported public catalog category"),
        };
        if path == "search" {
            ensure!(
                !term.trim().is_empty(),
                "Enter a song, album or artist to search the public Apple catalog"
            );
        }
        self.set_capacity(limits.cache_entries);
        let key = format!(
            "{}\0{kind}\0{term}\0{offset}\0{size}",
            country.to_ascii_uppercase()
        );
        let mut state = self.state.lock().await;
        if let Some(page) = state.cache.get(&key) {
            return Ok(page.clone());
        }
        let now = Instant::now();
        if let Some(until) = state.blocked_until.filter(|until| *until > now) {
            anyhow::bail!(
                "Apple catalog rate limit; retry in {} seconds",
                until.duration_since(now).as_secs() + 1
            );
        }
        while state
            .calls
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            state.calls.pop_front();
        }
        ensure!(
            state.calls.len() < 20,
            "Apple catalog request limit reached; retry in a minute"
        );
        state.calls.push_back(now);
        let requested = (offset + size + if id.is_some() { 2 } else { 1 }).min(MAX_RESULTS);
        let mut params = vec![
            ("media", "music".to_owned()),
            ("entity", entity.into()),
            ("country", country.to_ascii_uppercase()),
            ("limit", requested.to_string()),
        ];
        if let Some(id) = id {
            params.push(("id", id.into()));
        } else {
            params.push(("term", term.trim().into()));
        }
        let mut url = reqwest::Url::parse(&format!("{}/{path}", self.origin))
            .map_err(|_| anyhow::anyhow!("Invalid catalog endpoint"))?;
        url.query_pairs_mut()
            .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())));
        let mut response = self.client.get(url).send().await.map_err(|_| {
            anyhow::anyhow!(
                "Public Apple catalog could not be reached; check the network and retry"
            )
        })?;
        if response.status().as_u16() == 429 {
            let delay = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.parse::<u64>().ok().or_else(|| {
                        httpdate::parse_http_date(v).ok().and_then(|date| {
                            date.duration_since(SystemTime::now())
                                .ok()
                                .map(|d| d.as_secs())
                        })
                    })
                })
                .unwrap_or(60)
                .clamp(1, 3600);
            state.blocked_until = Some(Instant::now() + Duration::from_secs(delay));
            anyhow::bail!("Apple catalog rate limit; retry in {delay} seconds");
        }
        ensure!(
            response.status().is_success(),
            "Apple catalog returned HTTP {}",
            response.status().as_u16()
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_BODY as u64),
            "Public catalog response exceeds 512 KiB"
        );
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("Public catalog response was interrupted"))?
        {
            ensure!(
                body.len().saturating_add(chunk.len()) <= MAX_BODY,
                "Public catalog response exceeds 512 KiB"
            );
            body.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|_| anyhow::anyhow!("Apple catalog returned invalid JSON"))?;
        let records = value
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("Apple catalog response has no results array"))?;
        ensure!(
            records.len() <= MAX_RESULTS + 1,
            "Apple catalog returned too many records"
        );
        let items = records
            .iter()
            .filter_map(|record| map_record(record, wanted))
            .take(MAX_RESULTS)
            .collect::<Vec<_>>();
        let more = items.len() > offset + size;
        let page = Page {
            items: items.into_iter().skip(offset).take(size).collect(),
            next: more.then(|| (offset + size).to_string()),
        };
        let capacity = self.capacity.load(Ordering::Relaxed);
        if state.cache.capacity != capacity {
            state.cache = Cache::new(capacity);
        }
        state.cache.insert(key, page.clone());
        self.entries
            .store(state.cache.entries.len(), Ordering::Relaxed);
        Ok(page)
    }
}
fn clean(value: Option<&Value>, max: usize) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}
fn map_record(record: &Value, wanted: &str) -> Option<Track> {
    let (id, title) = match wanted {
        "songs" if record["kind"] == "song" => (
            record["trackId"].as_u64()?,
            clean(record.get("trackName"), 512),
        ),
        "albums"
            if record["wrapperType"] == "collection" && record["collectionType"] == "Album" =>
        {
            (
                record["collectionId"].as_u64()?,
                clean(record.get("collectionName"), 512),
            )
        }
        "artists" if record["wrapperType"] == "artist" => (
            record["artistId"].as_u64()?,
            clean(record.get("artistName"), 256),
        ),
        _ => return None,
    };
    if id == 0 || title.is_empty() {
        return None;
    }
    let artwork = record
        .get("artworkUrl100")
        .and_then(Value::as_str)
        .filter(|url| url.len() <= 2048 && artwork::artwork_url(url, 100).is_ok())
        .map(|url| url.replace("100x100bb", "{w}x{h}bb"));
    let url = record
        .get(if wanted == "songs" {
            "trackViewUrl"
        } else if wanted == "albums" {
            "collectionViewUrl"
        } else {
            "artistLinkUrl"
        })
        .and_then(Value::as_str)
        .filter(|url| url.len() <= 2048)
        .and_then(|url| reqwest::Url::parse(url).ok())
        .filter(|url| {
            url.scheme() == "https"
                && matches!(url.host_str(), Some("music.apple.com" | "itunes.apple.com"))
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|url| url.to_string());
    Some(Track {
        id: id.to_string(),
        kind: wanted.into(),
        title,
        artist: clean(record.get("artistName"), 256),
        album: clean(record.get("collectionName"), 512),
        duration_ms: record
            .get("trackTimeMillis")
            .and_then(Value::as_u64)
            .filter(|n| *n <= 86400000)
            .unwrap_or(0),
        artwork,
        url,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn server(
        body: String,
        status: &str,
        extra: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let mut len = 0;
            while !request[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                len += socket.read(&mut request[len..]).await.unwrap();
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request[..len].to_vec()).unwrap()
        });
        (origin, task)
    }
    #[tokio::test]
    async fn anonymous_search_maps_metadata_caches_and_never_uses_previews() {
        let items=(11..14).map(|id|json!({"wrapperType":"track","kind":"song","trackId":id,"trackName":"Our song","artistName":"Our artist","collectionName":"Our album","trackTimeMillis":123456,"artworkUrl100":"https://is1-ssl.mzstatic.com/image/100x100bb.jpg","trackViewUrl":"https://music.apple.com/pl/album/our-song/1?i=11","previewUrl":"https://preview.test/must-not-be-used"})).collect::<Vec<_>>();
        let (origin, task) = server(json!({"results":items}).to_string(), "200 OK", "").await;
        let catalog = Catalog::with_origin(&origin);
        let mut limits = crate::config::PerformanceMode::Balanced.limits();
        limits.page_size = 2;
        let page = catalog
            .browse("search:songs", "A & B", None, "PL", limits)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].id, "11");
        assert_eq!(page.items[0].duration_ms, 123456);
        assert_eq!(page.next.as_deref(), Some("2"));
        let request = task.await.unwrap().to_lowercase();
        assert!(
            request.contains("term=a+%26+b")
                && request.contains("entity=song")
                && request.contains("country=pl")
        );
        for header in ["authorization:", "music-user-token:", "cookie:"] {
            assert!(!request.contains(header));
        }
        assert!(
            !serde_json::to_string(&page)
                .unwrap()
                .contains("preview.test")
        );
        assert_eq!(
            catalog
                .browse("search:songs", "A & B", None, "PL", limits)
                .await
                .unwrap()
                .items[0]
                .id,
            "11",
            "cache must work after fixture server has closed"
        );
    }
    #[tokio::test]
    async fn album_lookup_filters_container_and_rejects_external_cursors() {
        let (origin,task)=server(json!({"results":[{"wrapperType":"collection","collectionType":"Album","collectionId":123,"collectionName":"Our album"},{"wrapperType":"track","kind":"song","trackId":11,"trackName":"Our song"}]}).to_string(),"200 OK","").await;
        let catalog = Catalog::with_origin(&origin);
        let limits = crate::config::PerformanceMode::Balanced.limits();
        let page = catalog
            .browse("open:albums:123", "", None, "PL", limits)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].kind, "songs");
        let request = task.await.unwrap();
        assert!(request.starts_with("GET /lookup?") && request.contains("id=123"));
        assert!(
            catalog
                .browse(
                    "search:songs",
                    "test",
                    Some("https://evil.test"),
                    "PL",
                    limits
                )
                .await
                .is_err()
        );
        assert!(
            catalog
                .browse("library", "", None, "PL", limits)
                .await
                .unwrap_err()
                .to_string()
                .contains("Personal")
        );
    }
    #[tokio::test]
    async fn retry_after_prevents_another_network_request() {
        let (origin, task) =
            server("{}".into(), "429 Too Many Requests", "Retry-After: 60\r\n").await;
        let catalog = Catalog::with_origin(&origin);
        let limits = crate::config::PerformanceMode::Balanced.limits();
        assert!(
            catalog
                .browse("search:songs", "one", None, "PL", limits)
                .await
                .unwrap_err()
                .to_string()
                .contains("retry")
        );
        task.await.unwrap();
        assert!(
            catalog
                .browse("search:songs", "two", None, "PL", limits)
                .await
                .unwrap_err()
                .to_string()
                .contains("retry")
        );
    }
    #[tokio::test]
    async fn oversized_headers_fail_before_body_read_and_errors_hide_query_data() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut input = [0u8; 4096];
            assert!(socket.read(&mut input).await.unwrap() > 0);
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        MAX_BODY + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let limits = crate::config::PerformanceMode::Balanced.limits();
        let error = Catalog::with_origin(&origin)
            .browse("search:songs", "private query", None, "PL", limits)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("512 KiB") && !error.contains("private query"));
        task.await.unwrap();
        let (origin, task) = server("personal backend details".into(), "403 Forbidden", "").await;
        let error = Catalog::with_origin(&origin)
            .browse("search:songs", "private query", None, "PL", limits)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("HTTP 403") && !error.contains("private") && !error.contains("personal")
        );
        task.await.unwrap();
    }
    #[test]
    fn metadata_controls_and_external_links_are_not_forwarded() {
        let track=map_record(&json!({"kind":"song","trackId":1,"trackName":"A\u{1b}[31m song\n","trackViewUrl":"https://untrusted.test/","artworkUrl100":"http://is1-ssl.mzstatic.com/test.jpg"}),"songs").unwrap();
        assert!(!track.title.chars().any(char::is_control));
        assert!(track.url.is_none() && track.artwork.is_none());
    }
}

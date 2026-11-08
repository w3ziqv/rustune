use super::{LoadError, Lyrics, LyricsQuery};
use serde::Deserialize;
use std::time::{Duration, Instant, SystemTime};

pub struct Provider {
    client: reqwest::Client,
    base: reqwest::Url,
    next_request: Instant,
    retry_until: Option<Instant>,
    suspended: bool,
}
impl Provider {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!(
                    "Rustune/",
                    env!("CARGO_PKG_VERSION"),
                    " (https://github.com/w3ziqv/rustune)"
                ))
                .timeout(Duration::from_secs(8))
                .build()?,
            base: reqwest::Url::parse("https://lrclib.net/")?,
            next_request: Instant::now(),
            retry_until: None,
            suspended: false,
        })
    }
    async fn request(
        &mut self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<Option<Vec<u8>>, LoadError> {
        if self.suspended || self.retry_until.is_some_and(|t| Instant::now() < t) {
            return Err(LoadError::RateLimited);
        }
        if let Some(delay) = self.next_request.checked_duration_since(Instant::now()) {
            tokio::time::sleep(delay).await;
        }
        let mut url = self.base.join(path).map_err(|_| LoadError::Invalid)?;
        url.query_pairs_mut()
            .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())));
        let response = self.client.get(url).send().await;
        self.next_request = Instant::now() + Duration::from_millis(250);
        let mut response = response.map_err(|_| LoadError::Network)?;
        if matches!(response.status().as_u16(), 429 | 503) {
            let delay = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| {
                    h.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                        httpdate::parse_http_date(h)
                            .ok()
                            .map(|d| d.duration_since(SystemTime::now()).unwrap_or_default())
                    })
                })
                .unwrap_or(Duration::from_secs(60));
            // A positive jitter prevents synchronized retries, and never shortens Retry-After.
            let delay = delay.saturating_add(Duration::from_millis(fastrand::u64(1..=250)));
            self.retry_until = Instant::now().checked_add(delay);
            self.suspended = self.retry_until.is_none();
            return Err(LoadError::RateLimited);
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(LoadError::Network);
        }
        const MAX_BODY: usize = 512 * 1024;
        if response.content_length().unwrap_or(0) > MAX_BODY as u64 {
            return Err(LoadError::TooLarge);
        }
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"))
        {
            return Err(LoadError::Invalid);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| LoadError::Network)? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(LoadError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }
    pub async fn lookup(&mut self, query: &LyricsQuery) -> Result<Option<Lyrics>, LoadError> {
        if query.title.trim().is_empty()
            || query.artist.trim().is_empty()
            || [&query.title, &query.artist, &query.album]
                .iter()
                .any(|s| s.len() > 2048)
        {
            return Err(LoadError::Invalid);
        }
        let mut params = vec![
            ("track_name", query.title.clone()),
            ("artist_name", query.artist.clone()),
        ];
        if !query.album.is_empty() {
            params.push(("album_name", query.album.clone()));
        }
        if (1..=3600).contains(&query.duration_secs) {
            params.push(("duration", query.duration_secs.to_string()));
        }
        if let Some(body) = self.request("/api/get", &params).await? {
            let record: Record = serde_json::from_slice(&body).map_err(|_| LoadError::Invalid)?;
            if record.matches(query)
                && let Some(lyrics) = record.lyrics()
            {
                return Ok(Some(lyrics));
            }
        }
        let params = vec![
            ("track_name", query.title.clone()),
            ("artist_name", query.artist.clone()),
        ];
        let Some(body) = self.request("/api/search", &params).await? else {
            return Ok(None);
        };
        let mut records: Vec<Record> =
            serde_json::from_slice(&body).map_err(|_| LoadError::Invalid)?;
        if records.len() > 20 {
            return Err(LoadError::TooLarge);
        }
        records.retain(|r| r.matches(query));
        records.sort_by_key(|r| normalize(&r.album_name) != normalize(&query.album));
        Ok(records.into_iter().find_map(|r| r.lyrics()))
    }
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: u64,
    track_name: String,
    artist_name: String,
    #[serde(default)]
    album_name: String,
    duration: f64,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}
impl Record {
    fn matches(&self, q: &LyricsQuery) -> bool {
        normalize(&self.track_name) == normalize(&q.title)
            && normalize(&self.artist_name) == normalize(&q.artist)
            && self.duration.is_finite()
            && (q.duration_secs == 0 || (self.duration - q.duration_secs as f64).abs() <= 3.0)
    }
    fn lyrics(self) -> Option<Lyrics> {
        let mut lyrics = Lyrics::parse(
            self.plain_lyrics.as_deref(),
            self.synced_lyrics.as_deref(),
            self.instrumental,
        )
        .ok()?;
        lyrics.source_url = format!("https://lrclib.net/api/get/{}", self.id);
        Some(lyrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn server(responses: Vec<String>) -> (Provider, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut data = Vec::new();
                while !data.ends_with(b"\r\n\r\n") {
                    let mut b = [0; 1024];
                    let n = stream.read(&mut b).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&b[..n]);
                }
                requests.push(String::from_utf8(data).unwrap());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        let mut p = Provider::new().unwrap();
        p.client = reqwest::Client::builder()
            .user_agent("Rustune/test")
            .build()
            .unwrap();
        p.base = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        (p, task)
    }
    fn reply(status: &str, body: &str, headers: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        )
    }
    fn query() -> LyricsQuery {
        LyricsQuery {
            title: "Our Song & More".into(),
            artist: "Our Artist".into(),
            album: "Our Album".into(),
            duration_secs: 100,
            generation: 0,
        }
    }
    #[tokio::test]
    async fn exact_lookup_is_identified_encoded_and_has_no_authorization() {
        let body = r#"{"id":1,"trackName":"Our Song & More","artistName":"Our Artist","albumName":"Our Album","duration":100,"instrumental":false,"plainLyrics":"Our own words","syncedLyrics":"[00:01.00]Our own words"}"#;
        let (mut p, server) = server(vec![reply("200 OK", body, "")]).await;
        let lyrics = p.lookup(&query()).await.unwrap().unwrap();
        assert!(lyrics.synced);
        assert_eq!(lyrics.source_url, "https://lrclib.net/api/get/1");
        let req = server.await.unwrap().remove(0);
        assert!(req.contains("track_name=Our+Song+%26+More"));
        assert!(req.to_lowercase().contains("user-agent: rustune/"));
        assert!(!req.to_lowercase().contains("authorization:"));
    }
    #[tokio::test]
    async fn missing_exact_match_uses_only_matching_search_candidate() {
        let body = r#"[{"id":2,"trackName":"Wrong Song","artistName":"Our Artist","albumName":"Our Album","duration":100,"plainLyrics":"Wrong"},{"id":3,"trackName":"Our Song & More","artistName":"Our Artist","albumName":"Our Album","duration":101,"plainLyrics":"Our own words"}]"#;
        let (mut p, server) = server(vec![
            reply("404 Not Found", "{}", ""),
            reply("200 OK", body, ""),
        ])
        .await;
        let lyrics = p.lookup(&query()).await.unwrap().unwrap();
        assert_eq!(lyrics.source_url, "https://lrclib.net/api/get/3");
        assert_eq!(server.await.unwrap().len(), 2);
    }
    #[tokio::test]
    async fn rate_limit_honors_retry_after_without_another_request() {
        let (mut p, server) = server(vec![reply(
            "429 Too Many Requests",
            "{}",
            "Retry-After: 120\r\n",
        )])
        .await;
        assert!(matches!(
            p.lookup(&query()).await,
            Err(LoadError::RateLimited)
        ));
        assert!(matches!(
            p.lookup(&query()).await,
            Err(LoadError::RateLimited)
        ));
        assert!(p.retry_until.unwrap().duration_since(Instant::now()) > Duration::from_secs(110));
        assert_eq!(server.await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn oversized_response_is_rejected_before_body_allocation() {
        let (mut p,server)=server(vec!["HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n".into()]).await;
        assert!(matches!(p.lookup(&query()).await, Err(LoadError::TooLarge)));
        server.await.unwrap();
    }
}

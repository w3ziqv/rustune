//! Fixed-origin Apple Music catalog and private-library search.
mod playlist_write;
use super::reads::{Budget, Reads};
use crate::{
    artwork,
    auth::{TokenStamp, token_publication, token_stamp},
    cache::Cache,
    config::{Limits, Paths},
    model::{Page, Track},
};
use anyhow::{Result, anyhow, ensure};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderValue},
};
use rustune_core::public_apple::{application_token, read_private_secret};
use serde_json::Value;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;
const MAX_BODY: usize = 2 * 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Account {
    generation: u64,
}
struct State {
    cache: Cache<String, (Instant, Page)>,
    stamp: Option<TokenStamp>,
    region: Option<(Instant, Option<String>)>,
    budget: Budget,
    playlist_epoch: u64,
}

#[derive(Clone)]
pub struct AppleCatalog {
    capacity: Arc<AtomicUsize>,
    entries: Arc<AtomicUsize>,
    client: Client,
    paths: Paths,
    origin: String,
    state: Arc<Mutex<State>>,
    pages: Arc<Reads<String, Page>>,
    regions: Arc<Reads<Account, Option<String>>>,
    slots: Arc<Semaphore>,
    writes: Arc<tokio::sync::Mutex<()>>,
}

struct MusicPage {
    key: String,
    kind: String,
    term: String,
    offset: usize,
    size: usize,
    max_results: usize,
    page_size: usize,
    reuse_playlist: bool,
    playlist_epoch: Option<u64>,
    account: Option<Account>,
    storefront: String,
}
impl AppleCatalog {
    pub fn new(paths: Paths) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Rustune/0.1 Apple Music search")
            .build()
            .map_err(|_| anyhow!("Cannot initialize Apple Music search"))?;
        Ok(Self {
            capacity: Arc::new(AtomicUsize::new(8)),
            entries: Arc::new(AtomicUsize::new(0)),
            client,
            paths,
            origin: "https://api.music.apple.com".into(),
            state: Arc::new(Mutex::new(State {
                cache: Cache::new(8),
                stamp: None,
                region: None,
                budget: Budget::default(),
                playlist_epoch: 0,
            })),
            pages: Arc::new(Reads::new()),
            regions: Arc::new(Reads::new()),
            slots: Arc::new(Semaphore::new(2)),
            writes: Arc::new(tokio::sync::Mutex::new(())),
        })
    }
    #[cfg(test)]
    pub(super) fn with_origin(paths: Paths, origin: String) -> Result<Self> {
        let mut catalog = Self::new(paths)?;
        catalog.origin = origin;
        Ok(catalog)
    }
    pub fn cache_entries(&self) -> usize {
        self.entries.load(Ordering::Relaxed)
    }
    pub fn set_capacity(&self, capacity: usize) {
        let capacity = capacity.min(32);
        if self.capacity.swap(capacity, Ordering::Relaxed) != capacity
            && let Ok(mut state) = self.state.try_lock()
        {
            self.clear_cache(&mut state);
        }
    }
    async fn headers(&self, private: bool) -> Result<HeaderMap> {
        let token = application_token(&self.client, &self.paths).await?;
        let text = Zeroizing::new(format!("Bearer {}", token.trim()));
        let mut header = HeaderValue::from_str(&text)
            .map_err(|_| anyhow!("Invalid public Apple authorization"))?;
        header.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", header);
        headers.insert(
            "origin",
            HeaderValue::from_static("https://music.apple.com"),
        );
        headers.insert(
            "referer",
            HeaderValue::from_static("https://music.apple.com/"),
        );
        headers.insert("accept", HeaderValue::from_static("application/json"));
        if private {
            let token = read_private_secret(&self.paths.config.join("music-user-token"))?;
            let mut header = HeaderValue::from_str(token.trim())
                .map_err(|_| anyhow!("Invalid Music User Token format"))?;
            header.set_sensitive(true);
            headers.insert("music-user-token", header);
        }
        Ok(headers)
    }
    pub(super) fn account_generation(&self) -> Option<u64> {
        self.account().map(|account| account.generation)
    }

    pub(super) fn publication_lock(&self) -> Result<nix::fcntl::Flock<std::fs::File>> {
        token_publication(&self.paths)
    }

    fn account(&self) -> Option<Account> {
        self.refresh_account(&mut self.state.lock().expect("Apple catalog state"))
    }

    fn refresh_account(&self, state: &mut State) -> Option<Account> {
        let current = token_stamp(&self.paths);
        if current != state.stamp {
            state.stamp = current;
            state.region = None;
            self.clear_cache(state);
        }
        current.map(|stamp| Account {
            generation: stamp.generation(),
        })
    }

    fn with_account<T>(
        &self,
        expected: Option<Account>,
        action: impl FnOnce(&mut State) -> T,
    ) -> Result<T> {
        let _publication = expected.map(|_| self.publication_lock()).transpose()?;
        let mut state = self.state.lock().expect("Apple catalog state");
        if let Some(expected) = expected {
            ensure!(
                self.refresh_account(&mut state) == Some(expected),
                "Account credentials changed; restart the library search"
            );
        }
        Ok(action(&mut state))
    }

    fn clear_cache(&self, state: &mut State) {
        let capacity = self.capacity.load(Ordering::Relaxed);
        state.cache = Cache::new(capacity);
        state.playlist_epoch = state.playlist_epoch.wrapping_add(1);
        self.entries.store(0, Ordering::Relaxed);
    }

    fn check_account(&self, expected: Option<Account>) -> Result<()> {
        if let Some(expected) = expected {
            ensure!(
                self.account() == Some(expected),
                "Account credentials changed; restart the library search"
            );
        }
        Ok(())
    }

    async fn account_region(&self, account: Account) -> Result<Option<String>> {
        let catalog = self.clone();
        let region = self
            .regions
            .read(account, async move {
                if let Some(region) = catalog.with_account(Some(account), |state| {
                    state
                        .region
                        .as_ref()
                        .filter(|(checked, _)| checked.elapsed() <= Duration::from_secs(300))
                        .map(|(_, region)| region.clone())
                })? {
                    return Ok(region);
                }
                let response = catalog.get("me/storefront", &[], Some(account)).await;
                let region = response.ok().and_then(|value| {
                    let region = value["data"][0]["id"].as_str()?;
                    (region.len() == 2 && region.bytes().all(|b| b.is_ascii_alphabetic()))
                        .then(|| region.to_ascii_lowercase())
                });
                catalog.with_account(Some(account), |state| {
                    state.region = Some((Instant::now(), region.clone()));
                })?;
                Ok(region)
            })
            .await?;
        self.check_account(Some(account))?;
        Ok(region)
    }

    async fn get(
        &self,
        path: &str,
        params: &[(&str, String)],
        account: Option<Account>,
    ) -> Result<Value> {
        let _slot = self
            .slots
            .acquire()
            .await
            .expect("open Apple catalog slots");
        self.check_account(account)?;
        let headers = self.headers(account.is_some()).await?;
        self.check_account(account)?;
        let mut url = Url::parse(&format!("{}/v1/{path}", self.origin))
            .map_err(|_| anyhow!("Invalid fixed Apple search origin"))?;
        url.query_pairs_mut()
            .extend_pairs(params.iter().map(|(k, v)| (*k, v.as_str())));
        self.state
            .lock()
            .expect("Apple catalog state")
            .budget
            .reserve("Apple Music search")?;
        let mut response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|_| anyhow!("Apple Music search could not be reached"))?;
        if response.status().as_u16() == 429 {
            let seconds = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60)
                .clamp(1, 3600);
            self.state
                .lock()
                .expect("Apple catalog state")
                .budget
                .block(seconds);
            return Err(anyhow!(
                "Apple Music search rate limit; retry in {seconds} seconds"
            ));
        }
        if account.is_some() {
            ensure!(
                response.status().is_success(),
                "Personal library/account access rejected (HTTP {}); check your Music User Token",
                response.status().as_u16()
            );
        } else {
            ensure!(
                response.status().is_success(),
                "Apple Music catalog returned HTTP {}",
                response.status().as_u16()
            );
        }
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_BODY as u64),
            "Apple Music response exceeds 2 MiB"
        );
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Apple Music search response interrupted"))?
        {
            ensure!(
                body.len() + chunk.len() <= MAX_BODY,
                "Apple Music response exceeds 2 MiB"
            );
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| anyhow!("Apple Music returned invalid search metadata"))
    }
    #[cfg(test)]
    pub async fn browse(
        &self,
        kind: &str,
        term: &str,
        next: Option<&str>,
        country: &str,
        limits: Limits,
    ) -> Result<Page> {
        self.browse_with_playlist_pages(kind, term, next, country, limits, false)
            .await
    }

    pub(super) async fn browse_with_playlist_pages(
        &self,
        kind: &str,
        term: &str,
        next: Option<&str>,
        country: &str,
        limits: Limits,
        reuse_playlist: bool,
    ) -> Result<Page> {
        self.set_capacity(limits.cache_entries);
        tokio::time::timeout(
            Duration::from_secs(15),
            self.browse_inner(kind, term, next, country, limits, reuse_playlist),
        )
        .await
        .map_err(|_| anyhow!("Apple Music search timed out; retry"))?
    }
    async fn browse_inner(
        &self,
        kind: &str,
        term: &str,
        next: Option<&str>,
        country: &str,
        limits: Limits,
        reuse_playlist: bool,
    ) -> Result<Page> {
        ensure!(
            country.len() == 2 && country.bytes().all(|b| b.is_ascii_alphabetic()),
            "Invalid catalog country"
        );
        ensure!(
            term.len() <= 512 && !term.chars().any(char::is_control),
            "Invalid search text"
        );
        let private = super::playlist::private_kind(kind);
        let account = self.account();
        let (pinned_region, raw_offset) = match next {
            None => (None, "0"),
            Some(cursor) => {
                let (region, offset) = cursor
                    .split_once(':')
                    .ok_or_else(|| anyhow!("Invalid Apple Music page cursor"))?;
                ensure!(
                    region.len() == 2 && region.bytes().all(|b| b.is_ascii_alphabetic()),
                    "Invalid Apple Music page region"
                );
                if private {
                    let (generation, offset) = offset.split_once(':').ok_or_else(|| {
                        anyhow!("Private page expired; restart the library search")
                    })?;
                    ensure!(
                        u64::from_str_radix(generation, 16)
                            .ok()
                            .zip(account)
                            .is_some_and(|(g, a)| g == a.generation),
                        "Private page expired; restart the library search"
                    );
                    (Some(region), offset)
                } else {
                    (Some(region), offset)
                }
            }
        };
        ensure!(
            !raw_offset.is_empty()
                && raw_offset.len() <= 5
                && raw_offset.bytes().all(|b| b.is_ascii_digit()),
            "Invalid Apple Music page cursor"
        );
        let offset = raw_offset
            .parse::<usize>()
            .unwrap_or(crate::queue::MAX_QUEUE);
        let max_results = if kind.starts_with("search:") {
            200
        } else {
            crate::queue::MAX_QUEUE
        };
        ensure!(offset < max_results, "Catalog offset exceeds limit");
        ensure!(
            !matches!(kind, "recent" | "search:playlists"),
            "Recent history and playlist search are not implemented; browse Playlists"
        );
        let page_size = if kind.starts_with("open:library-playlists:") {
            25
        } else {
            limits.page_size.clamp(1, 25)
        };
        let size = page_size.min(max_results - offset);
        let region = match account.filter(|_| private || pinned_region.is_none()) {
            Some(account) => match self.account_region(account).await {
                Ok(region) => region,
                Err(error) if private => return Err(error),
                Err(_) => None,
            },
            None => None,
        };
        if private {
            ensure!(
                region.is_some(),
                "Personal library requires an accepted Music User Token; use Login in Settings"
            );
            if let Some(pinned) = pinned_region {
                ensure!(
                    region
                        .as_deref()
                        .is_some_and(|r| r.eq_ignore_ascii_case(pinned)),
                    "Account region changed; restart the library search"
                );
            }
        }
        let storefront = pinned_region
            .map(str::to_ascii_lowercase)
            .or_else(|| region.clone())
            .unwrap_or_else(|| country.to_ascii_lowercase());
        let region_source = if region.as_deref() == Some(storefront.as_str()) {
            "account region"
        } else if pinned_region.is_some() {
            "pinned region"
        } else {
            "selected region"
        };
        let account = account.filter(|_| private);
        let term = term.trim().to_owned();
        let playlist_epoch = if kind.starts_with("open:library-playlists:") {
            Some(self.with_account(account, |state| state.playlist_epoch)?)
        } else {
            None
        };
        let key = format!(
            "music\0{storefront}\0{kind}\0{term}\0{offset}\0{size}\0{}\0{}",
            account.map_or(0, |a| a.generation),
            playlist_epoch.unwrap_or(0),
        );
        let catalog = self.clone();
        let query = MusicPage {
            key: key.clone(),
            kind: kind.to_owned(),
            term,
            offset,
            size,
            max_results,
            page_size,
            reuse_playlist,
            playlist_epoch,
            account,
            storefront,
        };
        let mut page = self
            .pages
            .read(key, async move { catalog.fetch_page(query).await })
            .await?;
        self.check_account(account)?;
        page.context = Some(format!(
            "{} · {} · {region_source}",
            if private {
                "My library"
            } else {
                "Apple Music catalog"
            },
            page.storefront
                .as_deref()
                .unwrap_or(country)
                .to_ascii_uppercase(),
        ));
        Ok(page)
    }

    async fn fetch_page(&self, query: MusicPage) -> Result<Page> {
        let MusicPage {
            key,
            kind,
            term,
            offset,
            size,
            max_results,
            page_size,
            reuse_playlist,
            playlist_epoch,
            account,
            storefront,
        } = query;
        let kind = kind.as_str();
        let private = account.is_some();
        let playlist_tracks = kind.starts_with("open:library-playlists:");
        if let Some(page) = self.with_account(account, |state| {
            if kind == "playlists" || (playlist_tracks && !reuse_playlist) {
                return None;
            }
            state
                .cache
                .get(&key)
                .filter(|(cached, _)| {
                    !playlist_tracks || cached.elapsed() <= Duration::from_secs(30)
                })
                .map(|(_, page)| page.clone())
        })? {
            return Ok(page);
        }
        let mut params = vec![("limit", size.to_string()), ("offset", offset.to_string())];
        let (path, wanted, search) = match kind {
            "search:songs" | "search:albums" | "search:artists" | "search:library-songs" => {
                ensure!(
                    !term.trim().is_empty(),
                    "Enter a song, album or artist to search"
                );
                let wanted = kind.strip_prefix("search:").expect("matched search kind");
                params.push(("term", term.trim().to_owned()));
                params.push(("types", wanted.into()));
                (
                    if private {
                        "me/library/search".into()
                    } else {
                        format!("catalog/{storefront}/search")
                    },
                    wanted,
                    true,
                )
            }
            "library" => ("me/library/songs".into(), "library-songs", false),
            "playlists" => ("me/library/playlists".into(), "library-playlists", false),
            _ if kind.starts_with("open:library-playlists:") => {
                let id = kind
                    .strip_prefix("open:library-playlists:")
                    .expect("matched prefix");
                ensure!(super::playlist::valid_id(id), "Invalid library playlist ID");
                params.push(("include", "catalog".into()));
                (
                    format!("me/library/playlists/{id}/tracks"),
                    "library-songs",
                    false,
                )
            }
            _ if kind.starts_with("open:albums:") || kind.starts_with("open:artists:") => {
                let (prefix, id) = kind
                    .rsplit_once(':')
                    .ok_or_else(|| anyhow!("Invalid catalog lookup"))?;
                ensure!(
                    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()),
                    "Invalid catalog lookup ID"
                );
                if prefix == "open:albums" {
                    (
                        format!("catalog/{storefront}/albums/{id}/tracks"),
                        "songs",
                        false,
                    )
                } else {
                    (
                        format!("catalog/{storefront}/artists/{id}/albums"),
                        "albums",
                        false,
                    )
                }
            }
            _ => return Err(anyhow!("Unsupported Apple Music search category")),
        };
        let response = self.get(&path, &params, account).await?;
        let result = if search {
            &response["results"][wanted]
        } else {
            &response
        };
        let records: &[Value] = if search && result.is_null() && response["results"].is_object() {
            // Apple's successful no-match response omits the requested result type.
            &[]
        } else {
            result["data"]
                .as_array()
                .ok_or_else(|| anyhow!("Apple Music response has no result array"))?
        };
        ensure!(
            records.len() <= size,
            "Apple Music page exceeds requested limit; nothing committed"
        );
        let mut seen = HashSet::new();
        let mut items = records
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                map_resource(v, wanted).map(|mut t| {
                    if kind.starts_with("open:library-playlists:") {
                        t.source_index = Some(offset + i);
                    }
                    t
                })
            })
            .filter(|v| !search || seen.insert(v.id.clone()))
            .take(size)
            .collect::<Vec<_>>();
        for track in &mut items {
            track.provider = "music".into();
            track.storefront = storefront.clone();
        }
        if search {
            items.sort_by_key(|track| std::cmp::Reverse(rank(&term, track)));
        }
        let cursor = |offset: usize| match account {
            Some(account) => format!("{:016x}:{offset}", account.generation),
            None => offset.to_string(),
        };
        let more = result.get("next").and_then(Value::as_str).is_some();
        let page = Page {
            skipped: if search {
                Vec::new()
            } else {
                records
                    .iter()
                    .filter(|r| map_resource(r, wanted).is_none())
                    .map(|_| crate::catalog::album::Skipped {
                        count: 1,
                        reason: "Unsupported or invalid album resource".into(),
                    })
                    .collect()
            },
            total: result["meta"]["total"].as_u64().map(|n| n as usize),
            previous: (offset > 0).then(|| cursor(offset.saturating_sub(page_size))),
            storefront: Some(storefront.clone()),
            items,
            next: (more && offset + size < max_results).then(|| cursor(offset + size)),
            truncated: more && offset + size == max_results,
            context: None,
            account_generation: account.map(|account| account.generation),
        };
        self.with_account(account, |state| {
            if playlist_epoch.is_some_and(|epoch| epoch != state.playlist_epoch) {
                return Err(anyhow!(
                    "Playlist changed while loading; refresh the playlist"
                ));
            }
            if state.cache.capacity != self.capacity.load(Ordering::Relaxed) {
                self.clear_cache(state);
            }
            if kind != "playlists"
                && playlist_epoch.is_none_or(|epoch| epoch == state.playlist_epoch)
            {
                state.cache.insert(key, (Instant::now(), page.clone()));
            }
            self.entries
                .store(state.cache.entries.len(), Ordering::Relaxed);
            Ok(())
        })??;
        Ok(page)
    }
}
fn clean(value: &Value, max: usize) -> String {
    value
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}
pub(super) fn map_resource(value: &Value, wanted: &str) -> Option<Track> {
    if value["type"] != wanted {
        return None;
    }
    let attributes = &value["attributes"];
    let id = if wanted == "library-songs" {
        attributes["playParams"]["catalogId"]
            .as_str()
            .or_else(|| {
                value["relationships"]["catalog"]["data"]
                    .as_array()
                    .and_then(|a| a.iter().find(|r| r["type"] == "songs"))
                    .and_then(|r| r["id"].as_str())
            })
            .or_else(|| value["id"].as_str())
    } else {
        value["id"].as_str()
    }?;
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return None;
    }
    let title = clean(&attributes["name"], 512);
    if title.is_empty() {
        return None;
    }
    let cover = attributes["artwork"]["url"]
        .as_str()
        .filter(|v| v.len() <= 2048 && artwork::artwork_url(v, 100).is_ok())
        .map(str::to_owned);
    let url = attributes["url"]
        .as_str()
        .filter(|url| url.len() <= 2048)
        .and_then(|v| Url::parse(v).ok())
        .filter(|u| {
            u.as_str().len() <= 2048
                && u.scheme() == "https"
                && u.host_str() == Some("music.apple.com")
                && u.username().is_empty()
                && u.password().is_none()
        })
        .map(|u| u.to_string());
    Some(Track {
        can_edit: if wanted == "library-playlists" {
            attributes["canEdit"].as_bool()
        } else {
            None
        },
        disc_number: attributes["discNumber"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
        track_number: attributes["trackNumber"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
        unavailable: (attributes.get("playParams").is_some_and(Value::is_null)
            || attributes["isPlayable"].as_bool() == Some(false))
        .then(|| "Unavailable in this storefront".into()),
        id: id.into(),
        kind: wanted.into(),
        title,
        artist: clean(&attributes["artistName"], 256),
        album: clean(&attributes["albumName"], 512),
        duration_ms: attributes["durationInMillis"]
            .as_u64()
            .filter(|v| *v <= 86400000)
            .unwrap_or(0),
        artwork: cover,
        url,
        ..Track::default()
    })
}
fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .take(512)
        .collect()
}
fn distance(a: &str, b: &str) -> usize {
    let a = a.chars().take(256).collect::<Vec<_>>();
    let b = b.chars().take(256).collect::<Vec<_>>();
    let mut row = (0..=b.len()).collect::<Vec<_>>();
    for (i, left) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let diagonal = previous;
            previous = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(previous + 1)
                .min(diagonal + usize::from(left != right));
        }
    }
    row[b.len()]
}
fn rank(query: &str, track: &Track) -> i32 {
    let q = normalize(query);
    let title = normalize(&track.title);
    let artist = normalize(&track.artist);
    let mut score = if q == title {
        2000
    } else if !title.is_empty() && (q.starts_with(&title) || q.ends_with(&title)) {
        1000
    } else if q.contains(&title) {
        500
    } else {
        0
    };
    let rest = q.strip_prefix(&title).or_else(|| q.strip_suffix(&title));
    if let Some(rest) = rest.filter(|v| !v.is_empty()) {
        let edits = distance(rest, &artist);
        let max = rest.chars().count().max(artist.chars().count());
        if edits * 4 <= max {
            score += 800 - edits as i32 * 20;
        }
    }
    if q.contains(&artist) && !artist.is_empty() {
        score += 300;
    }
    score
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn account_region_and_library_use_the_token_but_catalog_requests_do_not() {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: dir.path().join("config"),
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
            runtime: dir.path().join("run"),
        };
        paths.init().unwrap();
        let fake = format!(
            "{}.{}.fixture",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256","kid":"fixture"}"#),
            URL_SAFE_NO_PAD.encode(r#"{"iss":"fixture","exp":4102444800}"#)
        );
        crate::config::private_write(&paths.cache.join("apple-app-token"), fake.as_bytes())
            .unwrap();
        crate::config::private_write(&paths.config.join("music-user-token"), b"fixture-user")
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for body in [
                json!({"data":[{"id":"pl"}]}),
                json!({"results":{"songs":{"data":[{"id":"22","type":"songs","attributes":{"name":"A&W","artistName":"Other Performer"}},{"id":"11","type":"songs","attributes":{"name":"A&W","artistName":"Lana Del Rey"}}],"next":"https://must-not-follow.test/"}}}),
                json!({"results":{"library-songs":{"data":[{"id":"i.fixture","type":"library-songs","attributes":{"name":"A&W","artistName":"Lana Del Rey","playParams":{"catalogId":"11"}}}]}}}),
                json!({"results":{},"meta":{}}),
                json!({"results":{}}),
                json!({"results":{"songs":{"data":[]}}}),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 8192];
                let mut n = 0;
                while !request[..n].windows(4).any(|v| v == b"\r\n\r\n") {
                    n += socket.read(&mut request[n..]).await.unwrap();
                }
                requests.push(String::from_utf8(request[..n].to_vec()).unwrap());
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        let mut apple = AppleCatalog::new(paths).unwrap();
        apple.origin = origin;
        let limits = crate::config::PerformanceMode::Balanced.limits();
        let first = apple
            .browse("search:songs", "A&W lana del ray", None, "DE", limits)
            .await
            .unwrap();
        assert_eq!(first.items[0].id, "11");
        assert!(first.context.as_deref().unwrap().contains("PL · account"));
        assert_eq!(first.next.as_deref(), Some("25"));
        let cached = apple
            .browse("search:songs", "A&W lana del ray", None, "DE", limits)
            .await
            .unwrap();
        assert_eq!(cached.items[0].id, "11");
        let library = apple
            .browse("search:library-songs", "A&W", None, "DE", limits)
            .await
            .unwrap();
        assert_eq!(library.items[0].id, "11");
        for kind in ["search:songs", "search:library-songs"] {
            let empty = apple
                .browse(kind, "zzqxwjk987654321notasong", None, "PL", limits)
                .await
                .unwrap();
            assert!(empty.items.is_empty());
            assert!(empty.next.is_none());
            assert!(empty.context.is_some());
        }
        let pinned = apple
            .browse("search:songs", "same query", Some("us:25"), "DE", limits)
            .await
            .unwrap();
        assert!(
            pinned
                .context
                .as_deref()
                .unwrap()
                .contains("US · pinned region")
        );
        assert!(
            apple
                .browse("search:library-songs", "song", Some("us:25"), "DE", limits)
                .await
                .is_err()
        );
        assert!(
            apple
                .browse(
                    "search:songs",
                    "A&W",
                    Some("https://evil.test"),
                    "PL",
                    limits
                )
                .await
                .is_err()
        );
        let requests = task.await.unwrap();
        assert!(
            requests[0]
                .to_lowercase()
                .contains("music-user-token: fixture-user")
        );
        assert!(requests[0].starts_with("GET /v1/me/storefront"));
        assert!(
            requests
                .last()
                .unwrap()
                .starts_with("GET /v1/catalog/us/search?")
        );
        assert!(requests[1].starts_with("GET /v1/catalog/pl/search?"));
        assert!(!requests[1].to_lowercase().contains("music-user-token:"));
        assert!(!requests[1].to_lowercase().contains("cookie:"));
        assert!(requests[1].contains("%26"));
        assert!(requests[2].starts_with("GET /v1/me/library/search?"));
        assert!(
            requests[2]
                .to_lowercase()
                .contains("music-user-token: fixture-user")
        );
    }
    #[test]
    fn apple_music_maps_real_catalog_artwork_and_library_catalog_ids_without_previews() {
        let song = json!({"id":"1655349125","type":"songs","attributes":{"name":"A&W","artistName":"Lana Del Rey","albumName":"Did you know","durationInMillis":433916,"artwork":{"url":"https://is1-ssl.mzstatic.com/image/{w}x{h}bb.jpg"},"url":"https://music.apple.com/pl/album/a-w/1655349115?i=1655349125","previews":[{"url":"https://must-not-use.test/audio"}]}});
        let mapped = map_resource(&song, "songs").unwrap();
        assert_eq!(mapped.id, "1655349125");
        assert_eq!(mapped.title, "A&W");
        assert_eq!(mapped.artist, "Lana Del Rey");
        assert!(mapped.artwork.is_some());
        assert!(
            !serde_json::to_string(&mapped)
                .unwrap()
                .contains("must-not-use")
        );
        let library = json!({"id":"i.private-fixture","type":"library-songs","attributes":{"name":"A&W","artistName":"Lana Del Rey","playParams":{"catalogId":"1655349125"}}});
        assert_eq!(
            map_resource(&library, "library-songs").unwrap().id,
            "1655349125"
        );
    }
    #[test]
    fn combined_title_artist_ranking_tolerates_artist_typos_without_artist_aliases() {
        let original = Track {
            title: "A&W".into(),
            artist: "Lana Del Rey".into(),
            ..Track::default()
        };
        let cover = Track {
            title: "A&W".into(),
            artist: "Other Performer".into(),
            ..Track::default()
        };
        let unrelated = Track {
            title: "Other Song".into(),
            artist: "Lana Del Rey".into(),
            ..Track::default()
        };
        for query in ["A&W lana del rey", "A&W lana del ray", "lana del ray A&W"] {
            assert!(rank(query, &original) > rank(query, &cover));
            assert!(rank(query, &original) > rank(query, &unrelated));
        }
    }
}

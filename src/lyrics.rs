use anyhow::{Result, ensure};
mod provider;
use crate::{cache::Cache, config::PerformanceMode, model::Track};
pub use provider::Provider;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
#[derive(Debug)]
pub struct LyricLine {
    pub at_ms: Option<u64>,
    pub text: String,
}
#[derive(Debug)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
    pub instrumental: bool,
    pub source_url: String,
}
const MAX_TEXT: usize = 128 * 1024;
const MAX_LINES: usize = 4096;
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(1024).collect()
}
fn timestamp(s: &str) -> Option<u64> {
    let (m, s) = s.split_once(':')?;
    let m = m.parse::<u64>().ok()?;
    let (seconds, fraction) = s.split_once('.').unwrap_or((s, ""));
    let seconds = seconds.parse::<u64>().ok()?;
    if m > 1440
        || seconds >= 60
        || fraction.len() > 3
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let millis = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()? * 10u64.pow(3 - fraction.len() as u32)
    };
    Some((m * 60 + seconds) * 1000 + millis)
}
impl Lyrics {
    pub fn parse(plain: Option<&str>, synced: Option<&str>, instrumental: bool) -> Result<Self> {
        ensure!(
            plain.map_or(0, str::len) <= MAX_TEXT && synced.map_or(0, str::len) <= MAX_TEXT,
            "lyrics exceed text limit"
        );
        let mut lines = Vec::new();
        let mut offset = 0i64;
        if !instrumental {
            for raw in synced.unwrap_or("").lines().take(MAX_LINES) {
                if let Some(value) = raw
                    .strip_prefix("[offset:")
                    .and_then(|s| s.strip_suffix(']'))
                {
                    offset = value
                        .parse::<i64>()
                        .unwrap_or(0)
                        .clamp(-86_400_000, 86_400_000);
                    continue;
                }
                let mut tail = raw;
                let mut stamps = Vec::new();
                while let Some(rest) = tail.strip_prefix('[') {
                    let Some((tag, remaining)) = rest.split_once(']') else {
                        break;
                    };
                    let Some(time) = timestamp(tag) else {
                        break;
                    };
                    stamps.push(time);
                    tail = remaining;
                    if stamps.len() >= MAX_LINES {
                        break;
                    }
                }
                let text = clean(tail.trim());
                for time in stamps {
                    if lines.len() >= MAX_LINES {
                        break;
                    }
                    lines.push(LyricLine {
                        at_ms: Some(time),
                        text: text.clone(),
                    });
                }
            }
        }
        for line in &mut lines {
            line.at_ms = line.at_ms.map(|t| (t as i64 + offset).max(0) as u64);
        }
        lines.sort_by_key(|l| l.at_ms);
        let timed = !lines.is_empty();
        if !timed && !instrumental {
            lines = plain
                .unwrap_or("")
                .lines()
                .take(MAX_LINES)
                .map(|l| LyricLine {
                    at_ms: None,
                    text: clean(l),
                })
                .collect();
        }
        ensure!(
            instrumental || lines.iter().any(|l| !l.text.is_empty()),
            "lyrics unavailable"
        );
        ensure!(
            lines.iter().map(|l| l.text.len()).sum::<usize>() <= MAX_TEXT,
            "expanded lyrics exceed limit"
        );
        Ok(Self {
            lines,
            synced: timed,
            instrumental,
            source_url: "https://lrclib.net".into(),
        })
    }
    pub fn active_line(&self, seconds: f64) -> Option<usize> {
        if !self.synced || !seconds.is_finite() {
            return None;
        }
        let ms = (seconds.max(0.0) * 1000.0) as u64;
        self.lines
            .partition_point(|l| l.at_ms.is_some_and(|t| t <= ms))
            .checked_sub(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
    pub generation: u64,
}
impl LyricsQuery {
    pub fn from_track(track: &Track) -> Option<Self> {
        if track.title.trim().is_empty() || track.artist.trim().is_empty() {
            return None;
        }
        Some(Self {
            title: track
                .title
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect(),
            artist: track
                .artist
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect(),
            album: track
                .album
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect(),
            duration_secs: track.duration_ms / 1000,
            generation: 0,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Network,
    RateLimited,
    TooLarge,
    Invalid,
}
impl LoadError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Network => "Lyrics service unavailable. R retries.",
            Self::RateLimited => "LRCLIB requested a pause; retry later with R.",
            Self::TooLarge => "Lyrics response exceeds memory limits.",
            Self::Invalid => "Lyrics response could not be read.",
        }
    }
}
#[derive(Clone)]
pub enum LyricsState {
    Found(Arc<Lyrics>),
    Missing,
    Error(LoadError),
}
#[derive(Clone, PartialEq, Eq)]
pub struct LyricsRequest {
    pub query: LyricsQuery,
    pub profile: PerformanceMode,
}
pub struct LyricsResult {
    pub request: LyricsRequest,
    pub state: LyricsState,
}
pub fn worker() -> (
    watch::Sender<Option<LyricsRequest>>,
    mpsc::Receiver<LyricsResult>,
    tokio::task::JoinHandle<()>,
) {
    let (jobs, mut pending) = watch::channel::<Option<LyricsRequest>>(None);
    let (done, results) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let mut provider = None;
        let mut cache = Cache::new(8);
        while pending.changed().await.is_ok() {
            let request = pending.borrow_and_update().clone();
            let Some(request) = request else {
                continue;
            };
            cache.capacity = request.profile.limits().cache_entries;
            while cache.entries.len() > cache.capacity {
                cache.entries.pop_front();
            }
            let state = if let Some((at, state)) =
                cache
                    .get(&request.query)
                    .filter(|(at, state): &&(Instant, LyricsState)| {
                        !matches!(state, LyricsState::Error(_))
                            || at.elapsed() < Duration::from_secs(30)
                    }) {
                let _ = at;
                state.clone()
            } else {
                if provider.is_none() {
                    provider = Provider::new().ok();
                }
                let state = match provider.as_mut() {
                    Some(p) => match p.lookup(&request.query).await {
                        Ok(Some(l)) => LyricsState::Found(Arc::new(l)),
                        Ok(None) => LyricsState::Missing,
                        Err(e) => LyricsState::Error(e),
                    },
                    None => LyricsState::Error(LoadError::Network),
                };
                cache.insert(request.query.clone(), (Instant::now(), state.clone()));
                state
            };
            if pending.has_changed().unwrap_or(true) {
                continue;
            }
            if done.send(LyricsResult { request, state }).await.is_err() {
                break;
            }
        }
    });
    (jobs, results, task)
}

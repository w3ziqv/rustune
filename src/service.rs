use crate::{
    backend::{MusicService, PlaybackBackend, musickit::MusicKit},
    cache::Cache,
    config::{Config, Paths, PerformanceMode, private_write},
    ipc::{Request, read_frame, write_frame},
    model::{Page, Playback, Repeat, Track},
    queue::Queue,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::UnixListener,
    sync::{Semaphore, mpsc, oneshot, watch},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub playback: Playback,
    pub queue_len: usize,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub profile: PerformanceMode,
    pub pid: u32,
    pub browser_pid: Option<u32>,
    pub backend: String,
    #[serde(default)]
    pub playback_available: bool,
    pub browser_shared: bool,
    pub uptime_secs: u64,
    pub cache_entries: usize,
    pub message: String,
    pub sleep_remaining: Option<u64>,
}
#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub data: Value,
    pub error: Option<String>,
}
impl Reply {
    pub fn result(r: Result<Value>) -> Self {
        match r {
            Ok(data) => Self {
                ok: true,
                data,
                error: None,
            },
            Err(e) => Self {
                ok: false,
                data: Value::Null,
                error: Some(format!("{e:#}")),
            },
        }
    }
    pub fn value(self) -> Result<Value> {
        ensure!(
            self.ok,
            "{}",
            self.error.as_deref().unwrap_or("service error")
        );
        Ok(self.data)
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SavedState {
    queue: Queue,
    history: Vec<String>,
    dismissed: Option<String>,
}
pub struct Envelope {
    pub request: Request,
    pub reply: oneshot::Sender<Reply>,
}
pub type Sender = mpsc::Sender<Envelope>;
pub async fn send(tx: &Sender, request: Request) -> Result<Value> {
    let (reply, rx) = oneshot::channel();
    tx.send(Envelope { request, reply })
        .await
        .context("service stopped")?;
    tokio::time::timeout(Duration::from_secs(50), rx)
        .await
        .context("service request timed out")??
        .value()
}

struct Engine {
    backend: MusicKit,
    queue: Queue,
    playback: Playback,
    cache: Cache<String, Page>,
    history: Vec<String>,
    started: Instant,
    message: String,
    sleep: Option<Instant>,
    volume_before_mute: f64,
    shutdown: bool,
    dismissed: Option<String>,
}
impl Engine {
    fn new(config: Config, paths: Paths) -> Result<Self> {
        let p = paths.data.join("state.json");
        let mut saved = SavedState::default();
        let mut message = "Ready. / to search • :login to authorize".to_owned();
        if config.restore_queue && p.exists() {
            match (||->Result<SavedState>{
                ensure!(fs::metadata(&p)?.len()<=16*1024*1024,"saved state exceeds limit");
                let s:SavedState=serde_json::from_slice(&fs::read(&p)?)?;
                ensure!(s.queue.tracks.len()<=config.max_queue && s.history.len()<=50,"saved state exceeds item limits");
                for t in &s.queue.tracks {validate_track(t)?;}
                ensure!(s.history.iter().all(|v|v.len()<=512),"saved history exceeds limits");
                Ok(s)
            })(){Ok(s)=>saved=s,Err(_)=>message="Saved queue is invalid; ignored. Original file retained until the next queue edit.".into()}
        }
        saved.queue.current = saved
            .queue
            .current
            .filter(|i| *i < saved.queue.tracks.len());
        let cap = config.performance.limits().cache_entries;
        let playback = Playback {
            volume: config.volume,
            ..Playback::default()
        };
        Ok(Self {
            backend: MusicKit::new(config, paths),
            queue: saved.queue,
            playback,
            cache: Cache::new(cap),
            history: saved.history,
            started: Instant::now(),
            message,
            sleep: None,
            volume_before_mute: 0.7,
            shutdown: false,
            dismissed: saved.dismissed,
        })
    }
    fn status(&self) -> Status {
        Status {
            playback: self.playback.clone(),
            queue_len: self.queue.tracks.len(),
            current: self.queue.current,
            repeat: self.queue.repeat,
            shuffle: self.queue.shuffle,
            profile: self.backend.config.performance,
            pid: std::process::id(),
            browser_pid: self.backend.pid(),
            backend: self.backend.name().into(),
            playback_available: self.backend.config.backend != crate::config::BackendKind::Disabled,
            browser_shared: self.backend.config.backend == crate::config::BackendKind::Firefox,
            uptime_secs: self.started.elapsed().as_secs(),
            cache_entries: self.cache.entries.len(),
            message: self.message.clone(),
            sleep_remaining: self
                .sleep
                .map(|t| t.saturating_duration_since(Instant::now()).as_secs()),
        }
    }
    fn save(&self) -> Result<()> {
        private_write(
            &self.backend.paths.data.join("state.json"),
            &serde_json::to_vec(&SavedState {
                queue: self.queue.clone(),
                history: self.history.clone(),
                dismissed: self.dismissed.clone(),
            })?,
        )
    }
    async fn play_index(&mut self, i: usize) -> Result<()> {
        let t = self
            .queue
            .tracks
            .get(i)
            .context("queue index out of range")?;
        self.backend
            .execute("play_track", json!({"id":t.id}))
            .await?;
        self.queue.current = Some(i);
        self.dismissed = None;
        self.playback = self.backend.snapshot().await?;
        self.save()?;
        Ok(())
    }
    async fn refresh(&mut self) -> Result<()> {
        if self.sleep.is_some_and(|t| t <= Instant::now()) {
            self.sleep = None;
            if self.backend.pid().is_some() {
                self.backend.execute("pause", Value::Null).await?;
            }
            self.message = "Sleep timer finished".into();
        }
        if self.backend.pid().is_none() {
            return Ok(());
        }
        let previous = self.playback.clone();
        match self.backend.snapshot().await {
            Ok(s) => self.playback = s,
            Err(_) => {
                self.backend.close().await;
                self.playback.playing = false;
                self.message =
                    "Browser disconnected. Use Play to reconnect; :login if authorization expired."
                        .into();
                return Ok(());
            }
        }
        if previous.authorized != self.playback.authorized {
            self.cache = Cache::new(self.backend.config.performance.limits().cache_entries);
        }
        if self.backend.login_mode && self.playback.authorized {
            self.backend.finish_login().await?;
            self.message = "Apple Music authorized. Ready for playback.".into();
        }
        if self.playback.preview {
            self.message =
                "Preview rejected: full-song subscription/DRM playback is unavailable.".into();
        }
        // Use the official playback state, not a guessed elapsed-time threshold.
        if previous.playing
            && !self.playback.playing
            && self.playback.ended
            && !self.playback.preview
            && let Some(i) = self.queue.candidate(false)
        {
            self.play_index(i).await?;
        }
        if self.backend.config.notifications
            && previous.track.as_ref().map(|t| &t.id) != self.playback.track.as_ref().map(|t| &t.id)
            && let Some(t) = &self.playback.track
        {
            let _ = tokio::process::Command::new("notify-send")
                .args(["--app-name=Rustune", "--", &t.title, &t.artist])
                .kill_on_drop(true)
                .status()
                .await;
        }
        Ok(())
    }
    async fn handle(&mut self, request: Request) -> Result<Value> {
        if self.backend.config.backend == crate::config::BackendKind::Disabled
            && matches!(
                request,
                Request::Play
                    | Request::Pause
                    | Request::Toggle
                    | Request::Next
                    | Request::Previous
                    | Request::Seek(_)
                    | Request::Volume(_)
                    | Request::Mute
                    | Request::Login
                    | Request::Logout
                    | Request::Browse { .. }
                    | Request::PlayIndex(_)
                    | Request::Enqueue { play: true, .. }
                    | Request::Sleep(1..)
            )
        {
            anyhow::bail!(crate::backend::musickit::NATIVE_UNAVAILABLE);
        }
        if self.backend.config.backend == crate::config::BackendKind::Firefox
            && self.backend.pid().is_none()
            && matches!(
                request,
                Request::Play
                    | Request::Pause
                    | Request::Toggle
                    | Request::Next
                    | Request::Previous
                    | Request::Seek(_)
                    | Request::Volume(_)
                    | Request::Mute
                    | Request::Login
                    | Request::Browse { .. }
            )
        {
            self.backend
                .start(matches!(request, Request::Login))
                .await?;
            self.playback = self.backend.snapshot().await?;
            self.cache = Cache::new(self.backend.config.performance.limits().cache_entries);
            if let Some(track) = self.playback.track.as_ref()
                && self.queue.tracks.is_empty()
                && (self.dismissed.as_ref() != Some(&track.id) || self.playback.playing)
            {
                validate_track(track)?;
                self.queue.push(track.clone())?;
                self.queue.current = Some(0);
                self.dismissed = None;
                self.save()?;
            }
        }
        let mut save = false;
        match request {
            Request::Subscribe => anyhow::bail!("subscription requires a dedicated connection"),
            Request::Status => return Ok(serde_json::to_value(self.status())?),
            Request::QueuePage { offset } => {
                return Ok(
                    json!({"items":self.queue.tracks.iter().skip(offset).take(self.backend.config.performance.limits().page_size).collect::<Vec<_>>(),"offset":offset,"total":self.queue.tracks.len()}),
                );
            }
            Request::History => return Ok(json!(self.history)),
            Request::Login => {
                self.cache = Cache::new(self.backend.config.performance.limits().cache_entries);
                self.backend.start(true).await?;
                self.message =
                    if self.backend.config.backend == crate::config::BackendKind::Firefox {
                        "Connected to your existing Firefox Apple Music session."
                    } else {
                        "Complete Apple ID and 2FA in Chromium; :status shows authorization."
                    }
                    .into();
            }
            Request::Logout => {
                self.cache = Cache::new(self.backend.config.performance.limits().cache_entries);
                self.backend.execute("logout", Value::Null).await?;
                self.backend.close().await;
                self.playback = Playback::default();
                self.message = "Signed out of MusicKit. Use :login to authorize again.".into();
            }
            Request::Play | Request::Toggle if !self.playback.playing => {
                if self.backend.pid().is_none()
                    || !self.queue.can_resume(self.playback.track.as_ref())
                {
                    let i = self.queue.current.unwrap_or(0);
                    self.play_index(i).await?;
                } else {
                    self.backend.execute("play", Value::Null).await?;
                }
            }
            Request::Pause | Request::Toggle => {
                if self.backend.pid().is_some() {
                    self.backend.execute("pause", Value::Null).await?;
                }
            }
            Request::Play => {}
            Request::Next => {
                if let Some(i) = self.queue.candidate(true) {
                    self.play_index(i).await?;
                } else if self.backend.pid().is_some() {
                    self.backend.execute("pause", Value::Null).await?;
                }
            }
            Request::Previous => {
                let i = self.queue.current.unwrap_or(0).saturating_sub(1);
                self.play_index(i).await?;
            }
            Request::Seek(seconds) => {
                ensure!(
                    seconds.is_finite() && seconds >= 0.0 && seconds <= self.playback.duration,
                    "seek must be within the current track"
                );
                self.backend
                    .execute("seek", json!({"seconds":seconds}))
                    .await?;
            }
            Request::Volume(volume) => {
                ensure!(
                    volume.is_finite() && (0.0..=1.0).contains(&volume),
                    "volume must be 0..1"
                );
                self.backend.config.volume = volume;
                if self.backend.pid().is_some() {
                    self.backend
                        .execute("volume", json!({"volume":volume}))
                        .await?;
                }
                self.playback.volume = volume;
                self.backend.config.save(&self.backend.paths)?;
            }
            Request::Mute => {
                let v = if self.backend.config.volume > 0.0 {
                    self.volume_before_mute = self.backend.config.volume;
                    0.0
                } else {
                    self.volume_before_mute
                };
                self.backend.config.volume = v;
                self.playback.volume = v;
                if self.backend.pid().is_some() {
                    self.backend.execute("volume", json!({"volume":v})).await?;
                }
            }
            Request::Shuffle => {
                self.queue.shuffle = !self.queue.shuffle;
                save = true;
            }
            Request::Repeat => {
                self.queue.repeat = match self.queue.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
                save = true;
            }
            Request::Sleep(seconds) => {
                ensure!(seconds <= 86400, "sleep timer maximum is 24 hours");
                self.sleep = if seconds == 0 {
                    None
                } else {
                    Some(Instant::now() + Duration::from_secs(seconds))
                };
            }
            Request::Browse { kind, term, next } => {
                ensure!(kind.len() <= 256, "browse kind too long");
                let key = format!("{kind}\0{term}\0{}", next.as_deref().unwrap_or(""));
                if let Some(page) = self.cache.get(&key) {
                    return Ok(serde_json::to_value(page)?);
                }
                let page = self
                    .backend
                    .browse(
                        &kind,
                        &term,
                        next.as_deref(),
                        self.backend.config.performance.limits().page_size,
                    )
                    .await?;
                if kind.starts_with("search") && !term.is_empty() {
                    self.history.retain(|s| s != &term);
                    self.history.insert(0, term);
                    self.history.truncate(50);
                    self.save()?;
                }
                let result = serde_json::to_value(&page)?;
                self.cache.insert(key, page);
                return Ok(result);
            }
            Request::Enqueue { track, next, play } => {
                validate_track(&track)?;
                ensure!(
                    track.kind == "songs" || track.kind == "library-songs",
                    "open this album/artist/playlist first"
                );
                ensure!(
                    self.queue.tracks.len() < self.backend.config.max_queue,
                    "queue limit reached"
                );
                let mut i = self.queue.tracks.len();
                self.queue.push(track)?;
                if next {
                    let target = self.queue.current.map_or(0, |i| i + 1);
                    self.queue.move_item(i, target);
                    i = target;
                }
                self.save()?;
                if play {
                    self.play_index(i).await?;
                }
                save = true;
            }
            Request::PlayIndex(i) => self.play_index(i).await?,
            Request::Remove(i) => {
                ensure!(i < self.queue.tracks.len(), "queue index out of range");
                if self.queue.current == Some(i) {
                    self.dismissed = self.playback.track.as_ref().map(|t| t.id.clone());
                }
                if self.queue.current == Some(i) && self.backend.pid().is_some() {
                    self.backend.execute("pause", Value::Null).await?;
                }
                self.queue.remove(i);
                save = true;
            }
            Request::Move { from, to } => {
                ensure!(self.queue.move_item(from, to), "queue index out of range");
                save = true;
            }
            Request::Clear => {
                self.dismissed = self.playback.track.as_ref().map(|t| t.id.clone());
                if self.backend.pid().is_some() {
                    self.backend.execute("pause", Value::Null).await?;
                }
                self.queue = Queue::default();
                save = true;
            }
            Request::Theme(theme) => {
                let mut c = self.backend.config.clone();
                c.theme = theme;
                Config::parse(&toml::to_string(&c)?)?;
                self.backend.config = c;
                self.backend.config.save(&self.backend.paths)?;
            }
            Request::Profile(profile) => {
                self.backend.config.performance = profile;
                self.cache = Cache::new(profile.limits().cache_entries);
                self.backend.config.save(&self.backend.paths)?;
            }
            Request::Stop => self.shutdown = true,
        }
        if save {
            self.save()?;
        }
        // Browser snapshots are refreshed by the service, never inferred from commands.
        if self.backend.pid().is_some() && !self.backend.login_mode {
            self.refresh().await?;
        }
        Ok(serde_json::to_value(self.status())?)
    }
}
pub fn validate_track(t: &Track) -> Result<()> {
    ensure!(
        !t.id.is_empty()
            && t.id.len() <= 128
            && t.id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "invalid track ID"
    );
    ensure!(
        [&t.title, &t.artist, &t.album, &t.kind]
            .iter()
            .all(|s| s.len() <= 2048 && !s.chars().any(char::is_control)),
        "invalid track metadata"
    );
    ensure!(
        t.artwork
            .as_ref()
            .is_none_or(|s| s.len() <= 4096 && s.starts_with("https://")),
        "invalid artwork URL"
    );
    Ok(())
}
pub async fn run(config: Config, paths: Paths) -> Result<()> {
    paths.init()?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(paths.runtime.join("service.lock"))?;
    let _guard = nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| anyhow::anyhow!("another Rustune service is running: {e}"))?;
    let socket = paths.socket();
    if socket.exists() {
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    private_write(
        &paths.runtime.join("service.pid"),
        std::process::id().to_string().as_bytes(),
    )?;
    let uid = fs::metadata(&paths.runtime)?.uid();
    let (tx, mut rx) = mpsc::channel::<Envelope>(32);
    let mut engine = Engine::new(config.clone(), paths.clone())?;
    let (state_tx, state_rx) = watch::channel(engine.status());
    let subscriber = state_rx.clone();
    let mpris = crate::mpris::start(tx.clone(), state_rx).await;
    if mpris.is_err() {
        tracing::warn!("MPRIS unavailable; session D-Bus is required");
    }
    let permits = Arc::new(Semaphore::new(8));
    let (quit_accept, mut accept_quit) = oneshot::channel::<()>();
    let accept = tokio::spawn(async move {
        let mut clients = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _=&mut accept_quit=>break,
                accepted=listener.accept()=>{
                    let Ok((mut s,_))=accepted else{break;};
                    if s.peer_cred().is_err() || s.peer_cred().is_ok_and(|c|c.uid()!=uid){continue;}
                    let Ok(permit)=permits.clone().try_acquire_owned() else {continue;};let sender=tx.clone();let mut updates=subscriber.clone();
                    clients.spawn(async move {
                        let _permit=permit;
                        let operation=async {
                            let b=tokio::time::timeout(Duration::from_secs(5),read_frame(&mut s)).await??;
                            ensure!(b.len()<=65536,"command exceeds 64 KiB");
                            let request:Request=serde_json::from_slice(&b)?;
                            if matches!(request,Request::Subscribe) {
                                loop {
                                    let status=updates.borrow_and_update().clone();
                                    write_frame(&mut s,&Reply::result(Ok(serde_json::to_value(status)?))).await?;
                                    let mut byte=[0u8;1];
                                    tokio::select! {
                                        r=updates.changed()=>{if r.is_err(){break;}}
                                        _=tokio::io::AsyncReadExt::read(&mut s,&mut byte)=>break,
                                    }
                                }
                                return Ok::<(),anyhow::Error>(());
                            }
                            let r=Reply::result(send(&sender,request).await);write_frame(&mut s,&r).await
                        };
                        let _=operation.await;
                    });
                }
                _=clients.join_next(),if !clients.is_empty()=>{}
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            while clients.join_next().await.is_some() {}
        })
        .await;
    });
    // The static origin is cheap and carries no credentials. Chromium remains lazy.

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        let active = engine.backend.pid().is_some() || engine.sleep.is_some();
        let delay = Duration::from_millis(engine.backend.config.performance.limits().refresh_ms);
        tokio::select! {
            Some(e)=rx.recv()=>{
                let r=engine.handle(e.request).await;
                if let Err(ref error)=r {engine.message=error.to_string();}
                let _=e.reply.send(Reply::result(r));
                state_tx.send_replace(engine.status());
                if engine.shutdown{break;}
            }
            _=tokio::time::sleep(delay),if active=>{
                if let Err(e)=engine.refresh().await{engine.message=e.to_string();}
                state_tx.send_replace(engine.status());
            }
            _=tokio::signal::ctrl_c()=>break,
            _=term.recv()=>break,
        }
    }
    engine.backend.close().await;
    drop(state_tx);
    let _ = quit_accept.send(());
    let _ = accept.await;

    drop(mpris);
    let _ = fs::remove_file(socket);
    let _ = fs::remove_file(paths.runtime.join("service.pid"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn engine() -> (tempfile::TempDir, Engine) {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: d.path().join("config"),
            data: d.path().join("data"),
            cache: d.path().join("cache"),
            runtime: d.path().join("run"),
        };
        paths.init().unwrap();
        let config = Config {
            backend: crate::config::BackendKind::MusicKit,
            ..Config::default()
        };
        let mut e = Engine::new(config, paths).unwrap();
        for id in ["a", "b", "c"] {
            e.queue
                .push(Track {
                    id: id.into(),
                    kind: "songs".into(),
                    ..Track::default()
                })
                .unwrap();
        }
        e.queue.current = Some(0);
        (d, e)
    }
    #[tokio::test]
    async fn failed_next_does_not_change_current_selection() {
        let (_dir, mut e) = engine();
        assert!(e.handle(Request::Next).await.is_err());
        assert_eq!(e.queue.current, Some(0));
    }
    #[tokio::test]
    async fn disabled_playback_commands_explain_missing_engine_without_mutating_queue() {
        let (_dir, mut e) = engine();
        e.backend.config.backend = crate::config::BackendKind::Disabled;
        for request in [
            Request::Play,
            Request::Pause,
            Request::Next,
            Request::Seek(10.0),
            Request::Sleep(60),
            Request::PlayIndex(999),
        ] {
            let error = e.handle(request).await.unwrap_err().to_string();
            assert!(error.contains("Native Linux playback"), "{error}");
            assert_eq!(e.queue.current, Some(0));
        }
    }
    #[tokio::test]
    async fn stopped_browser_logout_cannot_report_a_false_success() {
        let (_dir, mut e) = engine();
        assert!(
            e.handle(Request::Logout).await.is_err(),
            "no token/browser means authorization was not revoked"
        );
    }
    #[tokio::test]
    async fn logout_invalidates_private_cached_pages_even_on_failure() {
        let (_dir, mut e) = engine();
        e.cache.insert(
            "library\0\0".into(),
            Page {
                items: vec![Track {
                    id: "private".into(),
                    ..Track::default()
                }],
                next: None,
            },
        );
        let _ = e.handle(Request::Logout).await;
        assert!(e.cache.entries.is_empty());
    }
}

#[cfg(test)]
mod shared_browser_tests {
    use super::*;
    #[tokio::test]
    async fn first_pause_connects_to_the_existing_firefox_session() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: d.path().join("config"),
            data: d.path().join("data"),
            cache: d.path().join("cache"),
            runtime: d.path().join("run"),
        };
        paths.init().unwrap();
        let listener = tokio::net::UnixListener::bind(paths.runtime.join("firefox.sock")).unwrap();
        let mut engine = Engine::new(
            Config {
                backend: crate::config::BackendKind::Firefox,
                ..Config::default()
            },
            paths,
        )
        .unwrap();
        let client = tokio::spawn(async move { engine.handle(Request::Pause).await });
        let connected = tokio::time::timeout(Duration::from_millis(500), listener.accept()).await;
        client.abort();
        assert!(
            connected.is_ok(),
            "Pause must connect to an existing Firefox session rather than silently succeeding"
        );
    }
}

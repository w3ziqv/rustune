use crate::backend::{
    controller::Controller,
    worker::{Control, Event},
};
use crate::{
    catalog::Catalog,
    config::{Config, Paths, PerformanceMode, private_write},
    ipc::{Request, read_frame, write_frame},
    model::{Playback, Repeat, Track},
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
    #[serde(default)]
    pub queue_revision: u64,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub profile: PerformanceMode,
    #[serde(default = "crate::config::default_catalog_country")]
    pub catalog_country: String,
    pub pid: u32,
    pub browser_pid: Option<u32>,
    #[serde(default)]
    pub audio_pid: Option<u32>,
    pub backend: String,
    #[serde(default)]
    pub playback_available: bool,
    #[serde(default)]
    pub start_available: bool,
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
    config: Config,
    paths: Paths,
    queue: Queue,
    playback: Playback,
    catalog: Arc<Catalog>,
    history: Vec<String>,
    started: Instant,
    message: String,
    shutdown: bool,
    dismissed: Option<String>,
    audio: Option<Controller>,
    audio_generation: u64,
    event_tx: mpsc::Sender<(u64, Event)>,
    events: mpsc::Receiver<(u64, Event)>,
    sleep_until: Option<Instant>,
    unmuted_volume: f64,
}
impl Engine {
    async fn start_audio(&mut self, index: usize) -> Result<()> {
        ensure!(
            crate::backend::worker::available(&self.paths),
            crate::backend::NATIVE_UNAVAILABLE
        );
        let track = self
            .queue
            .tracks
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Queue index out of range"))?
            .clone();
        Control::Start {
            id: track.id.clone(),
            volume: self.playback.volume,
        }
        .validate()?;
        if let Some(audio) = self.audio.take() {
            audio.stop().await;
        }
        self.audio_generation = self.audio_generation.wrapping_add(1);
        self.audio = Some(
            Controller::start(
                track.id.clone(),
                self.playback.volume,
                self.audio_generation,
                self.event_tx.clone(),
            )
            .await?,
        );
        self.queue.current = Some(index);
        self.dismissed = None;
        self.playback = Playback {
            volume: self.playback.volume,
            track: Some(track),
            state: 1,
            ..Playback::default()
        };
        self.message = "Preparing native Apple playback and CDM license…".into();
        Ok(())
    }
    async fn audio_control(&mut self, control: Control) -> Result<()> {
        if self.audio.is_none() {
            ensure!(
                crate::backend::worker::available(&self.paths),
                crate::backend::NATIVE_UNAVAILABLE
            );
        }
        self.audio
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No native track is active; choose a song and Play"))?
            .send(control)
            .await
    }
    async fn native_event(&mut self, generation: u64, event: Event) {
        if generation != self.audio_generation {
            return;
        }
        match event {
            Event::Ready {
                duration,
                sample_rate,
                channels,
            } => {
                self.playback.authorized = true;
                self.playback.duration = duration;
                self.playback.source_codec = Some("AAC".into());
                self.playback.source_sample_rate_hz = Some(sample_rate);
                self.playback.source_channels = Some(channels);
                self.playback.target_bitrate_kbps = Some(256);
                self.message = "Native CDM license accepted; starting Linux audio".into();
            }
            Event::State {
                playing,
                position,
                encrypted_packets,
                decoded_frames,
                nonzero_samples,
            } => {
                self.playback.playing = playing && self.playback.authorized && decoded_frames > 0;
                self.playback.state = if self.playback.playing { 2 } else { 3 };
                self.playback.position = position;
                self.playback.encrypted_packets = encrypted_packets;
                self.playback.decoded_audio_frames = decoded_frames;
                self.playback.nonzero_output_samples = nonzero_samples;
                self.message = if self.playback.playing {
                    "Native Apple Music · CDM → AAC → ALSA"
                } else {
                    "Native playback paused"
                }
                .into();
            }
            Event::Ended {
                position,
                encrypted_packets,
                decoded_frames,
                nonzero_samples,
            } => {
                self.playback.playing = false;
                self.playback.ended = true;
                self.playback.position = position;
                self.playback.state = 4;
                self.playback.encrypted_packets = encrypted_packets;
                self.playback.decoded_audio_frames = decoded_frames;
                self.playback.nonzero_output_samples = nonzero_samples;
                self.message = "Native playback reached the end of the complete stream".into();
                if let Some(audio) = self.audio.take() {
                    audio.stop().await;
                }
                if self.queue.can_resume(self.playback.track.as_ref())
                    && let Some(next) = self.queue.candidate(false)
                {
                    if let Err(error) = self.start_audio(next).await {
                        self.message = error.to_string();
                    } else {
                        let _ = self.save();
                    }
                }
            }
            Event::Error(message) => {
                self.playback.playing = false;
                self.playback.state = 0;
                self.message = message;
                if let Some(audio) = self.audio.take() {
                    audio.stop().await;
                }
            }
        }
    }
    fn new(config: Config, paths: Paths) -> Result<Self> {
        let p = paths.data.join("state.json");
        let mut saved = SavedState::default();
        let mut message =
            "Ready. L enters a Music User Token. Search → song → Play starts native playback."
                .to_owned();
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
        let catalog = Arc::new(Catalog::new()?);
        catalog.set_capacity(config.performance.limits().cache_entries);
        let playback = Playback {
            volume: config.volume,
            ..Playback::default()
        };
        let (event_tx, events) = mpsc::channel(32);
        let unmuted_volume = config.volume;
        Ok(Self {
            config,
            paths,
            queue: saved.queue,
            playback,
            catalog,
            history: saved.history,
            started: Instant::now(),
            message,
            shutdown: false,
            dismissed: saved.dismissed,
            audio: None,
            audio_generation: 0,
            event_tx,
            events,
            sleep_until: None,
            unmuted_volume,
        })
    }
    fn status(&self) -> Status {
        Status {
            playback: self.playback.clone(),
            queue_len: self.queue.tracks.len(),
            queue_revision: self.queue.revision,
            current: self.queue.current,
            repeat: self.queue.repeat,
            shuffle: self.queue.shuffle,
            profile: self.config.performance,
            catalog_country: self.config.catalog_country.clone(),
            pid: std::process::id(),
            browser_pid: None,
            audio_pid: self.audio.as_ref().and_then(Controller::pid),
            backend: "Native CDM / AAC / ALSA".into(),
            playback_available: self.audio.is_some()
                || crate::backend::worker::available(&self.paths),
            start_available: crate::backend::worker::available(&self.paths),
            browser_shared: false,
            uptime_secs: self.started.elapsed().as_secs(),
            cache_entries: self.catalog.cache_entries(),
            message: self.message.clone(),
            sleep_remaining: self
                .sleep_until
                .map(|until| until.saturating_duration_since(Instant::now()).as_secs()),
        }
    }
    fn save(&self) -> Result<()> {
        private_write(
            &self.paths.data.join("state.json"),
            &serde_json::to_vec(&SavedState {
                queue: self.queue.clone(),
                history: self.history.clone(),
                dismissed: self.dismissed.clone(),
            })?,
        )
    }
    async fn handle(&mut self, request: Request) -> Result<Value> {
        let mut save = false;
        match request {
            Request::PlayQueued {
                revision,
                index,
                id,
            } => {
                ensure!(
                    revision == self.queue.revision
                        && self.queue.tracks.get(index).is_some_and(|t| t.id == id),
                    "Queue changed; refresh it before playing this track"
                );
                self.start_audio(index).await?;
                save = true;
            }
            Request::Subscribe => anyhow::bail!("subscription requires a dedicated connection"),
            Request::Status => return Ok(serde_json::to_value(self.status())?),
            Request::QueuePage { offset } => {
                return Ok(
                    json!({"items":self.queue.tracks.iter().skip(offset).take(self.config.performance.limits().page_size).collect::<Vec<_>>(),"offset":offset,"total":self.queue.tracks.len(),"revision":self.queue.revision}),
                );
            }
            Request::History => return Ok(json!(self.history)),
            Request::RecordSearch(term) => {
                ensure!(
                    !term.is_empty() && term.len() <= 512 && !term.chars().any(char::is_control),
                    "Invalid search history entry"
                );
                self.history.retain(|v| v != &term);
                self.history.insert(0, term);
                self.history.truncate(50);
                save = true;
            }
            Request::Browse { kind, term, next } => {
                return Ok(serde_json::to_value(
                    self.catalog
                        .browse(
                            &kind,
                            &term,
                            next.as_deref(),
                            &self.config.catalog_country,
                            self.config.performance.limits(),
                        )
                        .await?,
                )?);
            }
            Request::Play | Request::Toggle if self.audio.is_none() || self.playback.ended => {
                ensure!(
                    crate::backend::worker::available(&self.paths),
                    crate::backend::NATIVE_UNAVAILABLE
                );
                let index = self
                    .queue
                    .current
                    .or_else(|| (!self.queue.tracks.is_empty()).then_some(0))
                    .ok_or_else(|| anyhow::anyhow!("The queue is empty; choose a song first"))?;
                self.start_audio(index).await?;
                save = true;
            }
            Request::Play => self.audio_control(Control::Pause(false)).await?,
            Request::Pause => self.audio_control(Control::Pause(true)).await?,
            Request::Toggle => {
                self.audio_control(Control::Pause(self.playback.playing))
                    .await?
            }
            Request::Next | Request::Previous => {
                ensure!(
                    crate::backend::worker::available(&self.paths),
                    crate::backend::NATIVE_UNAVAILABLE
                );
                let index = if matches!(request, Request::Next) {
                    self.queue.candidate(true)
                } else {
                    self.queue.current.and_then(|v| v.checked_sub(1))
                }
                .ok_or_else(|| anyhow::anyhow!("No track in that direction"))?;
                self.start_audio(index).await?;
                save = true;
            }
            Request::PlayIndex(index) => {
                self.start_audio(index).await?;
                save = true;
            }
            Request::Seek(position) => {
                if self.audio.is_none() {
                    ensure!(
                        crate::backend::worker::available(&self.paths),
                        crate::backend::NATIVE_UNAVAILABLE
                    );
                }
                ensure!(
                    position.is_finite() && position >= 0.0 && position < self.playback.duration,
                    "Seek position must be within the active track"
                );
                self.audio_control(Control::Seek(position)).await?;
            }
            Request::Volume(volume) => {
                ensure!(
                    volume.is_finite() && (0.0..=1.0).contains(&volume),
                    "Volume must be between 0 and 1"
                );
                if self.audio.is_some() {
                    self.audio_control(Control::Volume(volume)).await?;
                }
                self.playback.volume = volume;
                self.config.volume = volume;
                self.config.save(&self.paths)?;
            }
            Request::Mute => {
                let volume = if self.playback.volume > 0.0 {
                    self.unmuted_volume = self.playback.volume;
                    0.0
                } else {
                    self.unmuted_volume.max(0.1)
                };
                if self.audio.is_some() {
                    self.audio_control(Control::Volume(volume)).await?;
                }
                self.playback.volume = volume;
            }
            Request::Sleep(seconds @ 1..) => {
                ensure!(
                    crate::backend::worker::available(&self.paths),
                    crate::backend::NATIVE_UNAVAILABLE
                );
                ensure!(seconds <= 86400, "Sleep timer is limited to 24 hours");
                self.sleep_until = Some(Instant::now() + Duration::from_secs(seconds));
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
            Request::Sleep(0) => {
                self.sleep_until = None;
                self.message = "Sleep timer cancelled".into();
            }
            Request::Enqueue { track, next, play } => {
                validate_track(&track)?;
                if play {
                    ensure!(
                        crate::backend::worker::available(&self.paths),
                        crate::backend::NATIVE_UNAVAILABLE
                    );
                    Control::Start {
                        id: track.id.clone(),
                        volume: self.playback.volume,
                    }
                    .validate()?;
                }
                ensure!(
                    track.kind == "songs" || track.kind == "library-songs",
                    "open this album/artist/playlist first"
                );
                ensure!(
                    self.queue.tracks.len() < self.config.max_queue,
                    "queue limit reached"
                );
                let i = self.queue.tracks.len();
                self.queue.push(track)?;
                self.message = "Added to queue".into();
                if next {
                    let target = self.queue.current.map_or(0, |i| i + 1);
                    self.queue.move_item(i, target);
                }
                if play {
                    let index = if next {
                        self.queue.current.map_or(0, |v| v + 1)
                    } else {
                        i
                    };
                    self.start_audio(index).await?;
                }
                self.save()?;
                save = true;
            }
            Request::QueueEdit {
                revision,
                index,
                id,
                to,
            } => {
                ensure!(
                    revision == self.queue.revision
                        && self
                            .queue
                            .tracks
                            .get(index)
                            .is_some_and(|track| track.id == id),
                    "Queue changed; refresh it before editing this track"
                );
                if let Some(target) = to {
                    ensure!(
                        self.queue.move_item(index, target),
                        "Queue index out of range"
                    );
                } else {
                    self.queue.remove(index);
                }
                save = true;
            }
            Request::Remove(i) => {
                ensure!(i < self.queue.tracks.len(), "queue index out of range");
                if self.queue.current == Some(i) {
                    self.dismissed = self.playback.track.as_ref().map(|t| t.id.clone());
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
                self.queue.clear();
                save = true;
            }
            Request::Theme(theme) => {
                let mut c = self.config.clone();
                c.theme = theme;
                Config::parse(&toml::to_string(&c)?)?;
                self.config = c;
                self.config.save(&self.paths)?;
            }
            Request::Profile(profile) => {
                self.config.performance = profile;
                self.catalog.set_capacity(profile.limits().cache_entries);
                self.config.save(&self.paths)?;
            }
            Request::Artwork(enabled) => {
                self.config.artwork = enabled;
                self.config.save(&self.paths)?;
            }
            Request::Lyrics(enabled) => {
                self.config.lyrics = enabled;
                self.config.save(&self.paths)?;
            }
            Request::CatalogCountry(country) => {
                let mut config = self.config.clone();
                config.catalog_country = country;
                config = Config::parse(&toml::to_string(&config)?)?;
                config.save(&self.paths)?;
                self.config = config;
            }
            Request::Stop => self.shutdown = true,
        }
        if save {
            self.save()?;
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
    ensure!(
        t.url
            .as_ref()
            .is_none_or(|url| url.len() <= 2048 && !url.chars().any(char::is_control)),
        "invalid catalog link"
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
    let catalog = engine.catalog.clone();
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
                    let Ok(permit)=permits.clone().try_acquire_owned() else {continue;};let sender=tx.clone();let mut updates=subscriber.clone();let metadata=catalog.clone();
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
                            let result=if let Request::Browse {kind,term,next}=request {
                                let status=updates.borrow().clone();
                                let page=metadata.browse(&kind,&term,next.as_deref(),&status.catalog_country,status.profile.limits()).await;
                                if page.is_ok() && kind.starts_with("search:") && !term.trim().is_empty() {let _=send(&sender,Request::RecordSearch(term.trim().into())).await;}
                                page.and_then(|p|serde_json::to_value(p).map_err(Into::into))
                            } else {send(&sender,request).await};
                            let r=Reply::result(result);write_frame(&mut s,&r).await
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

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            Some((generation,event))=engine.events.recv()=>{engine.native_event(generation,event).await;state_tx.send_replace(engine.status());}
            _=timer.tick()=>{
                if engine.sleep_until.is_some_and(|until|Instant::now()>=until){engine.sleep_until=None;let _=engine.audio_control(Control::Pause(true)).await;engine.message="Sleep timer paused playback".into();}
            }
            Some(e)=rx.recv()=>{
                let r=engine.handle(e.request).await;
                if let Err(ref error)=r {engine.message=error.to_string();}
                let _=e.reply.send(Reply::result(r));
                state_tx.send_replace(engine.status());
                if engine.shutdown{break;}
            }
            _=tokio::signal::ctrl_c()=>break,
            _=term.recv()=>break,
        }
    }
    drop(state_tx);
    if let Some(audio) = engine.audio.take() {
        audio.stop().await;
    }
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
        let config = Config::default();
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
    async fn native_events_require_the_current_worker_and_audio_progress() {
        let (_dir, mut engine) = engine();
        engine
            .native_event(
                99,
                crate::backend::worker::Event::Ready {
                    duration: 359.0,
                    sample_rate: 44100,
                    channels: 2,
                },
            )
            .await;
        assert!(!engine.playback.authorized);
        engine
            .native_event(
                0,
                crate::backend::worker::Event::Ready {
                    duration: 359.0,
                    sample_rate: 44100,
                    channels: 2,
                },
            )
            .await;
        assert!(engine.playback.authorized);
        assert!(!engine.playback.playing);
        engine
            .native_event(
                0,
                crate::backend::worker::Event::State {
                    playing: true,
                    position: 1.0,
                    encrypted_packets: 45,
                    decoded_frames: 45,
                    nonzero_samples: 1000,
                },
            )
            .await;
        assert!(engine.playback.playing);
        assert_eq!(engine.playback.position, 1.0);
    }
    #[tokio::test]
    async fn stale_queue_play_cannot_start_a_different_song() {
        let (_dir, mut engine) = engine();
        let revision = engine.queue.revision;
        for (expected, id) in [(revision - 1, "a"), (revision, "b")] {
            let error = engine
                .handle(Request::PlayQueued {
                    revision: expected,
                    index: 0,
                    id: id.into(),
                })
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("Queue changed"), "{error}");
            assert_eq!(engine.queue.current, Some(0));
        }
    }
    #[tokio::test]
    async fn failed_next_does_not_change_current_selection() {
        let (_dir, mut e) = engine();
        assert!(e.handle(Request::Next).await.is_err());
        assert_eq!(e.queue.current, Some(0));
    }
    #[tokio::test]
    async fn checked_queue_edits_reject_changed_revision_or_track_identity() {
        let (_dir, mut engine) = engine();
        let revision = engine.queue.revision;
        for (expected, id) in [(revision - 1, "a"), (revision, "b")] {
            let request:Request=serde_json::from_value(json!({"command":"queue_edit","args":{"revision":expected,"index":0,"id":id,"to":null}})).unwrap();
            assert!(engine.handle(request).await.is_err());
            assert_eq!(engine.queue.tracks[0].id, "a");
        }
        let request:Request=serde_json::from_value(json!({"command":"queue_edit","args":{"revision":revision,"index":0,"id":"a","to":null}})).unwrap();
        engine.handle(request).await.unwrap();
        assert_eq!(engine.queue.tracks[0].id, "b");
    }
    #[tokio::test]
    async fn disabled_playback_commands_explain_missing_engine_without_mutating_queue() {
        let (_dir, mut e) = engine();
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
}

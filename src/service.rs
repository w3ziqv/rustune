mod audio;
mod operations;
mod persistence;
mod playlist_journal;
mod playlist_write;
use crate::backend::driver::{AudioDriver, DriverFactory, NativeFactory};
use operations::{AlbumEvent, DiskDone, Operation, Pending, Preparing};
use persistence::Receipt;
use playlist_write::PlaylistWriteStatus;
#[cfg(test)]
mod playlist_tests;
#[cfg(test)]
mod session_tests;
use crate::backend::worker::{Control, Event};
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
    #[serde(default)]
    pub equalizer: rustune_core::equalizer::EqSettings,
    #[serde(default)]
    pub equalizer_applied: Option<rustune_core::equalizer::EqSettings>,
    #[serde(default)]
    pub equalizer_headroom_db: f64,
    #[serde(default)]
    pub equalizer_samples: u64,
    #[serde(default)]
    pub equalizer_input_rms: f64,
    #[serde(default)]
    pub equalizer_output_rms: f64,
    pub playback: Playback,
    pub queue_len: usize,
    #[serde(default)]
    pub queue_revision: u64,
    pub current: Option<usize>,
    #[serde(default)]
    pub current_entry_id: Option<u64>,
    #[serde(default)]
    pub next_entry_id: Option<u64>,
    #[serde(default)]
    pub can_next: bool,
    #[serde(default)]
    pub can_previous: bool,
    #[serde(default)]
    pub operation: Option<Operation>,
    #[serde(default)]
    pub audio_error: Option<String>,
    #[serde(default)]
    pub config_error: Option<String>,
    #[serde(default)]
    pub playlist_write: Option<PlaylistWriteStatus>,
    #[serde(default)]
    pub undo_available: bool,
    #[serde(default)]
    pub recovery_required: bool,
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
    equalizer_applied: Option<rustune_core::equalizer::EqSettings>,
    equalizer_headroom_db: f64,
    equalizer_samples: u64,
    equalizer_input_rms: f64,
    equalizer_output_rms: f64,
    config: Config,
    paths: Paths,
    queue: Queue,
    playback: Playback,
    catalog: Arc<Catalog>,
    history: Vec<String>,
    started: Instant,
    message: String,
    shutdown: bool,
    audio: Option<Box<dyn AudioDriver>>,
    factory: Arc<dyn DriverFactory>,
    desired_paused: bool,
    audio_error: Option<String>,
    transport_epoch: u64,
    staged_transport_epoch: u64,
    deferred_ended: bool,
    receipts: Vec<Receipt>,
    undo: Option<Queue>,
    pending: Option<Pending>,
    disk_tx: mpsc::Sender<DiskDone>,
    disk_events: mpsc::Receiver<DiskDone>,
    preparing: Option<Preparing>,
    album_generation: u64,
    album_tx: mpsc::Sender<AlbumEvent>,
    album_events: mpsc::Receiver<AlbumEvent>,
    operation: Option<Operation>,
    playlist_write_status: Option<PlaylistWriteStatus>,
    playlist_write_task: Option<tokio::task::JoinHandle<()>>,
    playlist_write_retry: Option<Request>,
    playlist_write_tx: mpsc::Sender<PlaylistWriteStatus>,
    playlist_write_events: mpsc::Receiver<PlaylistWriteStatus>,
    retry_request: Option<Request>,
    recovery_required: bool,
    uncertain_disk: bool,
    config_tx: watch::Sender<Option<Config>>,
    config_results: watch::Receiver<Option<String>>,
    config_worker: tokio::task::JoinHandle<()>,
    audio_generation: u64,
    event_tx: mpsc::Sender<(u64, Event)>,
    events: mpsc::Receiver<(u64, Event)>,
    sleep_until: Option<Instant>,
    unmuted_volume: f64,
}
impl Engine {
    fn new(config: Config, paths: Paths) -> Result<Self> {
        let restored = persistence::load(&paths, config.max_queue);
        let recovery_required = restored.is_err();
        let message=if recovery_required {"Saved state retained: recovery required. Use recover-state to preserve it and start a new session."}else{"Ready. Album actions load the complete album; queue restore never starts audio."}.to_owned();
        let mut saved = restored.unwrap_or_default();
        if !config.restore_queue {
            saved.queue.clear();
        }
        let catalog = Arc::new(Catalog::new(&paths)?);
        catalog.set_capacity(config.performance.limits().cache_entries);
        let playback = Playback {
            volume: config.volume,
            ..Playback::default()
        };
        let (event_tx, events) = mpsc::channel(32);
        let unmuted_volume = config.volume;
        let (disk_tx, disk_events) = mpsc::channel(1);
        let (album_tx, album_events) = mpsc::channel(4);
        let (playlist_write_tx, playlist_write_events) = mpsc::channel(1);
        let (config_tx, mut config_rx) = watch::channel(Some(config.clone()));
        let (config_result_tx, config_results) = watch::channel(None);
        let config_paths = paths.clone();
        let config_worker = tokio::spawn(async move {
            while config_rx.changed().await.is_ok() {
                let Some(c) = config_rx.borrow_and_update().clone() else {
                    break;
                };
                let p = config_paths.clone();
                let result = tokio::task::spawn_blocking(move || c.save(&p)).await;
                config_result_tx.send_replace(if matches!(result,Ok(Ok(()))) {None}else{Some("Configuration could not be saved; check free space and directory permissions".into())});
            }
        });
        Ok(Self {
            equalizer_applied: None,
            equalizer_headroom_db: 0.0,
            equalizer_samples: 0,
            equalizer_input_rms: 0.0,
            equalizer_output_rms: 0.0,
            config,
            paths,
            queue: saved.queue,
            playback,
            catalog,
            history: saved.history,
            started: Instant::now(),
            message,
            shutdown: false,
            audio: None,
            factory: Arc::new(NativeFactory),
            desired_paused: false,
            audio_error: None,
            transport_epoch: 0,
            staged_transport_epoch: 0,
            deferred_ended: false,
            receipts: saved.receipts,
            undo: None,
            pending: None,
            disk_tx,
            disk_events,
            preparing: None,
            album_generation: 0,
            album_tx,
            album_events,
            operation: None,
            playlist_write_status: None,
            playlist_write_task: None,
            playlist_write_retry: None,
            playlist_write_tx,
            playlist_write_events,
            retry_request: None,
            recovery_required,
            uncertain_disk: false,
            config_tx,
            config_results,
            config_worker,
            audio_generation: 0,
            event_tx,
            events,
            sleep_until: None,
            unmuted_volume,
        })
    }
    fn status(&self) -> Status {
        Status {
            equalizer: self.config.equalizer.clone(),
            equalizer_applied: self.equalizer_applied.clone(),
            equalizer_headroom_db: self.equalizer_headroom_db,
            equalizer_samples: self.equalizer_samples,
            equalizer_input_rms: self.equalizer_input_rms,
            equalizer_output_rms: self.equalizer_output_rms,
            playback: self.playback.clone(),
            queue_len: self.queue.tracks.len(),
            queue_revision: self.queue.revision,
            current: self.queue.current,
            current_entry_id: self.queue.current_entry_id(),
            next_entry_id: self
                .queue
                .candidate(true)
                .map(|i| self.queue.tracks[i].entry_id),
            can_next: self.queue.candidate(true).is_some(),
            can_previous: self.queue.previous_candidate().is_some() || self.playback.position > 3.0,
            operation: self.operation.clone(),
            audio_error: self.audio_error.clone(),
            config_error: self.config_results.borrow().clone(),
            playlist_write: self.playlist_write_status.clone(),
            undo_available: self.undo.is_some(),
            recovery_required: self.recovery_required,
            repeat: self.queue.repeat,
            shuffle: self.queue.shuffle,
            profile: self.config.performance,
            catalog_country: self.config.catalog_country.clone(),
            pid: std::process::id(),
            browser_pid: None,
            audio_pid: self.audio.as_ref().and_then(|a| a.pid()),
            backend: "Native CDM / AAC / ALSA".into(),
            playback_available: self.audio.is_some() || self.factory.available(&self.paths),
            start_available: self.factory.available(&self.paths),
            browser_shared: false,
            uptime_secs: self.started.elapsed().as_secs(),
            cache_entries: self.catalog.cache_entries(),
            message: self.message.clone(),
            sleep_remaining: self
                .sleep_until
                .map(|until| until.saturating_duration_since(Instant::now()).as_secs()),
        }
    }
    async fn handle(&mut self, request: Request) -> Result<Value> {
        use crate::ipc::QueueAction;
        let fingerprint = serde_json::to_string(&request)?;
        let mut save = false;
        match &request {
            Request::QueueAction {
                key,
                revision,
                action,
            } => {
                self.retry_request = Some(request.clone());
                self.queue_action(key.clone(), *revision, action.clone(), fingerprint)
                    .await?;
                return Ok(serde_json::to_value(self.status())?);
            }
            Request::Album { .. } | Request::Playlist { .. } => {
                self.begin_album(request)?;
                return Ok(serde_json::to_value(self.status())?);
            }
            _ => {}
        }
        match request {
            Request::PlaylistWrite { .. } => self.begin_playlist_write(request)?,
            Request::RetryPlaylistWrite => {
                ensure!(
                    self.playlist_write_status
                        .as_ref()
                        .is_some_and(|s| s.can_retry),
                    "Playlist outcome needs review or is already confirmed; automatic retry is disabled"
                );
                let request = self
                    .playlist_write_retry
                    .clone()
                    .context("No playlist operation to retry")?;
                self.begin_playlist_write(request)?;
            }
            Request::Equalizer(settings) => {
                settings.validate()?;
                if self.audio.is_some() {
                    self.audio_control(Control::Equalizer(settings.clone()))
                        .await?;
                }
                self.config.equalizer = settings;
                self.config_tx.send_replace(Some(self.config.clone()));
                self.message = if self.audio.is_some() {
                    "Applying equalizer to native audio…"
                } else {
                    "Equalizer applied locally; saving configuration in background"
                }
                .into();
            }
            Request::PlayQueued {
                revision,
                index,
                id,
            } => {
                ensure!(
                    revision == self.queue.revision
                        && self.queue.tracks.get(index).is_some_and(|t| t.id == id),
                    "Queue changed; refresh before playing"
                );
                let entry_id = self.queue.tracks[index].entry_id;
                self.queue_action(
                    crate::ipc::operation_key(),
                    revision,
                    QueueAction::PlayEntry { entry_id },
                    fingerprint,
                )
                .await?;
            }
            Request::Subscribe => anyhow::bail!("subscription requires a dedicated connection"),
            Request::Status => return Ok(serde_json::to_value(self.status())?),
            Request::QueuePage { offset } => {
                return Ok(
                    json!({"items":self.queue.tracks.iter().skip(offset).take(self.config.performance.limits().page_size).collect::<Vec<_>>(),"offset":offset,"total":self.queue.tracks.len(),"revision":self.queue.revision,"upcoming":self.queue.upcoming_ids()}),
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
                if !self.factory.available(&self.paths) {
                    self.fail_audio(crate::backend::NATIVE_UNAVAILABLE.into())
                        .await;
                    anyhow::bail!(crate::backend::NATIVE_UNAVAILABLE);
                }
                let index = self
                    .queue
                    .current
                    .or_else(|| (!self.queue.tracks.is_empty()).then_some(0))
                    .context("The queue is empty; choose a song first")?;
                if self.queue.current.is_none() {
                    self.queue_action(
                        crate::ipc::operation_key(),
                        self.queue.revision,
                        QueueAction::PlayEntry {
                            entry_id: self.queue.tracks[index].entry_id,
                        },
                        fingerprint,
                    )
                    .await?;
                    return Ok(serde_json::to_value(self.status())?);
                }
                if let Err(error) = self.start_audio(index).await {
                    self.fail_audio(error.to_string()).await;
                    return Err(error);
                }
            }
            Request::Play | Request::Pause | Request::Toggle => {
                let paused = match request {
                    Request::Pause => true,
                    Request::Play => false,
                    _ => !self.desired_paused,
                };
                self.audio_control(Control::Pause(paused)).await?;
                self.desired_paused = paused;
                self.transport_epoch += 1;
            }
            Request::Next | Request::Previous => {
                ensure!(
                    self.factory.available(&self.paths),
                    crate::backend::NATIVE_UNAVAILABLE
                );
                ensure!(
                    self.pending.is_none(),
                    "Queue write in progress; retry navigation"
                );
                if matches!(request, Request::Previous) && self.playback.position > 3.0 {
                    self.audio_control(Control::Seek(0.0)).await?;
                } else {
                    let mut q = self.queue.clone();
                    let next = if matches!(request, Request::Next) {
                        q.advance(true)
                    } else {
                        q.previous()
                    };
                    if let Some(index) = next {
                        let id = q.tracks[index].entry_id;
                        self.stage_navigation(
                            q,
                            operations::After::Start {
                                entry_id: id,
                                paused: self.desired_paused,
                            },
                            crate::ipc::operation_key(),
                        )?;
                    } else if matches!(request, Request::Next) {
                        self.stop_audio().await;
                    } else {
                        anyhow::bail!("No previous track in listening history");
                    }
                }
            }
            Request::PlayIndex(_) => {
                ensure!(
                    self.factory.available(&self.paths),
                    crate::backend::NATIVE_UNAVAILABLE
                );
                anyhow::bail!(
                    "Positional playback is obsolete; use PlayEntry with entry ID and revision"
                );
            }
            Request::Seek(position) => {
                if self.audio.is_none() {
                    ensure!(
                        self.factory.available(&self.paths),
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
                self.config_tx.send_replace(Some(self.config.clone()));
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
                    self.factory.available(&self.paths),
                    crate::backend::NATIVE_UNAVAILABLE
                );
                ensure!(seconds <= 86400, "Sleep timer is limited to 24 hours");
                self.sleep_until = Some(Instant::now() + Duration::from_secs(seconds));
            }
            Request::Shuffle => {
                self.queue_action(
                    crate::ipc::operation_key(),
                    self.queue.revision,
                    QueueAction::SetShuffle {
                        enabled: !self.queue.shuffle,
                    },
                    fingerprint,
                )
                .await?;
            }
            Request::Repeat => {
                let repeat = match self.queue.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
                self.queue_action(
                    crate::ipc::operation_key(),
                    self.queue.revision,
                    QueueAction::SetRepeat { repeat },
                    fingerprint,
                )
                .await?;
            }
            Request::Sleep(0) => {
                self.sleep_until = None;
                self.message = "Sleep timer cancelled".into();
            }
            Request::Enqueue { track, next, play } => {
                let context = crate::queue::QueueContext::single();
                let action = if play {
                    QueueAction::PlayContext {
                        tracks: vec![track],
                        context,
                        start: 0,
                        shuffle: false,
                    }
                } else if next {
                    QueueAction::PlayNext {
                        tracks: vec![track],
                        context,
                    }
                } else {
                    QueueAction::Append {
                        tracks: vec![track],
                        context,
                    }
                };
                self.queue_action(
                    crate::ipc::operation_key(),
                    self.queue.revision,
                    action,
                    fingerprint,
                )
                .await?;
            }
            Request::QueueEdit {
                revision,
                index,
                id,
                to,
            } => {
                ensure!(
                    revision == self.queue.revision
                        && self.queue.tracks.get(index).is_some_and(|t| t.id == id),
                    "Queue changed; refresh before editing"
                );
                let entry_id = self.queue.tracks[index].entry_id;
                let action = if let Some(to) = to {
                    ensure!(to < self.queue.tracks.len(), "Queue index out of range");
                    let target = if to > index { to + 1 } else { to };
                    QueueAction::Move {
                        entry_id,
                        before: self.queue.tracks.get(target).map(|e| e.entry_id),
                    }
                } else {
                    QueueAction::RemoveAndAdvance { entry_id }
                };
                self.queue_action(crate::ipc::operation_key(), revision, action, fingerprint)
                    .await?;
            }
            Request::Remove(_) | Request::Move { .. } => anyhow::bail!(
                "Positional edits are obsolete; use QueueAction with entry ID and revision"
            ),
            Request::Clear => {
                self.queue_action(
                    crate::ipc::operation_key(),
                    self.queue.revision,
                    QueueAction::ClearUpcoming,
                    fingerprint,
                )
                .await?;
            }
            Request::Theme(theme) => {
                let mut c = self.config.clone();
                c.theme = theme;
                Config::parse(&toml::to_string(&c)?)?;
                self.config = c;
                self.config_tx.send_replace(Some(self.config.clone()));
            }
            Request::Profile(profile) => {
                self.config.performance = profile;
                self.catalog.set_capacity(profile.limits().cache_entries);
                self.config_tx.send_replace(Some(self.config.clone()));
            }
            Request::Artwork(enabled) => {
                self.config.artwork = enabled;
                self.config_tx.send_replace(Some(self.config.clone()));
            }
            Request::Lyrics(enabled) => {
                self.config.lyrics = enabled;
                self.config_tx.send_replace(Some(self.config.clone()));
            }
            Request::CatalogCountry(country) => {
                let mut config = self.config.clone();
                config.catalog_country = country;
                config = Config::parse(&toml::to_string(&config)?)?;
                self.config_tx.send_replace(Some(config.clone()));
                self.config = config;
            }
            Request::CancelOperation(key) => self.cancel_operation(&key)?,
            Request::RetryOperation => {
                let mut request = self
                    .retry_request
                    .clone()
                    .context("No operation to retry")?;
                match &mut request {
                    Request::Album { revision, key, .. }
                    | Request::Playlist { revision, key, .. }
                    | Request::QueueAction { revision, key, .. } => {
                        if !self.receipts.iter().any(|r| r.key == *key) {
                            *revision = self.queue.revision;
                        }
                    }
                    _ => anyhow::bail!("This operation cannot be retried"),
                }
                let fingerprint = serde_json::to_string(&request)?;
                match request {
                    Request::Album { .. } | Request::Playlist { .. } => {
                        self.begin_album(request)?
                    }
                    Request::QueueAction {
                        key,
                        revision,
                        action,
                    } => {
                        self.queue_action(key, revision, action, fingerprint)
                            .await?
                    }
                    _ => unreachable!("validated retry"),
                }
            }
            Request::StopAudio => {
                self.stop_audio().await;
                self.transport_epoch += 1;
                self.deferred_ended = false;
                if let Some(pending) = &mut self.pending {
                    pending.after = operations::After::Stop;
                }
            }
            Request::Shutdown | Request::Stop => self.shutdown = true,
            Request::RecoverState => {
                ensure!(
                    !self.uncertain_disk,
                    "Installed state has uncertain durability; restart service to reconcile it without discarding the file"
                );
                ensure!(self.recovery_required, "No saved state needs recovery");
                let path = self.paths.data.join("state.json");
                let dest = self.paths.data.join(format!(
                    "state.recovery-{}.json",
                    crate::ipc::operation_key()
                ));
                fs::rename(path, dest).map_err(|_| {
                    anyhow::anyhow!("Could not preserve invalid state for recovery")
                })?;
                self.recovery_required = false;
                self.message="Invalid state preserved in state.recovery-*.json; previous valid state remains state.previous.json".into();
            }
            Request::QueueAction { .. } | Request::Album { .. } | Request::Playlist { .. } => {
                unreachable!("handled above")
            }
        }
        if save && self.pending.is_none() && !self.recovery_required {
            self.stage_navigation(
                self.queue.clone(),
                operations::After::Keep,
                crate::ipc::operation_key(),
            )?;
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
        [&t.provider, &t.storefront]
            .iter()
            .all(|v| v.len() <= 32 && !v.chars().any(char::is_control))
            && t.album_id.as_ref().is_none_or(|v| v.len() <= 128
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
            && t.unavailable
                .as_ref()
                .is_none_or(|v| v.len() <= 256 && !v.chars().any(char::is_control)),
        "invalid track context"
    );
    ensure!(
        t.source_index.is_none_or(|i| i < crate::queue::MAX_QUEUE),
        "invalid source position"
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
    run_with(config, paths, Arc::new(NativeFactory), None, None).await
}
pub(crate) async fn run_with(
    config: Config,
    paths: Paths,
    factory: Arc<dyn DriverFactory>,
    catalog: Option<Arc<Catalog>>,
    bus_address: Option<String>,
) -> Result<()> {
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
    engine.factory = factory;
    if let Some(catalog) = catalog {
        engine.catalog = catalog;
    }
    let (state_tx, state_rx) = watch::channel(engine.status());
    let subscriber = state_rx.clone();
    let catalog = engine.catalog.clone();
    let mpris = crate::mpris::start_at(tx.clone(), state_rx, bus_address.as_deref()).await;
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
            _=engine.config_results.changed()=>{state_tx.send_replace(engine.status());}
            Some(status)=engine.playlist_write_events.recv()=>{engine.playlist_write_status=Some(status);engine.playlist_write_task=None;state_tx.send_replace(engine.status());}
            Some(done)=engine.disk_events.recv()=>{engine.disk_complete(done).await;state_tx.send_replace(engine.status());}
            Some(event)=engine.album_events.recv()=>{engine.album_event(event).await;state_tx.send_replace(engine.status());}
            Some((generation,event))=engine.events.recv()=>{engine.native_event(generation,event).await;state_tx.send_replace(engine.status());}
            _=timer.tick()=>{
                if engine.sleep_until.is_some_and(|until|Instant::now()>=until){engine.sleep_until=None;let _=engine.audio_control(Control::Pause(true)).await;engine.desired_paused=true;engine.transport_epoch+=1;engine.message="Sleep timer paused playback".into();}
            }
            Some(e)=rx.recv()=>{
                let was_pending=engine.pending.is_some();
                let r=engine.handle(e.request).await;
                if let Err(ref error)=r {engine.message=error.to_string();}
                if !was_pending && r.is_ok() && engine.pending.as_ref().is_some_and(|p|p.reply.is_none()) {engine.pending.as_mut().expect("checked pending").reply=Some(e.reply);} else {let _=e.reply.send(Reply::result(r));}
                state_tx.send_replace(engine.status());
                if engine.shutdown{break;}
            }
            _=tokio::signal::ctrl_c()=>break,
            _=term.recv()=>break,
        }
    }
    if let Some(preparing) = engine.preparing.take() {
        preparing.task.abort();
    }
    if engine.pending.is_some()
        && let Some(done) = engine.disk_events.recv().await
    {
        engine.disk_complete(done).await;
    }
    if let Some(mut task) = engine.playlist_write_task.take()
        && tokio::time::timeout(Duration::from_secs(12), &mut task)
            .await
            .is_err()
    {
        task.abort();
        let _ = task.await;
    }
    engine.config_tx.send_replace(None);
    let _ = engine.config_worker.await;
    let final_config = engine.config.clone();
    let final_paths = paths.clone();
    let _ = tokio::task::spawn_blocking(move || final_config.save(&final_paths)).await;
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
    pub(super) fn engine() -> (tempfile::TempDir, Engine) {
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
        let done = engine.disk_events.recv().await.unwrap();
        engine.disk_complete(done).await;
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

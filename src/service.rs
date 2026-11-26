mod audio;
mod connections;
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
    ipc::Request,
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
    sync::{mpsc, oneshot, watch},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Appearance {
    pub theme: String,
    pub artwork: bool,
    pub artwork_style: crate::config::ArtworkStyle,
    pub lyrics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    #[serde(default)]
    pub helper_protocol: Option<u32>,
    #[serde(default)]
    pub client_pids: Vec<u32>,
    #[serde(default)]
    pub appearance: Option<Appearance>,
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
    #[serde(default)]
    pub account_generation: Option<u64>,
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
impl Status {
    pub fn can_play(&self) -> bool {
        self.audio_pid.is_some() || (self.start_available && self.queue_len > 0)
    }
    pub fn can_pause(&self) -> bool {
        self.audio_pid.is_some() && self.playback.track.is_some()
    }
    pub fn can_seek(&self) -> bool {
        self.can_pause() && self.playback.duration > 0.0
    }
    pub fn can_go_next(&self) -> bool {
        self.start_available && self.can_next
    }
    pub fn can_go_previous(&self) -> bool {
        self.start_available && self.can_previous
    }
    pub fn playback_status(&self) -> &'static str {
        if self.playback.playing {
            "Playing"
        } else if self.playback.phase == crate::model::PlaybackPhase::Paused {
            "Paused"
        } else {
            "Stopped"
        }
    }
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

struct ClientRegistration {
    clients: Arc<std::sync::Mutex<std::collections::BTreeMap<u32, usize>>>,
    pid: Option<u32>,
}
impl ClientRegistration {
    fn new(
        clients: Arc<std::sync::Mutex<std::collections::BTreeMap<u32, usize>>>,
        pid: Option<u32>,
    ) -> Self {
        if let Some(pid) = pid
            && let Ok(mut entries) = clients.lock()
        {
            *entries.entry(pid).or_default() += 1;
        }
        Self { clients, pid }
    }
}
impl Drop for ClientRegistration {
    fn drop(&mut self) {
        if let Some(pid) = self.pid
            && let Ok(mut entries) = self.clients.lock()
            && let Some(count) = entries.get_mut(&pid)
        {
            *count -= 1;
            if *count == 0 {
                entries.remove(&pid);
            }
        }
    }
}

struct Engine {
    clients: Arc<std::sync::Mutex<std::collections::BTreeMap<u32, usize>>>,
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
    last_seek_sent: u64,
    last_seek_applied: u64,
    pending_position: Option<f64>,
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
            track: saved.queue.current_track().cloned(),
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
            clients: Arc::default(),
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
            factory: Arc::new(NativeFactory::default()),
            desired_paused: false,
            last_seek_sent: 0,
            last_seek_applied: 0,
            pending_position: None,
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
        let account_generation = self.catalog.account_generation();
        Status {
            helper_protocol: Some(rustune_core::protocol::PROTOCOL_VERSION),
            client_pids: self
                .clients
                .lock()
                .map(|v| v.keys().copied().collect())
                .unwrap_or_default(),
            appearance: Some(Appearance {
                theme: self.config.theme.clone(),
                artwork: self.config.artwork,
                artwork_style: self.config.artwork_style,
                lyrics: self.config.lyrics,
            }),
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
            can_previous: self.queue.previous_candidate().is_some(),
            operation: self.operation.clone(),
            audio_error: self.audio_error.clone(),
            config_error: self.config_results.borrow().clone(),
            playlist_write: self
                .playlist_write_status
                .as_ref()
                .filter(|write| {
                    account_generation.is_some() && write.account_generation == account_generation
                })
                .cloned(),
            undo_available: self.undo.is_some(),
            recovery_required: self.recovery_required,
            repeat: self.queue.repeat,
            shuffle: self.queue.shuffle,
            profile: self.config.performance,
            catalog_country: self.config.catalog_country.clone(),
            account_generation,
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
        // Interpret remote transport intents only when dequeued by the service.
        let request = match request {
            Request::SetPosition {
                entry_id,
                position_us,
            } => {
                let status = self.status();
                let length = crate::model::micros(status.playback.duration);
                if !status.can_seek()
                    || status.current_entry_id != Some(entry_id)
                    || position_us < 0
                    || position_us > length
                {
                    return Ok(serde_json::to_value(status)?);
                }
                Request::Seek(if position_us == length {
                    status.playback.duration
                } else {
                    position_us as f64 / 1_000_000.0
                })
            }
            Request::SeekBy(offset) => {
                let target = self.pending_position.unwrap_or(self.playback.position)
                    + offset as f64 / 1_000_000.0;
                Request::Seek(target.max(0.0).min(self.playback.duration))
            }
            Request::MprisToggle => {
                let status = self.status();
                ensure!(
                    status.can_pause() || status.can_play(),
                    "No playable track is available"
                );
                Request::Toggle
            }
            Request::RelativeSeek(offset) => {
                let status = self.status();
                if !status.can_seek() {
                    return Ok(serde_json::to_value(status)?);
                }
                let current =
                    crate::model::micros(self.pending_position.unwrap_or(status.playback.position));
                let target = current.saturating_add(offset).max(0);
                let length = crate::model::micros(status.playback.duration);
                if target > length {
                    Request::MprisNext
                } else if target == length {
                    Request::Seek(status.playback.duration)
                } else {
                    Request::Seek(target as f64 / 1_000_000.0)
                }
            }
            Request::SetRepeat(v) if v == self.queue.repeat && self.pending.is_none() => {
                return Ok(serde_json::to_value(self.status())?);
            }
            Request::SetShuffle(v) if v == self.queue.shuffle && self.pending.is_none() => {
                return Ok(serde_json::to_value(self.status())?);
            }
            Request::MprisPlay if self.status().can_play() => Request::Play,
            Request::MprisPause if self.status().can_pause() => Request::Pause,
            Request::MprisPlay | Request::MprisPause => {
                return Ok(serde_json::to_value(self.status())?);
            }
            other => other,
        };
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
            Request::PlaylistWrite {
                account_generation, ..
            } => {
                let generation = account_generation
                    .context("Confirm the playlist write under the current account")?;
                self.begin_playlist_write(request, generation)?;
            }
            Request::RetryPlaylistWrite => {
                let expected = self.playlist_write_status.as_ref()
                    .filter(|status| status.can_retry)
                    .and_then(|status| status.account_generation)
                    .context("Playlist outcome needs review or is already confirmed; automatic retry is disabled")?;
                let request = self
                    .playlist_write_retry
                    .clone()
                    .context("No playlist operation to retry")?;
                self.begin_playlist_write(request, expected)?;
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
            Request::QueuePage { offset, annotated } => {
                let page: Vec<_> = self
                    .queue
                    .tracks
                    .iter()
                    .skip(offset)
                    .take(self.config.performance.limits().page_size)
                    .collect();
                let payload = if annotated {
                    // Bounded per-rows ranks keep paged scrolling wire-sized
                    // independently of the whole queue length.
                    let wanted: std::collections::HashSet<u64> =
                        page.iter().map(|entry| entry.entry_id).collect();
                    let ranks = self.queue.upcoming_ranks_for(&wanted);
                    let annotations: serde_json::Map<String, serde_json::Value> = page
                        .iter()
                        .map(|entry| {
                            let rank = ranks.get(&entry.entry_id);
                            (
                                entry.entry_id.to_string(),
                                rank.map_or(serde_json::Value::Null, |rank| {
                                    serde_json::json!(rank)
                                }),
                            )
                        })
                        .collect();
                    json!({"items":page,"offset":offset,"total":self.queue.tracks.len(),"revision":self.queue.revision,"upcoming_ranks":annotations})
                } else {
                    json!({"items":page,"offset":offset,"total":self.queue.tracks.len(),"revision":self.queue.revision,"upcoming":self.queue.upcoming_ids()})
                };
                return Ok(payload);
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
                    self.fail_audio(self.factory.unavailable_reason().into())
                        .await;
                    anyhow::bail!(self.factory.unavailable_reason());
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
            Request::Next | Request::Previous | Request::MprisNext | Request::MprisPrevious => {
                let remote = matches!(request, Request::MprisNext | Request::MprisPrevious);
                let previous = matches!(request, Request::Previous | Request::MprisPrevious);
                if remote {
                    let status = self.status();
                    if (previous && !status.can_go_previous())
                        || (!previous && !status.can_go_next())
                    {
                        return Ok(serde_json::to_value(status)?);
                    }
                }
                if !remote && previous && self.playback.position > 3.0 && self.audio.is_some() {
                    self.seek(0.0).await?;
                } else {
                    ensure!(
                        self.factory.available(&self.paths),
                        self.factory.unavailable_reason()
                    );
                    ensure!(
                        self.pending.is_none(),
                        "Queue write in progress; retry navigation"
                    );
                    let mut q = self.queue.clone();
                    let index = if previous {
                        q.previous()
                    } else {
                        q.advance(true)
                    };
                    if let Some(index) = index {
                        let after = if remote && self.audio.is_none() {
                            operations::After::Stop
                        } else {
                            operations::After::Start {
                                entry_id: q.tracks[index].entry_id,
                                paused: self.desired_paused,
                            }
                        };
                        self.stage_navigation(q, after, crate::ipc::operation_key())?;
                    } else if previous {
                        anyhow::bail!("No previous track in listening history");
                    } else {
                        self.stop_audio().await;
                    }
                }
            }
            Request::PlayIndex(_) => {
                ensure!(
                    self.factory.available(&self.paths),
                    self.factory.unavailable_reason()
                );
                anyhow::bail!(
                    "Positional playback is obsolete; use PlayEntry with entry ID and revision"
                );
            }
            Request::Seek(position) => {
                if self.audio.is_none() {
                    ensure!(
                        self.factory.available(&self.paths),
                        self.factory.unavailable_reason()
                    );
                }
                ensure!(
                    position.is_finite()
                        && position >= 0.0
                        && position <= self.playback.duration
                        && self.playback.duration > 0.0,
                    "Seek position must be within the active track"
                );
                self.seek(position).await?;
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
                    self.factory.unavailable_reason()
                );
                ensure!(seconds <= 86400, "Sleep timer is limited to 24 hours");
                self.sleep_until = Some(Instant::now() + Duration::from_secs(seconds));
            }
            Request::Shuffle | Request::SetShuffle(_) => {
                let enabled = match request {
                    Request::SetShuffle(value) => value,
                    _ => !self.queue.shuffle,
                };
                self.queue_action(
                    crate::ipc::operation_key(),
                    self.queue.revision,
                    QueueAction::SetShuffle { enabled },
                    fingerprint,
                )
                .await?;
            }
            Request::Repeat | Request::SetRepeat(_) => {
                let repeat = if let Request::SetRepeat(value) = request {
                    value
                } else {
                    match self.queue.repeat {
                        Repeat::Off => Repeat::All,
                        Repeat::All => Repeat::One,
                        Repeat::One => Repeat::Off,
                    }
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
            Request::ArtworkStyle(style) => {
                self.config.artwork_style = style;
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
                self.cancel_deferred_play()?;
                self.stop_audio().await;
                self.transport_epoch += 1;
                self.deferred_ended = false;
                if let Some(pending) = &mut self.pending {
                    pending.after = operations::After::Stop;
                }
            }
            Request::SeekBy(_)
            | Request::MprisToggle
            | Request::RelativeSeek(_)
            | Request::SetPosition { .. }
            | Request::MprisPlay
            | Request::MprisPause => unreachable!("resolved at dispatch"),
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
    run_with(
        config,
        paths,
        Arc::new(NativeFactory::default()),
        None,
        None,
    )
    .await
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
    engine.factory.refresh().await;
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
    let (quit_accept, accept_quit) = oneshot::channel();
    let accept = tokio::spawn(connections::serve(
        listener,
        uid,
        tx,
        subscriber,
        catalog,
        engine.clients.clone(),
        accept_quit,
    ));

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _=engine.config_results.changed()=>{state_tx.send_replace(engine.status());}
            Some(status)=engine.playlist_write_events.recv()=>{engine.playlist_write_status=Some(status);engine.playlist_write_task=None;state_tx.send_replace(engine.status());}
            Some(done)=engine.disk_events.recv()=>{engine.disk_complete(done).await;state_tx.send_replace(engine.status());}
            Some(event)=engine.album_events.recv()=>{engine.album_event(event).await;state_tx.send_replace(engine.status());}
            Some((generation,event))=engine.events.recv()=>{let seeked=engine.native_event(generation,event).await;
                state_tx.send_replace(engine.status());
                if let (Some(position),Ok((bus,_)))=(seeked,&mpris) {
                    let _=crate::mpris::emit_seeked(bus,position).await;
                }}
            _=timer.tick()=>{
                engine.factory.refresh().await;
                let next_account = engine.catalog.account_generation();
                let changed = {
                    let state = state_tx.borrow();
                    next_account != state.account_generation
                        || engine.factory.available(&engine.paths) != state.start_available
                };
                if changed {
                    state_tx.send_replace(engine.status());
                }
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
    #[tokio::test]
    async fn annotated_queue_pages_bind_the_wire_to_the_page_not_the_queue() {
        use crate::queue::{self, QueueContext};
        async fn page(engine: &mut Engine, annotated: bool) -> (usize, serde_json::Value) {
            let value = engine
                .handle(Request::QueuePage {
                    offset: 0,
                    annotated,
                })
                .await
                .unwrap();
            let wire = serde_json::to_vec(&value).unwrap().len();
            (wire, value)
        }
        let (_dir, mut small) = engine();
        let (small_legacy_wire, _small_legacy) = page(&mut small, false).await;
        let (_small_annotated_wire, small_annotated) = page(&mut small, true).await;
        let (_dir2, mut engine) = engine();
        let filler = (0..queue::MAX_QUEUE - engine.queue.tracks.len()).map(|i| Track {
            id: format!("t{i}"),
            kind: "songs".into(),
            ..Track::default()
        });
        engine
            .queue
            .append(filler.collect(), QueueContext::single())
            .unwrap();
        let page_size = engine.config.performance.limits().page_size;
        let (legacy_wire, legacy) = page(&mut engine, false).await;
        let (annotated_wire, annotated) = page(&mut engine, true).await;
        assert!(
            legacy_wire > small_legacy_wire * 5,
            "legacy wire grows with the queue: {legacy_wire} vs {small_legacy_wire}"
        );
        assert!(
            legacy_wire - annotated_wire > 40_000,
            "the redundant upcoming list dominates legacy responses"
        );
        let ranks_wire = serde_json::to_vec(&annotated["upcoming_ranks"])
            .unwrap()
            .len();
        assert!(
            ranks_wire < 2048,
            "rank annotations stay page-bounded at 10k entries: {ranks_wire} B"
        );
        assert_eq!(
            legacy["upcoming"].as_array().map(Vec::len),
            Some(queue::MAX_QUEUE - 1),
            "legacy responses keep the full upcoming list"
        );
        assert!(
            annotated.get("upcoming").is_none(),
            "annotated responses never carry the full list"
        );
        let ranks = annotated["upcoming_ranks"].as_object().unwrap();
        assert_eq!(ranks.len(), page_size.min(queue::MAX_QUEUE));
        for (entry_id, rank) in ranks {
            let entry_id = entry_id.parse::<u64>().unwrap();
            let expected = legacy["upcoming"]
                .as_array()
                .unwrap()
                .iter()
                .position(|id| id.as_u64() == Some(entry_id))
                .map(|position| position as u64 + 1)
                .unwrap_or(0);
            assert_eq!(rank.as_u64().unwrap_or(0), expected);
        }
        let current = engine.queue.current_entry_id().unwrap();
        assert!(
            ranks[&current.to_string()].is_null(),
            "the playing row is not upcoming and stays unranked"
        );
        assert_eq!(
            small_annotated["upcoming_ranks"]["2"].as_u64(),
            Some(1),
            "a fresh session still ranks visible rows for small queues"
        );
        assert_eq!(small_annotated["upcoming_ranks"]["3"].as_u64(), Some(2));
    }
}

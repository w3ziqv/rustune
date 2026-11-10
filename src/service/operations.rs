//! Revision checked queue transactions, asynchronous album preparation and persistence.
use super::*;
use crate::{
    catalog::album::{PreparedAlbum, Skipped},
    ipc::{AlbumAction, QueueAction},
};
use persistence::{Receipt, STATE_VERSION, SavedState};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub phase: String,
    pub loaded: usize,
    pub skipped: Vec<Skipped>,
    pub message: String,
    pub committed_revision: Option<u64>,
}
pub(super) enum After {
    Keep,
    Start { entry_id: u64, paused: bool },
    Stop,
    Undo,
}
pub(super) struct Pending {
    pub after: After,
    pub undo: Option<Queue>,
    pub reply: Option<oneshot::Sender<Reply>>,
    pub key: String,
}
pub(super) struct DiskDone {
    pub(super) state: SavedState,
    pub(super) result: Result<()>,
}
pub(super) enum AlbumEvent {
    Progress(u64, String, usize),
    Done(u64, String, Result<PreparedAlbum>),
}
enum DownloadSource {
    Album(crate::catalog::album::AlbumRef),
    Playlist(crate::catalog::playlist::PlaylistRef),
}
pub(super) struct Preparing {
    generation: u64,
    key: String,
    revision: u64,
    fingerprint: String,
    action: AlbumAction,
    pub(super) task: tokio::task::JoinHandle<()>,
}
impl Engine {
    fn fingerprint(request: &Request) -> Result<String> {
        Ok(serde_json::to_string(request)?)
    }
    fn seen_request(&self, key: &str, fingerprint: &str) -> Result<bool> {
        ensure!(
            !key.is_empty()
                && key.len() <= 128
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "Invalid operation key"
        );
        if let Some(old) = self.receipts.iter().find(|r| r.key == key) {
            ensure!(
                old.fingerprint == fingerprint,
                "Operation key was already used with different arguments"
            );
            return Ok(true);
        }
        ensure!(
            !self.recovery_required,
            "Saved queue needs recovery; use recover-state before editing"
        );
        Ok(false)
    }
    pub(super) fn stage(
        &mut self,
        queue: Queue,
        after: After,
        undo: Option<Queue>,
        key: String,
        fingerprint: String,
    ) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "Queue write in progress; retry after it completes"
        );
        ensure!(
            queue.tracks.len() <= self.config.max_queue,
            "queue limit reached"
        );
        for entry in &queue.tracks {
            validate_track(&entry.track)?;
            persistence::validate_context(&entry.context)?;
        }
        let mut receipts = self.receipts.clone();
        receipts.push(Receipt {
            key: key.clone(),
            fingerprint,
            revision: queue.revision,
        });
        if receipts.len() > 64 {
            receipts.remove(0);
        }
        let state = SavedState {
            version: STATE_VERSION,
            queue,
            history: self.history.clone(),
            dismissed: None,
            receipts,
        };
        let paths = self.paths.clone();
        let max = self.config.max_queue;
        let tx = self.disk_tx.clone();
        self.staged_transport_epoch = self.transport_epoch;
        self.pending = Some(Pending {
            after,
            undo,
            reply: None,
            key: key.clone(),
        });
        if self.operation.as_ref().is_some_and(|o| o.id == key) {
            let op = self.operation.as_mut().expect("operation exists");
            op.phase = "saving".into();
            op.message = "Validating and saving queue; audio unchanged".into();
        } else if self.preparing.is_none() {
            self.operation = Some(Operation {
                id: key,
                phase: "saving".into(),
                message: "Validating and saving queue; audio unchanged".into(),
                ..Operation::default()
            });
        }
        tokio::spawn(async move {
            let done = tokio::task::spawn_blocking(move || {
                let result = persistence::store(&paths, &state, max);
                DiskDone { state, result }
            })
            .await;
            if let Ok(done) = done {
                let _ = tx.send(done).await;
            }
        });
        Ok(())
    }
    pub(super) fn stage_navigation(&mut self, q: Queue, after: After, key: String) -> Result<()> {
        self.stage(q, after, None, key, "transport".into())
    }
    pub(super) async fn queue_action(
        &mut self,
        key: String,
        revision: u64,
        action: QueueAction,
        fingerprint: String,
    ) -> Result<()> {
        if self.seen_request(&key, &fingerprint)? {
            self.message = "Operation already committed; no duplicate entries added".into();
            return Ok(());
        }
        ensure!(
            revision == self.queue.revision,
            "Queue changed; refresh before retrying this action"
        );
        ensure!(
            self.pending.is_none(),
            "Queue write in progress; retry after it completes"
        );
        let mut q = self.queue.clone();
        let mut undo = None;
        let mut after = After::Keep;
        match action {
            QueueAction::PlayContext {
                tracks,
                context,
                start,
                shuffle,
            } => {
                validate_songs(&tracks)?;
                undo = Some(self.queue.clone());
                q.replace(tracks, start, shuffle, context)?;
                after = After::Start {
                    entry_id: q.current_entry_id().expect("replace selects entry"),
                    paused: false,
                };
            }
            QueueAction::PlayEntry { entry_id } => {
                let index = q.index(entry_id).context("Queue entry no longer exists")?;
                q.select(index);
                after = After::Start {
                    entry_id,
                    paused: false,
                };
            }
            QueueAction::PlayNext { tracks, context } => {
                validate_songs(&tracks)?;
                q.insert_next(tracks, context)?;
            }
            QueueAction::Append { tracks, context } => {
                validate_songs(&tracks)?;
                q.append(tracks, context)?;
            }
            QueueAction::ClearUpcoming => {
                undo = Some(self.queue.clone());
                q.clear_upcoming();
            }
            QueueAction::Undo => {
                q = self
                    .undo
                    .clone()
                    .context("No queue replacement or clear to undo")?;
                q.next_entry_id = q.next_entry_id.max(self.queue.next_entry_id);
                q.revision = self.queue.revision.saturating_add(1);
                after = After::Undo;
            }
            QueueAction::SetShuffle { enabled } => q.set_shuffle(enabled),
            QueueAction::SetRepeat { repeat } => {
                q.repeat = repeat;
                q.changed();
            }
            QueueAction::MoveBy { entry_id, delta } => {
                ensure!(matches!(delta, -1 | 1), "Move one place at a time");
                let from = q.index(entry_id).context("Queue entry no longer exists")?;
                let to = from
                    .checked_add_signed(isize::from(delta))
                    .context("Already at the start")?;
                ensure!(q.move_item(from, to), "Already at the end");
            }
            QueueAction::Move { entry_id, before } => {
                let from = q.index(entry_id).context("Queue entry no longer exists")?;
                let to = if let Some(id) = before {
                    q.index(id).context("Target entry no longer exists")?
                } else {
                    q.tracks.len()
                };
                let to = if to > from { to - 1 } else { to };
                ensure!(q.move_item(from, to), "Invalid queue move");
            }
            QueueAction::RemoveAndAdvance { entry_id } => {
                let index = q.index(entry_id).context("Queue entry no longer exists")?;
                let current = q.current == Some(index);
                let successor = if current {
                    q.candidate(true)
                        .filter(|i| *i != index)
                        .map(|i| q.tracks[i].entry_id)
                } else {
                    None
                };
                q.remove(index);
                if current {
                    if let Some(id) = successor {
                        let next = q.index(id).expect("successor retained");
                        q.select(next);
                        after = if self.audio.is_some() {
                            After::Start {
                                entry_id: id,
                                paused: self.desired_paused,
                            }
                        } else {
                            After::Stop
                        };
                    } else {
                        after = After::Stop;
                    }
                }
            }
        }
        self.stage(q, after, undo, key, fingerprint)
    }
    pub(super) async fn disk_complete(&mut self, done: DiskDone) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let continue_ended = (done.result.is_err()
            && !done
                .result
                .as_ref()
                .is_err_and(|e| e.is::<persistence::DurabilityUncertain>()))
            || matches!(pending.after, After::Keep);
        let result = match done.result {
            Err(error) => {
                if error.is::<persistence::DurabilityUncertain>() {
                    self.queue = done.state.queue;
                    self.receipts = done.state.receipts;
                    self.stop_audio().await;
                    self.recovery_required = true;
                    self.uncertain_disk = true;
                }
                self.message = error.to_string();
                if let Some(op) = &mut self.operation
                    && op.id == pending.key
                {
                    op.phase = "failed".into();
                    op.message = self.message.clone();
                }
                Err(error)
            }
            Ok(()) => {
                self.queue = done.state.queue;
                self.receipts = done.state.receipts;
                if let Some(undo) = pending.undo.take() {
                    self.undo = Some(undo);
                }
                if let Some(op) = &mut self.operation
                    && op.id == pending.key
                {
                    op.phase = "committed".into();
                    op.committed_revision = Some(self.queue.revision);
                    op.message = "Queue saved".into();
                }
                self.message = "Queue committed and saved".into();
                match pending.after {
                    After::Keep => {}
                    After::Undo => {
                        self.stop_audio().await;
                        self.undo = None;
                    }
                    After::Stop => {
                        self.stop_audio().await;
                    }
                    After::Start { entry_id, paused } => {
                        // Pause/Play received during disk I/O takes precedence over the old intent.
                        let paused = if self.transport_epoch != self.staged_transport_epoch {
                            self.desired_paused
                        } else {
                            paused
                        };
                        if let Some(index) = self.queue.index(entry_id)
                            && let Err(error) = self.start_audio_paused(index, paused).await
                        {
                            self.fail_audio(error.to_string()).await;
                        }
                    }
                }
                Ok(serde_json::to_value(self.status()).expect("status serializes"))
            }
        };
        if let Some(reply) = pending.reply.take() {
            let _ = reply.send(Reply::result(result));
        }
        if self.deferred_ended {
            self.deferred_ended = false;
            if continue_ended && !self.recovery_required {
                self.advance_audio(false).await;
            }
        }
    }
    pub(super) fn begin_album(&mut self, request: Request) -> Result<()> {
        let fingerprint = Self::fingerprint(&request)?;
        let (key, revision, source, action) = match request.clone() {
            Request::Album {
                key,
                revision,
                album,
                action,
            } => {
                album.validate()?;
                (key, revision, DownloadSource::Album(album), action)
            }
            Request::Playlist {
                key,
                revision,
                playlist,
                action,
            } => {
                playlist.validate()?;
                (key, revision, DownloadSource::Playlist(playlist), action)
            }
            _ => anyhow::bail!("Invalid album/playlist request"),
        };
        if self.seen_request(&key, &fingerprint)? {
            self.message = "Album operation already committed".into();
            return Ok(());
        }
        if let Some(pending) = self.preparing.as_ref().filter(|p| p.key == key) {
            ensure!(
                pending.fingerprint == fingerprint,
                "Operation key is in use with different arguments"
            );
            return Ok(());
        }
        ensure!(
            self.preparing.is_none() && self.pending.is_none(),
            "Another bulk operation is in progress"
        );
        ensure!(
            revision == self.queue.revision,
            "Queue changed; refresh before loading album"
        );
        let catalog = self.catalog.clone();
        let tx = self.album_tx.clone();
        let id = key.clone();
        let limits = self.config.performance.limits();
        self.retry_request = Some(request);
        self.operation = Some(Operation {
            id: key.clone(),
            phase: "preparing".into(),
            message: "Loading complete album/playlist; queue and audio unchanged".into(),
            ..Operation::default()
        });
        self.album_generation = self.album_generation.wrapping_add(1);
        let generation = self.album_generation;
        let task = tokio::spawn(async move {
            let progress = |n| {
                let _ = tx.try_send(AlbumEvent::Progress(generation, id.clone(), n));
            };
            let prepared = match source {
                DownloadSource::Album(album) => catalog.album(&album, limits, progress).await,
                DownloadSource::Playlist(playlist) => {
                    catalog.playlist(&playlist, limits, progress).await
                }
            };
            let _ = tx.send(AlbumEvent::Done(generation, id, prepared)).await;
        });
        self.preparing = Some(Preparing {
            generation,
            key,
            revision,
            fingerprint,
            action,
            task,
        });
        Ok(())
    }
    pub(super) async fn album_event(&mut self, event: AlbumEvent) {
        match event {
            AlbumEvent::Progress(generation, key, n) => {
                if self
                    .preparing
                    .as_ref()
                    .is_some_and(|p| p.key == key && p.generation == generation)
                    && let Some(op) = &mut self.operation
                {
                    op.loaded = n;
                }
            }
            AlbumEvent::Done(generation, key, result) => {
                if !self
                    .preparing
                    .as_ref()
                    .is_some_and(|p| p.key == key && p.generation == generation)
                {
                    return;
                }
                let pending = self.preparing.take().expect("checked album operation");
                let skipped = result
                    .as_ref()
                    .map(|a| a.skipped.clone())
                    .unwrap_or_default();
                let result = match result {
                    Err(error) => Err(error),
                    Ok(album) => {
                        let action = match pending.action {
                            AlbumAction::PlayFromPosition {
                                source_index,
                                shuffle,
                            } => {
                                let start = album
                                    .tracks
                                    .iter()
                                    .position(|t| t.source_index == Some(source_index));
                                start.context("Selected playlist occurrence is unavailable; queue unchanged").map(|start|QueueAction::PlayContext{tracks:album.tracks,context:album.context,start,shuffle})
                            }
                            AlbumAction::Append => Ok(QueueAction::Append {
                                tracks: album.tracks,
                                context: album.context,
                            }),
                            AlbumAction::PlayNext => Ok(QueueAction::PlayNext {
                                tracks: album.tracks,
                                context: album.context,
                            }),
                            AlbumAction::Play {
                                start_song,
                                shuffle,
                            } => {
                                let start = start_song.as_ref().map_or(Some(0), |id| {
                                    album.tracks.iter().position(|t| t.id == *id)
                                });
                                let count = album.tracks.len();
                                start
                                    .context("Selected song is unavailable in this complete album")
                                    .map(|start| QueueAction::PlayContext {
                                        tracks: album.tracks,
                                        context: album.context,
                                        start: if shuffle && start_song.is_none() {
                                            fastrand::usize(..count)
                                        } else {
                                            start
                                        },
                                        shuffle,
                                    })
                            }
                        };
                        match action {
                            Ok(action) => {
                                self.queue_action(
                                    pending.key,
                                    pending.revision,
                                    action,
                                    pending.fingerprint,
                                )
                                .await
                            }
                            Err(error) => Err(error),
                        }
                    }
                };
                if let Some(op) = &mut self.operation {
                    op.skipped = skipped;
                    if let Err(error) = result {
                        op.phase = "failed".into();
                        op.message = error.to_string();
                        self.message = op.message.clone();
                    }
                }
            }
        }
    }
    pub(super) fn cancel_operation(&mut self, key: &str) -> Result<()> {
        ensure!(
            self.pending.as_ref().is_none_or(|p| p.key != key),
            "Queue is already being saved; cancellation is no longer possible"
        );
        let pending = self
            .preparing
            .take()
            .context("No album is being prepared")?;
        if pending.key != key {
            self.preparing = Some(pending);
            anyhow::bail!("Operation changed; refresh before cancelling");
        }
        pending.task.abort();
        if let Some(op) = &mut self.operation {
            op.phase = "cancelled".into();
            op.message = "Cancelled before queue commit".into();
        }
        Ok(())
    }
}
fn validate_songs(tracks: &[Track]) -> Result<()> {
    ensure!(
        !tracks.is_empty() && tracks.len() <= crate::queue::MAX_QUEUE,
        "Invalid track count"
    );
    for track in tracks {
        validate_track(track)?;
        ensure!(
            matches!(track.kind.as_str(), "songs" | "library-songs") && track.unavailable.is_none(),
            "Choose an available song or use album actions"
        );
    }
    Ok(())
}

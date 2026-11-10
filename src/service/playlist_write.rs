//! Snapshot account writes without blocking transport or mutating the queue.
use super::*;
use crate::{
    catalog::playlist_write::{
        MAX_WRITE_TRACKS, PlaylistWriteResult, WriteAction, WriteFailure, payload,
    },
    ipc::PlaylistWriteSource,
};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistWriteStatus {
    pub id: String,
    pub phase: String,
    pub tracks: usize,
    pub result: Option<PlaylistWriteResult>,
    pub message: String,
    pub can_retry: bool,
}
impl Engine {
    fn export_ids(&self, source: &PlaylistWriteSource) -> Result<Vec<String>> {
        let tracks: Vec<&Track> = match source {
            PlaylistWriteSource::Queue { revision } => {
                ensure!(
                    *revision == self.queue.revision,
                    "Queue changed; review the snapshot before exporting"
                );
                ensure!(
                    self.queue.tracks.len() <= MAX_WRITE_TRACKS,
                    "Playlist export exceeds 1000 tracks; nothing sent"
                );
                ensure!(
                    !self.queue.tracks.is_empty(),
                    "Queue is empty; use New empty playlist"
                );
                self.queue.tracks.iter().map(|e| &e.track).collect()
            }
            PlaylistWriteSource::Entry { revision, entry_id } => {
                ensure!(
                    *revision == self.queue.revision,
                    "Queue changed; refresh before exporting this entry"
                );
                vec![
                    &self.queue.tracks[self
                        .queue
                        .index(*entry_id)
                        .context("Queue entry no longer exists")?]
                    .track,
                ]
            }
            PlaylistWriteSource::Track { track } => vec![track],
            PlaylistWriteSource::Empty => Vec::new(),
        };
        tracks
            .into_iter()
            .map(|t| {
                validate_track(t)?;
                ensure!(
                    matches!(t.kind.as_str(), "songs" | "library-songs")
                        && t.unavailable.is_none()
                        && !t.id.is_empty()
                        && t.id.len() <= 32
                        && t.id.bytes().all(|b| b.is_ascii_digit()),
                    "Export contains unsupported or unavailable songs; nothing sent"
                );
                Ok(t.id.clone())
            })
            .collect()
    }
    pub(super) fn begin_playlist_write(&mut self, request: Request) -> Result<()> {
        let Request::PlaylistWrite {
            key,
            source,
            action,
        } = &request
        else {
            anyhow::bail!("Invalid playlist write request");
        };
        ensure!(
            playlist_journal::valid_key(key),
            "Invalid playlist write key"
        );
        if let Some(last) = &self.playlist_write_status
            && last.id == *key
            && self.playlist_write_task.is_some()
        {
            ensure!(
                self.playlist_write_retry.as_ref().is_some_and(
                    |r| serde_json::to_string(r).ok() == serde_json::to_string(&request).ok()
                ),
                "Playlist key is already in use with different arguments"
            );
            return Ok(());
        }
        ensure!(
            self.playlist_write_task.is_none(),
            "Another account write is in progress"
        );
        let account = self.catalog.playlist_account()?;
        let fingerprint = serde_json::to_string(&request)?;
        let ids = self.export_ids(source).map_err(|e| e.to_string());
        let paths = self.paths.clone();
        let catalog = self.catalog.clone();
        let tx = self.playlist_write_tx.clone();
        let action = action.clone();
        let key = key.clone();
        self.playlist_write_status = Some(PlaylistWriteStatus {
            id: key.clone(),
            phase: "preparing".into(),
            message: "Preparing account write; queue/audio unchanged".into(),
            ..PlaylistWriteStatus::default()
        });
        self.playlist_write_retry = Some(request);
        self.playlist_write_task = Some(tokio::spawn(async move {
            let status = run_write(paths, catalog, key, action, ids, account, fingerprint).await;
            let _ = tx.send(status).await;
        }));
        Ok(())
    }
}
async fn run_write(
    paths: Paths,
    catalog: Arc<Catalog>,
    key: String,
    action: WriteAction,
    ids: std::result::Result<Vec<String>, String>,
    account: crate::catalog::playlist_write::CredentialStamp,
    fingerprint: String,
) -> PlaylistWriteStatus {
    let journal_paths = paths.clone();
    let journal_key = key.clone();
    let begin = tokio::task::spawn_blocking(move || {
        let guard = playlist_journal::acquire(&journal_paths)?;
        let mut journal = playlist_journal::Journal::load(&journal_paths)?;
        let replay = journal.begin(&journal_paths, &journal_key, &fingerprint, account)?;
        Ok::<_, anyhow::Error>((journal, replay, guard))
    })
    .await;
    let (journal, replay, guard) = match begin {
        Ok(Ok(result)) => result,
        other => {
            let (message, uncertain) = match other {
                Ok(Err(error)) => {
                    let u = error
                        .downcast_ref::<WriteFailure>()
                        .is_some_and(|e| e.uncertain);
                    (error.to_string(), u)
                }
                _ => (
                    "Playlist receipt writer failed; no account request was started".into(),
                    false,
                ),
            };
            return PlaylistWriteStatus {
                id: key,
                phase: if uncertain { "uncertain" } else { "rejected" }.into(),
                message,
                can_retry: !uncertain,
                ..PlaylistWriteStatus::default()
            };
        }
    };
    if let Some(result) = replay {
        return PlaylistWriteStatus {
            id: key,
            phase: "completed".into(),
            tracks: result.tracks,
            result: Some(result),
            message: "Already confirmed; no duplicate account write sent".into(),
            can_retry: false,
        };
    }
    let outcome = match ids {
        Err(message) => Err(WriteFailure::rejected(message)),
        Ok(ids) => match payload(&action, &ids) {
            Err(e) => Err(WriteFailure::rejected(e.to_string())),
            Ok(_) => catalog.write_playlist(&action, &ids, account).await,
        },
    };
    let (result, message, uncertain) = match outcome {
        Ok(result) => (Some(result), "Apple confirmed playlist write".into(), false),
        Err(error) => (None, error.message, error.uncertain),
    };
    let save_result = result.clone();
    let save_key = key.clone();
    let saved = tokio::task::spawn_blocking(move || {
        // Keep the lock inside blocking I/O even if shutdown aborts its async owner.
        let _guard = guard;
        let mut journal = journal;
        journal.finish(&paths, &save_key, save_result, uncertain)
    })
    .await;
    if !matches!(saved, Ok(Ok(()))) {
        return PlaylistWriteStatus{id:key,phase:"uncertain".into(),tracks:result.as_ref().map_or(0,|r|r.tracks),result,message:"Could not durably record playlist result; inspect Apple Music before a new operation. This key will not be sent again automatically".into(),can_retry:false};
    }
    PlaylistWriteStatus {
        id: key,
        phase: if result.is_some() {
            "completed"
        } else if uncertain {
            "uncertain"
        } else {
            "rejected"
        }
        .into(),
        tracks: result.as_ref().map_or(0, |r| r.tracks),
        can_retry: result.is_none() && !uncertain,
        result,
        message,
    }
}

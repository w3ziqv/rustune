//! Versioned, bounded private state. Invalid files require explicit recovery.
use super::*;
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
pub const STATE_VERSION: u32 = 2;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub key: String,
    pub fingerprint: String,
    pub revision: u64,
}
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SavedState {
    pub version: u32,
    pub queue: Queue,
    pub history: Vec<String>,
    pub dismissed: Option<String>,
    pub receipts: Vec<Receipt>,
}
fn validate(state: &SavedState, max_queue: usize) -> Result<()> {
    ensure!(
        state.queue.tracks.len() <= max_queue
            && state.history.len() <= 50
            && state.receipts.len() <= 64,
        "saved state exceeds item limits"
    );
    for entry in &state.queue.tracks {
        validate_track(&entry.track)?;
        validate_context(&entry.context)?;
    }
    ensure!(
        state
            .history
            .iter()
            .all(|v| v.len() <= 512 && !v.chars().any(char::is_control)),
        "invalid search history"
    );
    ensure!(
        state.receipts.iter().all(|r| r.key.len() <= 128
            && !r.key.is_empty()
            && r.key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && r.fingerprint.len() <= 65536),
        "Invalid saved operation receipts"
    );
    Ok(())
}
pub(super) fn encode(state: &SavedState, max_queue: usize) -> Result<Vec<u8>> {
    validate(state, max_queue)?;
    // Write through a bounded buffer: serde cannot allocate an oversized state first.
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(b.len()) > MAX_STATE_BYTES {
                return Err(std::io::Error::other("state exceeds 16 MiB"));
            }
            self.0.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer(&mut bytes, state)
        .map_err(|_| anyhow::anyhow!("State exceeds 16 MiB or is invalid; nothing committed"))?;
    Ok(bytes.0)
}
pub(super) fn load(paths: &Paths, max_queue: usize) -> Result<SavedState> {
    let path = paths.data.join("state.json");
    if !path.exists() {
        return Ok(SavedState {
            version: STATE_VERSION,
            ..SavedState::default()
        });
    }
    ensure!(
        fs::metadata(&path)?.len() <= MAX_STATE_BYTES as u64,
        "saved state exceeds 16 MiB"
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(MAX_STATE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_STATE_BYTES, "saved state exceeds 16 MiB");
    let mut state: SavedState =
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Saved state is invalid"))?;
    ensure!(
        state.version <= STATE_VERSION,
        "Saved state is from a newer Rustune version"
    );
    state.queue.migrate()?;
    state.version = STATE_VERSION;
    validate(&state, max_queue)?;
    Ok(state)
}
#[derive(Debug)]
pub(super) struct DurabilityUncertain;
impl std::fmt::Display for DurabilityUncertain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Queue installed but directory sync failed; durability uncertain. Audio stopped and edits locked. Restart service to reconcile saved state")
    }
}
impl std::error::Error for DurabilityUncertain {}
pub(super) fn store(paths: &Paths, state: &SavedState, max_queue: usize) -> Result<()> {
    store_with_sync(paths, state, max_queue, |p| {
        fs::File::open(p).and_then(|f| f.sync_all())
    })
}
fn store_with_sync(
    paths: &Paths,
    state: &SavedState,
    max_queue: usize,
    sync: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> Result<()> {
    let bytes = encode(state, max_queue)?;
    let path = paths.data.join("state.json");
    if path.exists() {
        // Retain only a validated previous state. Never overwrite a corrupt original.
        load(paths, max_queue)?;
        private_write(&paths.data.join("state.previous.json"), &fs::read(&path)?)
            .map_err(|_| anyhow::anyhow!("Could not preserve previous queue; nothing committed"))?;
    }
    private_write(&path, &bytes).map_err(|_| {
        anyhow::anyhow!("Could not save queue; check free space and data directory permissions")
    })?;
    sync(&paths.data).map_err(|_| anyhow::Error::new(DurabilityUncertain))?;
    Ok(())
}
pub(super) fn validate_context(c: &crate::queue::QueueContext) -> Result<()> {
    ensure!(
        c.playlist_id
            .as_ref()
            .is_none_or(|id| crate::catalog::playlist::valid_id(id)),
        "invalid playlist context"
    );
    ensure!(
        [&c.provider, &c.storefront, &c.title]
            .iter()
            .all(|v| v.len() <= 2048 && !v.chars().any(char::is_control))
            && c.album_id.as_ref().is_none_or(|v| v.len() <= 128
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))),
        "invalid queue context"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn paths(root: &std::path::Path) -> Paths {
        Paths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            runtime: root.join("run"),
        }
    }
    #[test]
    fn migrates_legacy_duplicates_and_round_trips_without_autoplay() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        p.init().unwrap();
        private_write(&p.data.join("state.json"),br#"{"queue":{"tracks":[{"id":"1","kind":"songs"},{"id":"1","kind":"songs"}],"current":1,"shuffle":true,"repeat":"all"}}"#).unwrap();
        let state = load(&p, 10000).unwrap();
        assert_eq!(state.queue.tracks.len(), 2);
        assert_ne!(
            state.queue.tracks[0].entry_id,
            state.queue.tracks[1].entry_id
        );
        assert!(state.queue.shuffle);
        store(&p, &state, 10000).unwrap();
        assert_eq!(load(&p, 10000).unwrap().queue.current, Some(1));
        assert!(p.data.join("state.previous.json").exists());
    }
    #[test]
    fn failure_after_rename_is_distinguished_from_rejected_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        paths.init().unwrap();
        let mut state = SavedState {
            version: STATE_VERSION,
            ..SavedState::default()
        };
        state
            .queue
            .push(Track {
                id: "1".into(),
                kind: "songs".into(),
                ..Track::default()
            })
            .unwrap();
        let error = store_with_sync(&paths, &state, 10000, |_| {
            Err(std::io::Error::other("synthetic fsync failure"))
        })
        .unwrap_err();
        assert!(error.is::<DurabilityUncertain>());
        assert_eq!(load(&paths, 10000).unwrap().queue.tracks.len(), 1);
    }
    #[test]
    fn exact_16_mib_round_trips_and_one_more_byte_is_rejected_before_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        paths.init().unwrap();
        let mut state = SavedState {
            version: STATE_VERSION,
            ..SavedState::default()
        };
        let track = Track {
            id: "1".into(),
            kind: "songs".into(),
            title: "ą".repeat(1024),
            artist: "ą".repeat(1024),
            album: "ą".repeat(1024),
            url: Some(String::new()),
            ..Track::default()
        };
        for _ in 0..2500 {
            state.queue.push(track.clone()).unwrap();
        }
        let base = serde_json::to_vec(&state).unwrap().len();
        assert!(base < MAX_STATE_BYTES);
        let mut remaining = MAX_STATE_BYTES - base;
        for entry in &mut state.queue.tracks {
            let n = remaining.min(2048);
            entry.track.url = Some("a".repeat(n));
            remaining -= n;
        }
        assert_eq!(remaining, 0);
        assert_eq!(encode(&state, 10000).unwrap().len(), MAX_STATE_BYTES);
        store(&paths, &state, 10000).unwrap();
        assert_eq!(load(&paths, 10000).unwrap().queue.tracks.len(), 2500);
        state
            .queue
            .tracks
            .iter_mut()
            .find(|e| e.url.as_ref().unwrap().len() < 2048)
            .unwrap()
            .track
            .url
            .as_mut()
            .unwrap()
            .push('a');
        assert!(store(&paths, &state, 10000).is_err());
        assert_eq!(
            fs::metadata(paths.data.join("state.json")).unwrap().len(),
            16_777_216
        );
    }
    #[test]
    fn oversized_unicode_and_invalid_original_never_overwrite_state() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        p.init().unwrap();
        let mut state = SavedState {
            version: STATE_VERSION,
            ..SavedState::default()
        };
        state
            .queue
            .push(Track {
                id: "1".into(),
                kind: "songs".into(),
                ..Track::default()
            })
            .unwrap();
        store(&p, &state, 10000).unwrap();
        let old = fs::read(p.data.join("state.json")).unwrap();
        state.queue.clear();
        let t = Track {
            id: "1".into(),
            kind: "songs".into(),
            title: "ą".repeat(1024),
            artist: "ą".repeat(1024),
            album: "ą".repeat(1024),
            ..Track::default()
        };
        for _ in 0..2800 {
            state.queue.push(t.clone()).unwrap();
        }
        assert!(store(&p, &state, 10000).is_err());
        assert_eq!(fs::read(p.data.join("state.json")).unwrap(), old);
        private_write(&p.data.join("state.json"), b"broken original").unwrap();
        assert!(store(&p, &SavedState::default(), 10000).is_err());
        assert_eq!(
            fs::read(p.data.join("state.json")).unwrap(),
            b"broken original"
        );
    }
}

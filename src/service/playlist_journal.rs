//! Durable bounded receipts for non-idempotent account POSTs. Contains no tokens.
use super::*;
use crate::catalog::playlist_write::{CredentialStamp, PlaylistWriteResult};
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Sending,
    Completed,
    Rejected,
    Uncertain,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    key: String,
    fingerprint: String,
    account: CredentialStamp,
    phase: Phase,
    result: Option<PlaylistWriteResult>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Journal {
    version: u8,
    records: Vec<Receipt>,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            version: 1,
            records: Vec::new(),
        }
    }
}
impl Journal {
    pub fn load(paths: &Paths) -> Result<Self> {
        use std::io::Read;
        let path = paths.data.join("playlist-writes.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => anyhow::bail!("Playlist write journal is unavailable; original retained"),
            Ok(_) => {}
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| {
                anyhow::anyhow!("Playlist write journal is unavailable; original retained")
            })?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file()
                && meta.uid() == nix::unistd::Uid::effective().as_raw()
                && meta.mode() & 0o077 == 0,
            "Playlist journal must be a private owned regular file"
        );
        ensure!(
            meta.len() <= MAX_BYTES as u64,
            "Playlist write journal exceeds limit; original retained"
        );
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "Playlist write journal exceeds limit"
        );
        let journal: Self = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Playlist write journal is invalid; original retained"))?;
        let mut keys = std::collections::HashSet::new();
        ensure!(
            journal.version == 1
                && journal.records.len() <= 64
                && journal.records.iter().all(|r| valid_key(&r.key)
                    && keys.insert(&r.key)
                    && !r.fingerprint.is_empty()
                    && r.fingerprint.len() <= 65536
                    && r.account.size <= 16384
                    && match r.phase {
                        Phase::Completed => r
                            .result
                            .as_ref()
                            .is_some_and(|v| crate::catalog::playlist::valid_id(&v.playlist_id)
                                && v.tracks <= 1000),
                        _ => r.result.is_none(),
                    }),
            "Invalid playlist write journal; original retained"
        );
        Ok(journal)
    }
    fn save(&self, paths: &Paths) -> Result<()> {
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "Playlist write journal byte limit reached"
        );
        private_write(&paths.data.join("playlist-writes.json"), &bytes).map_err(|_| {
            anyhow::anyhow!(
                "Could not save playlist write receipt; check disk and directory permissions"
            )
        })?;
        fs::File::open(&paths.data)
            .and_then(|f| f.sync_all())
            .map_err(|_| {
                anyhow::anyhow!("Playlist receipt durability uncertain; review before retrying")
            })?;
        Ok(())
    }
    pub fn begin(
        &mut self,
        paths: &Paths,
        key: &str,
        fingerprint: &str,
        account: CredentialStamp,
    ) -> Result<Option<PlaylistWriteResult>> {
        ensure!(
            valid_key(key) && fingerprint.len() <= 65536,
            "Invalid playlist operation key or arguments"
        );
        if let Some(record) = self.records.iter_mut().find(|r| r.key == key) {
            ensure!(
                record.fingerprint == fingerprint,
                "Playlist operation key already used with different arguments"
            );
            ensure!(
                record.account == account,
                "Playlist operation belongs to different local credentials"
            );
            match record.phase {Phase::Completed=>return record.result.clone().map(Some).context("Invalid completed playlist receipt; inspect before writing"),Phase::Sending|Phase::Uncertain=>return Err(crate::catalog::playlist_write::WriteFailure::uncertain("Playlist write outcome uncertain; inspect Apple Music before creating a new operation. This key will not be sent again").into()),Phase::Rejected=>record.phase=Phase::Sending}
        } else {
            if self.records.len() == 64 {
                let i=self.records.iter().position(|r|matches!(r.phase,Phase::Completed|Phase::Rejected)).context("Playlist write journal is full of unresolved operations; review it before writing")?;
                self.records.remove(i);
            }
            self.records.push(Receipt {
                key: key.into(),
                fingerprint: fingerprint.into(),
                account,
                phase: Phase::Sending,
                result: None,
            });
        }
        self.save(paths)?;
        Ok(None)
    }
    pub fn finish(
        &mut self,
        paths: &Paths,
        key: &str,
        result: Option<PlaylistWriteResult>,
        uncertain: bool,
    ) -> Result<()> {
        let record = self
            .records
            .iter_mut()
            .find(|r| r.key == key)
            .context("Missing playlist operation receipt")?;
        record.phase = if result.is_some() {
            Phase::Completed
        } else if uncertain {
            Phase::Uncertain
        } else {
            Phase::Rejected
        };
        record.result = result;
        self.save(paths)
    }
}

pub(super) fn acquire(paths: &Paths) -> Result<nix::fcntl::Flock<fs::File>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(paths.data.join("playlist-writes.lock"))
        .map_err(|_| anyhow::anyhow!("Playlist journal lock unavailable; nothing sent"))?;
    nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|_| anyhow::anyhow!("Another playlist writer is active; nothing sent"))
}
pub(super) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sending_and_uncertain_receipts_never_replay_post_but_completed_keys_survive_restart() {
        let (_dir, e) = super::super::tests::engine();
        let stamp = CredentialStamp {
            inode: 1,
            mtime: 2,
            nanos: 0,
            size: 4,
        };
        let mut j = Journal::load(&e.paths).unwrap();
        assert!(j.begin(&e.paths, "key", "args", stamp).unwrap().is_none());
        let mut j = Journal::load(&e.paths).unwrap();
        assert!(
            j.begin(&e.paths, "key", "args", stamp)
                .unwrap_err()
                .to_string()
                .contains("uncertain")
        );
        j.finish(&e.paths, "key", None, false).unwrap();
        assert!(j.begin(&e.paths, "key", "args", stamp).unwrap().is_none());
        let result = PlaylistWriteResult {
            playlist_id: "p.fixture".into(),
            tracks: 2,
        };
        j.finish(&e.paths, "key", Some(result), false).unwrap();
        let mut j = Journal::load(&e.paths).unwrap();
        assert_eq!(
            j.begin(&e.paths, "key", "args", stamp)
                .unwrap()
                .unwrap()
                .tracks,
            2
        );
        assert!(j.begin(&e.paths, "key", "other", stamp).is_err());
        assert!(
            j.begin(
                &e.paths,
                "key",
                "args",
                CredentialStamp { inode: 2, ..stamp }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod corruption_tests {
    use super::*;
    #[tokio::test]
    async fn malformed_completed_receipt_is_retained_and_blocks_writes() {
        let (_dir, e) = super::super::tests::engine();
        let bytes=br#"{"version":1,"records":[{"key":"key","fingerprint":"args","account":{"inode":1,"mtime":2,"nanos":0,"size":4},"phase":"completed","result":null}]}"#;
        private_write(&e.paths.data.join("playlist-writes.json"), bytes).unwrap();
        assert!(Journal::load(&e.paths).is_err());
        assert_eq!(
            fs::read(e.paths.data.join("playlist-writes.json")).unwrap(),
            bytes
        );
    }
}
#[cfg(test)]
mod link_tests {
    use super::*;
    #[tokio::test]
    async fn broken_journal_symlink_is_retained_and_blocks_writes() {
        let (_dir, e) = super::super::tests::engine();
        let path = e.paths.data.join("playlist-writes.json");
        std::os::unix::fs::symlink("missing", &path).unwrap();
        assert!(Journal::load(&e.paths).is_err());
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }
}

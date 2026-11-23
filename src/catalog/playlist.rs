//! Private library playlist identities and complete ordered playback contexts.
use super::*;
use crate::{catalog::album::PreparedAlbum, queue::QueueContext};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistRef {
    pub id: String,
    pub storefront: String,
    pub title: String,
    pub can_edit: bool,
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
impl PlaylistRef {
    pub fn validate(&self) -> Result<()> {
        ensure!(valid_id(&self.id), "Invalid library playlist ID");
        ensure!(
            self.storefront.len() == 2 && self.storefront.bytes().all(|b| b.is_ascii_alphabetic()),
            "Invalid playlist storefront"
        );
        ensure!(
            self.title.len() <= 2048 && !self.title.chars().any(char::is_control),
            "Invalid playlist title"
        );
        Ok(())
    }
    pub fn context(&self) -> QueueContext {
        QueueContext {
            provider: "music".into(),
            storefront: self.storefront.clone(),
            playlist_id: Some(self.id.clone()),
            title: self.title.clone(),
            ..QueueContext::default()
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlaylistRead {
    pub(crate) generation: u64,
    pub(super) epoch: u64,
}

impl Catalog {
    pub(crate) fn playlist_binding(&self, generation: u64) -> Result<PlaylistRead> {
        self.apple
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Apple Music unavailable"))?
            .playlist_binding(generation)
    }

    pub(crate) fn check_playlist_binding(&self, binding: PlaylistRead) -> Result<()> {
        ensure!(
            self.playlist_binding(binding.generation)? == binding,
            "Playlist changed while loading; refresh the playlist"
        );
        Ok(())
    }

    pub async fn playlist(
        &self,
        playlist: &PlaylistRef,
        limits: Limits,
        progress: impl FnMut(usize),
    ) -> Result<PreparedAlbum> {
        let generation = self
            .account_generation()
            .ok_or_else(|| anyhow::anyhow!("No current Apple Music account"))?;
        self.prepare_playlist(
            playlist,
            self.playlist_binding(generation)?,
            limits,
            progress,
        )
        .await
    }

    pub(crate) async fn prepare_playlist(
        &self,
        playlist: &PlaylistRef,
        binding: PlaylistRead,
        limits: Limits,
        progress: impl FnMut(usize),
    ) -> Result<PreparedAlbum> {
        playlist.validate()?;
        // Collection uses the same profile and account from the first page through commit.
        self.check_playlist_binding(binding)?;
        self.collect_context(
            &format!("open:library-playlists:{}", playlist.id),
            playlist.context(),
            Some(binding),
            limits,
            progress,
        )
        .await
    }

    pub(super) async fn playlist_page(
        &self,
        kind: &str,
        cursor: &str,
        storefront: &str,
        limits: Limits,
        binding: PlaylistRead,
    ) -> Result<Page> {
        use tokio::time::{Instant, sleep_until, timeout};
        timeout(Duration::from_secs(75), async {
            loop {
                self.check_playlist_binding(binding)?;
                let result = self
                    .browse(kind, "", Some(cursor), storefront, limits)
                    .await;
                self.check_playlist_binding(binding)?;
                let Some(limit) = result
                    .as_ref()
                    .err()
                    .and_then(|error| error.downcast_ref::<super::reads::LocalBudget>())
                else {
                    return result;
                };
                // No active read or provider permit survives into this wait.
                while Instant::now() < limit.retry_at {
                    sleep_until(
                        limit
                            .retry_at
                            .min(Instant::now() + Duration::from_millis(250)),
                    )
                    .await;
                    self.check_playlist_binding(binding)?;
                }
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Apple Music playlist page timed out waiting for the request budget; retry"
            )
        })?
    }
}
pub fn page_cursor(storefront: &str, generation: u64, offset: usize) -> String {
    format!(
        "music:{}:{generation:016x}:{offset}",
        storefront.to_ascii_lowercase()
    )
}

pub(crate) fn private_kind(kind: &str) -> bool {
    matches!(
        kind,
        "library" | "search:library-songs" | "playlists" | "search:playlists" | "recent"
    ) || kind.starts_with("open:library-playlists:")
}

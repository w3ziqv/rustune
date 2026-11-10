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
impl Catalog {
    pub async fn playlist(
        &self,
        playlist: &PlaylistRef,
        limits: Limits,
        progress: impl FnMut(usize),
    ) -> Result<PreparedAlbum> {
        playlist.validate()?;
        let account = self.playlist_account()?;
        let prepared = self
            .collect_context(
                &format!("open:library-playlists:{}", playlist.id),
                playlist.context(),
                limits,
                progress,
            )
            .await?;
        ensure!(
            self.playlist_account()? == account,
            "Account credentials changed during playlist preparation; nothing committed"
        );
        Ok(prepared)
    }
}
pub(super) fn private_kind(kind: &str) -> bool {
    matches!(
        kind,
        "library" | "search:library-songs" | "playlists" | "search:playlists" | "recent"
    ) || kind.starts_with("open:library-playlists:")
}

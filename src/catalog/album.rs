//! Complete album preparation. Never mutates playback or the session.
use super::*;
use crate::{
    queue::{MAX_QUEUE, QueueContext},
    service::validate_track,
};
use serde::{Deserialize, Serialize};

pub const MAX_ALBUM_BYTES: usize = 12 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: String,
    pub provider: String,
    pub storefront: String,
    pub title: String,
}
impl AlbumRef {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.len() <= 20
                && self.id.bytes().all(|b| b.is_ascii_digit()),
            "Invalid album ID"
        );
        ensure!(
            matches!(self.provider.as_str(), "music" | "itunes")
                && self.storefront.len() == 2
                && self.storefront.bytes().all(|b| b.is_ascii_alphabetic()),
            "Invalid album source"
        );
        ensure!(
            self.title.len() <= 2048 && !self.title.chars().any(char::is_control),
            "Invalid album title"
        );
        Ok(())
    }
    pub fn context(&self) -> QueueContext {
        QueueContext {
            provider: self.provider.clone(),
            storefront: self.storefront.clone(),
            album_id: Some(self.id.clone()),
            playlist_id: None,
            title: self.title.clone(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Skipped {
    pub count: usize,
    pub reason: String,
}
#[derive(Debug)]
pub struct PreparedAlbum {
    pub tracks: Vec<Track>,
    pub skipped: Vec<Skipped>,
    pub context: QueueContext,
}
impl Catalog {
    pub async fn album(
        &self,
        album: &AlbumRef,
        limits: Limits,
        progress: impl FnMut(usize),
    ) -> Result<PreparedAlbum> {
        album.validate()?;
        self.collect_context(
            &format!("open:albums:{}", album.id),
            album.context(),
            limits,
            progress,
        )
        .await
    }
    pub(super) async fn collect_context(
        &self,
        kind: &str,
        context: QueueContext,
        limits: Limits,
        mut progress: impl FnMut(usize),
    ) -> Result<PreparedAlbum> {
        let mut cursor = Some(format!("{}:{}:0", context.provider, context.storefront));
        let mut tracks = Vec::new();
        let mut skipped = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0;
        let mut count = 0;
        let mut expected = None;
        while let Some(next) = cursor.take() {
            ensure!(
                seen.insert(next.clone()),
                "Album/playlist pagination repeated a page"
            );
            ensure!(
                seen.len() <= MAX_QUEUE,
                "Album/playlist page limit reached; nothing committed"
            );
            let page = self
                .browse(kind, "", Some(&next), &context.storefront, limits)
                .await?;
            if let Some(total) = page.total {
                ensure!(
                    expected.is_none_or(|old| old == total),
                    "Album/playlist total changed during pagination; retry"
                );
                expected = Some(total);
                ensure!(
                    total <= MAX_QUEUE,
                    "Album/playlist exceeds 10000 records; nothing committed"
                );
            }
            count += page.items.len() + page.skipped.iter().map(|s| s.count).sum::<usize>();
            ensure!(
                count <= MAX_QUEUE,
                "Album/playlist exceeds 10000 records; nothing committed"
            );
            skipped.extend(page.skipped);
            for mut track in page.items {
                let reason = if !matches!(track.kind.as_str(), "songs" | "library-songs") {
                    Some("Unsupported media type".into())
                } else if let Some(reason) = &track.unavailable {
                    Some(reason.clone())
                } else if track.id.len() > 32 || !track.id.bytes().all(|b| b.is_ascii_digit()) {
                    Some("Native backend requires a catalog song ID".into())
                } else {
                    None
                };
                if let Some(reason) = reason {
                    skipped.push(Skipped { count: 1, reason });
                    continue;
                }
                if context.album_id.is_some() {
                    track.album_id = context.album_id.clone();
                }
                track.provider = context.provider.clone();
                track.storefront = context.storefront.clone();
                validate_track(&track)?;
                bytes += serde_json::to_vec(&track)?.len();
                ensure!(
                    bytes <= MAX_ALBUM_BYTES,
                    "Album/playlist exceeds byte limit; nothing committed"
                );
                tracks.push(track);
            }
            progress(count);
            cursor = page.next;
        }
        ensure!(
            expected.is_none_or(|total| total == count),
            "Album/playlist download is incomplete; nothing committed"
        );
        // Only sort when every record supplies both trusted ordinals. Otherwise preserve the source relation.
        if context.album_id.is_some()
            && tracks
                .iter()
                .all(|t| t.disc_number.is_some() && t.track_number.is_some())
        {
            tracks.sort_by_key(|t| (t.disc_number, t.track_number));
        }
        ensure!(
            !tracks.is_empty(),
            "Album/playlist has no playable native songs"
        );
        let mut summary: Vec<Skipped> = Vec::new();
        for skip in skipped {
            if let Some(s) = summary.iter_mut().find(|s| s.reason == skip.reason) {
                s.count += skip.count;
            } else {
                summary.push(skip);
            }
        }
        Ok(PreparedAlbum {
            tracks,
            skipped: summary,
            context,
        })
    }
}

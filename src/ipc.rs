use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const MAX_FRAME: usize = 4 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Request {
    QueueAction {
        key: String,
        revision: u64,
        action: QueueAction,
    },
    Album {
        key: String,
        revision: u64,
        album: crate::catalog::album::AlbumRef,
        action: AlbumAction,
    },
    Playlist {
        key: String,
        revision: u64,
        playlist: crate::catalog::playlist::PlaylistRef,
        action: ContextAction,
    },
    PlaylistWrite {
        key: String,
        source: PlaylistWriteSource,
        action: crate::catalog::playlist_write::WriteAction,
    },
    RetryPlaylistWrite,
    CancelOperation(String),
    RetryOperation,
    StopAudio,
    Shutdown,
    RecoverState,
    Equalizer(rustune_core::equalizer::EqSettings),
    Status,
    Subscribe,
    QueuePage {
        offset: usize,
    },
    History,
    RecordSearch(String),
    Pause,
    Play,
    Toggle,
    Next,
    Previous,
    Seek(f64),
    Volume(f64),
    Mute,
    Shuffle,
    Repeat,
    Browse {
        kind: String,
        term: String,
        next: Option<String>,
    },
    Enqueue {
        track: crate::model::Track,
        next: bool,
        play: bool,
    },
    PlayIndex(usize),
    PlayQueued {
        revision: u64,
        index: usize,
        id: String,
    },
    Remove(usize),
    QueueEdit {
        revision: u64,
        index: usize,
        id: String,
        to: Option<usize>,
    },
    Move {
        from: usize,
        to: usize,
    },
    Clear,
    Sleep(u64),
    Theme(String),
    Artwork(bool),
    Lyrics(bool),
    CatalogCountry(String),
    Profile(crate::config::PerformanceMode),
    Stop,
}
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let n = reader.read_u32().await? as usize;
    anyhow::ensure!(n > 0 && n <= MAX_FRAME, "IPC frame exceeds size limit");
    let mut b = vec![0; n];
    reader.read_exact(&mut b).await?;
    Ok(b)
}
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    let b = serde_json::to_vec(value)?;
    anyhow::ensure!(b.len() <= MAX_FRAME, "IPC frame exceeds size limit");
    writer.write_u32(b.len() as u32).await?;
    writer.write_all(&b).await?;
    Ok(())
}

pub async fn request(paths: &crate::config::Paths, request: &Request) -> Result<serde_json::Value> {
    let operation = async {
        let mut s = tokio::net::UnixStream::connect(paths.socket())
            .await
            .map_err(|_| {
                anyhow::anyhow!("Rustune service is not running; run rustune or rustune daemon")
            })?;
        write_frame(&mut s, request).await?;
        let b = read_frame(&mut s).await?;
        serde_json::from_slice::<crate::service::Reply>(&b)?.value()
    };
    tokio::time::timeout(std::time::Duration::from_secs(55), operation)
        .await
        .map_err(|_| anyhow::anyhow!("service timed out"))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum QueueAction {
    PlayContext {
        tracks: Vec<crate::model::Track>,
        context: crate::queue::QueueContext,
        start: usize,
        shuffle: bool,
    },
    PlayEntry {
        entry_id: u64,
    },
    PlayNext {
        tracks: Vec<crate::model::Track>,
        context: crate::queue::QueueContext,
    },
    Append {
        tracks: Vec<crate::model::Track>,
        context: crate::queue::QueueContext,
    },
    RemoveAndAdvance {
        entry_id: u64,
    },
    Move {
        entry_id: u64,
        before: Option<u64>,
    },
    MoveBy {
        entry_id: u64,
        delta: i8,
    },
    ClearUpcoming,
    Undo,
    SetShuffle {
        enabled: bool,
    },
    SetRepeat {
        repeat: crate::model::Repeat,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum ContextAction {
    PlayFromPosition {
        source_index: usize,
        shuffle: bool,
    },
    Play {
        #[serde(default)]
        start_song: Option<String>,
        shuffle: bool,
    },
    PlayNext,
    Append,
}
pub type AlbumAction = ContextAction;
pub fn operation_key() -> String {
    format!("{:016x}{:016x}", fastrand::u64(..), fastrand::u64(..))
}
pub fn queue_action(revision: u64, action: QueueAction) -> Request {
    Request::QueueAction {
        key: operation_key(),
        revision,
        action,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PlaylistWriteSource {
    Queue { revision: u64 },
    Entry { revision: u64, entry_id: u64 },
    Track { track: Box<crate::model::Track> },
    Empty,
}

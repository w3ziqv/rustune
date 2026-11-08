use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const MAX_FRAME: usize = 4 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Request {
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

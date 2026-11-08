use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub artwork: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playback {
    pub authorized: bool,
    pub playing: bool,
    pub state: u32,
    pub ended: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub track: Option<Track>,
    pub preview: bool,
    /// An explicitly reported target, never a measured source bitrate.
    pub target_bitrate_kbps: Option<u16>,
}
impl Playback {
    pub fn quality_label(&self) -> String {
        match self.target_bitrate_kbps {
            Some(n @ (64 | 256)) => format!("Reported target {n} kb/s · source quality unknown"),
            _ => "Source quality unknown".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Track>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Auto,
    DataSaver,
    Standard,
    HighQuality,
    Lossless,
    HiResLossless,
}
pub fn quality_supported(quality: Quality) -> bool {
    quality == Quality::Auto
}

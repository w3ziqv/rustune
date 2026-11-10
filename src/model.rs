use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: String,
    pub provider: String,
    pub storefront: String,
    pub album_id: Option<String>,
    pub source_index: Option<usize>,
    pub can_edit: Option<bool>,
    pub disc_number: Option<u32>,
    pub track_number: Option<u32>,
    pub unavailable: Option<String>,
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
    pub phase: PlaybackPhase,
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
    pub source_codec: Option<String>,
    pub source_sample_rate_hz: Option<u32>,
    pub source_channels: Option<u16>,
    pub encrypted_packets: u64,
    pub decoded_audio_frames: u64,
    pub nonzero_output_samples: u64,
}
impl Playback {
    pub fn quality_label(&self) -> String {
        if let (Some(codec), Some(rate), Some(channels)) = (
            &self.source_codec,
            self.source_sample_rate_hz,
            self.source_channels,
        ) {
            return format!(
                "{codec} · {:.1} kHz · {channels} ch · reported target {} kb/s",
                f64::from(rate) / 1000.0,
                self.target_bitrate_kbps
                    .map_or("unknown".into(), |v| v.to_string())
            );
        }
        match self.target_bitrate_kbps {
            Some(n @ (64 | 256)) => format!("Reported target {n} kb/s · source quality unknown"),
            _ => "Source quality unknown".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Track>,
    #[serde(default)]
    pub skipped: Vec<crate::catalog::album::Skipped>,
    #[serde(default)]
    pub total: Option<usize>,
    pub next: Option<String>,
    #[serde(default)]
    pub previous: Option<String>,
    #[serde(default)]
    pub storefront: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackPhase {
    #[default]
    Stopped,
    Preparing,
    Playing,
    Paused,
    Ended,
    Failed,
}

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Control {
    Start {
        id: String,
        volume: f64,
    },
    Prepare {
        id: String,
        volume: f64,
        paused: bool,
    },
    Pause(bool),
    Volume(f64),
    Seek(f64),
    Stop,
    Equalizer(crate::equalizer::EqSettings),
}
impl Control {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Equalizer(settings) => settings.validate()?,
            Self::Start { id, volume } | Self::Prepare { id, volume, .. } => {
                ensure!(
                    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit()),
                    "Native playback supports catalog songs only"
                );
                ensure!(
                    volume.is_finite() && (0.0..=1.0).contains(volume),
                    "Invalid audio volume"
                );
            }
            Self::Volume(volume) => ensure!(
                volume.is_finite() && (0.0..=1.0).contains(volume),
                "Invalid audio volume"
            ),
            Self::Seek(position) => ensure!(
                position.is_finite() && (0.0..=21600.0).contains(position),
                "Invalid audio seek position"
            ),
            _ => {}
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum Event {
    EqualizerApplied {
        settings: crate::equalizer::EqSettings,
        headroom_db: f64,
    },
    EqualizerStats {
        samples: u64,
        input_rms: f64,
        output_rms: f64,
    },
    Ready {
        duration: f64,
        sample_rate: u32,
        channels: u16,
    },
    State {
        playing: bool,
        position: f64,
        encrypted_packets: u64,
        decoded_frames: u64,
        nonzero_samples: u64,
    },
    Ended {
        position: f64,
        encrypted_packets: u64,
        decoded_frames: u64,
        nonzero_samples: u64,
    },
    Error(String),
}

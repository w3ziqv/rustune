//! Persistent, bounded equalizer settings shared by UI and audio.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
pub const FREQUENCIES: [f64; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
pub const PRESETS: [&str; 4] = ["Flat", "Bass", "Vocal", "Treble"];
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f64,
    pub bands_db: [f64; 10],
}
impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            bands_db: [0.0; 10],
        }
    }
}
impl EqSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.preamp_db.is_finite()
                && (-12.0..=12.0).contains(&self.preamp_db)
                && self
                    .bands_db
                    .iter()
                    .all(|g| g.is_finite() && (-12.0..=12.0).contains(g)),
            "EQ gains and preamp must be finite values between -12 and +12 dB"
        );
        Ok(())
    }
    pub fn preset(index: usize) -> Self {
        let bands_db = match index {
            1 => [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            2 => [-2.0, -2.0, -1.0, 0.0, 1.0, 3.0, 4.0, 2.0, 0.0, -1.0],
            3 => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 4.0],
            _ => [0.0; 10],
        };
        Self {
            enabled: true,
            preamp_db: 0.0,
            bands_db,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equalizer_config_rejects_nonfinite_and_excessive_gains() {
        assert!(EqSettings::default().validate().is_ok());
        let mut eq = EqSettings::default();
        eq.bands_db[3] = f64::NAN;
        assert!(eq.validate().is_err());
        eq.bands_db[3] = 12.5;
        assert!(eq.validate().is_err());
        eq.bands_db[3] = -12.5;
        assert!(eq.validate().is_err());
        eq.bands_db[3] = 12.0;
        eq.preamp_db = 13.0;
        assert!(eq.validate().is_err());
        eq.preamp_db = -12.0;
        assert!(eq.validate().is_ok());
        assert!(serde_json::from_str::<EqSettings>(r#"{"bands_db":[0,0]}"#).is_err());
    }
    #[test]
    fn bass_and_vocal_presets_act_on_different_real_bands() {
        let bass = EqSettings::preset(1);
        let vocal = EqSettings::preset(2);
        assert!(bass.enabled && vocal.enabled);
        assert!(bass.bands_db[1] > bass.bands_db[6]);
        assert!(vocal.bands_db[6] > vocal.bands_db[1]);
    }
}

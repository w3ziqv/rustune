//! Bounded stereo peaking EQ using the W3C/RBJ Audio EQ Cookbook.
use anyhow::{Result, ensure};
use rustune_core::equalizer::{EqSettings, FREQUENCIES};
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [[f64; 2]; 2],
    active: bool,
}
impl Biquad {
    fn magnitude(&self, omega: f64) -> f64 {
        if !self.active {
            return 1.0;
        }
        let numerator = (self.b[0] + self.b[1] * omega.cos() + self.b[2] * (2.0 * omega).cos())
            .powi(2)
            + (self.b[1] * omega.sin() + self.b[2] * (2.0 * omega).sin()).powi(2);
        let denominator = (1.0 + self.a[0] * omega.cos() + self.a[1] * (2.0 * omega).cos()).powi(2)
            + (self.a[0] * omega.sin() + self.a[1] * (2.0 * omega).sin()).powi(2);
        (numerator / denominator.max(1e-30)).sqrt()
    }
    fn peak(rate: f64, frequency: f64, gain: f64) -> Self {
        if gain.abs() < 1e-12 || frequency >= rate * 0.49 {
            return Self::default();
        }
        let omega = std::f64::consts::TAU * frequency / rate;
        let alpha = omega.sin() / (2.0 * std::f64::consts::SQRT_2);
        let amplitude = 10.0f64.powf(gain / 40.0);
        let a0 = 1.0 + alpha / amplitude;
        Self {
            b: [
                (1.0 + alpha * amplitude) / a0,
                -2.0 * omega.cos() / a0,
                (1.0 - alpha * amplitude) / a0,
            ],
            a: [-2.0 * omega.cos() / a0, (1.0 - alpha / amplitude) / a0],
            z: [[0.0; 2]; 2],
            active: true,
        }
    }
    fn sample(&mut self, input: f64, channel: usize) -> f64 {
        if !self.active {
            return input;
        }
        let output = self.b[0] * input + self.z[channel][0];
        self.z[channel][0] = self.b[1] * input - self.a[0] * output + self.z[channel][1];
        self.z[channel][1] = self.b[2] * input - self.a[1] * output;
        for state in &mut self.z[channel] {
            if state.abs() < 1e-20 {
                *state = 0.0;
            }
        }
        output
    }
}
#[derive(Clone)]
struct Bank {
    filters: [Biquad; 10],
    gain: f64,
    headroom: f64,
    enabled: bool,
}
impl Bank {
    fn new(rate: u32, settings: &EqSettings) -> Self {
        let filters: [Biquad; 10] = std::array::from_fn(|i| {
            Biquad::peak(f64::from(rate), FREQUENCIES[i], settings.bands_db[i])
        });
        let max_frequency = f64::from(rate) * 0.49;
        let peak = (0..256)
            .map(|i| 20.0 * (max_frequency / 20.0).powf(f64::from(i) / 255.0))
            .chain(FREQUENCIES.into_iter().filter(|f| *f < max_frequency))
            .map(|frequency| {
                filters
                    .iter()
                    .map(|f| f.magnitude(std::f64::consts::TAU * frequency / f64::from(rate)))
                    .product::<f64>()
            })
            .fold(1.0, f64::max);
        let boost = 20.0 * peak.log10();
        Self {
            filters,
            gain: 10.0f64.powf((settings.preamp_db - boost) / 20.0),
            headroom: -boost,
            enabled: settings.enabled,
        }
    }
    fn sample(&mut self, input: f64, channel: usize) -> f64 {
        if !self.enabled {
            return input;
        }
        let mut output = input;
        for filter in &mut self.filters {
            output = filter.sample(output, channel);
        }
        output * self.gain
    }
}
pub struct Equalizer {
    rate: u32,
    settings: EqSettings,
    current: Bank,
    previous: Bank,
    fade: usize,
    fade_frames: usize,
    pub processed_samples: u64,
    pub input_power: f64,
    pub output_power: f64,
}
impl Equalizer {
    pub fn new(rate: u32, settings: EqSettings) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&rate),
            "Unsupported equalizer sample rate"
        );
        settings.validate()?;
        let current = Bank::new(rate, &settings);
        Ok(Self {
            rate,
            settings,
            current: current.clone(),
            previous: current,
            fade: 0,
            fade_frames: (rate / 50) as usize,
            processed_samples: 0,
            input_power: 0.0,
            output_power: 0.0,
        })
    }
    pub fn settings(&self) -> &EqSettings {
        &self.settings
    }
    pub fn configure(&mut self, settings: EqSettings) -> Result<()> {
        settings.validate()?;
        if self.settings == settings {
            return Ok(());
        }
        self.previous = self.current.clone();
        self.current = Bank::new(self.rate, &settings);
        self.settings = settings;
        self.fade = self.fade_frames;
        self.processed_samples = 0;
        self.input_power = 0.0;
        self.output_power = 0.0;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.current = Bank::new(self.rate, &self.settings);
        self.previous = self.current.clone();
        self.fade = 0;
    }
    pub fn headroom_db(&self) -> f64 {
        if self.settings.enabled {
            self.current.headroom
        } else {
            0.0
        }
    }
    pub fn process(&mut self, input: &[i16], volume: f64, output: &mut Vec<i16>) {
        output.clear();
        output.reserve(input.len());
        for pair in input.as_chunks::<2>().0 {
            for (channel, value) in pair.iter().enumerate() {
                let sample = f64::from(*value);
                let current = self.current.sample(sample, channel);
                let filtered = if self.fade > 0 {
                    let previous = self.previous.sample(sample, channel);
                    let weight = 1.0 - self.fade as f64 / self.fade_frames.max(1) as f64;
                    previous + (current - previous) * weight
                } else {
                    current
                };
                if self.settings.enabled {
                    self.processed_samples += 1;
                    self.input_power += sample * sample;
                    self.output_power += filtered * filtered;
                }
                output.push((filtered * volume).clamp(-32768.0, 32767.0) as i16);
            }
            self.fade = self.fade.saturating_sub(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjacent_boosts_reserve_headroom_for_the_combined_filter_response() {
        let mut settings = EqSettings::preset(0);
        settings.bands_db[2] = 12.0;
        settings.bands_db[3] = 12.0;
        let eq = Equalizer::new(44100, settings).unwrap();
        assert!(
            eq.headroom_db() < -13.0,
            "combined headroom {}",
            eq.headroom_db()
        );
    }
    fn tone(frequency: f64, right: bool) -> Vec<i16> {
        (0..44100)
            .flat_map(|n| {
                let v = (2000.0 * (std::f64::consts::TAU * frequency * n as f64 / 44100.0).sin())
                    as i16;
                [v, if right { v } else { 0 }]
            })
            .collect()
    }
    fn power(samples: &[i16], channel: usize) -> f64 {
        samples
            .iter()
            .skip(22050 + channel)
            .step_by(2)
            .map(|s| f64::from(*s).powi(2))
            .sum::<f64>()
    }
    #[test]
    fn disabled_and_flat_eq_preserve_samples_exactly() {
        let input = tone(1234.0, true);
        let mut output = Vec::new();
        let mut eq = Equalizer::new(44100, EqSettings::default()).unwrap();
        eq.process(&input, 1.0, &mut output);
        assert_eq!(input, output);
        let mut eq = Equalizer::new(44100, EqSettings::preset(0)).unwrap();
        eq.process(&input, 1.0, &mut output);
        assert_eq!(input, output);
    }
    #[test]
    fn actual_one_khz_cut_matches_decibels_and_never_leaks_stereo() {
        let mut settings = EqSettings::preset(0);
        settings.bands_db[5] = -6.0;
        let input = tone(1000.0, false);
        let mut output = Vec::new();
        let mut eq = Equalizer::new(44100, settings).unwrap();
        eq.process(&input, 1.0, &mut output);
        let db = 10.0 * (power(&output, 0) / power(&input, 0)).log10();
        assert!((db + 6.0).abs() < 0.15, "measured cut {db} dB");
        assert!(output.iter().skip(1).step_by(2).all(|v| *v == 0));
    }
    #[test]
    fn boost_uses_headroom_and_runtime_bypass_returns_to_bit_exact_output() {
        let input = tone(1000.0, true);
        let mut settings = EqSettings::preset(0);
        settings.bands_db[5] = 6.0;
        let mut eq = Equalizer::new(44100, settings).unwrap();
        assert!(eq.headroom_db() <= -5.9);
        let mut output = Vec::new();
        eq.process(&input, 1.0, &mut output);
        assert!(output.iter().all(|v| v.abs() < 2200));
        eq.configure(EqSettings::default()).unwrap();
        eq.process(&input, 1.0, &mut output);
        assert_eq!(&output[4410..], &input[4410..]);
        eq.reset();
        eq.process(&vec![0; 88200], 1.0, &mut output);
        assert!(output.iter().all(|v| *v == 0));
    }
}

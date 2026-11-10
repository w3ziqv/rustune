//! Audio acknowledgements and generation-safe transport transitions.
use super::*;
use crate::model::PlaybackPhase;
impl Engine {
    pub(super) async fn stop_audio(&mut self) {
        self.audio_generation = self.audio_generation.wrapping_add(1);
        if let Some(audio) = self.audio.take() {
            audio.stop().await;
        }
        self.playback = Playback {
            volume: self.playback.volume,
            track: self.queue.current_track().cloned(),
            phase: PlaybackPhase::Stopped,
            ..Playback::default()
        };
        self.desired_paused = false;
        self.audio_error = None;
    }
    pub(super) async fn fail_audio(&mut self, message: String) {
        self.audio_generation = self.audio_generation.wrapping_add(1);
        if let Some(audio) = self.audio.take() {
            audio.stop().await;
        }
        self.playback.track = self.queue.current_track().cloned();
        self.playback.playing = false;
        self.playback.phase = PlaybackPhase::Failed;
        self.playback.state = 0;
        self.message = message;
        self.audio_error = Some(self.message.clone());
    }
    pub(super) async fn start_audio(&mut self, index: usize) -> Result<()> {
        self.start_audio_paused(index, false).await
    }
    pub(super) async fn start_audio_paused(&mut self, index: usize, paused: bool) -> Result<()> {
        ensure!(
            self.factory.available(&self.paths),
            crate::backend::NATIVE_UNAVAILABLE
        );
        let track = self
            .queue
            .tracks
            .get(index)
            .context("Queue entry no longer exists")?
            .track
            .clone();
        Control::Prepare {
            id: track.id.clone(),
            volume: self.playback.volume,
            paused,
        }
        .validate()?;
        self.stop_audio().await;
        self.desired_paused = paused;
        self.audio_error = None;
        self.queue.current = Some(index);
        self.playback = Playback {
            volume: self.playback.volume,
            track: Some(track.clone()),
            phase: PlaybackPhase::Preparing,
            state: 1,
            ..Playback::default()
        };
        self.message = "Preparing native Apple playback and CDM license…".into();
        self.audio = Some(
            self.factory
                .start(
                    track.id,
                    self.playback.volume,
                    paused,
                    self.audio_generation,
                    self.event_tx.clone(),
                )
                .await?,
        );
        self.audio_control(Control::Equalizer(self.config.equalizer.clone()))
            .await?;
        self.equalizer_applied = None;
        self.equalizer_samples = 0;
        Ok(())
    }
    pub(super) async fn audio_control(&mut self, control: Control) -> Result<()> {
        if self.audio.is_none() {
            ensure!(
                self.factory.available(&self.paths),
                crate::backend::NATIVE_UNAVAILABLE
            );
        }
        self.audio
            .as_mut()
            .context("No native track is active; choose a song and Play")?
            .send(control)
            .await
    }
    pub(super) async fn advance_audio(&mut self, manual: bool) {
        if self.pending.is_some() {
            self.deferred_ended = true;
            return;
        }
        let mut q = self.queue.clone();
        if let Some(next) = q.advance(manual) {
            let id = q.tracks[next].entry_id;
            let key = crate::ipc::operation_key();
            if let Err(error) = self.stage_navigation(
                q,
                operations::After::Start {
                    entry_id: id,
                    paused: self.desired_paused,
                },
                key,
            ) {
                self.fail_audio(error.to_string()).await;
            }
        } else {
            self.stop_audio().await;
            self.message = "End of queue".into();
        }
    }
    pub(super) async fn native_event(&mut self, generation: u64, event: Event) {
        if generation != self.audio_generation {
            return;
        }
        match event {
            Event::EqualizerApplied {
                settings,
                headroom_db,
            } => {
                self.equalizer_applied = Some(settings);
                self.equalizer_headroom_db = headroom_db;
                self.equalizer_samples = 0;
            }
            Event::EqualizerStats {
                samples,
                input_rms,
                output_rms,
            } => {
                self.equalizer_samples = samples;
                self.equalizer_input_rms = input_rms;
                self.equalizer_output_rms = output_rms;
            }
            Event::Ready {
                duration,
                sample_rate,
                channels,
            } => {
                self.playback.authorized = true;
                self.playback.duration = duration;
                self.playback.source_codec = Some("AAC".into());
                self.playback.source_sample_rate_hz = Some(sample_rate);
                self.playback.source_channels = Some(channels);
                self.playback.target_bitrate_kbps = Some(256);
                self.message = "Native CDM license accepted; waiting for audio state".into();
            }
            Event::State {
                playing,
                position,
                encrypted_packets,
                decoded_frames,
                nonzero_samples,
            } => {
                self.playback.playing = playing
                    && self.playback.authorized
                    && decoded_frames > 0
                    && !self.desired_paused;
                self.playback.phase = if self.playback.playing {
                    PlaybackPhase::Playing
                } else if self.desired_paused {
                    PlaybackPhase::Paused
                } else {
                    PlaybackPhase::Preparing
                };
                self.playback.state = if self.playback.playing { 2 } else { 3 };
                self.playback.position = position;
                self.playback.encrypted_packets = encrypted_packets;
                self.playback.decoded_audio_frames = decoded_frames;
                self.playback.nonzero_output_samples = nonzero_samples;
                self.message = if self.playback.playing {
                    "Native Apple Music · CDM → AAC → ALSA"
                } else if self.desired_paused {
                    "Native playback paused"
                } else {
                    "Waiting for decoded audio"
                }
                .into();
            }
            Event::Ended {
                position,
                encrypted_packets,
                decoded_frames,
                nonzero_samples,
            } => {
                self.playback.playing = false;
                self.playback.ended = true;
                self.playback.phase = PlaybackPhase::Ended;
                self.playback.state = 4;
                self.playback.position = position;
                self.playback.encrypted_packets = encrypted_packets;
                self.playback.decoded_audio_frames = decoded_frames;
                self.playback.nonzero_output_samples = nonzero_samples;
                self.audio_generation = self.audio_generation.wrapping_add(1);
                if let Some(audio) = self.audio.take() {
                    audio.stop().await;
                }
                self.advance_audio(false).await;
            }
            Event::Error(message) => self.fail_audio(message).await,
        }
    }
}

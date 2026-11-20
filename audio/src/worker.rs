//! One native CDM per owned process; credentials never travel through IPC.
use crate::config::Paths;
use anyhow::{Result, anyhow, ensure};
use ffmpeg_next as ffmpeg;
use rustune_core::protocol::{Control, Event};
use std::{
    io::{Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

fn seek_at_end(target: f64, duration: f64) -> Result<bool> {
    let target_us = (target * 1_000_000.0).round() as i64;
    let duration_us = (duration * 1_000_000.0).round() as i64;
    ensure!(
        target_us <= duration_us,
        "Seek position is beyond the end of the track"
    );
    Ok(target_us == duration_us)
}

fn verify_end(position: f64, duration: f64, encrypted: u64, decoded: u64) -> Result<()> {
    ensure!(
        encrypted > 0 && decoded > 0,
        "No encrypted audio was decoded"
    );
    ensure!(
        position.is_finite()
            && duration.is_finite()
            && duration > 0.0
            && position >= duration - 0.25
            && position <= duration + 1.0,
        "Audio stream ended before the complete track was decoded"
    );
    Ok(())
}
fn read_control(reader: &mut impl Read) -> Result<Control> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let size = u32::from_be_bytes(length) as usize;
    ensure!((1..=1024).contains(&size), "Invalid audio command size");
    let mut body = vec![0; size];
    reader.read_exact(&mut body)?;
    let command: Control =
        serde_json::from_slice(&body).map_err(|_| anyhow!("Invalid audio command"))?;
    command.validate()?;
    Ok(command)
}
fn emit(event: &Event) -> Result<()> {
    let bytes = serde_json::to_vec(event)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(())
}
pub async fn run(paths: &Paths) -> Result<()> {
    let result = run_inner(paths).await;
    if let Err(error) = result {
        // Every error crossing this boundary is a locally written, sanitized message.
        let _ = emit(&Event::Error(error.to_string()));
    }
    Ok(())
}
async fn run_inner(paths: &Paths) -> Result<()> {
    let first = read_control(&mut std::io::stdin())
        .map_err(|_| anyhow!("Invalid initial audio worker command"))?;
    let Control::Hello { version } = first else {
        anyhow::bail!("Replace both rustune and rustune-audio with a matching pair");
    };
    let supported = rustune_core::protocol::PROTOCOL_VERSION;
    emit(&Event::Hello { version: supported })?;
    if version != supported {
        return Ok(());
    }
    let (id, volume, paused) = match read_control(&mut std::io::stdin())? {
        Control::Stop => return Ok(()),
        Control::Prepare { id, volume, paused } => (id, volume, paused),
        _ => return Err(anyhow!("Audio worker requires a start command")),
    };
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        while let Ok(command) = read_control(&mut stdin) {
            if tx.send(command).is_err() {
                return;
            }
        }
        let _ = tx.send(Control::Stop);
    });
    let cdm = cdm_host::CdmHost::open(paths.cache.join("drm/libwidevinecdm.so"))
        .map_err(|_| anyhow!("Native CDM is unavailable or incompatible"))?;
    let session = super::apple::Session::new(paths).await?;
    let manifest = session.prepare(&id, &cdm).await?;
    drop(session);
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
    ffmpeg::init().map_err(|_| anyhow!("Cannot initialize native media decoder"))?;
    let mut options = ffmpeg::Dictionary::new();
    for (key, value) in [
        ("tls_verify", "1"),
        ("protocol_whitelist", "https,tls,tcp"),
        ("rw_timeout", "15000000"),
        ("probesize", "262144"),
        ("analyzeduration", "1000000"),
    ] {
        options.set(key, value);
    }
    let mut input = ffmpeg::format::input_with_dictionary(manifest.media.as_str(), options)
        .map_err(|_| anyhow!("Cannot open encrypted audio stream"))?;
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| anyhow!("Stream has no supported audio"))?;
    let index = stream.index();
    let time_base = f64::from(stream.time_base());
    let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|_| anyhow!("Invalid native audio parameters"))?;
    let mut decoder = context
        .decoder()
        .audio()
        .map_err(|_| anyhow!("Cannot open native AAC decoder"))?;
    ensure!(
        decoder.id() == ffmpeg::codec::Id::AAC
            && decoder.channels() <= 2
            && (8000..=192000).contains(&decoder.rate()),
        "Unsupported native audio format"
    );
    let rate = decoder.rate();
    let channels = decoder.channels();
    let pcm = alsa::PCM::new("default", alsa::Direction::Playback, true)
        .map_err(|_| anyhow!("Cannot open default Linux audio output"))?;
    {
        let hw = alsa::pcm::HwParams::any(&pcm)
            .map_err(|_| anyhow!("Cannot configure Linux audio output"))?;
        let configured = (|| -> alsa::Result<()> {
            hw.set_access(alsa::pcm::Access::RWInterleaved)?;
            hw.set_format(alsa::pcm::Format::s16())?;
            hw.set_channels(2)?;
            hw.set_rate(rate, alsa::ValueOr::Nearest)?;
            hw.set_buffer_time_near(100000, alsa::ValueOr::Nearest)?;
            pcm.hw_params(&hw)?;
            let sw = pcm.sw_params_current()?;
            sw.set_start_threshold(1)?;
            pcm.sw_params(&sw)?;
            Ok(())
        })();
        configured.map_err(|_| anyhow!("Cannot configure Linux audio output"))?;
    }
    let output_rate = pcm
        .hw_params_current()
        .and_then(|hw| hw.get_rate())
        .map_err(|_| anyhow!("Cannot inspect Linux audio output"))?;
    let mut resampler = super::pcm::Converter::new(
        decoder.format(),
        decoder.channel_layout(),
        rate,
        output_rate,
    )?;
    emit(&Event::Ready {
        duration: manifest.duration,
        sample_rate: rate,
        channels,
    })?;
    let mut state = AudioState::new(output_rate, volume, paused)?;
    let mut landing = None;
    let mut frame = ffmpeg::frame::Audio::empty();
    'playback: loop {
        let mut eof = false;
        let mut requested_end = false;
        loop {
            cdm.poll().map_err(|_| {
                anyhow!("Native CDM needs renewed authorization; restart this track")
            })?;
            state.commands(&rx)?;
            if state.stop {
                let _ = pcm.drop();
                return Ok(());
            }
            if let Some((request_id, target)) = state.seek.take() {
                requested_end = seek_at_end(target, manifest.duration)?;
                if !requested_end {
                    input
                        .seek((target * 1_000_000.0) as i64, ..)
                        .map_err(|_| anyhow!("This native stream does not support seeking"))?;
                }
                decoder.flush();
                resampler.reset()?;
                state.equalizer.reset();
                let _ = pcm.drop();
                pcm.prepare()
                    .map_err(|_| anyhow!("Cannot reset audio output after seeking"))?;
                state.written = 0;
                if requested_end {
                    landing = None;
                    state.base = manifest.duration;
                    emit(&Event::SeekApplied {
                        request_id,
                        position: state.base,
                    })?;
                    break;
                }
                landing = Some((request_id, target));
            }
            if state.paused && landing.is_none() {
                state.report(&pcm, output_rate)?;
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(&mut input) {
                Ok(()) => {
                    if packet.stream() != index {
                        continue;
                    }
                    let data = packet
                        .data()
                        .ok_or_else(|| anyhow!("Empty encrypted audio packet"))?;
                    ensure!(
                        data.len() <= 1024 * 1024,
                        "Native audio packet exceeds limit"
                    );
                    let clear = if let Some(side) = packet.side_data().find(|s| {
                        s.kind() == ffmpeg::codec::packet::side_data::Type::EncryptionInfo
                    }) {
                        let info = super::cenc::PacketEncryption::parse(side.data(), data.len())
                            .map_err(|_| anyhow!("Unsupported CENC packet metadata"))?;
                        let clear=cdm.decrypt(data,info.key_id,info.iv,&info.subsamples).map_err(|_|anyhow!("CDM refused to decrypt audio; license or output requirements were not met"))?;
                        ensure!(
                            clear.len() == data.len(),
                            "CDM returned an invalid audio packet"
                        );
                        state.encrypted += 1;
                        Some(clear)
                    } else {
                        None
                    };
                    if let Some(clear) = clear {
                        packet
                            .data_mut()
                            .ok_or_else(|| anyhow!("Invalid AAC packet"))?
                            .copy_from_slice(&clear);
                    }
                    decoder
                        .send_packet(&packet)
                        .map_err(|_| anyhow!("Native AAC decoding failed"))?;
                }
                Err(ffmpeg::Error::Eof) => {
                    decoder
                        .send_eof()
                        .map_err(|_| anyhow!("Cannot finish native AAC decoding"))?;
                    eof = true;
                }
                Err(_) => return Err(anyhow!("Native audio stream was interrupted")),
            }
            loop {
                match decoder.receive_frame(&mut frame) {
                    Ok(()) => {}
                    Err(ffmpeg::Error::Eof) => break,
                    Err(ffmpeg::Error::Other { errno }) if errno == nix::libc::EAGAIN => break,
                    Err(_) => return Err(anyhow!("Native AAC decoding failed")),
                }
                state.decoded += 1;
                let timestamp = frame.timestamp().map(|pts| pts as f64 * time_base);
                let frame_duration = frame.samples() as f64 / f64::from(rate);
                let applied = match landing {
                    Some((id, target)) => {
                        match landing_position(target, timestamp, frame_duration)? {
                            Some(position) => Some((id, position)),
                            None => continue,
                        }
                    }
                    None => None,
                };
                let samples = resampler.convert(&frame)?;
                if samples.is_empty() {
                    continue;
                }
                if let Some((request_id, position)) = applied {
                    landing = None;
                    state.base = position;
                    emit(&Event::SeekApplied {
                        request_id,
                        position,
                    })?;
                } else if state.written == 0
                    && let Some(position) = timestamp
                {
                    state.base = position.max(0.0);
                }
                state.output(&pcm, output_rate, &rx, &cdm, samples)?;
                if state.stop || state.seek.is_some() {
                    break;
                }
            }
            if landing.is_none() {
                state.report(&pcm, output_rate)?;
            }
            if eof {
                ensure!(
                    landing.is_none(),
                    "Stream ended before the requested seek position"
                );
                break;
            }
        }
        let mut flushed = false;
        for _ in 0..32 {
            match resampler.flush()? {
                Some(samples) => state.output(&pcm, output_rate, &rx, &cdm, samples)?,
                None => {
                    flushed = true;
                    break;
                }
            }
            if state.stop {
                let _ = pcm.drop();
                return Ok(());
            }
            if state.seek.is_some() {
                continue 'playback;
            }
        }
        ensure!(
            flushed,
            "PCM converter did not finish within its buffer limit"
        );
        // Poll the device's real queued frame count. Some PipeWire ALSA versions
        // block indefinitely in snd_pcm_drain, including for a nonblocking handle.
        let mut deadline = Instant::now() + Duration::from_secs(2);
        loop {
            cdm.poll().map_err(|_| {
                anyhow!("Native CDM needs renewed authorization; restart this track")
            })?;
            state.commands(&rx)?;
            if state.stop {
                let _ = pcm.drop();
                return Ok(());
            }
            if state.seek.is_some() {
                continue 'playback;
            }
            if state.paused {
                state.report(&pcm, output_rate)?;
                deadline = Instant::now() + Duration::from_secs(2);
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if pcm.state() == alsa::pcm::State::XRun
                || pcm.delay().is_ok_and(|n| n <= 0)
                || Instant::now() >= deadline
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        ensure!(
            pcm.state() == alsa::pcm::State::XRun || pcm.delay().is_ok_and(|n| n <= 0),
            "Audio output did not finish its queued frames"
        );
        // XRun after EOF means the submitted ring is empty. PipeWire can still
        // report a fixed downstream latency; allow those final samples to leave.
        if let Ok(latency) = pcm.delay() {
            std::thread::sleep(Duration::from_secs_f64(
                (latency.max(0) as f64 / f64::from(output_rate) + 0.02).min(0.25),
            ));
        }
        let position = state.base + state.written as f64 / f64::from(output_rate);
        // An explicit cursor move to the endpoint submits no PCM. Natural EOF
        // still requires the decoder counters and output-clock verification.
        if !requested_end {
            verify_end(position, manifest.duration, state.encrypted, state.decoded)?;
        }
        let _ = pcm.drop();
        emit(&Event::Ended {
            position,
            encrypted_packets: state.encrypted,
            decoded_frames: state.decoded,
            nonzero_samples: state.nonzero,
        })?;
        return Ok(());
    }
}
/// Decode through demux preroll even while paused. The clock starts at the
/// timestamp of the first retained frame; earlier discarded requests get no ack.
fn landing_position(
    target: f64,
    timestamp: Option<f64>,
    frame_duration: f64,
) -> Result<Option<f64>> {
    let start = timestamp
        .filter(|value| value.is_finite())
        .ok_or_else(|| anyhow!("Missing timestamp after seeking"))?;
    Ok((start + frame_duration > target).then_some(start.max(0.0)))
}
struct AudioState {
    equalizer: super::equalizer::Equalizer,
    pcm_buffer: Vec<i16>,
    volume: f64,
    paused: bool,
    stop: bool,
    seek: Option<(u64, f64)>,
    base: f64,
    written: u64,
    encrypted: u64,
    decoded: u64,
    nonzero: u64,
    last_event: Instant,
}
impl AudioState {
    fn new(output_rate: u32, volume: f64, paused: bool) -> Result<Self> {
        Ok(Self {
            equalizer: super::equalizer::Equalizer::new(
                output_rate,
                rustune_core::equalizer::EqSettings::default(),
            )?,
            pcm_buffer: Vec::with_capacity(4096),
            volume,
            paused,
            stop: false,
            seek: None,
            base: 0.0,
            written: 0,
            encrypted: 0,
            decoded: 0,
            nonzero: 0,
            last_event: Instant::now() - Duration::from_secs(1),
        })
    }
    fn commands(&mut self, rx: &mpsc::Receiver<Control>) -> Result<()> {
        while let Ok(command) = rx.try_recv() {
            match command {
                Control::Equalizer(settings) => {
                    self.equalizer.configure(settings)?;
                    emit(&Event::EqualizerApplied {
                        settings: self.equalizer.settings().clone(),
                        headroom_db: self.equalizer.headroom_db(),
                    })?;
                }
                Control::Pause(paused) => {
                    self.paused = paused;
                    self.last_event = Instant::now() - Duration::from_secs(1);
                }
                Control::Volume(volume) => self.volume = volume,
                Control::Seek {
                    request_id,
                    position,
                } => self.seek = Some((request_id, position)),
                Control::Stop => self.stop = true,
                Control::Hello { .. } | Control::Prepare { .. } => self.stop = true,
            }
        }
        Ok(())
    }
    fn position(&self, pcm: &alsa::PCM, rate: u32) -> f64 {
        let queued = pcm.delay().unwrap_or(0).max(0) as u64;
        self.base + self.written.saturating_sub(queued) as f64 / f64::from(rate)
    }
    fn report(&mut self, pcm: &alsa::PCM, rate: u32) -> Result<()> {
        if self.last_event.elapsed() >= Duration::from_millis(500) {
            emit(&Event::State {
                playing: !self.paused && self.written > 0,
                position: self.position(pcm, rate),
                encrypted_packets: self.encrypted,
                decoded_frames: self.decoded,
                nonzero_samples: self.nonzero,
            })?;
            self.last_event = Instant::now();
            let samples = self.equalizer.processed_samples;
            if samples > 0 {
                emit(&Event::EqualizerStats {
                    samples,
                    input_rms: (self.equalizer.input_power / samples as f64).sqrt() / 32768.0,
                    output_rms: (self.equalizer.output_power / samples as f64).sqrt() / 32768.0,
                })?;
            }
        }
        Ok(())
    }
    fn output(
        &mut self,
        pcm: &alsa::PCM,
        rate: u32,
        rx: &mpsc::Receiver<Control>,
        cdm: &cdm_host::CdmHost,
        samples: &[i16],
    ) -> Result<()> {
        ensure!(samples.len() <= 64 * 1024, "PCM frame exceeds limit");
        let mut scaled = std::mem::take(&mut self.pcm_buffer);
        self.equalizer.process(samples, self.volume, &mut scaled);
        self.nonzero += scaled.iter().filter(|s| **s != 0).count() as u64;
        let io = pcm
            .io_i16()
            .map_err(|_| anyhow!("Cannot write Linux audio output"))?;
        let mut offset = 0;
        let mut last_progress = Instant::now();
        while offset < scaled.len() {
            cdm.poll().map_err(|_| {
                anyhow!("Native CDM needs renewed authorization; restart this track")
            })?;
            self.commands(rx)?;
            if self.stop || self.seek.is_some() {
                self.pcm_buffer = scaled;
                return Ok(());
            }
            if self.paused {
                self.report(pcm, rate)?;
                std::thread::sleep(Duration::from_millis(10));
                last_progress = Instant::now();
                continue;
            }
            match io.writei(&scaled[offset..]) {
                Ok(n) if n > 0 => {
                    offset += n * 2;
                    self.written += n as u64;
                    last_progress = Instant::now();
                }
                Ok(_) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) if e.errno() == nix::libc::EAGAIN => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) => pcm
                    .try_recover(e, true)
                    .map_err(|_| anyhow!("Linux audio output failed"))?,
            }
            ensure!(
                last_progress.elapsed() < Duration::from_secs(3),
                "Linux audio output stopped accepting samples"
            );
        }
        self.pcm_buffer = scaled;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_microsecond_track_length_selects_eof_without_demuxing_past_it() {
        let duration = 93.99454999999998;
        assert!(seek_at_end(93.99455, duration).unwrap());
        assert!(!seek_at_end(93.994549, duration).unwrap());
        assert!(seek_at_end(93.994551, duration).is_err());
    }
    #[test]
    fn burst_keeps_only_the_applied_request_id_and_target() {
        let (tx, rx) = mpsc::sync_channel(4);
        for (request_id, position) in [(41, 6.0), (42, 10.0)] {
            tx.send(Control::Seek {
                request_id,
                position,
            })
            .unwrap();
        }
        let mut state = AudioState::new(44100, 0.5, true).unwrap();
        state.commands(&rx).unwrap();
        assert_eq!(state.seek.take(), Some((42, 10.0)));
    }
    #[test]
    fn seek_lands_on_the_first_retained_frame_timestamp() {
        assert_eq!(landing_position(5.0, Some(4.9), 0.02).unwrap(), None);
        assert_eq!(landing_position(5.0, Some(4.99), 0.02).unwrap(), Some(4.99));
        assert!(landing_position(5.0, None, 0.02).is_err());
    }
    #[test]
    fn truncated_stream_cannot_be_reported_as_a_complete_song() {
        assert!(verify_end(30.0, 359.7, 1200, 1200).is_err());
        assert!(verify_end(359.68, 359.73, 15400, 15400).is_ok());
        assert!(verify_end(f64::NAN, 359.7, 15400, 15400).is_err());
        assert!(verify_end(359.7, 359.7, 0, 15400).is_err());
    }
    #[test]
    fn worker_refuses_non_catalog_ids_and_nonfinite_controls() {
        assert!(
            Control::Prepare {
                paused: false,
                id: "1440838314".into(),
                volume: 0.7
            }
            .validate()
            .is_ok()
        );
        assert!(Control::Pause(true).validate().is_ok());
        assert!(
            Control::Seek {
                request_id: 1,
                position: 12.0
            }
            .validate()
            .is_ok()
        );
        for control in [
            Control::Prepare {
                paused: false,
                id: "../private".into(),
                volume: 0.7,
            },
            Control::Volume(f64::NAN),
            Control::Volume(1.1),
            Control::Seek {
                request_id: 1,
                position: f64::INFINITY,
            },
            Control::Seek {
                request_id: 1,
                position: -1.0,
            },
        ] {
            assert!(control.validate().is_err());
        }
    }
}

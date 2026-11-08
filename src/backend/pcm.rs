//! Bounded native PCM conversion, including rate-conversion tail samples.
use anyhow::{Result, anyhow, ensure};
use ffmpeg_next as ffmpeg;
pub struct Converter {
    context: ffmpeg::software::resampling::Context,
    output: ffmpeg::frame::Audio,
    capacity: usize,
}
impl Converter {
    pub fn new(
        format: ffmpeg::format::Sample,
        layout: ffmpeg::ChannelLayout,
        input_rate: u32,
        output_rate: u32,
    ) -> Result<Self> {
        ensure!(
            (8000..=192000).contains(&input_rate) && (8000..=192000).contains(&output_rate),
            "Unsupported PCM sample rate"
        );
        let context = ffmpeg::software::resampling::Context::get(
            format,
            layout,
            input_rate,
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed),
            ffmpeg::ChannelLayout::STEREO,
            output_rate,
        )
        .map_err(|_| anyhow!("Cannot initialize native PCM conversion"))?;
        Ok(Self {
            context,
            output: ffmpeg::frame::Audio::empty(),
            capacity: 0,
        })
    }
    fn prepare(&mut self, input_samples: usize) -> Result<()> {
        ensure!(input_samples <= 8192, "AAC frame exceeds sample limit");
        let delayed = self.context.delay().map_or(0, |d| d.input.max(0) as usize);
        ensure!(delayed <= 8192, "PCM conversion delay exceeds limit");
        let count = ((input_samples + delayed) as u64 * u64::from(self.context.output().rate))
            .div_ceil(u64::from(self.context.input().rate)) as usize
            + 32;
        ensure!(count <= 32768, "Converted PCM frame exceeds limit");
        if self.capacity < count {
            self.output = ffmpeg::frame::Audio::new(
                self.context.output().format,
                count,
                ffmpeg::ChannelLayout::STEREO,
            );
            self.capacity = count;
        }
        self.output.set_samples(self.capacity);
        Ok(())
    }
    pub fn convert(&mut self, input: &ffmpeg::frame::Audio) -> Result<&[i16]> {
        self.prepare(input.samples())?;
        self.context
            .run(input, &mut self.output)
            .map_err(|_| anyhow!("Native PCM conversion failed"))?;
        Ok(self.output.plane::<i16>(0))
    }
    pub fn flush(&mut self) -> Result<Option<&[i16]>> {
        self.prepare(0)?;
        self.context
            .flush(&mut self.output)
            .map_err(|_| anyhow!("Cannot finish native PCM conversion"))?;
        Ok((self.output.samples() > 0).then(|| self.output.plane::<i16>(0)))
    }
    pub fn reset(&mut self) -> Result<()> {
        let input = *self.context.input();
        let output = *self.context.output();
        *self = Self::new(input.format, input.channel_layout, input.rate, output.rate)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_conversion_preserves_the_complete_sample_count_and_tail() {
        ffmpeg::init().unwrap();
        let format = ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed);
        let mut input = ffmpeg::frame::Audio::new(format, 1024, ffmpeg::ChannelLayout::STEREO);
        input.set_rate(44100);
        input.plane_mut::<i16>(0).fill(1000);
        let mut converter =
            Converter::new(format, ffmpeg::ChannelLayout::STEREO, 44100, 48000).unwrap();
        let mut output = 0;
        for _ in 0..100 {
            let samples = converter.convert(&input).unwrap();
            assert!(samples.len() < 4096);
            output += samples.len() / 2;
        }
        for _ in 0..32 {
            match converter.flush().unwrap() {
                Some(samples) => output += samples.len() / 2,
                None => break,
            }
        }
        assert!(
            (output as i64 - 111456).abs() <= 1,
            "output samples {output}"
        );
        converter.reset().unwrap();
        assert!(converter.convert(&input).unwrap().len() < 4096);
    }
}

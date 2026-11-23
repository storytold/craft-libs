//! Bounded microphone audio: interleaved `f32` buffers, a channel downmix and resampling to the
//! 16 kHz mono speech models take.
//!
//! A capture device reports whatever it likes — 44.1 kHz stereo, 48 kHz mono, a buffer of
//! non-finite samples — so every conversion clamps to a range and every length is capped. The
//! limits below are checked by [`AudioBuffer::validate`] before a recognizer converts anything:
//! a hostile sample rate, channel count or buffer length is an error, not an allocation.

/// The sample rate speech models expect.
pub const TARGET_RATE: u32 = 16_000;

/// The highest input rate accepted. Rates above this are refused rather than resampled: no
/// consumer device records there, and the interpolation would carry no more speech.
pub const MAX_SAMPLE_RATE: u32 = 384_000;

/// The most channels accepted in one buffer.
pub const MAX_CHANNELS: u16 = 32;

/// The longest mono result a conversion keeps: one hour at [`TARGET_RATE`]. Longer input is
/// truncated rather than allocated.
pub const MAX_SAMPLES: usize = TARGET_RATE as usize * 60 * 60;

/// The most frames a buffer may hold: one hour at [`TARGET_RATE`], the same span as
/// [`MAX_SAMPLES`] for mono.
pub const MAX_AUDIO_SECONDS: u64 = 60 * 60;

/// The largest buffer in bytes (interleaved `f32`): the one-hour mono cap in [`MAX_SAMPLES`].
pub const MAX_AUDIO_BYTES: u64 = MAX_SAMPLES as u64 * 4;

/// The caps a recognizer applies to the audio it accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The longest buffer in seconds.
    pub max_audio_seconds: u64,
    /// The most interleaved samples in one buffer.
    pub max_samples: usize,
    /// The most bytes in one buffer (four per sample).
    pub max_bytes: u64,
    /// The most segments one session may report.
    pub max_segments: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_audio_seconds: MAX_AUDIO_SECONDS, max_samples: MAX_SAMPLES, max_bytes: MAX_AUDIO_BYTES, max_segments: 100_000 }
    }
}

impl Limits {
    /// A limit set with one second of audio and one segment, for tests of the boundaries.
    pub fn tiny() -> Self {
        Limits { max_audio_seconds: 1, max_samples: TARGET_RATE as usize, max_bytes: u64::from(TARGET_RATE) * 4, max_segments: 1 }
    }
}

/// Why a buffer cannot be used as microphone audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// A sample rate of zero has no duration and nothing to resample from.
    ZeroSampleRate,
    /// The rate is above [`MAX_SAMPLE_RATE`].
    SampleRateTooHigh { rate: u32, max: u32 },
    /// A channel count of zero has no frames.
    ZeroChannels,
    /// The channel count is above [`MAX_CHANNELS`].
    TooManyChannels { channels: u16, max: u16 },
    /// The sample count is not a whole number of frames.
    PartialFrame { samples: usize, channels: u16 },
    /// The buffer holds more samples than the configured limit.
    TooManySamples { samples: usize, max: usize },
    /// The buffer holds more bytes than the configured limit.
    TooManyBytes { bytes: u64, max: u64 },
    /// The buffer spans more time than the configured limit.
    TooLong { frames: usize, rate: u32, max_seconds: u64 },
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::ZeroSampleRate => f.write_str("the sample rate is zero"),
            AudioError::SampleRateTooHigh { rate, max } => write!(f, "the sample rate {rate} Hz is above {max} Hz"),
            AudioError::ZeroChannels => f.write_str("the channel count is zero"),
            AudioError::TooManyChannels { channels, max } => write!(f, "the channel count {channels} is above {max}"),
            AudioError::PartialFrame { samples, channels } => {
                write!(f, "{samples} samples are not a whole number of {channels}-channel frames")
            }
            AudioError::TooManySamples { samples, max } => write!(f, "{samples} samples are above the limit of {max}"),
            AudioError::TooManyBytes { bytes, max } => write!(f, "{bytes} bytes are above the limit of {max}"),
            AudioError::TooLong { frames, rate, max_seconds } => {
                write!(f, "{frames} frames at {rate} Hz are longer than {max_seconds} s")
            }
        }
    }
}

impl std::error::Error for AudioError {}

/// A block of captured audio. `samples` are interleaved by [`AudioBuffer::channels`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioBuffer {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        AudioBuffer { samples, sample_rate, channels }
    }

    /// The frames (one sample per channel) the buffer holds. A zero channel count has no frames.
    pub fn frames(&self) -> usize {
        let channels = usize::from(self.channels);
        self.samples.len().checked_div(channels).unwrap_or(0)
    }

    /// The buffer's duration in seconds, zero when the format is unusable.
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.frames() as f64 / f64::from(self.sample_rate) }
    }

    /// Check the format and the length against `limits`. This is the only place a hostile
    /// sample rate, channel count or partial frame is noticed; a buffer is never converted
    /// before it passes.
    pub fn validate(&self, limits: &Limits) -> Result<(), AudioError> {
        if self.sample_rate == 0 {
            return Err(AudioError::ZeroSampleRate);
        }
        if self.sample_rate > MAX_SAMPLE_RATE {
            return Err(AudioError::SampleRateTooHigh { rate: self.sample_rate, max: MAX_SAMPLE_RATE });
        }
        if self.channels == 0 {
            return Err(AudioError::ZeroChannels);
        }
        if self.channels > MAX_CHANNELS {
            return Err(AudioError::TooManyChannels { channels: self.channels, max: MAX_CHANNELS });
        }
        if !self.samples.len().is_multiple_of(usize::from(self.channels)) {
            return Err(AudioError::PartialFrame { samples: self.samples.len(), channels: self.channels });
        }
        if self.samples.len() > limits.max_samples {
            return Err(AudioError::TooManySamples { samples: self.samples.len(), max: limits.max_samples });
        }
        let bytes = (self.samples.len() as u64).saturating_mul(4);
        if bytes > limits.max_bytes {
            return Err(AudioError::TooManyBytes { bytes, max: limits.max_bytes });
        }
        let frames = self.frames();
        let allowed = u128::from(self.sample_rate).saturating_mul(u128::from(limits.max_audio_seconds));
        if (frames as u128) > allowed {
            return Err(AudioError::TooLong { frames, rate: self.sample_rate, max_seconds: limits.max_audio_seconds });
        }
        Ok(())
    }

    /// The buffer as mono, averaging the channels (and clamping anything not finite).
    pub fn mono(&self) -> Vec<f32> {
        downmix(&self.samples, self.channels)
    }

    /// The buffer as 16 kHz mono, ready for a [`crate::Recognizer`].
    pub fn mono_16k(&self) -> Vec<f32> {
        resample_linear(&self.mono(), self.sample_rate, TARGET_RATE)
    }
}

/// Average interleaved channels into one. A zero channel count has no samples to average, so
/// it yields nothing; a trailing partial frame contributes the channels it has.
pub fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels);
    if channels == 0 {
        return Vec::new();
    }
    let take = samples.len().min(MAX_SAMPLES.saturating_mul(channels));
    let samples = &samples[..take];
    match channels {
        1 => samples.iter().map(|sample| clean(*sample)).collect(),
        _ => samples.chunks(channels).map(|frame| clean(frame.iter().copied().sum::<f32>() / frame.len().max(1) as f32)).collect(),
    }
}

/// Resample mono audio by linear interpolation. Same-rate input comes back unchanged; either
/// rate being zero yields nothing, and the result never exceeds [`MAX_SAMPLES`].
pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == 0 || to == 0 || samples.is_empty() {
        return Vec::new();
    }
    if from == to {
        return samples.iter().take(MAX_SAMPLES).map(|sample| clean(*sample)).collect();
    }
    let ratio = f64::from(to) / f64::from(from);
    let out_len = ((samples.len() as f64) * ratio).round();
    let out_len = if out_len.is_finite() { (out_len as usize).min(MAX_SAMPLES) } else { 0 };
    let step = f64::from(from) / f64::from(to);
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * step;
        let i0 = pos.floor().max(0.0) as usize;
        let frac = (pos - i0 as f64) as f32;
        let a = samples.get(i0).copied().unwrap_or(0.0);
        let b = samples.get(i0 + 1).copied().unwrap_or(a);
        out.push(clean(a + (b - a) * frac));
    }
    out
}

fn clean(value: f32) -> f32 {
    if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mono_buffer_keeps_its_samples_and_a_stereo_one_averages_them() {
        let mono = AudioBuffer::new(vec![0.5, -0.5], TARGET_RATE, 1);
        assert_eq!(mono.frames(), 2);
        assert_eq!(mono.mono(), vec![0.5, -0.5]);
        let stereo = AudioBuffer::new(vec![1.0, -1.0, 0.5, 0.5], TARGET_RATE, 2);
        assert_eq!(stereo.frames(), 2);
        assert_eq!(stereo.mono(), vec![0.0, 0.5]);
        assert_eq!(downmix(&[1.0, 0.0, 0.0], 2), vec![0.5, 0.0], "a trailing partial frame uses the channels it has");
        assert!(downmix(&[1.0], 0).is_empty());
    }

    #[test]
    fn hostile_formats_are_refused_before_conversion() {
        let limits = Limits::default();
        let cases = [
            AudioBuffer::new(vec![], 0, 1),
            AudioBuffer::new(vec![], MAX_SAMPLE_RATE + 1, 1),
            AudioBuffer::new(vec![], TARGET_RATE, 0),
            AudioBuffer::new(vec![], TARGET_RATE, MAX_CHANNELS + 1),
            AudioBuffer::new(vec![0.0; 3], TARGET_RATE, 2),
        ];
        for buffer in cases {
            assert!(buffer.validate(&limits).is_err(), "{buffer:?}");
        }
        let tight = Limits { max_samples: 100, ..Limits::default() };
        assert!(matches!(AudioBuffer::new(vec![0.0; 101], TARGET_RATE, 1).validate(&tight), Err(AudioError::TooManySamples { .. })));
        let tight_bytes = Limits { max_bytes: 400, ..Limits::default() };
        assert!(matches!(AudioBuffer::new(vec![0.0; 101], TARGET_RATE, 1).validate(&tight_bytes), Err(AudioError::TooManyBytes { .. })));
        assert!(AudioBuffer::new(vec![], TARGET_RATE, 1).validate(&limits).is_ok(), "an empty buffer is valid");
        let one_second = AudioBuffer::new(vec![0.0; TARGET_RATE as usize], TARGET_RATE, 1);
        assert!(one_second.validate(&Limits::tiny()).is_ok());
        let two_seconds = AudioBuffer::new(vec![0.0; TARGET_RATE as usize * 2], TARGET_RATE, 1);
        let one_second_span = Limits { max_audio_seconds: 1, ..Limits::default() };
        assert!(matches!(two_seconds.validate(&one_second_span), Err(AudioError::TooLong { .. })));
        let wide = AudioBuffer::new(vec![0.0; MAX_SAMPLE_RATE as usize], MAX_SAMPLE_RATE, 1);
        assert!(wide.validate(&Limits::default()).is_ok(), "one second at the highest accepted rate is valid");
    }

    #[test]
    fn resampling_reaches_sixteen_kilohertz_and_never_exceeds_its_cap() {
        let source: Vec<f32> = (0..4_800).map(|i| (i as f32 / 4_800.0) * 2.0 - 1.0).collect();
        let out = resample_linear(&source, 48_000, TARGET_RATE);
        assert_eq!(out.len(), 1_600);
        assert!(out.iter().all(|sample| sample.is_finite() && (-1.0..=1.0).contains(sample)));
        assert_eq!(resample_linear(&source, 48_000, 48_000), source);
        assert!(resample_linear(&source, 0, TARGET_RATE).is_empty());
        assert!(resample_linear(&[], 48_000, TARGET_RATE).is_empty());
        let hostile = vec![0.0; MAX_SAMPLES];
        assert_eq!(resample_linear(&hostile, 1, MAX_SAMPLE_RATE).len(), MAX_SAMPLES, "a hostile ratio is capped");
        assert_eq!(resample_linear(&[f32::NAN, f32::INFINITY, 4.0], 1, 1), vec![0.0, 0.0, 1.0], "non-finite samples become silence");
    }

    #[test]
    fn a_stereo_buffer_reaches_mono_sixteen_kilohertz() {
        let stereo = AudioBuffer::new(vec![0.0; 48_000 * 2], 48_000, 2);
        let mono = stereo.mono_16k();
        assert!((mono.len() as i64 - TARGET_RATE as i64).abs() <= 1, "one second at 16 kHz: {}", mono.len());
        assert!(mono.iter().all(|sample| sample.is_finite()));
        assert_eq!(stereo.duration_seconds(), 1.0);
    }
}

//! Planar `f32` audio buffer.

/// Planar f32 audio.
///
/// Every channel is expected to hold the same number of frames; [`AudioBuffer::frames`] reports the
/// shortest channel so a ragged buffer never causes out-of-bounds access.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioBuffer {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// One `Vec` of samples per channel.
    pub channels: Vec<Vec<f32>>,
}

impl AudioBuffer {
    /// A silent buffer with `channels` channels of `frames` frames each.
    pub fn new(sample_rate: u32, channels: usize, frames: usize) -> Self {
        Self { sample_rate, channels: vec![vec![0.0; frames]; channels] }
    }

    /// Number of frames (the length of the shortest channel; 0 when there are no channels).
    pub fn frames(&self) -> usize {
        self.channels.iter().map(Vec::len).min().unwrap_or(0)
    }

    /// Number of channels.
    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    /// Duration in seconds (0 when the sample rate is 0).
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 / f64::from(self.sample_rate)
    }

    /// Interleaved copy of the samples (frame-major: `L0 R0 L1 R1 ...`).
    pub fn interleaved(&self) -> Vec<f32> {
        let frames = self.frames();
        let mut out = Vec::with_capacity(frames.saturating_mul(self.channels.len()));
        for f in 0..frames {
            for ch in &self.channels {
                out.push(ch.get(f).copied().unwrap_or(0.0));
            }
        }
        out
    }

    /// Build a planar buffer from interleaved samples. A trailing partial frame is dropped;
    /// `channels == 0` yields an empty buffer.
    pub fn from_interleaved(sr: u32, channels: usize, data: &[f32]) -> Self {
        if channels == 0 {
            return Self { sample_rate: sr, channels: Vec::new() };
        }
        let frames = data.len() / channels;
        let mut out: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(frames)).collect();
        for frame in data.chunks_exact(channels) {
            for (ch, &s) in out.iter_mut().zip(frame) {
                ch.push(s);
            }
        }
        Self { sample_rate: sr, channels: out }
    }

    /// Absolute peak over all channels (NaN samples are ignored).
    pub fn peak(&self) -> f32 {
        self.channels.iter().flat_map(|c| c.iter()).fold(0.0f32, |acc, &s| acc.max(s.abs()))
    }
}

//! PCM helpers shared by the WAV, AIFF and FLAC writers: quantization (with optional TPDF
//! dither), interleaving and layout checks.

use crate::{AudioBuffer, AudioError, BitDepth, Result};
use std::io::Write;

/// Converts `f32` samples to integers of a given bit depth, clamping to [-1, 1] and optionally
/// adding triangular (TPDF) dither of ±1 LSB.
pub(crate) struct Quantizer {
    bits: u32,
    dither: bool,
    rng: u64,
}

impl Quantizer {
    /// A quantizer for one channel; each channel gets an independent dither sequence.
    pub(crate) fn new(bits: u32, dither: bool, channel: usize) -> Self {
        let rng = 0x9E37_79B9_7F4A_7C15u64 ^ (channel as u64).wrapping_add(1).wrapping_mul(0xD1B5_4A32_D192_ED03);
        Self { bits: bits.clamp(2, 32), dither, rng: if rng == 0 { 1 } else { rng } }
    }

    /// One quantizer per channel.
    pub(crate) fn per_channel(bits: u32, dither: bool, channels: usize) -> Vec<Self> {
        (0..channels).map(|c| Self::new(bits, dither, c)).collect()
    }

    /// Uniform in [0, 1) from a xorshift64* generator (deterministic, no OS entropy needed).
    fn uniform(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn quantize(&mut self, x: f32) -> i32 {
        let x = if x.is_nan() { 0.0 } else { x.clamp(-1.0, 1.0) };
        let scale = (1u64 << (self.bits - 1)) as f64;
        let mut v = f64::from(x) * scale;
        if self.dither {
            v += self.uniform() - self.uniform();
        }
        let max = scale - 1.0;
        v.round().clamp(-scale, max) as i32
    }
}

/// Bits per sample at `depth`, and whether they are float.
pub(crate) fn sample_format(depth: BitDepth) -> (u16, bool) {
    match depth {
        BitDepth::Int16 => (16, false),
        BitDepth::Int24 => (24, false),
        BitDepth::Int32 => (32, false),
        BitDepth::Float32 => (32, true),
    }
}

/// Interleaves `frames` frames of planar audio (missing samples are silence) as `bits`-bit
/// samples and writes them to `out`, a few thousand frames at a time through `scratch`.
pub(crate) fn write_interleaved(
    out: &mut impl Write,
    scratch: &mut Vec<u8>,
    block: &[Vec<f32>],
    frames: usize,
    quantizers: &mut [Quantizer],
    (bits, is_float): (u16, bool),
    big_endian: bool,
) -> std::io::Result<()> {
    let size = usize::from(bits / 8);
    let mut start = 0;
    while start < frames {
        let end = frames.min(start + 4096);
        scratch.clear();
        for f in start..end {
            for (c, q) in quantizers.iter_mut().enumerate() {
                let s = block.get(c).and_then(|ch| ch.get(f)).copied().unwrap_or(0.0);
                match (is_float, big_endian) {
                    (true, false) => scratch.extend_from_slice(&sanitize_float(s).to_le_bytes()),
                    (true, true) => scratch.extend_from_slice(&sanitize_float(s).to_be_bytes()),
                    (false, false) => scratch.extend_from_slice(q.quantize(s).to_le_bytes().get(..size).unwrap_or_default()),
                    (false, true) => scratch.extend_from_slice(q.quantize(s).to_be_bytes().get(4 - size..).unwrap_or_default()),
                }
            }
        }
        out.write_all(scratch)?;
        start = end;
    }
    Ok(())
}

/// Preserve finite float samples, mapping NaN to 0 and infinities to their sign (±1).
fn sanitize_float(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else if x.is_nan() {
        0.0
    } else {
        x.signum()
    }
}

/// Check that a buffer is encodable: at least one channel and a non-zero sample rate.
pub(crate) fn validate_buffer(buf: &AudioBuffer) -> Result<()> {
    validate_layout(buf.channels.len(), buf.sample_rate)
}

/// Check that a stream is encodable: at least one channel and a non-zero sample rate.
pub(crate) fn validate_layout(channels: usize, sample_rate: u32) -> Result<()> {
    if channels == 0 {
        return Err(AudioError::Encode("buffer has no channels".into()));
    }
    if channels > usize::from(u16::MAX) {
        return Err(AudioError::Encode("too many channels".into()));
    }
    if sample_rate == 0 {
        return Err(AudioError::Encode("sample rate is 0".into()));
    }
    Ok(())
}

//! Canonical RIFF/WAVE header and interleaved sample blocks for non-seekable sinks.
//!
//! Extracted from AniCraft's WAV writer, preserving its byte-level compatibility.
//! Unlike `FileEncoder`, these primitives do not require `Seek` and leave final lengths,
//! chunk padding and sink error handling to the caller. The caller must supply a complete
//! frame count and append exactly that many frames. For odd-length data, append a pad byte
//! and account for it in the RIFF size; current AniCraft output is always stereo (even).
//! Non-finite samples become silence, including float output. This deliberately differs
//! from the general encoder's infinity handling. Prefer `FileEncoder` for new seekable sinks.

/// Sample formats written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bits {
    Int16,
    Int24,
    Float32,
}

impl Bits {
    fn bytes(self) -> u16 {
        match self {
            Bits::Int16 => 2,
            Bits::Int24 => 3,
            Bits::Float32 => 4,
        }
    }
}

const PCM: u16 = 1;
const FLOAT: u16 = 3;

/// A WAV header for `frames` frames of `channels` channels.
pub fn header(rate: u32, channels: u16, bits: Bits, frames: u64) -> Option<Vec<u8>> {
    let b = bits.bytes();
    let data = u32::try_from(frames.checked_mul(u64::from(b) * u64::from(channels))?).ok()?;
    let riff = data.checked_add(36)?;
    let block = channels.checked_mul(b)?;
    let tag = if bits == Bits::Float32 { FLOAT } else { PCM };
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&riff.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&tag.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate.saturating_mul(u32::from(block))).to_le_bytes());
    h.extend_from_slice(&block.to_le_bytes());
    h.extend_from_slice(&(b * 8).to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    Some(h)
}

/// Appends samples in `bits` format. Integers are rounded and clipped to range; `x · 32768`
/// for 16-bit, so 16-bit sources read by [`crate::decode`] come back bit-identical.
pub fn append(out: &mut Vec<u8>, samples: &[f32], bits: Bits) {
    out.reserve(samples.len() * usize::from(bits.bytes()));
    for &s in samples {
        let s = if s.is_finite() { s } else { 0.0 };
        match bits {
            Bits::Int16 => out.extend_from_slice(&((s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).to_le_bytes()),
            Bits::Int24 => {
                let v = (f64::from(s) * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend_from_slice(v.to_le_bytes().get(..3).unwrap_or(&[0, 0, 0]));
            }
            Bits::Float32 => out.extend_from_slice(&s.to_le_bytes()),
        }
    }
}


//! Canonical RIFF/WAVE header and interleaved sample blocks for non-seekable sinks.
//!
//! Extracted from AniCraft's WAV writer, preserving its byte-level compatibility.
//! Unlike `FileEncoder`, these primitives do not require `Seek` and leave final lengths,
//! chunk padding and sink error handling to the caller. The caller must supply a complete
//! frame count and append exactly that many frames. For odd-length data, append a pad byte
//! (the header includes that padding in the RIFF size); current AniCraft output is always stereo (even).
//! Non-finite samples become silence, including float output. This deliberately differs
//! from the general encoder's infinity handling. Prefer `FileEncoder` for new seekable sinks.

/// Sample formats written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bits {
    /// Signed 16-bit PCM.
    Int16,
    /// Signed 24-bit PCM.
    Int24,
    /// IEEE 754 32-bit float.
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
/// Returns `None` for zero rate/channels or any size/byte-rate field overflow.
pub fn header(rate: u32, channels: u16, bits: Bits, frames: u64) -> Option<Vec<u8>> {
    if rate == 0 || channels == 0 {
        return None;
    }
    let b = bits.bytes();
    let data = u32::try_from(frames.checked_mul(u64::from(b) * u64::from(channels))?).ok()?;
    let riff = data.checked_add(36)?.checked_add(data & 1)?;
    let block = channels.checked_mul(b)?;
    let byte_rate = rate.checked_mul(u32::from(block))?;
    let tag = if bits == Bits::Float32 { FLOAT } else { PCM };
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&riff.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&tag.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&byte_rate.to_le_bytes());
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn canonical_header_and_odd_padding() {
        let h = header(48_000, 2, Bits::Int16, 1).unwrap();
        assert_eq!(h.len(), 44);
        assert_eq!(&h[..12], b"RIFF\x28\0\0\0WAVE");
        assert_eq!(&h[28..36], &[0, 238, 2, 0, 4, 0, 16, 0]);
        let h = header(8_000, 1, Bits::Int24, 1).unwrap();
        assert_eq!(&h[4..8], &40u32.to_le_bytes());
        assert_eq!(&h[40..44], &3u32.to_le_bytes());
        assert!(header(0, 2, Bits::Int16, 1).is_none());
        assert!(header(48_000, 0, Bits::Int16, 1).is_none());
        assert!(header(u32::MAX, 2, Bits::Int16, 1).is_none());
        assert!(header(48_000, 2, Bits::Float32, u64::MAX).is_none());
        assert!(header(48_000, u16::MAX, Bits::Float32, 0).is_none());
    }

    #[test]
    fn samples_preserve_rounding_clipping_silence_and_chunk_boundaries() {
        let samples = [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        let mut pcm = Vec::new();
        append(&mut pcm, &samples, Bits::Int16);
        let expected = [-32768i16, -32768, -16384, 0, 16384, 32767, 32767, 0, 0, 0];
        assert_eq!(pcm, expected.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>());
        for bits in [Bits::Int16, Bits::Int24, Bits::Float32] {
            let mut whole = Vec::new();
            append(&mut whole, &samples, bits);
            let mut blocks = Vec::new();
            for block in samples.chunks(3) {
                append(&mut blocks, block, bits);
            }
            assert_eq!(whole, blocks);
            assert_eq!(&whole[whole.len() - 3 * usize::from(bits.bytes())..], vec![0; 3 * usize::from(bits.bytes())]);
        }
    }
}

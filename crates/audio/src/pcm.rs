//! Byte-level PCM helpers shared by the WAV, AIFF and FLAC readers: bounded readers and sample
//! decoding to `f32`.

use crate::{AudioError, Result};

/// Maximum number of decoded `f32` samples (all channels) we will produce: 4 GiB of output.
pub(crate) const MAX_DECODE_SAMPLES: u64 = (4u64 << 30) / 4;

/// Little-endian `u16` at `pos`.
#[cfg(feature = "decode-native")]
pub(crate) fn le_u16(b: &[u8], pos: usize) -> Option<u16> {
    let s = b.get(pos..pos.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

/// Little-endian `u32` at `pos`.
#[cfg(feature = "decode-native")]
pub(crate) fn le_u32(b: &[u8], pos: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(pos..pos.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(s))
}

/// Little-endian `u64` at `pos`.
#[cfg(feature = "decode-native")]
pub(crate) fn le_u64(b: &[u8], pos: usize) -> Option<u64> {
    let s: [u8; 8] = b.get(pos..pos.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(s))
}

/// Big-endian `u16` at `pos`.
#[cfg(feature = "decode-native")]
pub(crate) fn be_u16(b: &[u8], pos: usize) -> Option<u16> {
    let s: [u8; 2] = b.get(pos..pos.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_be_bytes(s))
}

/// Big-endian `u32` at `pos`.
#[cfg(feature = "decode-native")]
pub(crate) fn be_u32(b: &[u8], pos: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(pos..pos.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_be_bytes(s))
}

/// Big-endian `u64` at `pos`.
#[cfg(feature = "symphonia")]
#[cfg(feature = "decode-native")]
pub(crate) fn be_u64(b: &[u8], pos: usize) -> Option<u64> {
    let s: [u8; 8] = b.get(pos..pos.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_be_bytes(s))
}

/// Four-byte tag at `pos`.
#[cfg(feature = "decode-native")]
pub(crate) fn tag(b: &[u8], pos: usize) -> Option<[u8; 4]> {
    b.get(pos..pos.checked_add(4)?)?.try_into().ok()
}

/// How the samples in a PCM byte stream are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PcmKind {
    /// Signed (or, for 1-byte WAV, unsigned) integers occupying `bytes` bytes each, left-justified.
    Int { bytes: usize, big_endian: bool, unsigned: bool },
    /// IEEE float, 4 or 8 bytes.
    Float { bytes: usize, big_endian: bool },
}

impl PcmKind {
    pub(crate) fn bytes(self) -> usize {
        match self {
            PcmKind::Int { bytes, .. } | PcmKind::Float { bytes, .. } => bytes,
        }
    }
}

/// Decode one sample from exactly `kind.bytes()` bytes. Returns 0 for a short slice.
fn decode_sample(s: &[u8], kind: PcmKind) -> f32 {
    match kind {
        PcmKind::Int { bytes, big_endian, unsigned } => {
            if bytes == 0 || bytes > 4 || s.len() < bytes {
                return 0.0;
            }
            let mut acc: u32 = 0;
            for i in 0..bytes {
                let idx = if big_endian { i } else { bytes - 1 - i };
                let mut byte = s.get(idx).copied().unwrap_or(0);
                if unsigned && i == 0 {
                    byte ^= 0x80;
                }
                acc = (acc << 8) | u32::from(byte);
            }
            let shifted = acc << (32 - 8 * bytes as u32);
            (f64::from(shifted as i32) / 2_147_483_648.0) as f32
        }
        PcmKind::Float { bytes, big_endian } => {
            let v = match bytes {
                4 => {
                    let Some(a) = s.get(..4).and_then(|x| <[u8; 4]>::try_from(x).ok()) else { return 0.0 };
                    if big_endian { f32::from_be_bytes(a) } else { f32::from_le_bytes(a) }
                }
                8 => {
                    let Some(a) = s.get(..8).and_then(|x| <[u8; 8]>::try_from(x).ok()) else { return 0.0 };
                    (if big_endian { f64::from_be_bytes(a) } else { f64::from_le_bytes(a) }) as f32
                }
                _ => 0.0,
            };
            if v.is_finite() { v } else { 0.0 }
        }
    }
}

/// Deinterleave `frames` frames of `channels`-channel PCM from `data` into planar `f32`.
pub(crate) fn deinterleave(data: &[u8], kind: PcmKind, channels: usize, frames: u64) -> Result<Vec<Vec<f32>>> {
    let bytes = kind.bytes();
    let block = bytes.checked_mul(channels).filter(|&b| b > 0).ok_or_else(|| AudioError::Malformed("zero-sized sample frame".into()))?;
    let available = (data.len() / block) as u64;
    let frames = frames.min(available);
    let total = frames.checked_mul(channels as u64).ok_or_else(|| AudioError::TooLarge("sample count overflows".into()))?;
    if total > MAX_DECODE_SAMPLES {
        return Err(AudioError::TooLarge(format!("{total} samples exceeds the {MAX_DECODE_SAMPLES}-sample decode limit")));
    }
    let frames = usize::try_from(frames).map_err(|_| AudioError::TooLarge("frame count exceeds address space".into()))?;
    let mut out: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(frames)).collect();
    for frame in data.chunks_exact(block).take(frames) {
        for (ch, s) in out.iter_mut().zip(frame.chunks_exact(bytes)) {
            ch.push(decode_sample(s, kind));
        }
    }
    Ok(out)
}

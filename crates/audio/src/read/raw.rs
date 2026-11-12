//! The integer PCM of a WAV or AIFF file as stored, for a caller that moves it bit for bit
//! (essence embedded in an AAF or OMF file) instead of decoding it to `f32`.

use super::source::Src;
use super::{aiff, wav};
use crate::pcm::PcmKind;

/// Integer PCM taken out of a WAV or AIFF file: interleaved little-endian samples of
/// `bits.div_ceil(8)` bytes each, signed except for 8-bit, which is unsigned (the WAVE convention).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPcm {
    /// The samples; empty for a header with no data after it (a descriptor's summary).
    pub data: Vec<u8>,
    /// Interleaved channels (at least 1).
    pub channels: u16,
    /// Sample rate in Hz (at least 1).
    pub sample_rate: u32,
    /// Bits a sample occupies in `data`: 8, 16, 24 or 32.
    pub bits: u16,
}

/// The samples of a layout: `len` bytes at `start`, as far as the file has them.
fn stored(b: &[u8], start: u64, len: u64) -> &[u8] {
    let start = usize::try_from(start).unwrap_or(usize::MAX).min(b.len());
    let end = start.saturating_add(usize::try_from(len).unwrap_or(usize::MAX)).min(b.len());
    b.get(start..end).unwrap_or_default()
}

/// The integer PCM of a WAV file (plain or `WAVE_FORMAT_EXTENSIBLE` PCM), bit for bit. `None` for
/// anything else: not a WAV, float or compressed samples, no `fmt ` or `data` chunk. A file cut
/// short gives the samples that are there.
pub fn wav_pcm(bytes: &[u8]) -> Option<RawPcm> {
    let wav::WavParse::Native(layout) = wav::parse(&Src::Slice(bytes)).ok()? else { return None };
    let PcmKind::Int { bytes: width, big_endian: false, .. } = layout.kind else { return None };
    Some(RawPcm {
        data: stored(bytes, layout.data_start, layout.data_len).to_vec(),
        channels: layout.info.channels,
        sample_rate: layout.info.sample_rate,
        bits: u16::try_from(width.checked_mul(8)?).ok()?,
    })
}

/// The integer PCM of an AIFF or AIFF-C (`NONE`, `twos`, `sowt`) file, converted to the layout of
/// [`RawPcm`]: little-endian, and 8-bit samples made unsigned. `None` for anything else.
pub fn aiff_pcm(bytes: &[u8]) -> Option<RawPcm> {
    let aiff::AiffParse::Native(layout) = aiff::parse(&Src::Slice(bytes)).ok()? else { return None };
    let PcmKind::Int { bytes: width, big_endian, .. } = layout.kind else { return None };
    let stored = stored(bytes, layout.data_start, layout.data_len);
    let mut data = Vec::with_capacity(stored.len());
    for s in stored.chunks_exact(width.max(1)) {
        match (width, big_endian) {
            (1, _) => data.extend(s.iter().map(|b| b ^ 0x80)),
            (_, true) => data.extend(s.iter().rev()),
            (_, false) => data.extend_from_slice(s),
        }
    }
    Some(RawPcm { data, channels: layout.info.channels, sample_rate: layout.info.sample_rate, bits: u16::try_from(width.checked_mul(8)?).ok()? })
}

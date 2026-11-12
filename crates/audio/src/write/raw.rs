//! Minimal WAVE and AIFF files around integer PCM the caller already has as bytes (essence
//! embedded in an AAF or OMF file): the 44-byte canonical WAVE header and the plain AIFF header,
//! with nothing a reader of such a summary could trip over (no `JUNK`, no extensible `fmt `).
//! [`crate::encode`] writes full WAV/BWF/RF64 and AIFF files from `f32`.
//!
//! PCM here is interleaved little-endian samples of `bits.div_ceil(8)` bytes, 8-bit unsigned
//! (the layout of a WAVE `data` chunk). Sizes saturate at the 32-bit fields instead of wrapping.

use super::aiff::u32_to_extended;

/// A complete WAVE file around little-endian PCM.
pub fn pcm_wav_file(pcm: &[u8], channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let mut v = pcm_wav_header(u32::try_from(pcm.len()).unwrap_or(u32::MAX), channels, sample_rate, bits);
    v.extend_from_slice(pcm);
    if pcm.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// The 44 bytes of a canonical WAVE file up to and including the `data` chunk header, for
/// `data_len` bytes of PCM.
pub fn pcm_wav_header(data_len: u32, channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let block = channels.max(1).saturating_mul(bits.div_ceil(8));
    let mut v = Vec::with_capacity(44);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&36u32.saturating_add(data_len).saturating_add(data_len & 1).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.max(1).to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&sample_rate.saturating_mul(u32::from(block)).to_le_bytes());
    v.extend_from_slice(&block.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v
}

/// A complete AIFF file (big-endian, 8-bit signed) from little-endian PCM.
pub fn pcm_aiff_file(pcm: &[u8], channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let width = usize::from(bits.div_ceil(8)).max(1);
    let frames = pcm.len() / (width * usize::from(channels.max(1)));
    let mut v = pcm_aiff_header(u32::try_from(frames).unwrap_or(u32::MAX), channels, sample_rate, bits);
    let head = v.len();
    for s in pcm.chunks_exact(width) {
        if width == 1 {
            v.extend(s.iter().map(|b| b ^ 0x80)); // WAVE 8-bit is unsigned, AIFF signed
        } else {
            v.extend(s.iter().rev());
        }
    }
    if (v.len() - head) % 2 == 1 {
        v.push(0);
    }
    v
}

/// The 54 bytes of an AIFF file up to the sound data (`FORM`, `COMM`, `SSND` with its offset and
/// block size), for `frames` sample frames.
pub fn pcm_aiff_header(frames: u32, channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let data_len = u64::from(frames).saturating_mul(u64::from(channels.max(1))).saturating_mul(u64::from(bits.div_ceil(8)));
    let chunk = |extra: u64| u32::try_from(extra.saturating_add(data_len)).unwrap_or(u32::MAX);
    let mut v = Vec::with_capacity(54);
    v.extend_from_slice(b"FORM");
    v.extend_from_slice(&chunk(4 + 26 + 16 + (data_len & 1)).to_be_bytes());
    v.extend_from_slice(b"AIFFCOMM");
    v.extend_from_slice(&18u32.to_be_bytes());
    v.extend_from_slice(&channels.max(1).to_be_bytes());
    v.extend_from_slice(&frames.to_be_bytes());
    v.extend_from_slice(&bits.to_be_bytes());
    v.extend_from_slice(&u32_to_extended(sample_rate));
    v.extend_from_slice(b"SSND");
    v.extend_from_slice(&chunk(8).to_be_bytes());
    v.extend_from_slice(&[0; 8]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor for more than 4 GiB of PCM: the RIFF and FORM size fields saturate.
    #[test]
    fn huge_headers_saturate() {
        let h = pcm_wav_header(u32::MAX, 2, u32::MAX, 24);
        assert_eq!(h.get(4..8), Some(&u32::MAX.to_le_bytes()[..]));
        assert_eq!(h.get(28..32), Some(&u32::MAX.to_le_bytes()[..]), "byte rate");
        let a = pcm_aiff_header(u32::MAX, u16::MAX, 48_000, 32);
        assert_eq!(a.get(4..8), Some(&u32::MAX.to_be_bytes()[..]));
        assert_eq!(a.len(), 54);
    }

    #[test]
    fn headers_have_the_canonical_layout() {
        let h = pcm_wav_header(96, 2, 48_000, 24);
        assert_eq!(h.len(), 44);
        let mut expect = b"RIFF".to_vec();
        expect.extend_from_slice(&132u32.to_le_bytes());
        expect.extend_from_slice(b"WAVEfmt ");
        for x in [16u32, 1 | 2 << 16, 48_000, 288_000, 6 | 24 << 16] {
            expect.extend_from_slice(&x.to_le_bytes());
        }
        expect.extend_from_slice(b"data");
        expect.extend_from_slice(&96u32.to_le_bytes());
        assert_eq!(h, expect);
        // zero channels are written as one, and an odd data length counts its pad byte
        let odd = pcm_wav_file(&[1, 2, 3], 0, 8_000, 8);
        assert_eq!(odd.len(), 48);
        assert_eq!(odd.get(4..8), Some(&40u32.to_le_bytes()[..]));
        assert_eq!(odd.get(22..24), Some(&1u16.to_le_bytes()[..]));
        let a = pcm_aiff_header(8, 2, 44_100, 24);
        assert_eq!(a.get(..4), Some(&b"FORM"[..]));
        assert_eq!(a.get(4..8), Some(&(46u32 + 48).to_be_bytes()[..]));
        // 44 100 Hz as an 80-bit extended float
        assert_eq!(a.get(28..38), Some(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0][..]));
        assert_eq!(a.get(42..46), Some(&(8u32 + 48).to_be_bytes()[..]));
    }
}

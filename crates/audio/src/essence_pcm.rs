//! Compatibility WAVE and AIFF wrappers for embedded PCM essence in interchange files.
//! PCM inside the model is interleaved little-endian signed integers (8-bit unsigned for WAVE).

pub use crate::{pcm_aiff_header as aiff_header, pcm_wav_file as wav_file, pcm_wav_header as wav_header};

/// PCM, channels, sample rate and bits of a WAVE file (integer PCM only).
pub fn parse_wav(b: &[u8]) -> Option<(Vec<u8>, u16, u32, u16)> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut fmt = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32::from_le_bytes(b[at + 4..at + 8].try_into().ok()?) as usize;
        let body = &b[at + 8..(at + 8).saturating_add(len).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            let tag = u16::from_le_bytes([body[0], body[1]]);
            let ch = u16::from_le_bytes([body[2], body[3]]);
            let sr = u32::from_le_bytes(body[4..8].try_into().ok()?);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag != 1 && tag != 0xFFFE {
                return None;
            }
            if tag == 0xFFFE {
                // The first two bytes of the sub-format GUID carry the real format code.
                let sub = body.get(24..26)?;
                if u16::from_le_bytes([sub[0], sub[1]]) != 1 {
                    return None;
                }
            }
            fmt = Some((ch, sr, bits));
        } else if id == b"data" {
            let (ch, sr, bits) = fmt?;
            return Some((body.to_vec(), ch, sr, bits));
        }
        at = at.checked_add(8usize.checked_add(len)?.checked_add(len & 1)?)?;
    }
    None
}

/// A complete AIFF file (big-endian PCM) from little-endian PCM.
pub fn aiff_file(pcm: &[u8], channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let bps = bits.div_ceil(8) as usize;
    let frames = pcm.len() / (bps * channels.max(1) as usize).max(1);
    let mut v = aiff_header(frames as u32, channels, sample_rate, bits);
    for s in pcm.chunks_exact(bps.max(1)) {
        if bps == 1 {
            v.push(s[0] ^ 0x80); // WAVE 8-bit is unsigned, AIFF signed
        } else {
            v.extend(s.iter().rev());
        }
    }
    if (v.len() - 8) & 1 == 1 {
        v.push(0);
    }
    v
}

/// PCM (converted to little-endian), channels, sample rate and bits of an AIFF / AIFF-C (`NONE`
/// / `twos` / `sowt`) file.
pub fn parse_aiff(b: &[u8]) -> Option<(Vec<u8>, u16, u32, u16)> {
    if b.len() < 12 || &b[..4] != b"FORM" || !(&b[8..12] == b"AIFF" || &b[8..12] == b"AIFC") {
        return None;
    }
    let mut at = 12;
    let mut comm = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32::from_be_bytes(b[at + 4..at + 8].try_into().ok()?) as usize;
        let body = &b[at + 8..(at + 8).saturating_add(len).min(b.len())];
        if id == b"COMM" && body.len() >= 18 {
            let ch = u16::from_be_bytes([body[0], body[1]]);
            let bits = u16::from_be_bytes([body[6], body[7]]);
            let sr = from_extended(body[8..18].try_into().ok()?).round() as u32;
            let little = body.len() >= 22 && &body[18..22] == b"sowt";
            comm = Some((ch, sr, bits, little));
        } else if id == b"SSND" && body.len() >= 8 {
            let (ch, sr, bits, little) = comm?;
            let off = u32::from_be_bytes(body[..4].try_into().ok()?) as usize;
            let data = body.get(8usize.checked_add(off)?..)?;
            let bps = bits.div_ceil(8) as usize;
            let mut pcm = Vec::with_capacity(data.len());
            for s in data.chunks_exact(bps.max(1)) {
                if bps == 1 {
                    pcm.push(s[0] ^ 0x80);
                } else if little {
                    pcm.extend_from_slice(s);
                } else {
                    pcm.extend(s.iter().rev());
                }
            }
            return Some((pcm, ch, sr, bits));
        }
        at = at.checked_add(8usize.checked_add(len)?.checked_add(len & 1)?)?;
    }
    None
}

fn from_extended(b: [u8; 10]) -> f64 {
    let e = (u16::from_be_bytes([b[0], b[1]]) & 0x7FFF) as i32;
    let m = u64::from_be_bytes(b[2..10].try_into().unwrap_or([0; 8]));
    if e == 0 && m == 0 {
        return 0.0;
    }
    m as f64 / (1u64 << 63) as f64 * 2f64.powi(e - 16383)
}

#[cfg(test)]
mod tests {
    /// A mutated EDL whose clip ran for days produced an OMF descriptor for more than 4 GiB of
    /// PCM: the RIFF size field overflowed ("attempt to add with overflow").
    #[test]
    fn huge_wav_header_saturates() {
        let h = super::wav_header(u32::MAX, 2, u32::MAX, 24);
        assert_eq!(&h[4..8], &u32::MAX.to_le_bytes());
    }

    use super::*;

    #[test]
    fn accepts_truncated_essence_summaries() {
        let mut wav = wav_file(&[1, 2, 3, 4], 1, 48_000, 16);
        wav.truncate(46);
        assert_eq!(parse_wav(&wav), Some((vec![1, 2], 1, 48_000, 16)));
        let mut aiff = aiff_file(&[1, 2, 3, 4], 1, 48_000, 16);
        aiff.truncate(56);
        assert_eq!(parse_aiff(&aiff), Some((vec![1, 2], 1, 48_000, 16)));
    }

    #[test]
    fn wav_and_aiff_round_trip() {
        let pcm: Vec<u8> = (0..48u8).collect(); // 8 stereo 24-bit frames
        let w = wav_file(&pcm, 2, 48_000, 24);
        assert_eq!(parse_wav(&w), Some((pcm.clone(), 2, 48_000, 24)));
        let a = aiff_file(&pcm, 2, 44_100, 24);
        assert_eq!(parse_aiff(&a), Some((pcm.clone(), 2, 44_100, 24)));
        let p16: Vec<u8> = (0..32u8).collect();
        assert_eq!(parse_aiff(&aiff_file(&p16, 1, 96_000, 16)), Some((p16, 1, 96_000, 16)));
        assert!(parse_wav(b"RIFF").is_none());
        assert!(parse_aiff(&a[..30]).is_none());
        for cut in 0..w.len() {
            let _ = parse_wav(&w[..cut]);
            let _ = parse_aiff(&a[..cut.min(a.len())]);
        }
    }
}

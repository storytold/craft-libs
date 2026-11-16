//! Range-backed PCM WAVE reader and canonical 16-bit writer.
//!
//! Reads chunk headers and only the requested sample range; it never loads a whole file.

use crate::{AudioError, Result};
use craft_media::ByteSource;

/// Whether a prefix identifies a RIFF or RF64 WAVE file.
pub fn sniff(b: &[u8]) -> bool {
    b.len() >= 12 && (&b[0..4] == b"RIFF" || &b[0..4] == b"RF64") && &b[8..12] == b"WAVE"
}

const MAX_CHUNKS: usize = 1 << 16;
const IXML_MAX: u64 = 1 << 20;
/// Maximum sample data returned by a single range read.
pub const MAX_RANGE_BYTES: u64 = 4 << 20;

fn le16(b: &[u8], o: usize) -> u16 {
    b.get(o..o.saturating_add(2)).and_then(|s| s.try_into().ok()).map_or(0, u16::from_le_bytes)
}
fn le32(b: &[u8], o: usize) -> u32 {
    b.get(o..o.saturating_add(4)).and_then(|s| s.try_into().ok()).map_or(0, u32::from_le_bytes)
}
fn read_range(source: &impl ByteSource, off: u64, len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; len];
    source.read_at(off, &mut bytes)?;
    Ok(bytes)
}

/// A PCM WAVE file held by an arbitrary random-access source.
pub struct WavRange<S> {
    source: S,
    /// Number of complete sample frames.
    pub frames: u64,
    /// Channel count.
    pub channels: u16,
    /// Sample rate in Hz.
    pub rate: u32,
    /// Container bits per sample.
    pub bits: u16,
    /// Whether samples are floating point.
    pub float: bool,
    /// BWF time reference in samples, from bext or iXML.
    pub time_reference: Option<u64>,
    data_off: u64,
}

impl<S: ByteSource> WavRange<S> {
    /// Open with caller-selected bounds on channel count and sample rate.
    pub fn open(source: S, max_channels: u32, max_rate: u32) -> Result<Self> {
        let len = source.len();
        let mut pos: u64 = 12;
        let mut fmt = None;
        let mut data = None;
        let mut bext_tr = None;
        let mut ixml_tr = None;
        let mut ds64_data = None;
        for _ in 0..MAX_CHUNKS {
            let Some(body) = pos.checked_add(8).filter(|&b| b <= len) else { break };
            let mut hdr = [0u8; 8];
            source.read_at(pos, &mut hdr)?;
            let id = &hdr[0..4];
            let mut clen = u64::from(le32(&hdr, 4));
            let rest = len - body;
            if id == b"data" {
                if clen == 0xFFFF_FFFF {
                    clen = ds64_data.unwrap_or(rest);
                }
                clen = clen.min(rest);
            }
            if clen > rest {
                break;
            }
            match id {
                b"fmt " if clen >= 16 => {
                    let b = read_range(&source, body, clen.min(40) as usize)?;
                    let mut tag = le16(&b, 0);
                    if tag == 0xFFFE && clen >= 40 {
                        tag = le16(&b, 24);
                    }
                    fmt = Some((tag, le16(&b, 2), le32(&b, 4), le16(&b, 14)));
                }
                b"ds64" if clen >= 16 => {
                    let b = read_range(&source, body + 8, 8)?;
                    ds64_data = Some(u64::from(le32(&b, 0)) | u64::from(le32(&b, 4)) << 32);
                }
                b"data" => data = Some((body, clen)),
                b"bext" if clen >= 346 => {
                    let b = read_range(&source, body + 338, 8)?;
                    bext_tr = Some(u64::from(le32(&b, 0)) | u64::from(le32(&b, 4)) << 32);
                }
                b"iXML" => {
                    let b = read_range(&source, body, clen.min(IXML_MAX) as usize)?;
                    ixml_tr = ixml_time_reference(&b);
                }
                _ => {}
            }
            pos = body.saturating_add(clen).saturating_add(clen & 1);
        }
        let (tag, channels, rate, bits) = fmt.ok_or_else(|| AudioError::Malformed("missing fmt chunk".into()))?;
        let (data_off, dlen) = data.ok_or_else(|| AudioError::Malformed("missing data chunk".into()))?;
        let float = tag == 3;
        if !(tag == 1 || float)
            || channels == 0
            || rate == 0
            || u32::from(channels) > max_channels
            || rate > max_rate
            || !matches!(bits, 8 | 16 | 24 | 32 | 64)
        {
            return Err(AudioError::Unsupported(format!("WAV format tag {tag}, {bits} bits")));
        }
        let frames = dlen / (u64::from(channels) * u64::from(bits / 8));
        Ok(Self { source, frames, channels, rate, bits, float, time_reference: bext_tr.or(ixml_tr), data_off })
    }
    /// Bytes in one whole frame.
    pub fn frame_bytes(&self) -> usize {
        usize::from(self.channels) * usize::from(self.bits / 8)
    }
    /// Read the inclusive range of frames, capped to prevent unbounded allocation.
    pub fn read_frames(&self, first: u64, last: u64) -> Result<Vec<u8>> {
        if first > last || last >= self.frames {
            return Err(AudioError::Malformed("WAV frame range outside data".into()));
        }
        let n = (last - first + 1).saturating_mul(self.frame_bytes() as u64);
        if n > MAX_RANGE_BYTES {
            return Err(AudioError::TooLarge("WAV frame range".into()));
        }
        let off = self.data_off.saturating_add(first.saturating_mul(self.frame_bytes() as u64));
        read_range(&self.source, off, n as usize)
    }
    /// Decode a channel from a buffer returned by `read_frames`.
    pub fn sample(&self, block: &[u8], frame: usize, ch: usize) -> f32 {
        if ch >= usize::from(self.channels) {
            return 0.0;
        }
        let bps = usize::from(self.bits / 8);
        let Some(o) = frame.checked_mul(usize::from(self.channels)).and_then(|v| v.checked_add(ch)).and_then(|v| v.checked_mul(bps)) else {
            return 0.0;
        };
        let Some(b) = block.get(o..o.saturating_add(bps)) else { return 0.0 };
        match (self.bits, self.float, b) {
            (8, _, &[a]) => (a as f32 - 128.0) / 128.0,
            (16, _, &[a, b]) => i16::from_le_bytes([a, b]) as f32 / 32768.0,
            (24, _, &[a, b, c]) => (i32::from_le_bytes([0, a, b, c]) >> 8) as f32 / 8_388_608.0,
            (32, false, &[a, b, c, d]) => i32::from_le_bytes([a, b, c, d]) as f32 / 2_147_483_648.0,
            (32, true, &[a, b, c, d]) => f32::from_le_bytes([a, b, c, d]),
            (64, true, b) => f64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as f32,
            _ => 0.0,
        }
    }
}

fn ixml_time_reference(x: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(x);
    let field = |tag: &str| -> Option<u64> {
        let open = format!("<{tag}>");
        let a = text.find(&open)? + open.len();
        let b = a + text[a..].find('<')?;
        text[a..b].trim().parse().ok()
    };
    let low = field("BWF_TIME_REFERENCE_LOW")?;
    Some(low | field("BWF_TIME_REFERENCE_HIGH").unwrap_or(0) << 32)
}

/// Write interleaved samples as 16-bit PCM WAVE, using symmetric 32767 scaling.
#[cfg(feature = "pcm-encode")]
pub fn write_wav16(samples: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = samples.len().saturating_mul(2);
    let mut v = crate::pcm_wav_header(u32::try_from(data_len).unwrap_or(u32::MAX), channels, rate, 16);
    v.reserve(data_len);
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    v
}

#[cfg(all(test, feature = "pcm-encode"))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Source {
        bytes: Vec<u8>,
        reads: Arc<Mutex<Vec<(u64, usize)>>>,
    }
    impl ByteSource for Source {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
            self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((offset, buf.len()));
            self.bytes.read_at(offset, buf)
        }
    }
    #[test]
    fn bounded_reads_and_pcm_values() {
        let bytes = write_wav16(&[-1.0, -0.5, 0.0, 0.5, 1.0], 1, 48_000);
        let reads = Arc::new(Mutex::new(Vec::new()));
        let wav = WavRange::open(Source { bytes, reads: reads.clone() }, 64, 384_000).unwrap();
        assert_eq!((wav.frames, wav.channels, wav.rate, wav.bits), (5, 1, 48_000, 16));
        assert!(reads.lock().unwrap().iter().all(|(_, n)| *n <= 40));
        let data = wav.read_frames(1, 3).unwrap();
        assert_eq!(wav.sample(&data, 0, 0), -0.5);
        assert_eq!(wav.sample(&data, 2, 0), 0.5);
        assert!(wav.read_frames(0, 5).is_err());
        assert!(wav.read_frames(3, 1).is_err());
        assert_eq!(reads.lock().unwrap().last(), Some(&(46, 6)));
    }
    #[test]
    fn ixml_fallback_accepts_flat_fields() {
        let base = write_wav16(&[0.0], 1, 48_000);
        let mut bytes = base[..36].to_vec();
        let xml = b"<BWF_TIME_REFERENCE_LOW>5</BWF_TIME_REFERENCE_LOW><BWF_TIME_REFERENCE_HIGH>1</BWF_TIME_REFERENCE_HIGH>";
        bytes.extend_from_slice(b"iXML");
        bytes.extend_from_slice(&(xml.len() as u32).to_le_bytes());
        bytes.extend_from_slice(xml);
        if xml.len() & 1 != 0 {
            bytes.push(0);
        }
        bytes.extend_from_slice(&base[36..]);
        assert_eq!(WavRange::open(bytes.as_slice(), 64, 384_000).unwrap().time_reference, Some((1 << 32) + 5));
    }
}

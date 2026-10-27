//! Clean-room Ogg demuxer, written from RFC 3533 (the Ogg encapsulation format), RFC 7845 (Ogg
//! Opus) and the Vorbis I specification's Ogg mapping (section A).
//!
//! [`open`] reads every page (capture pattern, header, lacing values, CRC-32 checked; damaged
//! regions are skipped by searching for the next `OggS`), groups them by logical bitstream and
//! assembles packets — including packets that continue across pages — as lists of byte ranges,
//! with the granule position of the page each packet completes on. Codec headers (Opus:
//! `OpusHead` + `OpusTags`; Vorbis: identification, comment, setup) are collected separately from
//! the audio packets.
//!
//! [`OpusTiming`] implements RFC 7845 §4: packet start positions from granule positions and
//! TOC-derived durations, the pre-skip, and end trimming of the final page, so a player can seek
//! to any 48 kHz sample.
//!
//! Layer L0: no dependencies beyond `std`, no `unsafe`, builds for `wasm32-unknown-unknown`.
//!
//! From FilmCraft's `filmcraft-ogg` (storytold/filmcraft@8fda2f2). Public items keep their names, so
//! FilmCraft can re-export this module. `ByteSource` is the crate's one source trait (see the
//! README).

#![warn(missing_docs)]

use std::collections::HashMap;
use std::fmt;
use std::io;

pub use crate::source::{ByteSource, ReadSeekSource};

/// Errors from reading an Ogg file.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Reading the source failed.
    Io(io::Error),
    /// No Ogg page found.
    NotOgg,
    /// The file is damaged beyond recovery.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NotOgg => write!(f, "not an Ogg file"),
            Error::Invalid(s) => write!(f, "invalid Ogg: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The capture pattern at the start of a file.
pub fn sniff(head: &[u8]) -> bool {
    head.len() >= 27 && &head[..4] == b"OggS" && head[4] == 0
}

/// CRC-32 of RFC 3533 §6: polynomial 0x04C11DB7, initial value 0, no reflection, no final XOR.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            }
            *e = r;
        }
        t
    });
    data.iter().fold(0u32, |c, &b| (c << 8) ^ t[((c >> 24) as u8 ^ b) as usize])
}

/// The codec of a logical bitstream (from its first packet).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// Opus (RFC 7845).
    Opus,
    /// Vorbis.
    Vorbis,
    /// FLAC (the Ogg FLAC mapping).
    Flac,
    /// Theora video.
    Theora,
    /// Speex.
    Speex,
    /// A codec this module does not recognize.
    Unknown,
}

impl Codec {
    fn of(first: &[u8]) -> Codec {
        if first.starts_with(b"OpusHead") {
            Codec::Opus
        } else if first.starts_with(b"\x01vorbis") {
            Codec::Vorbis
        } else if first.starts_with(b"\x7fFLAC") {
            Codec::Flac
        } else if first.starts_with(b"\x80theora") {
            Codec::Theora
        } else if first.starts_with(b"Speex   ") {
            Codec::Speex
        } else {
            Codec::Unknown
        }
    }
    /// Header packets before the audio packets.
    fn header_count(self) -> usize {
        match self {
            Codec::Opus => 2,
            Codec::Vorbis | Codec::Theora => 3,
            Codec::Speex => 2,
            Codec::Flac | Codec::Unknown => 1,
        }
    }
    /// The shared identifier for this codec.
    pub fn codec_id(self) -> crate::CodecId {
        match self {
            Codec::Opus => crate::CodecId::Opus,
            Codec::Vorbis => crate::CodecId::Vorbis,
            Codec::Flac => crate::CodecId::Flac,
            Codec::Theora => crate::CodecId::Theora,
            Codec::Speex => crate::CodecId::Speex,
            Codec::Unknown => crate::CodecId::Unknown,
        }
    }
    /// A short display name: `"Opus"`, `"Vorbis"`.
    pub fn name(self) -> &'static str {
        match self {
            Codec::Opus => "Opus",
            Codec::Vorbis => "Vorbis",
            Codec::Flac => "FLAC",
            Codec::Theora => "Theora",
            Codec::Speex => "Speex",
            Codec::Unknown => "unknown",
        }
    }
}

/// A packet: its byte ranges (a packet may span pages) and the granule position of the page it
/// completes on when it is that page's last completed packet.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// Byte ranges `(offset, length)` of the packet's pieces, in order.
    pub parts: Vec<(u64, u32)>,
    /// Total packet size in bytes.
    pub size: u32,
    /// Granule position of the completing page if this is the last packet completed on it.
    pub granule: Option<i64>,
    /// Index of the page (within the stream) the packet completes on.
    pub page: u32,
    /// The packet completes on the stream's end-of-stream page.
    pub eos: bool,
    /// The first bytes of the packet (TOC for Opus, packet type for Vorbis).
    pub prefix: [u8; 2],
}

/// A logical bitstream.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Stream {
    /// The bitstream serial number.
    pub serial: u32,
    /// The codec, from the first packet.
    pub codec: Codec,
    /// Codec header packets (contents).
    pub headers: Vec<Vec<u8>>,
    /// Data packets after the headers.
    pub packets: Vec<Packet>,
    /// Pages read for this stream.
    pub pages: u32,
    /// Granule position of the last page with one.
    pub last_granule: Option<i64>,
    /// An end-of-stream page was read.
    pub saw_eos: bool,
}

impl Stream {
    /// What a decoder needs for this stream, in the container-neutral form the codec crates take:
    /// the `OpusHead`, the Vorbis headers Xiph-laced, or `fLaC` + the FLAC metadata blocks.
    pub fn codec_params(&self) -> crate::CodecParams {
        use crate::CodecId as Id;
        let first = self.headers.first().map(Vec::as_slice).unwrap_or_default();
        let le_u32 = |at: usize| first.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        match self.codec {
            Codec::Opus => crate::CodecParams::new(Id::Opus)
                .with_extradata(first)
                .with_sample_rate(48_000)
                .with_channels(first.get(9).copied().map_or(0, u16::from)),
            Codec::Vorbis => crate::CodecParams::new(Id::Vorbis)
                .with_extradata(crate::xiph_lace(&self.headers).unwrap_or_default())
                .with_sample_rate(le_u32(12))
                .with_channels(first.get(11).copied().map_or(0, u16::from)),
            // The first packet is 0x7F "FLAC", version, header count, then `fLaC` + STREAMINFO;
            // the other header packets are metadata blocks.
            Codec::Flac => {
                let mut extradata = first.get(9..).unwrap_or_default().to_vec();
                for h in self.headers.iter().skip(1) {
                    extradata.extend_from_slice(h);
                }
                // STREAMINFO follows `fLaC` and its 4-byte block header: 10 bytes of block and frame
                // sizes, then a 20-bit sample rate and 3 bits of channels less one.
                let info = first.get(27..30).map_or(0, |b| (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]));
                let channels = if info == 0 { 0 } else { ((info >> 1) & 7) as u16 + 1 };
                crate::CodecParams::new(Id::Flac).with_extradata(extradata).with_sample_rate(info >> 4).with_channels(channels)
            }
            c => crate::CodecParams::new(c.codec_id()),
        }
    }
}

/// An opened Ogg file.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct OggFile {
    /// The logical bitstreams, in the order their first pages appear.
    pub streams: Vec<Stream>,
    /// Pages dropped for a bad CRC, lost sync, truncation…
    pub warnings: Vec<String>,
}

struct Partial {
    parts: Vec<(u64, u32)>,
    size: u32,
    prefix: Vec<u8>,
}

/// Read every page and assemble the packets of every logical bitstream.
pub fn open(src: &(impl ByteSource + ?Sized)) -> Result<OggFile> {
    let len = src.len();
    let mut streams: Vec<Stream> = Vec::new();
    let mut by_serial: HashMap<u32, usize> = HashMap::new();
    let mut partial: HashMap<u32, Partial> = HashMap::new();
    let mut warnings = Vec::new();
    let mut pos = 0u64;
    let mut found_any = false;
    let mut recovery_budget = 64 << 20;
    let mut hdr = [0u8; 27 + 255];
    while pos + 27 <= len {
        src.read_at(pos, &mut hdr[..27])?;
        if &hdr[..4] != b"OggS" || hdr[4] != 0 {
            match find_capture(src, pos + 1, len, &mut recovery_budget)? {
                Some(p) => {
                    if found_any {
                        warnings.push(format!("lost sync at byte {pos}; resumed at {p}"));
                    }
                    pos = p;
                    continue;
                }
                None => break,
            }
        }
        let nseg = hdr[26] as usize;
        if pos + 27 + nseg as u64 > len {
            warnings.push(format!("truncated page header at byte {pos}"));
            break;
        }
        src.read_at(pos + 27, &mut hdr[27..27 + nseg])?;
        let body_len: u64 = hdr[27..27 + nseg].iter().map(|&b| b as u64).sum();
        let page_len = 27 + nseg as u64 + body_len;
        if pos + page_len > len {
            warnings.push(format!("truncated page at byte {pos}"));
            break;
        }
        let mut page = vec![0u8; page_len as usize];
        src.read_at(pos, &mut page)?;
        let stored_crc = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
        page[22..26].fill(0);
        if crc32(&page) != stored_crc {
            warnings.push(format!("bad CRC on page at byte {pos}"));
            pos += 1;
            continue;
        }
        found_any = true;
        let flags = page[5];
        let granule = i64::from_le_bytes(page[6..14].try_into().unwrap_or([0; 8]));
        let serial = u32::from_le_bytes([page[14], page[15], page[16], page[17]]);
        let lacing = page[27..27 + nseg].to_vec();
        let body_off = pos + 27 + nseg as u64;
        let si = *by_serial.entry(serial).or_insert_with(|| {
            streams.push(Stream {
                serial,
                codec: Codec::Unknown,
                headers: Vec::new(),
                packets: Vec::new(),
                pages: 0,
                last_granule: None,
                saw_eos: false,
            });
            streams.len() - 1
        });
        let st = &mut streams[si];
        let page_index = st.pages;
        st.pages += 1;
        if flags & 0x04 != 0 {
            st.saw_eos = true;
        }
        // a continued page without a pending packet (lost a page): drop the fragment
        let continued = flags & 0x01 != 0;
        if !continued && partial.remove(&serial).is_some() {
            warnings.push(format!("incomplete packet before page at byte {pos}"));
        }
        let mut completed: Vec<Packet> = Vec::new();
        let mut off = 0u64;
        let mut seg_start = 0u64;
        let mut skip_fragment = continued && !partial.contains_key(&serial);
        for (k, &l) in lacing.iter().enumerate() {
            off += l as u64;
            if l < 255 || k + 1 == nseg {
                let (a, n) = (seg_start, off - seg_start);
                seg_start = off;
                if skip_fragment {
                    if l < 255 {
                        skip_fragment = false;
                    }
                    continue;
                }
                let p = partial.entry(serial).or_insert_with(|| Partial { parts: Vec::new(), size: 0, prefix: Vec::new() });
                if n > 0 {
                    p.parts.push((body_off + a, n as u32));
                    p.size += n as u32;
                    let s = (body_off - pos + a) as usize;
                    let need = 2usize.saturating_sub(p.prefix.len()).min(n as usize);
                    p.prefix.extend_from_slice(&page[s..s + need]);
                }
                if l < 255
                    && let Some(p) = partial.remove(&serial)
                {
                    let mut prefix = [0u8; 2];
                    prefix[..p.prefix.len().min(2)].copy_from_slice(&p.prefix[..p.prefix.len().min(2)]);
                    completed.push(Packet { parts: p.parts, size: p.size, granule: None, page: page_index, eos: flags & 0x04 != 0, prefix });
                }
            }
        }
        if granule != -1 {
            st.last_granule = Some(granule);
            if let Some(last) = completed.last_mut() {
                last.granule = Some(granule);
            }
        }
        for pk in completed {
            if st.headers.is_empty() && st.packets.is_empty() {
                let data = read_parts(src, &pk.parts)?;
                st.codec = Codec::of(&data);
            }
            if st.headers.len() < st.codec.header_count() && st.packets.is_empty() {
                st.headers.push(read_parts(src, &pk.parts)?);
            } else {
                st.packets.push(pk);
            }
        }
        pos += page_len;
    }
    if !found_any {
        return Err(Error::NotOgg);
    }
    if !partial.is_empty() {
        warnings.push("file ends inside a packet".into());
    }
    Ok(OggFile { streams, warnings })
}

fn find_capture(src: &(impl ByteSource + ?Sized), from: u64, len: u64, budget: &mut usize) -> Result<Option<u64>> {
    let mut at = from;
    while at + 4 <= len && *budget > 0 {
        let n = ((len - at) as usize).min(1 << 20).min(*budget);
        let mut buf = vec![0u8; n];
        src.read_at(at, &mut buf)?;
        *budget = budget.saturating_sub(n);
        if let Some(i) = buf.windows(4).position(|w| w == b"OggS") {
            return Ok(Some(at + i as u64));
        }
        if n < 4 {
            break;
        }
        at += (n - 3) as u64;
    }
    Ok(None)
}

fn read_parts(src: &(impl ByteSource + ?Sized), parts: &[(u64, u32)]) -> Result<Vec<u8>> {
    let mut v = Vec::with_capacity(parts.iter().map(|p| p.1 as usize).sum());
    for &(o, n) in parts {
        let a = v.len();
        v.resize(a + n as usize, 0);
        src.read_at(o, &mut v[a..])?;
    }
    Ok(v)
}

impl OggFile {
    /// The first stream of `codec`.
    pub fn stream_of(&self, codec: Codec) -> Option<usize> {
        self.streams.iter().position(|s| s.codec == codec)
    }
    /// The bytes of packet `i` of stream `s`.
    pub fn read_packet(&self, src: &(impl ByteSource + ?Sized), s: usize, i: usize) -> Result<Vec<u8>> {
        let p = self.streams.get(s).and_then(|st| st.packets.get(i)).ok_or_else(|| Error::Invalid(format!("no packet {i} in stream {s}")))?;
        read_parts(src, &p.parts)
    }
}

/// Duration of an Opus packet in 48 kHz samples from its TOC (RFC 6716 §3.1); `None` if invalid.
pub fn opus_packet_samples(prefix: [u8; 2], size: u32) -> Option<u32> {
    if size == 0 {
        return None;
    }
    let toc = prefix[0];
    let config = toc >> 3;
    let frame = match config {
        0..=11 => [480, 960, 1920, 2880][(config & 3) as usize],
        12..=15 => [480, 960][(config & 1) as usize],
        _ => [120, 240, 480, 960][(config & 3) as usize],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => {
            if size < 2 {
                return None;
            }
            (prefix[1] & 0x3F) as u32
        }
    };
    let n = frame * frames;
    (n > 0 && n <= 5760).then_some(n)
}

/// RFC 7845 §4 timing of an Opus stream.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpusTiming {
    /// Samples to drop at the start (the `OpusHead` pre-skip).
    pub pre_skip: u32,
    /// Start of each packet's decoded output in output samples (48 kHz): 0 is the first sample
    /// after the pre-skip; the pre-skip samples have negative positions.
    pub starts: Vec<i64>,
    /// Decoded samples of each packet.
    pub durations: Vec<u32>,
    /// Playable samples (end trimming applied).
    pub total: i64,
}

impl OpusTiming {
    /// Timing of the Opus stream `s` (`pre_skip` from its `OpusHead`).
    pub fn of(stream: &Stream, pre_skip: u32) -> OpusTiming {
        let durations: Vec<u32> = stream.packets.iter().map(|p| opus_packet_samples(p.prefix, p.size).unwrap_or(0)).collect();
        let n = durations.len();
        let mut ends: Vec<Option<i64>> = vec![None; n];
        let mut i = 0;
        let mut cursor: Option<i64> = None;
        let mut trimmed_end: Option<i64> = None;
        while i < n {
            // packets completing on the same page: i..j, the last carries the page granule
            let mut j = i;
            while j < n && stream.packets[j].granule.is_none() {
                j += 1;
            }
            if j == n {
                // trailing packets without a granule (truncated file): continue from the cursor
                let mut c = cursor.unwrap_or(0);
                for k in i..n {
                    c += durations[k] as i64;
                    ends[k] = Some(c);
                }
                break;
            }
            let Some(g) = stream.packets[j].granule else { break };
            let sum: i64 = durations[i..=j].iter().map(|&d| d as i64).sum();
            let forward = cursor.map(|c| c + sum);
            let end = match forward {
                // end trimming on the last page (RFC 7845 §4.5): keep the forward positions
                Some(f) if stream.packets[j].eos && g < f => {
                    trimmed_end = Some(g);
                    f
                }
                Some(f) if f == g => f,
                // first page, or a discontinuity: trust the granule
                _ => g,
            };
            let mut e = end;
            for k in (i..=j).rev() {
                ends[k] = Some(e);
                e -= durations[k] as i64;
            }
            cursor = Some(end);
            i = j + 1;
        }
        let raw_starts: Vec<i64> = ends.iter().zip(&durations).map(|(e, &d)| e.unwrap_or(0) - d as i64).collect();
        let base = raw_starts.first().copied().unwrap_or(0).max(0);
        let shift = base + pre_skip as i64;
        let starts: Vec<i64> = raw_starts.iter().map(|s| s - shift).collect();
        let end_all = trimmed_end.or(cursor).unwrap_or(0);
        let total = (end_all - shift).max(0);
        OpusTiming { pre_skip, starts, durations, total }
    }
}

/// Vorbis identification header fields (Vorbis I §4.2.2).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VorbisInfo {
    /// Channel count.
    pub channels: u8,
    /// Samples per second.
    pub sample_rate: u32,
    /// Nominal bit rate in bits per second (0 or negative when not set).
    pub bitrate_nominal: i32,
}

impl VorbisInfo {
    /// Reads the Vorbis identification header; `None` when it is not one.
    pub fn parse(h: &[u8]) -> Option<VorbisInfo> {
        if h.len() < 30 || !h.starts_with(b"\x01vorbis") {
            return None;
        }
        Some(VorbisInfo {
            channels: h[11],
            sample_rate: u32::from_le_bytes([h[12], h[13], h[14], h[15]]),
            bitrate_nominal: i32::from_le_bytes([h[20], h[21], h[22], h[23]]),
        })
    }
}

#[cfg(test)]
mod tests;

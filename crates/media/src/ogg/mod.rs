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
    crc32_update(0, data)
}

/// `CRC_TABLES[k][b]`: the CRC register after feeding byte `b` followed by `k` zero bytes
/// (slice-by-16: sixteen independent lookups per 16 input bytes instead of a serial chain of 16).
static CRC_TABLES: [[u32; 256]; 16] = crc_tables();

const fn crc_tables() -> [[u32; 256]; 16] {
    let mut t = [[0u32; 256]; 16];
    let mut i = 0;
    while i < 256 {
        let mut r = (i as u32) << 24;
        let mut b = 0;
        while b < 8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            b += 1;
        }
        t[0][i] = r;
        i += 1;
    }
    let mut k = 1;
    while k < 16 {
        let mut i = 0;
        while i < 256 {
            let p = t[k - 1][i];
            t[k][i] = (p << 8) ^ t[0][(p >> 24) as usize];
            i += 1;
        }
        k += 1;
    }
    t
}

/// Continue an RFC 3533 CRC-32 over `data` from register `crc`.
pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    let t = &CRC_TABLES;
    let (chunks, rest) = data.as_chunks::<16>();
    for c in chunks {
        let w = crc ^ u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        crc = t[15][(w >> 24) as usize]
            ^ t[14][((w >> 16) & 0xFF) as usize]
            ^ t[13][((w >> 8) & 0xFF) as usize]
            ^ t[12][(w & 0xFF) as usize]
            ^ t[11][c[4] as usize]
            ^ t[10][c[5] as usize]
            ^ t[9][c[6] as usize]
            ^ t[8][c[7] as usize]
            ^ t[7][c[8] as usize]
            ^ t[6][c[9] as usize]
            ^ t[5][c[10] as usize]
            ^ t[4][c[11] as usize]
            ^ t[3][c[12] as usize]
            ^ t[2][c[13] as usize]
            ^ t[1][c[14] as usize]
            ^ t[0][c[15] as usize];
    }
    rest.iter().fold(crc, |c, &b| (c << 8) ^ t[0][((c >> 24) as u8 ^ b) as usize])
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
    /// Header packets before the audio packets (for FLAC see [`flac_header_count`]).
    fn header_count(self) -> usize {
        match self {
            Codec::Opus => 2,
            Codec::Vorbis | Codec::Theora => 3,
            Codec::Speex => 2, // see `speex_header_count`
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

/// The most header bytes kept for one stream: room for two of the largest FLAC metadata blocks (a
/// 4-byte block header and up to 2^24 - 1 bytes, such as a picture) and the rest of the headers.
const MAX_HEADER_BYTES: usize = 64 << 20;
/// Bytes of a stream's first packet that identify its codec and header count (the Speex header's
/// `extra_headers` field ends at 72).
const FIRST_PACKET_PREFIX: usize = 80;

/// Header packets of an Ogg FLAC stream, from its first packet (0x7F "FLAC", version, a 16-bit
/// big-endian count of the header packets after it, then `fLaC` and STREAMINFO): that count plus
/// the first. A count of 0 means unknown: the metadata packets then run to the one whose block
/// header has the last-block flag, or end with STREAMINFO when its own flag is set.
fn flac_header_count(first: &[u8]) -> usize {
    match first.get(7..9).map(|b| u16::from_be_bytes([b[0], b[1]])) {
        Some(n) if n > 0 => 1 + usize::from(n),
        _ if first.get(13).is_some_and(|b| b & 0x80 != 0) => 1,
        _ => usize::MAX,
    }
}

/// Header packets of a Speex stream: the Speex header, the comment header and the
/// `extra_headers` the Speex header states (a 32-bit little-endian field at offset 68).
fn speex_header_count(first: &[u8]) -> usize {
    let extra = first.get(68..72).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    2 + usize::try_from(extra).unwrap_or(usize::MAX).min(usize::from(u16::MAX))
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
    /// How many header packets precede the audio, once the first packet has said.
    header_count: usize,
    /// The bytes of `headers`, kept as they are added.
    header_bytes: usize,
    /// The page sequence number the next page should carry.
    next_seq: Option<u32>,
}

impl Stream {
    /// What a decoder needs for this stream, in the container-neutral form the codec crates take:
    /// the `OpusHead`, the Vorbis or Theora headers Xiph-laced, `fLaC` + the FLAC metadata blocks
    /// (the last one flagged last), or the Speex header.
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
                // block headers: STREAMINFO's after `fLaC`, then one per metadata packet
                let mut blocks = vec![4];
                for h in self.headers.iter().skip(1) {
                    blocks.push(extradata.len());
                    extradata.extend_from_slice(h);
                }
                // exactly the last block is flagged last, also when an audio frame ended the
                // metadata of a stream that does not state its header count
                let blocks: Vec<usize> = blocks.into_iter().filter(|&at| at < extradata.len()).collect();
                for (k, &at) in blocks.iter().enumerate() {
                    if k + 1 == blocks.len() {
                        extradata[at] |= 0x80;
                    } else {
                        extradata[at] &= 0x7F;
                    }
                }
                // STREAMINFO follows `fLaC` and its 4-byte block header: 10 bytes of block and frame
                // sizes, then a 20-bit sample rate and 3 bits of channels less one.
                let info = first.get(27..30).map_or(0, |b| (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]));
                let channels = if info == 0 { 0 } else { ((info >> 1) & 7) as u16 + 1 };
                crate::CodecParams::new(Id::Flac).with_extradata(extradata).with_sample_rate(info >> 4).with_channels(channels)
            }
            // the identification header: version, the frame size in macroblocks, then the 24-bit
            // big-endian picture width and height at offsets 14 and 17
            Codec::Theora => {
                let be_u24 = |at: usize| first.get(at..at + 3).map_or(0, |b| (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]));
                crate::CodecParams::new(Id::Theora)
                    .with_extradata(crate::xiph_lace(&self.headers).unwrap_or_default())
                    .with_size(be_u24(14), be_u24(17))
            }
            // the Speex header: rate at offset 36 and channels at 48, 32-bit little-endian
            Codec::Speex => crate::CodecParams::new(Id::Speex)
                .with_extradata(first)
                .with_sample_rate(le_u32(36))
                .with_channels(u16::try_from(le_u32(48)).unwrap_or(0)),
            Codec::Unknown => crate::CodecParams::new(Id::Unknown),
        }
    }
}

impl Stream {
    /// File a completed packet: the first one sets the codec and how many headers follow it, then
    /// header packets are read in, and the rest are audio.
    fn add_packet(&mut self, src: &(impl ByteSource + ?Sized), pk: Packet, warnings: &mut Warnings) -> Result<()> {
        let size = pk.size as usize;
        // the first packet is read once: for its codec, and then as a header (a packet too large
        // to be one is only read as far as the codec needs)
        let mut first = None;
        if self.headers.is_empty() && self.packets.is_empty() {
            let data = read_parts(src, &pk.parts, if size <= MAX_HEADER_BYTES { size } else { FIRST_PACKET_PREFIX })?;
            self.codec = Codec::of(&data);
            self.header_count = match self.codec {
                Codec::Flac => flac_header_count(&data),
                Codec::Speex => speex_header_count(&data),
                c => c.header_count(),
            };
            first = Some(data).filter(|d| d.len() == size);
        }
        if self.headers.len() >= self.header_count || !self.packets.is_empty() {
            self.packets.push(pk);
            return Ok(());
        }
        // a count the file states (FLAC, Speex) or leaves open is capped, so a hostile first
        // packet cannot turn the audio into headers; a packet past the cap is not read
        if self.headers.len() >= MAX_HEADERS || self.header_bytes.saturating_add(size) > MAX_HEADER_BYTES {
            warnings.push(format!("stream {}: header packets beyond {MAX_HEADERS} or {MAX_HEADER_BYTES} bytes read as data", self.serial));
            self.header_count = self.headers.len();
            self.packets.push(pk);
            return Ok(());
        }
        let data = match first {
            Some(data) => data,
            None => read_parts(src, &pk.parts, size)?,
        };
        if self.header_count == usize::MAX && !self.headers.is_empty() {
            // FLAC with an unstated count: metadata packets run to the one flagged last. A packet
            // that is no metadata block (an audio frame starts with 0xFF) ends them, so audio is
            // never held as headers.
            if !is_flac_metadata_block(&data) {
                self.header_count = self.headers.len();
                self.packets.push(pk);
                return Ok(());
            }
            if data[0] & 0x80 != 0 {
                self.header_count = self.headers.len() + 1;
            }
        }
        self.header_bytes = self.header_bytes.saturating_add(data.len());
        self.headers.push(data);
        Ok(())
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

/// Bytes the damage-recovery scan may read in one call.
const SCAN_CHUNK: usize = 64 << 10;
/// Bytes damage recovery may read again beyond twice the file's length (see [`Recovery`]).
const RECOVERY_ALLOWANCE: u64 = 1 << 20;
/// Cap on the packets kept from one file, all streams and headers together.
const MAX_PACKETS: usize = 1 << 26;
/// Bytes of file each kept packet needs beyond the first `MIN_PACKET_ALLOWANCE`. A kept packet
/// takes about 60 bytes of memory and a zero lacing value describes one in a single byte, so
/// without this a file could make the packet lists about 60 times its own size.
const FILE_BYTES_PER_PACKET: u64 = 8;
/// Packets any file may keep, however short.
const MIN_PACKET_ALLOWANCE: usize = 1 << 20;
/// The most header packets kept for one stream, whatever count its first packet states.
const MAX_HEADERS: usize = 1024;
/// Warnings kept about damaged pages; the rest are counted in one closing warning.
const MAX_DAMAGE_WARNINGS: usize = 1024;

/// The warnings of [`open`]: those about damaged pages are capped at `MAX_DAMAGE_WARNINGS`, so a
/// file of damage cannot make a warning per byte.
#[derive(Default)]
struct Warnings {
    list: Vec<String>,
    damage: usize,
}

impl Warnings {
    fn push(&mut self, warning: String) {
        self.list.push(warning);
    }
    fn damage(&mut self, warning: impl FnOnce() -> String) {
        if self.damage < MAX_DAMAGE_WARNINGS {
            self.list.push(warning());
        }
        self.damage += 1;
    }
    fn finish(mut self) -> Vec<String> {
        if let Some(more) = self.damage.checked_sub(MAX_DAMAGE_WARNINGS).filter(|&n| n > 0) {
            self.list.push(format!("{more} more damaged regions not listed"));
        }
        self.list
    }
}

/// The cost of damage recovery. Every read of a page or a scan in [`open`] is charged here: the
/// first read of a byte is free, and bytes read again (scanning through the body of a page that
/// failed its CRC, a scan's read past the pattern it found, the overlapping bodies of fake page
/// headers) are charged to a budget of twice the file's length plus `RECOVERY_ALLOWANCE`. Pages
/// are then read at most about three times over, and damage costs a large file no more, in
/// proportion, than a small one. Header packets are read once more when they are filed; packets do
/// not overlap, so that adds at most the file's length.
struct Recovery {
    budget: u64,
    /// Bytes before this offset have been read.
    seen: u64,
}

impl Recovery {
    fn new(len: u64) -> Recovery {
        Recovery { budget: len.saturating_mul(2).saturating_add(RECOVERY_ALLOWANCE), seen: 0 }
    }
    /// Charge reading `start..end`; false when the budget does not cover it.
    fn charge(&mut self, start: u64, end: u64) -> bool {
        let again = self.seen.min(end).saturating_sub(start);
        self.seen = self.seen.max(end);
        match self.budget.checked_sub(again) {
            Some(left) => {
                self.budget = left;
                true
            }
            None => {
                self.budget = 0;
                false
            }
        }
    }
}

/// Where a damage-recovery scan stopped.
#[derive(Debug, PartialEq, Eq)]
enum Capture {
    /// The next capture pattern starts here.
    At(u64),
    /// The source ended first.
    End,
    /// The file's recovery budget ran out first.
    BudgetSpent,
}

/// Read every page and assemble the packets of every logical bitstream.
pub fn open(src: &(impl ByteSource + ?Sized)) -> Result<OggFile> {
    let len = src.len();
    let mut streams: Vec<Stream> = Vec::new();
    let mut by_serial: HashMap<u32, usize> = HashMap::new();
    let mut partial: HashMap<u32, Partial> = HashMap::new();
    let mut warnings = Warnings::default();
    let mut pos = 0u64;
    let mut found_any = false;
    let mut recovery = Recovery::new(len);
    let mut packet_budget = usize::try_from(len / FILE_BYTES_PER_PACKET).unwrap_or(usize::MAX).clamp(MIN_PACKET_ALLOWANCE, MAX_PACKETS);
    let mut hdr = [0u8; 27 + 255];
    while pos + 27 <= len {
        src.read_at(pos, &mut hdr[..27])?;
        if !recovery.charge(pos, pos + 27) {
            warnings.push(past_budget(pos));
            break;
        }
        if &hdr[..4] != b"OggS" || hdr[4] != 0 {
            match find_capture(src, pos + 1, len, &mut recovery)? {
                Capture::At(p) => {
                    if found_any {
                        warnings.damage(|| format!("lost sync at byte {pos}; resumed at {p}"));
                    }
                    pos = p;
                    continue;
                }
                Capture::End => break,
                Capture::BudgetSpent => {
                    warnings.push(past_budget(pos));
                    break;
                }
            }
        }
        let nseg = hdr[26] as usize;
        let head = 27 + nseg;
        let page_len = if pos + head as u64 <= len {
            src.read_at(pos + 27, &mut hdr[27..head])?;
            head as u64 + hdr[27..head].iter().map(|&b| b as u64).sum::<u64>()
        } else {
            u64::MAX
        };
        let fits = pos.saturating_add(page_len) <= len;
        // the lacing values and the body: fake page headers a few bytes apart would otherwise
        // each have their overlapping bodies read and hashed for free
        if !recovery.charge(pos + 27, if fits { pos + page_len } else { (pos + head as u64).min(len) }) {
            warnings.push(past_budget(pos));
            break;
        }
        if !fits {
            // truncated, or a damaged lacing value: look for a page further on
            warnings.damage(|| format!("page at byte {pos} runs past the end of the file"));
            pos += 1;
            continue;
        }
        let mut page = vec![0u8; page_len as usize];
        page[..head].copy_from_slice(&hdr[..head]);
        src.read_at(pos + head as u64, &mut page[head..])?;
        let stored_crc = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
        page[22..26].fill(0);
        if crc32(&page) != stored_crc {
            warnings.damage(|| format!("bad CRC on page at byte {pos}"));
            pos += 1;
            continue;
        }
        found_any = true;
        let flags = page[5];
        let granule = i64::from_le_bytes(page[6..14].try_into().unwrap_or([0; 8]));
        let serial = u32::from_le_bytes([page[14], page[15], page[16], page[17]]);
        let seq = u32::from_le_bytes([page[18], page[19], page[20], page[21]]);
        let lacing = page[27..27 + nseg].to_vec();
        let body_off = pos + 27 + nseg as u64;
        // a beginning-of-stream page for a serial already seen starts the next link of a chained
        // file (RFC 3533 §4), which may reuse the serial; one that carries on the page sequence of
        // a stream with no end-of-stream page yet is a stray flag in that stream
        if flags & 0x02 != 0
            && let Some(&si) = by_serial.get(&serial)
        {
            let st = &streams[si];
            if !st.saw_eos && st.next_seq == Some(seq) {
                warnings.damage(|| format!("stray beginning-of-stream flag on page at byte {pos}, read as part of stream {serial}"));
            } else {
                if !st.saw_eos {
                    warnings.damage(|| format!("stream {serial} starts again at byte {pos} before its end-of-stream page"));
                }
                by_serial.remove(&serial);
                if partial.remove(&serial).is_some() {
                    warnings.damage(|| format!("incomplete packet before page at byte {pos}"));
                }
            }
        }
        let si = *by_serial.entry(serial).or_insert_with(|| {
            streams.push(Stream {
                serial,
                codec: Codec::Unknown,
                headers: Vec::new(),
                packets: Vec::new(),
                pages: 0,
                last_granule: None,
                saw_eos: false,
                header_count: 1,
                header_bytes: 0,
                next_seq: None,
            });
            streams.len() - 1
        });
        let st = &mut streams[si];
        // a gap in the page sequence numbers is a lost page: a packet it continued is incomplete
        if st.next_seq.is_some_and(|n| n != seq) {
            warnings.damage(|| format!("pages missing before page at byte {pos}"));
            partial.remove(&serial);
        }
        st.next_seq = Some(seq.wrapping_add(1));
        let page_index = st.pages;
        st.pages += 1;
        if flags & 0x04 != 0 {
            st.saw_eos = true;
        }
        // a page that does not continue a pending packet (a page was lost): drop the packet
        let continued = flags & 0x01 != 0;
        if !continued && partial.remove(&serial).is_some() {
            warnings.damage(|| format!("incomplete packet before page at byte {pos}"));
        }
        let mut completed: Vec<Packet> = Vec::new();
        let mut off = 0u64;
        let mut seg_start = 0u64;
        // a continued page without a pending packet (a page was lost): drop the fragment
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
                    let Some(size) = p.size.checked_add(n as u32) else {
                        warnings.push(format!("packet over 4 GiB dropped at byte {pos}"));
                        partial.remove(&serial);
                        skip_fragment = l == 255;
                        continue;
                    };
                    p.parts.push((body_off + a, n as u32));
                    p.size = size;
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
        let Some(left) = packet_budget.checked_sub(completed.len()) else {
            warnings.push(format!("stopped at byte {pos}: more packets than a file of this size can hold"));
            break;
        };
        packet_budget = left;
        if granule != -1 {
            st.last_granule = Some(granule);
            if let Some(last) = completed.last_mut() {
                last.granule = Some(granule);
            }
        }
        for pk in completed {
            st.add_packet(src, pk, &mut warnings)?;
        }
        pos += page_len;
    }
    if !found_any {
        return Err(Error::NotOgg);
    }
    if !partial.is_empty() {
        warnings.push("file ends inside a packet".into());
    }
    Ok(OggFile { streams, warnings: warnings.finish() })
}

fn past_budget(pos: u64) -> String {
    format!("damage at byte {pos}; the rest of the file is past the recovery budget")
}

/// A FLAC metadata block: a block type other than the invalid 127 (which an audio frame's 0xFF sync
/// byte would read as; types 7–126 are reserved but allowed) and as many bytes as its 4-byte
/// header says.
fn is_flac_metadata_block(data: &[u8]) -> bool {
    match data {
        [kind, a, b, c, body @ ..] => kind & 0x7F != 0x7F && body.len() == (usize::from(*a) << 16 | usize::from(*b) << 8 | usize::from(*c)),
        _ => false,
    }
}

/// Scan for the next capture pattern from `from`, charging the bytes read to `recovery`. Reads start small and double up to `SCAN_CHUNK`, so a nearby pattern costs a
/// small read.
fn find_capture(src: &(impl ByteSource + ?Sized), from: u64, len: u64, recovery: &mut Recovery) -> Result<Capture> {
    let mut at = from;
    let mut chunk = 256;
    let mut buf = Vec::new();
    while at + 4 <= len {
        // clamped before the cast, so a span of 2^32 bytes cannot truncate to 0 on 32-bit targets
        let positions = (len - at - 3).min(chunk as u64) as usize;
        let n = positions + 3;
        buf.resize(n, 0);
        src.read_at(at, &mut buf[..n])?;
        if !recovery.charge(at, at + n as u64) {
            return Ok(Capture::BudgetSpent);
        }
        if let Some(i) = buf[..n].windows(4).position(|w| w == b"OggS") {
            return Ok(Capture::At(at + i as u64));
        }
        at += positions as u64;
        chunk = (chunk * 2).min(SCAN_CHUNK);
    }
    Ok(Capture::End)
}

/// The first `max` bytes of a packet's parts.
fn read_parts(src: &(impl ByteSource + ?Sized), parts: &[(u64, u32)], max: usize) -> Result<Vec<u8>> {
    // Ranges come from a scan of some source; check them against this one before sizing a buffer.
    let len = src.len();
    if parts.iter().any(|&(o, n)| o.checked_add(u64::from(n)).is_none_or(|end| end > len)) {
        return Err(Error::Invalid("packet range lies outside the source".into()));
    }
    let mut v = Vec::with_capacity(parts.iter().map(|p| p.1 as usize).sum::<usize>().min(max));
    for &(o, n) in parts {
        let a = v.len();
        if a == max {
            break;
        }
        v.resize(a + (n as usize).min(max - a), 0);
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
        read_parts(src, &p.parts, usize::MAX)
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

/// Largest granule position [`OpusTiming::of`] trusts: 2^62 samples at 48 kHz is about 3 million years.
pub const MAX_OPUS_GRANULE: i64 = 1 << 62;

impl OpusTiming {
    /// Timing of the Opus stream `s` (`pre_skip` from its `OpusHead`).
    ///
    /// A granule position outside `0..=MAX_OPUS_GRANULE` (a negative value other than -1, or an
    /// absurdly large one) is treated like a missing granule: the page's packets continue from the
    /// previous position instead of jumping to a nonsense one.
    pub fn of(stream: &Stream, pre_skip: u32) -> OpusTiming {
        // Granule positions come straight from the file, so the arithmetic saturates: a hostile
        // value gives wrong positions, never an overflow.
        let durations: Vec<u32> = stream.packets.iter().map(|p| opus_packet_samples(p.prefix, p.size).unwrap_or(0)).collect();
        let n = durations.len();
        let mut ends: Vec<Option<i64>> = vec![None; n];
        let mut i = 0;
        let mut cursor: Option<i64> = None;
        let mut trimmed_end: Option<i64> = None;
        let granule_of = |k: usize| stream.packets.get(k).and_then(|p| p.granule).filter(|g| (0..=MAX_OPUS_GRANULE).contains(g));
        while i < n {
            // packets completing on the same page: i..j, the last carries the page granule
            let mut j = i;
            while j < n && granule_of(j).is_none() {
                j += 1;
            }
            if j == n {
                // trailing packets without a granule (truncated file): continue from the cursor
                let mut c = cursor.unwrap_or(0);
                for k in i..n {
                    c = c.saturating_add(i64::from(durations[k]));
                    ends[k] = Some(c);
                }
                // the total counts these packets too
                cursor = Some(c);
                break;
            }
            let Some(g) = granule_of(j) else { break };
            let sum = durations[i..=j].iter().fold(0i64, |a, &d| a.saturating_add(i64::from(d)));
            let eos = stream.packets[j].eos;
            let end = match cursor.map(|c| c.saturating_add(sum)) {
                // end trimming on the last page (RFC 7845 §4.5): keep the forward positions
                Some(f) if eos && g < f => {
                    trimmed_end = Some(g);
                    f
                }
                Some(f) if f == g => f,
                // the only page with audio is also the last: its packets start at 0 and the
                // shortfall is trimmed at the end (RFC 7845 §4.5)
                None if eos && g < sum => {
                    trimmed_end = Some(g);
                    sum
                }
                // first page, or a discontinuity: trust the granule
                _ => g,
            };
            let mut e = end;
            for k in (i..=j).rev() {
                ends[k] = Some(e);
                e = e.saturating_sub(i64::from(durations[k]));
            }
            cursor = Some(end);
            i = j + 1;
        }
        let raw_starts: Vec<i64> = ends.iter().zip(&durations).map(|(e, &d)| e.unwrap_or(0).saturating_sub(i64::from(d))).collect();
        let base = raw_starts.first().copied().unwrap_or(0).max(0);
        let shift = base.saturating_add(i64::from(pre_skip));
        let starts: Vec<i64> = raw_starts.iter().map(|s| s.saturating_sub(shift)).collect();
        let end_all = trimmed_end.or(cursor).unwrap_or(0);
        let total = end_all.saturating_sub(shift).max(0);
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

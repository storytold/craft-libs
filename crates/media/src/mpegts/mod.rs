//! Clean-room MPEG-2 Systems demuxer (ITU-T H.222.0 | ISO/IEC 13818-1).
//!
//! - **Transport Stream**: 188-byte packets, 192-byte BDAV / AVCHD packets (`.m2ts`, `.mts`:
//!   a 4-byte arrival timestamp before each packet) and 204-byte packets (Reed-Solomon parity).
//!   PAT / PMT (sections reassembled across packets, CRC checked), PES reassembly, PCR, PTS/DTS
//!   (33-bit wrap-around unwrapped), discontinuity indicators.
//! - **Program Stream** (`.mpg`, `.vob`, `.mod`, `.tod`): MPEG-2 and ISO/IEC 11172-1 (MPEG-1)
//!   packs, system headers, program stream maps, DVD private stream 1 sub-streams (AC-3, DTS,
//!   LPCM).
//! - **Elementary streams** are split into access units on open — coded frames for MPEG-1/2
//!   video (picture start codes, field pairs), H.264 and HEVC (access unit delimiters / first
//!   slice of a picture), audio frames for MPEG audio, ADTS / LATM AAC, AC-3 and E-AC-3, PES
//!   packets for LPCM — each with its PTS/DTS (a PES timestamp belongs to the first access unit
//!   that starts in the packet), random-access flag and, for MPEG video, picture type, temporal
//!   reference and field structure. That index is the seeking table.
//! - Units are read back on demand by re-reading the file from the unit's first packet, so the
//!   index costs a few dozen bytes per access unit.
//!
//! The module does no decoding. No dependencies beyond `std`, no `unsafe`, builds for
//! `wasm32-unknown-unknown`.
//!
//! From FilmCraft's `filmcraft-mpegts` (storytold/filmcraft@4bec22b). Public items keep their names.
//! `ByteSource` is the crate's one source trait.

#![warn(missing_docs)]

mod es;
pub use es::{FrameInfo, frame_bytes, frame_info, sei_has_recovery_point};
mod pes;
mod ps;
mod ts;

use std::fmt;
use std::io;

pub use crate::source::{ByteSource, ReadSeekSource};

/// Errors from reading a transport or program stream.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Neither a transport stream nor a program stream.
    NotMpeg,
    /// Reading the source failed.
    Io(io::Error),
    /// Structurally invalid data.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotMpeg => write!(f, "not an MPEG transport or program stream"),
            Error::Io(e) => write!(f, "I/O: {e}"),
            Error::Invalid(s) => write!(f, "invalid stream: {s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// Read as much as is available at `offset` (up to `buf.len()`); returns the count.
pub(crate) fn read_some<S: ByteSource + ?Sized>(src: &S, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    let len = src.len();
    if offset >= len {
        return Ok(0);
    }
    let n = ((len - offset) as usize).min(buf.len());
    src.read_at(offset, &mut buf[..n])?;
    Ok(n)
}

/// The container layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Transport stream with `packet_size`-byte packets whose 0x47 sync byte is at `sync_offset`
    /// (4 for 192-byte BDAV packets).
    Ts {
        /// 188, 192 or 204.
        packet_size: usize,
        /// Offset of the sync byte in a packet.
        sync_offset: usize,
    },
    /// Program stream (`mpeg1`: ISO/IEC 11172-1 packs).
    Ps {
        /// ISO/IEC 11172-1 packs.
        mpeg1: bool,
    },
}

impl Format {
    /// A display name: `"MPEG-2 TS"`, `"MPEG-2 PS"`…
    pub fn name(&self) -> &'static str {
        match self {
            Format::Ts { packet_size: 192, .. } => "MPEG-2 TS (BDAV/AVCHD)",
            Format::Ts { .. } => "MPEG-2 TS",
            Format::Ps { mpeg1: true } => "MPEG-1 System",
            Format::Ps { mpeg1: false } => "MPEG-2 PS",
        }
    }
}

/// Recognise a transport stream (five consecutive sync bytes at one of the packet sizes, within
/// the first packets) or a program stream (a pack header).
pub fn sniff(head: &[u8]) -> Option<Format> {
    for (size, off) in [(188usize, 0usize), (192, 4), (204, 0)] {
        for start in 0..size.min(head.len()) {
            let ok = (0..5).all(|k| head.get(start + off + k * size) == Some(&0x47));
            if ok {
                return Some(Format::Ts { packet_size: size, sync_offset: off });
            }
        }
    }
    // a pack header, possibly after a few zero bytes
    let z = head.iter().take(64).take_while(|&&b| b == 0).count();
    let p = z.saturating_sub(2);
    if head.len() >= p + 5 && head[p..p + 4] == [0, 0, 1, 0xBA] {
        return Some(Format::Ps { mpeg1: head[p + 4] >> 6 != 1 });
    }
    None
}

/// What kind of media a stream carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Pictures.
    Video,
    /// Sound.
    Audio,
    /// Subtitles (PGS, DVB, teletext).
    Subtitle,
    /// Anything else.
    Other,
}

/// What an elementary stream carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Codec {
    /// MPEG-1 video (ISO/IEC 11172-2).
    Mpeg1Video,
    /// MPEG-2 video (H.262).
    Mpeg2Video,
    /// H.264 / AVC, Annex B.
    H264,
    /// H.265 / HEVC, Annex B.
    Hevc,
    /// MPEG-1/2 audio (layers I-III; the layer is in each frame header).
    MpegAudio,
    /// AAC in ADTS frames.
    AacAdts,
    /// AAC in LATM/LOAS frames.
    AacLatm,
    /// AC-3.
    Ac3,
    /// E-AC-3.
    Eac3,
    /// DTS.
    Dts,
    /// Dolby TrueHD.
    TrueHd,
    /// Blu-ray / AVCHD LPCM (a 4-byte header per PES packet, big-endian samples).
    LpcmBluray,
    /// DVD LPCM (program stream private stream 1; format in [`Stream::lpcm`]).
    LpcmDvd,
    /// Subtitles; the text names the format.
    Subtitle(&'static str),
    /// Anything else: the stream type.
    Unknown(u8),
}

impl Codec {
    /// A display name: `"H.264"`, `"AAC (ADTS)"`, `"stream type 0x42"`.
    pub fn name(&self) -> String {
        match self {
            Codec::Mpeg1Video => "MPEG-1 Video".into(),
            Codec::Mpeg2Video => "MPEG-2 Video".into(),
            Codec::H264 => "H.264".into(),
            Codec::Hevc => "HEVC".into(),
            Codec::MpegAudio => "MPEG Audio".into(),
            Codec::AacAdts => "AAC (ADTS)".into(),
            Codec::AacLatm => "AAC (LATM)".into(),
            Codec::Ac3 => "AC-3".into(),
            Codec::Eac3 => "E-AC-3".into(),
            Codec::Dts => "DTS".into(),
            Codec::TrueHd => "Dolby TrueHD".into(),
            Codec::LpcmBluray => "LPCM (Blu-ray)".into(),
            Codec::LpcmDvd => "LPCM (DVD)".into(),
            Codec::Subtitle(s) => (*s).into(),
            Codec::Unknown(t) => format!("stream type 0x{t:02x}"),
        }
    }
    /// The crate-wide identifier. MPEG-1 video reports [`CodecId::Mpeg2Video`] (an MPEG-2
    /// decoder decodes it), MPEG audio reports [`CodecId::Mp2`] whatever its layer (the layer is
    /// in each frame header: [`frame_info`]'s `variant`), and DTS, TrueHD and subtitles have no
    /// identifier.
    ///
    /// [`CodecId::Mpeg2Video`]: crate::CodecId::Mpeg2Video
    /// [`CodecId::Mp2`]: crate::CodecId::Mp2
    pub fn codec_id(&self) -> crate::CodecId {
        use crate::CodecId as Id;
        match self {
            Codec::Mpeg1Video | Codec::Mpeg2Video => Id::Mpeg2Video,
            Codec::H264 => Id::H264,
            Codec::Hevc => Id::Hevc,
            Codec::MpegAudio => Id::Mp2,
            Codec::AacAdts | Codec::AacLatm => Id::Aac,
            Codec::Ac3 => Id::Ac3,
            Codec::Eac3 => Id::Eac3,
            Codec::LpcmBluray | Codec::LpcmDvd => Id::Pcm,
            Codec::Dts | Codec::TrueHd | Codec::Subtitle(_) | Codec::Unknown(_) => Id::Unknown,
        }
    }

    /// The kind of media.
    pub fn kind(&self) -> Kind {
        match self {
            Codec::Mpeg1Video | Codec::Mpeg2Video | Codec::H264 | Codec::Hevc => Kind::Video,
            Codec::MpegAudio
            | Codec::AacAdts
            | Codec::AacLatm
            | Codec::Ac3
            | Codec::Eac3
            | Codec::Dts
            | Codec::TrueHd
            | Codec::LpcmBluray
            | Codec::LpcmDvd => Kind::Audio,
            Codec::Subtitle(_) => Kind::Subtitle,
            Codec::Unknown(_) => Kind::Other,
        }
    }
}

/// MPEG-1/2 video picture properties of an access unit (its first picture).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PictureInfo {
    /// 1 I, 2 P, 3 B, 4 D.
    pub coding_type: u8,
    /// `temporal_reference`: display order within the GOP.
    pub temporal_reference: u16,
    /// 1 top field, 2 bottom field, 3 frame (two field pictures form one unit).
    pub structure: u8,
    /// `top_field_first` of the picture coding extension.
    pub top_field_first: bool,
    /// `repeat_first_field` (pulldown).
    pub repeat_first_field: bool,
    /// `progressive_frame`.
    pub progressive_frame: bool,
    /// A sequence header precedes the picture in this unit.
    pub sequence_header: bool,
    /// A GOP header precedes the picture (and its flags).
    pub gop: bool,
    /// `closed_gop` of that GOP header.
    pub closed_gop: bool,
    /// `broken_link` of that GOP header.
    pub broken_link: bool,
}

/// One access unit (a coded frame, an audio frame, or an LPCM PES packet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    /// Presentation / decoding time stamps, 90 kHz, 33-bit wrap-around unwrapped.
    pub pts: Option<i64>,
    /// See `pts`.
    pub dts: Option<i64>,
    /// Decoding can start here (I / IDR / IRAP pictures; every audio frame).
    pub key: bool,
    /// No other unit depends on it (B pictures of MPEG-1/2 video, non-reference H.264/HEVC).
    pub disposable: bool,
    /// Bytes of elementary stream.
    pub size: u32,
    /// TS: byte offset of the packet holding the first byte; PS: byte offset of its PES packet.
    pub(crate) pos: u64,
    /// Offset of the first byte within that packet's elementary-stream payload.
    pub(crate) offset: u32,
    /// MPEG-1/2 video: the picture's properties.
    pub picture: Option<PictureInfo>,
}

/// DVD LPCM format (from the private stream 1 header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LpcmFormat {
    /// Samples per second.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
    /// Bits per sample (16, 20 or 24).
    pub bits: u32,
}

/// Where a stream's PES packets come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamId {
    /// Transport stream PID.
    Pid(u16),
    /// Program stream stream_id, and the private stream 1 sub-stream id.
    Ps {
        /// PES stream_id.
        stream_id: u8,
        /// Private stream 1 sub-stream id.
        sub_id: Option<u8>,
    },
}

/// One elementary stream and its access-unit table.
#[derive(Clone, Debug)]
pub struct Stream {
    /// Where its PES packets come from.
    pub id: StreamId,
    /// PMT stream_type (TS), or the equivalent for program streams.
    pub stream_type: u8,
    /// What it carries.
    pub codec: Codec,
    /// ISO 639 language code from the PMT.
    pub language: Option<String>,
    /// The access units, in stream order.
    pub units: Vec<Unit>,
    /// DVD LPCM: the sample format.
    pub lpcm: Option<LpcmFormat>,
    /// PES packets seen.
    pub pes_packets: u64,
    /// Bytes of payload ignored because the stream could not be split (e.g. before the first
    /// frame header).
    pub skipped_bytes: u64,
}

impl Stream {
    /// The kind of media.
    pub fn kind(&self) -> Kind {
        self.codec.kind()
    }
    /// What a decoder needs for this stream, in the container-neutral form the codec crates take:
    /// the codec ([`Codec::codec_id`]) and, for DVD LPCM, the sample format (big-endian). These
    /// streams carry their configuration in band, so `extradata` is empty: H.264 and HEVC are
    /// Annex B with their parameter sets, AAC is ADTS or LATM frames, and a Blu-ray LPCM packet
    /// starts with its own 4-byte format header.
    pub fn codec_params(&self) -> crate::CodecParams {
        let mut p = crate::CodecParams::new(self.codec.codec_id());
        if let Some(f) = self.lpcm {
            let n = |v: u32| u16::try_from(v).unwrap_or(0);
            p = p.with_sample_rate(f.sample_rate).with_channels(n(f.channels)).with_pcm(crate::PcmFormat::int(n(f.bits), true, true));
        }
        p
    }
    /// First and last presentation timestamps (90 kHz) of the stream's units.
    pub fn pts_range(&self) -> Option<(i64, i64)> {
        let mut it = self.units.iter().filter_map(|u| u.pts);
        let first = it.next()?;
        Some(it.fold((first, first), |(a, b), p| (a.min(p), b.max(p))))
    }
}

/// A demuxed file: its streams with their access-unit tables.
#[derive(Clone, Debug)]
pub struct File {
    /// The container layout.
    pub format: Format,
    /// The elementary streams (TS: in PMT order).
    pub streams: Vec<Stream>,
    /// Program number of the program the streams belong to (TS).
    pub program: Option<u16>,
    /// Smallest and largest PCR (27 MHz, unwrapped) or SCR. For a transport stream this is the
    /// clock of the PMT's PCR_PID; without a PMT, of the first PID seen carrying a PCR. `None` if
    /// that PID never carries one.
    pub pcr_range: Option<(i64, i64)>,
    /// Problems found while scanning (lost sync, CRC errors, continuity errors…).
    pub warnings: Vec<String>,
}

/// Largest unit `read_unit` returns. The biggest real units are intra pictures (a few MB at
/// 4K/8K broadcast rates); a stream without PES timestamps can make one unit of a whole PID.
pub const MAX_UNIT_BYTES: u32 = 128 << 20;

/// Bytes reserved up front by `read_unit`; the buffer grows beyond this only as data is read.
const UNIT_PREALLOC: usize = 1 << 20;

impl File {
    /// The first stream of `kind` whose codec satisfies `f`.
    pub fn find(&self, kind: Kind, f: impl Fn(&Codec) -> bool) -> Option<usize> {
        self.streams.iter().position(|s| s.kind() == kind && !s.units.is_empty() && f(&s.codec))
    }

    /// Read access unit `i` of stream `s`.
    pub fn read_unit<S: ByteSource + ?Sized>(&self, src: &S, s: usize, i: usize) -> Result<Vec<u8>> {
        let st = self.streams.get(s).ok_or_else(|| Error::Invalid("no such stream".into()))?;
        let u = st.units.get(i).ok_or_else(|| Error::Invalid("no such unit".into()))?;
        if u.size > MAX_UNIT_BYTES {
            return Err(Error::Invalid(format!("unit {i} is {} bytes (limit {MAX_UNIT_BYTES})", u.size)));
        }
        // grow as bytes arrive: a damaged table must not reserve more than the file can supply
        let mut out = Vec::with_capacity((u.size as usize).min(UNIT_PREALLOC));
        match self.format {
            Format::Ts { packet_size, sync_offset } => ts::read(src, packet_size, sync_offset, st, u, &mut out)?,
            Format::Ps { .. } => ps::read(src, st, u, &mut out)?,
        }
        if out.len() < u.size as usize {
            return Err(Error::Invalid(format!("unit {i} truncated ({} of {} bytes)", out.len(), u.size)));
        }
        Ok(out)
    }
}

/// Scan a transport or program stream and index its access units.
pub fn open<S: ByteSource + ?Sized>(src: &S) -> Result<File> {
    let mut head = vec![0u8; 64 * 1024];
    let n = read_some(src, 0, &mut head)?;
    head.truncate(n);
    match sniff(&head).ok_or(Error::NotMpeg)? {
        Format::Ts { packet_size, sync_offset } => ts::scan(src, packet_size, sync_offset),
        Format::Ps { mpeg1 } => ps::scan(src, mpeg1),
    }
}

/// Unwraps the 33-bit PTS/DTS of ONE stream into an i64 timeline. Values are absolute, not
/// relative to the first one: the first timestamp comes back unchanged, every later wrap adds
/// 2^33, and a timestamp from just before a wrap that arrives after it (B pictures, slightly
/// out-of-order audio) is mapped below the wrap (negative if the stream started after it).
/// Keep one `Unwrap` per clock: the streams of a program share one.
#[derive(Default, Clone, Copy)]
pub(crate) struct Unwrap {
    last: Option<i64>,
    base: i64,
}

impl Unwrap {
    const WRAP: i64 = 1 << 33;
    /// 1 minute at 90 kHz: how close to the wrap point a pre-wrap timestamp must be.
    const NEAR_WRAP: i64 = 90_000 * 60;
    /// Start a stream-local unwrapper at a value already aligned to its program's clock epoch.
    pub fn aligned(raw: i64, value: i64) -> Self {
        Self { last: Some(value), base: value.saturating_sub(raw) }
    }
    /// Advance a program-level epoch anchor to a stream that has progressed farther.
    pub fn observe(&mut self, stream: Self) {
        if stream.last > self.last {
            *self = stream;
        }
    }
    pub fn unwrap(&mut self, raw: i64) -> i64 {
        let mut v = raw.saturating_add(self.base);
        if let Some(l) = self.last {
            if v < l.saturating_sub(Self::WRAP / 2) {
                self.base = self.base.saturating_add(Self::WRAP);
                v = v.saturating_add(Self::WRAP);
            } else if v > l.saturating_add(Self::WRAP / 2) && raw >= Self::WRAP - Self::NEAR_WRAP && l < self.base.saturating_add(Self::NEAR_WRAP) {
                // a timestamp from before the wrap: it does not move the timeline. Before the
                // first wrap and after later wraps this applies only right at the boundary; any
                // other forward jump is a new time base (spliced capture) and is followed.
                return v.saturating_sub(Self::WRAP);
            }
        }
        self.last = Some(v);
        v
    }
}

/// Unwraps the 27 MHz clock reference (PCR/SCR: 33-bit base × 300 + 9-bit extension) of one
/// clock into an i64 timeline and keeps the smallest and largest value seen.
#[derive(Default, Clone, Copy)]
pub(crate) struct ClockRange {
    last: Option<i64>,
    base: i64,
    range: Option<(i64, i64)>,
}

impl ClockRange {
    /// 33-bit base × 300 wraps at 2^33·300.
    const WRAP: i64 = (1 << 33) * 300;
    /// The (min, max) of the unwrapped values so far.
    pub fn range(&self) -> Option<(i64, i64)> {
        self.range
    }
    pub fn push(&mut self, raw: i64) {
        let mut v = raw.saturating_add(self.base);
        if let Some(l) = self.last
            && v < l.saturating_sub(Self::WRAP / 2)
        {
            self.base = self.base.saturating_add(Self::WRAP);
            v = v.saturating_add(Self::WRAP);
        }
        self.last = Some(v);
        self.range = Some(self.range.map_or((v, v), |(a, b)| (a.min(v), b.max(v))));
    }
}

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod work_tests;

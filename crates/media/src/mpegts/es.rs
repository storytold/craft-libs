//! Splitting elementary streams into access units while the file is scanned.
//!
//! The scanners feed each stream's payload bytes chunk by chunk (a TS packet's payload, a PS
//! packet's payload). A chunk is identified by the position the reader restarts from (`pos`) and
//! the elementary-stream offset of its first byte. Splitting needs only a few bytes beyond a
//! boundary, so only the most recent chunks are remembered.

use std::collections::VecDeque;

use super::{Codec, OpusTrim, PictureInfo, Unit};

/// How a stream is cut into units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// MPEG-1/2 video: one coded frame (a frame picture or two field pictures) per unit.
    MpegVideo,
    /// H.264 / HEVC: one access unit per unit (the first of a PES packet keeps its timestamps);
    /// NAL types give the flags.
    Nal { hevc: bool },
    /// Framed audio with self-describing frame headers.
    Audio(AudioSync),
    /// Opus: one access unit per unit, cut by its control header's payload size and starting
    /// after the header; an access unit without a control header runs to the end of its PES
    /// packet (Opus transport draft §6).
    Opus,
    /// MPEG-4 Part 2 video: a PES packet with a timestamp starts a unit; the first VOP's coding
    /// type gives the flags.
    Mpeg4,
    /// AV1: one PES packet per unit (Carriage of AV1 in MPEG-2 TS §3.4), a key unit where the
    /// packet that starts it sets the random access indicator.
    Av1,
    /// One PES packet per unit.
    Pes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioSync {
    Mpa,
    Adts,
    Loas,
    Ac3,
}

impl Mode {
    fn of(codec: &Codec) -> Mode {
        match codec {
            Codec::Mpeg1Video | Codec::Mpeg2Video => Mode::MpegVideo,
            Codec::H264 => Mode::Nal { hevc: false },
            Codec::Hevc => Mode::Nal { hevc: true },
            Codec::MpegAudio => Mode::Audio(AudioSync::Mpa),
            Codec::AacAdts => Mode::Audio(AudioSync::Adts),
            Codec::AacLatm => Mode::Audio(AudioSync::Loas),
            Codec::Ac3 | Codec::Eac3 => Mode::Audio(AudioSync::Ac3),
            Codec::Opus => Mode::Opus,
            Codec::Mpeg4Part2 => Mode::Mpeg4,
            Codec::Av1 => Mode::Av1,
            _ => Mode::Pes,
        }
    }
}

#[derive(Clone, Copy)]
struct PesMark {
    es_start: u64,
    pts: Option<i64>,
    dts: Option<i64>,
    used: bool,
}

/// A unit whose end is not known yet.
#[derive(Clone, Copy)]
struct Open {
    start: u64,
    pos: u64,
    offset: u32,
    pts: Option<i64>,
    dts: Option<i64>,
    key: bool,
    disposable: bool,
    /// A coded slice has been seen (H.264 / HEVC).
    vcl: bool,
    picture: Option<PictureInfo>,
}

/// MPEG video picture waiting for its coding extension (to tell fields from frames).
#[derive(Clone, Copy)]
struct PendingPicture {
    /// Start of the unit if the picture begins one (its first sequence / GOP header).
    unit_start: u64,
    /// The picture start code (timestamps belong to the unit whose picture starts in the PES).
    at: u64,
    info: PictureInfo,
}

pub(crate) struct Splitter {
    mode: Mode,
    /// Elementary-stream bytes fed so far.
    es_pos: u64,
    /// Recent chunks: (reader position, ES offset of the first byte).
    chunks: VecDeque<(u64, u64)>,
    pes: VecDeque<PesMark>,
    /// Unconsumed bytes (start codes / frame headers spanning chunks), starting at `buf_start`.
    buf: Vec<u8>,
    buf_start: u64,
    open: Option<Open>,
    pub units: Vec<Unit>,
    pub skipped: u64,
    /// Units beyond [`MAX_UNITS`] were dropped.
    pub truncated: bool,
    max_units: usize,
    // MPEG video state
    header_start: Option<u64>,
    seq_in_unit: bool,
    gop: Option<(bool, bool)>,
    pending: Option<PendingPicture>,
    open_field: Option<u8>,
    // audio state; for Opus, where the next access unit's control header starts (None: the
    // rest of the PES packet belongs to the open unit) or, once one is read, where its payload
    // ends
    next_frame: Option<u64>,
    /// Opus: how far the payload size of a control header cut off by the end of a chunk was read.
    opus_scan: OpusSizeScan,
    /// Opus: the open unit's start and end trims.
    open_trim: (u16, u16),
    /// Opus: the trims of the units so far.
    pub trims: Vec<OpusTrim>,
}

/// AC-3 frame sizes in 16-bit words for 48 / 44.1 / 32 kHz (A/52 Table 5.18), by
/// frmsizecod / 2; 44.1 kHz adds a word for odd frmsizecod.
const AC3_KBPS: [u32; 19] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640];

pub(crate) fn ac3_frame_bytes(fscod: u8, frmsizecod: u8) -> Option<usize> {
    let kbps = *AC3_KBPS.get((frmsizecod / 2) as usize)?;
    let words = match fscod {
        0 => kbps * 2,
        1 => kbps * 96_000 / 44_100 + (frmsizecod & 1) as u32,
        2 => kbps * 3,
        _ => return None,
    };
    Some(words as usize * 2)
}

const MPA_KBPS: [[u16; 15]; 5] = [
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448], // MPEG-1 layer I
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],    // MPEG-1 layer II
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],     // MPEG-1 layer III
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],    // MPEG-2/2.5 layer I
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],         // MPEG-2/2.5 layers II, III
];

/// MPEG audio frame header (ISO/IEC 11172-3 §2.4.2.3, 13818-3 LSF): (frame bytes, sample rate,
/// samples per frame).
pub(crate) fn mpa_header(b: &[u8]) -> Option<(usize, u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = (b[1] >> 3) & 3; // 0: 2.5, 2: MPEG-2, 3: MPEG-1
    let layer = (b[1] >> 1) & 3; // 3: I, 2: II, 1: III
    let br = (b[2] >> 4) as usize;
    let sr = (b[2] >> 2) & 3;
    let pad = ((b[2] >> 1) & 1) as usize;
    if version == 1 || layer == 0 || br == 0 || br == 15 || sr == 3 {
        return None;
    }
    let rate = [44_100u32, 48_000, 32_000][sr as usize]
        >> match version {
            3 => 0,
            2 => 1,
            _ => 2,
        };
    let table = match (version == 3, layer) {
        (true, 3) => 0,
        (true, 2) => 1,
        (true, _) => 2,
        (false, 3) => 3,
        (false, _) => 4,
    };
    let kbps = MPA_KBPS[table][br] as usize;
    let (bytes, samples) = match layer {
        3 => ((12 * kbps * 1000 / rate as usize + pad) * 4, 384),
        2 => (144 * kbps * 1000 / rate as usize + pad, 1152),
        _ if version == 3 => (144 * kbps * 1000 / rate as usize + pad, 1152),
        _ => (72 * kbps * 1000 / rate as usize + pad, 576),
    };
    (bytes >= 4).then_some((bytes, rate, samples))
}

/// What an audio frame header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    /// Samples per second.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u32,
    /// Samples per channel in the frame.
    pub samples: u32,
    /// MPEG audio layer (1-3); AC-3 bsid; ADTS object type (profile + 1).
    pub variant: u8,
}

const ADTS_RATES: [u32; 13] = [96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000, 7_350];

/// Bytes of the audio frame whose header starts `b` (MPEG audio, ADTS, LATM, AC-3, E-AC-3), or
/// `None` without a valid header there. For containers that store frames in chunks that need not
/// follow them (AVI): the frames are found again with it.
pub fn frame_bytes(codec: &Codec, b: &[u8]) -> Option<usize> {
    let sync = match codec {
        Codec::MpegAudio => AudioSync::Mpa,
        Codec::AacAdts => AudioSync::Adts,
        Codec::AacLatm => AudioSync::Loas,
        Codec::Ac3 | Codec::Eac3 => AudioSync::Ac3,
        _ => return None,
    };
    frame_len(sync, b)
}

/// Sample rate, channels and samples of the audio frame whose header starts `b` (the codecs of
/// [`frame_bytes`] except LATM), or `None` without a valid header there.
pub fn frame_info(codec: &Codec, b: &[u8]) -> Option<FrameInfo> {
    match codec {
        Codec::MpegAudio => {
            let (_, sample_rate, samples) = mpa_header(b)?;
            let layer = 4 - ((b[1] >> 1) & 3);
            Some(FrameInfo { sample_rate, channels: if b[3] >> 6 == 3 { 1 } else { 2 }, samples, variant: layer })
        }
        Codec::AacAdts => {
            frame_len(AudioSync::Adts, b)?;
            let sf = ((b[2] >> 2) & 15) as usize;
            let ch = (((b[2] & 1) << 2) | (b[3] >> 6)) as u32;
            Some(FrameInfo {
                sample_rate: *ADTS_RATES.get(sf)?,
                channels: if ch == 0 {
                    2
                } else if ch == 7 {
                    8
                } else {
                    ch
                },
                samples: 1024 * ((b[6] & 3) as u32 + 1),
                variant: (b[2] >> 6) + 1,
            })
        }
        Codec::Ac3 | Codec::Eac3 => {
            frame_len(AudioSync::Ac3, b)?;
            let bsid = b[5] >> 3;
            if bsid > 10 {
                // E-AC-3: fscod(2) numblkscod(2) acmod(3) lfeon(1)
                let fscod = b[4] >> 6;
                let (rate, blocks) = if fscod == 3 {
                    // fscod2 3 is reserved (no sample rate)
                    (*[24_000, 22_050, 16_000].get(((b[4] >> 4) & 3) as usize)?, 6)
                } else {
                    ([48_000, 44_100, 32_000][fscod as usize], [1, 2, 3, 6][((b[4] >> 4) & 3) as usize])
                };
                let acmod = (b[4] >> 1) & 7;
                return Some(FrameInfo {
                    sample_rate: rate,
                    channels: AC3_CHANNELS[acmod as usize] + (b[4] & 1) as u32,
                    samples: 256 * blocks,
                    variant: bsid,
                });
            }
            let fscod = b[4] >> 6;
            let rate = *[48_000u32, 44_100, 32_000].get(fscod as usize)?;
            // bsi: bsid(5) bsmod(3) acmod(3) [cmixlev(2)] [surmixlev(2)] [dsurmod(2)] lfeon(1)
            let b6 = *b.get(6)?;
            let acmod = b6 >> 5;
            let mut bit = 3; // bits of byte 6 consumed
            if acmod & 1 != 0 && acmod != 1 {
                bit += 2;
            }
            if acmod & 4 != 0 {
                bit += 2;
            }
            if acmod == 2 {
                bit += 2;
            }
            let word = u16::from_be_bytes([b6, *b.get(7)?]);
            let lfe = (word >> (15 - bit)) & 1;
            Some(FrameInfo { sample_rate: rate, channels: AC3_CHANNELS[acmod as usize] + lfe as u32, samples: 1536, variant: bsid })
        }
        _ => None,
    }
}

/// Whether the syncframe at the start of `b` is E-AC-3 (bsid 11-16) other than independent
/// substream 0: a dependent substream (strmtyp 1) or another program (A/52 §E2.3.1.1-2).
fn eac3_joins_unit(b: &[u8]) -> bool {
    match b.get(2..6) {
        Some(&[b2, _, _, b5]) => (11..=16).contains(&(b5 >> 3)) && (b2 >> 6 == 1 || (b2 >> 3) & 7 != 0),
        _ => false,
    }
}

/// Bytes of an SEI NAL scanned for a recovery point message; a longer NAL is judged by its start.
const SEI_WINDOW: usize = 2048;

/// Whether an H.264 SEI NAL (`nal` is the NAL header byte and the payload, as in the stream)
/// holds a recovery point message (payloadType 6, H.264 D.1.7) with recovery_frame_cnt 0.
pub fn sei_has_recovery_point(nal: &[u8]) -> bool {
    // undo emulation prevention so that payload sizes are in RBSP bytes
    let mut b = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &x in nal.iter().skip(1) {
        if zeros >= 2 && x == 3 {
            zeros = 0;
            continue;
        }
        zeros = if x == 0 { zeros + 1 } else { 0 };
        b.push(x);
    }
    // payloadType / payloadSize: bytes of 0xFF followed by the last byte
    let read = |i: &mut usize| {
        let mut v = 0usize;
        while let Some(&x) = b.get(*i) {
            *i += 1;
            v = v.saturating_add(x as usize);
            if x != 0xFF {
                break;
            }
        }
        v
    };
    let mut i = 0;
    // each message takes at least two bytes, so the loop ends at the end of the NAL
    while i + 2 <= b.len() {
        let ty = read(&mut i);
        let size = read(&mut i);
        // recovery_frame_cnt is ue(v): a leading 1 bit is the value 0, i.e. decoding from here is
        // correct at once. A count above 0 (gradual intra refresh) is not a safe seek point.
        if ty == 6 && b.get(i).is_some_and(|x| x & 0x80 != 0) {
            return true;
        }
        i = i.saturating_add(size);
    }
    false
}

/// Shortest E-AC-3 syncframe accepted. A real syncframe holds the 6 header bytes read here plus
/// bsi and audio blocks (the smallest real ones are tens of bytes), so shorter "frames"
/// (`0B 77 00 01` is 4 bytes) are noise; they would otherwise turn every 4 input bytes into a
/// 64-byte `Unit`.
const EAC3_MIN_FRAME: usize = 8;

/// Most units one stream may index. A `Unit` is 64 bytes, so this bounds a stream's table to
/// 512 MiB. It is 47 hours of 48 kHz AAC (the densest audio, 46.9 units/s), 9 hours of 240 fps
/// video: far beyond any real file, while a hostile file (a unit per 7 input bytes) cannot grow
/// the table without limit.
pub(crate) const MAX_UNITS: usize = 1 << 23;

/// Full-bandwidth channels per AC-3 audio coding mode (A/52 Table 5.8).
const AC3_CHANNELS: [u32; 8] = [2, 1, 2, 3, 3, 4, 4, 5];

/// What starts an Opus access unit.
#[derive(Debug, PartialEq, Eq)]
enum OpusHeader {
    /// More bytes are needed to tell.
    Incomplete,
    /// No control header: the access unit is the rest of the PES packet.
    Absent,
    /// A control header whose payload size is past [`super::MAX_UNIT_BYTES`].
    Invalid,
    /// A control header of `len` bytes, then `size` bytes of payload with these start and end
    /// trims.
    Found { len: usize, size: u64, trim: (u16, u16) },
}

/// How far the payload size of the control header at ES offset `at` was read: `next` is the
/// header index of the next size byte to read, and `size` is the sum of the bytes before it.
#[derive(Debug, Default)]
struct OpusSizeScan {
    at: u64,
    next: usize,
    size: u64,
    /// Size bytes read, by every header.
    #[cfg(test)]
    reads: u64,
}

/// The opus_control_header at the start of `b`, which is ES offset `at` (Opus transport draft
/// §6.2): the 11-bit prefix 0x3FF, the start trim, end trim and extension flags, the payload size
/// as bytes of 0xFF and a last byte, then the 13-bit trims and the extension. When `b` ends
/// before the header does, `scan` keeps the size read so far, and a call with more bytes of the
/// same header goes on from there, so each size byte is read once.
fn opus_control_header(b: &[u8], at: u64, scan: &mut OpusSizeScan) -> OpusHeader {
    match b {
        [] => return OpusHeader::Incomplete,
        [0x7F] => return OpusHeader::Incomplete,
        [0x7F, b1, ..] if b1 & 0xE0 == 0xE0 => {}
        _ => return OpusHeader::Absent,
    }
    let flags = b[1];
    if scan.at != at || scan.next < 2 {
        scan.at = at;
        scan.next = 2;
        scan.size = 0;
    }
    let mut i = scan.next;
    let mut size = scan.size;
    loop {
        let Some(&x) = b.get(i) else {
            scan.next = i;
            scan.size = size;
            return OpusHeader::Incomplete;
        };
        #[cfg(test)]
        {
            scan.reads += 1;
        }
        if size + u64::from(x) > u64::from(super::MAX_UNIT_BYTES) {
            return OpusHeader::Invalid;
        }
        if x != 0xFF {
            // the trims or the extension may still be cut off: a later call reads this last
            // size byte again
            scan.next = i;
            scan.size = size;
            size += u64::from(x);
            i += 1;
            break;
        }
        size += u64::from(x);
        i += 1;
    }
    let mut trim = [0u16; 2];
    for (k, bit) in [0x10u8, 0x08].into_iter().enumerate() {
        if flags & bit != 0 {
            let Some(v) = b.get(i..i + 2) else { return OpusHeader::Incomplete };
            trim[k] = u16::from_be_bytes([v[0], v[1]]) & 0x1FFF;
            i += 2;
        }
    }
    if flags & 0x04 != 0 {
        let Some(&n) = b.get(i) else { return OpusHeader::Incomplete };
        i += 1 + usize::from(n);
        if i > b.len() {
            return OpusHeader::Incomplete;
        }
    }
    OpusHeader::Found { len: i, size, trim: (trim[0], trim[1]) }
}

/// Rewrites an AV1 access unit from the start code form of Carriage of AV1 in MPEG-2 TS §3.2 to
/// the low overhead bitstream format: each OBU follows a start code 0x000001, and the 0x03 of
/// every 0x000003 in it is an emulation prevention byte. Temporal delimiters and bytes before the
/// first start code are left out, as Matroska and ISOBMFF store AV1. An OBU that carries
/// obu_size ends there; one without gets an obu_size for the bytes up to the next start code.
pub(crate) fn av1_obus(es: &mut Vec<u8>) {
    let mut out = Vec::with_capacity(es.len() + 16);
    let mut obu = Vec::new();
    let start_code = |i: usize| es.get(i..i + 3) == Some(&[0, 0, 1][..]);
    let mut i = (0..es.len()).find(|&i| start_code(i)).unwrap_or(es.len());
    while i < es.len() {
        // `i` is at a start code
        i += 3;
        obu.clear();
        let mut zeros = 0;
        while i < es.len() && !start_code(i) {
            let b = es[i];
            i += 1;
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            obu.push(b);
        }
        let Some(&h) = obu.first() else { continue };
        let header = 1 + usize::from(h & 0x04 != 0);
        if obu.len() < header || (h >> 3) & 0x0F == 2 {
            continue;
        }
        if h & 0x02 != 0 {
            // obu_size, leb128 (AV1 §4.10.5)
            let mut size = 0u64;
            let mut at = header;
            for k in 0..8 {
                let Some(&b) = obu.get(at) else { break };
                at += 1;
                size |= u64::from(b & 0x7F) << (7 * k);
                if b & 0x80 == 0 {
                    break;
                }
            }
            let end = usize::try_from(size).ok().and_then(|s| at.checked_add(s)).filter(|&e| e <= obu.len()).unwrap_or(obu.len());
            out.extend_from_slice(&obu[..end]);
        } else {
            out.push(h | 0x02);
            out.extend_from_slice(&obu[1..header]);
            let mut size = obu.len() - header;
            loop {
                let b = (size & 0x7F) as u8;
                size >>= 7;
                if size == 0 {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
            out.extend_from_slice(&obu[header..]);
        }
    }
    *es = out;
}

/// Frame length at the start of `b` for the given sync type (`None`: no valid header; `Some(0)`
/// is never returned).
fn frame_len(sync: AudioSync, b: &[u8]) -> Option<usize> {
    match sync {
        AudioSync::Mpa => mpa_header(b).map(|h| h.0),
        AudioSync::Adts => {
            if b.len() < 7 || b[0] != 0xFF || b[1] & 0xF6 != 0xF0 {
                return None;
            }
            let len = ((b[3] as usize & 3) << 11) | (b[4] as usize) << 3 | (b[5] as usize) >> 5;
            (len >= 7).then_some(len)
        }
        AudioSync::Loas => {
            if b.len() < 3 || b[0] != 0x56 || b[1] & 0xE0 != 0xE0 {
                return None;
            }
            let len = ((b[1] as usize & 0x1F) << 8) | b[2] as usize;
            (len > 0).then_some(len + 3)
        }
        AudioSync::Ac3 => {
            if b.len() < 6 || b[0] != 0x0B || b[1] != 0x77 {
                return None;
            }
            let bsid = b[5] >> 3;
            if bsid <= 10 {
                ac3_frame_bytes(b[4] >> 6, b[4] & 0x3F)
            } else if bsid <= 16 {
                // E-AC-3: frmsiz
                let len = ((((b[2] & 7) as usize) << 8) | b[3] as usize) * 2 + 2;
                (len >= EAC3_MIN_FRAME).then_some(len)
            } else {
                None
            }
        }
    }
}

impl Splitter {
    pub fn new(codec: &Codec) -> Splitter {
        Splitter {
            mode: Mode::of(codec),
            es_pos: 0,
            chunks: VecDeque::new(),
            pes: VecDeque::new(),
            buf: Vec::new(),
            buf_start: 0,
            open: None,
            units: Vec::new(),
            skipped: 0,
            truncated: false,
            max_units: MAX_UNITS,
            header_start: None,
            seq_in_unit: false,
            gop: None,
            pending: None,
            open_field: None,
            next_frame: None,
            opus_scan: OpusSizeScan::default(),
            open_trim: (0, 0),
            trims: Vec::new(),
        }
    }

    /// A PES packet starts (its payload follows in the next chunks).
    pub fn pes_start(&mut self, pts: Option<i64>, dts: Option<i64>) {
        self.pes.push_back(PesMark { es_start: self.es_pos, pts, dts, used: false });
        while self.pes.len() > 64 {
            self.pes.pop_front();
        }
        match self.mode {
            Mode::Pes | Mode::Av1 => self.begin_pes_unit(true),
            Mode::Nal { .. } | Mode::Mpeg4 => self.begin_pes_unit(pts.is_some() || dts.is_some()),
            Mode::Opus => {
                // a PES packet holds whole access units: the next one starts here
                self.close_opus(self.es_pos);
                self.next_frame = Some(self.es_pos);
            }
            _ => {}
        }
    }

    /// The packet that started the PES packet set the random access indicator: an AV1 unit
    /// that the packet starts is a key unit.
    pub fn random_access(&mut self) {
        if self.mode == Mode::Av1
            && let Some(o) = self.open.as_mut()
            && o.start == self.es_pos
        {
            o.key = true;
        }
    }

    /// Opus: end the open unit before ES byte `limit` (the end of its PES packet). A unit whose
    /// payload size runs past `limit` is dropped, and so are the bytes of a control header that
    /// the PES packet cuts off.
    fn close_opus(&mut self, limit: u64) {
        match self.next_frame {
            Some(end) if end > limit => {
                if let Some(o) = self.open.take() {
                    self.skipped += limit.saturating_sub(o.start);
                }
                self.open_trim = (0, 0);
            }
            Some(end) => {
                self.close(end);
                self.skipped += limit - end;
            }
            None => self.close(limit),
        }
        self.next_frame = None;
    }

    /// PES-delimited units: a packet with a timestamp (or any packet in plain PES mode) starts a
    /// new unit; one without continues the previous unit.
    fn begin_pes_unit(&mut self, new: bool) {
        if !new && self.open.is_some() {
            return;
        }
        let at = self.es_pos;
        self.close(at);
        // the chunk of this position arrives with the next feed: resolved there
        let mut o = Open { start: at, pos: u64::MAX, offset: 0, pts: None, dts: None, key: false, disposable: false, vcl: false, picture: None };
        if let Some((pts, dts)) = self.timestamps_at(at) {
            o.pts = pts;
            o.dts = dts;
        }
        if self.mode == Mode::Pes {
            o.key = true;
        }
        self.open = Some(o);
    }

    /// The unused timestamps of the PES packet holding `at` (consumed).
    fn timestamps_at(&mut self, at: u64) -> Option<(Option<i64>, Option<i64>)> {
        let m = self.pes.iter_mut().rev().find(|m| m.es_start <= at)?;
        if m.used || (m.pts.is_none() && m.dts.is_none()) {
            return None;
        }
        m.used = true;
        Some((m.pts, m.dts))
    }

    /// Reader position and offset of elementary-stream byte `at`.
    fn locate(&self, at: u64) -> Option<(u64, u32)> {
        let c = self.chunks.iter().rev().find(|c| c.1 <= at)?;
        Some((c.0, (at - c.1) as u32))
    }

    /// End the open unit at `end`.
    fn close(&mut self, end: u64) {
        let Some(o) = self.open.take() else { return };
        let (start_trim, end_trim) = std::mem::take(&mut self.open_trim);
        if end <= o.start || o.pos == u64::MAX {
            return;
        }
        if self.units.len() >= self.max_units {
            self.truncated = true;
            return;
        }
        if (start_trim, end_trim) != (0, 0) {
            self.trims.push(OpusTrim { unit: self.units.len(), start: start_trim, end: end_trim });
        }
        let size = (end - o.start).min(u32::MAX as u64) as u32;
        self.units.push(Unit {
            pts: o.pts,
            dts: o.dts,
            key: o.key,
            disposable: o.disposable,
            size,
            pos: o.pos,
            offset: o.offset,
            picture: o.picture,
        });
    }

    /// Payload bytes of one chunk (`pos`: where the reader restarts to find them).
    pub fn feed(&mut self, pos: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        // PES-delimited modes resolve a unit's position from `pos` right here and never look a
        // chunk up again, so remembering chunks there would only grow without bound (NAL and Opus
        // streams keep theirs: an access unit can start inside a PES; `keep_from` prunes them)
        if matches!(self.mode, Mode::MpegVideo | Mode::Audio(_) | Mode::Nal { .. } | Mode::Opus) {
            self.chunks.push_back((pos, self.es_pos));
        }
        if let Some(o) = self.open.as_mut()
            && o.pos == u64::MAX
            && o.start == self.es_pos
        {
            o.pos = pos;
            o.offset = 0;
        }
        let start = self.es_pos;
        self.es_pos += data.len() as u64;
        match self.mode {
            Mode::Pes | Mode::Av1 => {}
            Mode::Nal { hevc } => self.scan_nal(start, data, hevc),
            Mode::MpegVideo => self.scan_mpeg(start, data),
            Mode::Mpeg4 => self.scan_mpeg4(start, data),
            Mode::Audio(sync) => self.scan_audio(start, data, sync),
            Mode::Opus => self.scan_opus(start, data),
        }
        // forget chunks no pending position can refer to
        let keep_from = self.buf_start.min(self.pending.map_or(u64::MAX, |p| p.unit_start.min(p.at))).min(self.header_start.unwrap_or(u64::MAX));
        while self.chunks.len() > 2 && self.chunks[1].1 <= keep_from {
            self.chunks.pop_front();
        }
    }

    /// Append to the window and return it with its start.
    fn window(&mut self, start: u64, data: &[u8]) {
        if self.buf.is_empty() || self.buf_start + self.buf.len() as u64 != start {
            self.buf.clear();
            self.buf_start = start;
        }
        self.buf.extend_from_slice(data);
    }

    /// Keep only the last `keep` bytes of the window.
    fn trim(&mut self, keep: usize) {
        if self.buf.len() > keep {
            let drop = self.buf.len() - keep;
            self.buf.drain(..drop);
            self.buf_start += drop as u64;
        }
    }

    /// Start codes in the window from index `from`: (window index of `00 00 01`, code).
    fn next_start_code(&self, from: usize) -> Option<usize> {
        let b = &self.buf;
        let mut i = from;
        while i + 3 <= b.len() {
            match b[i + 2] {
                1 if b[i] == 0 && b[i + 1] == 0 => return Some(i),
                0 => i += 1,
                _ => i += 3,
            }
        }
        None
    }

    fn scan_nal(&mut self, start: u64, data: &[u8], hevc: bool) {
        self.window(start, data);
        let mut i = 0;
        let mut blocked = None;
        while let Some(k) = self.next_start_code(i) {
            // NAL header byte(s) and the first byte of the slice header needed
            if k + if hevc { 6 } else { 5 } > self.buf.len() {
                blocked = Some(k);
                break;
            }
            // a four-byte start code begins with the zero_byte that belongs to its NAL
            let at = self.buf_start + k as u64 - (k > 0 && self.buf.get(k - 1) == Some(&0)) as u64;
            let (Some(&h), Some(&h1)) = (self.buf.get(k + 3), self.buf.get(k + if hevc { 5 } else { 4 })) else {
                blocked = Some(k);
                break;
            };
            let (key, vcl, non_ref, au_start) = if hevc {
                let t = (h >> 1) & 0x3F;
                (
                    (16..=23).contains(&t),
                    t < 32,
                    // sub-layer non-reference pictures (TRAIL_N, TSA_N, STSA_N, RADL_N, RASL_N)
                    t < 16 && t.is_multiple_of(2),
                    // first_slice_segment_in_pic_flag, or a NAL that precedes the first slice
                    // (VPS, SPS, PPS, AUD, prefix SEI, 41..=44, 48..=55; H.265 7.4.2.4.4)
                    if t < 32 { h1 & 0x80 != 0 } else { matches!(t, 32..=35 | 39 | 41..=44 | 48..=55) },
                )
            } else {
                let t = h & 0x1F;
                (
                    t == 5,
                    (1..=5).contains(&t),
                    h & 0x60 == 0,
                    // first_mb_in_slice == 0 (ue(v) "1"), or a NAL that precedes the first slice
                    // (SEI, SPS, PPS, AUD, 14..=18; H.264 7.4.1.2.3)
                    match t {
                        1 | 2 | 5 => h1 & 0x80 != 0,
                        6..=9 | 14..=18 => true,
                        _ => false,
                    },
                )
            };
            let mut recovery = false;
            if !hevc && h & 0x1F == 6 {
                // the whole SEI NAL is needed to find a recovery point message among others
                let end = self.next_start_code(k + 3);
                if end.is_none() && self.buf.len() - k < SEI_WINDOW {
                    blocked = Some(k);
                    break;
                }
                let nal = self.buf.get(k + 3..end.unwrap_or(self.buf.len())).unwrap_or(&[]);
                recovery = sei_has_recovery_point(nal);
            }
            // a unit holds one access unit: a new one starts at the first NAL that cannot belong
            // to the slices already seen (a unit opened later than this NAL is left alone)
            if au_start && self.open.as_ref().is_some_and(|o| o.vcl && at > o.start) {
                self.begin_access_unit(at);
            }
            if let Some(o) = self.open.as_mut()
                && at >= o.start
            {
                // a recovery point is a random-access point (the pictures before its recovery
                // frame count completes may be imperfect)
                o.key |= recovery;
                if vcl {
                    o.key |= key;
                    // disposable while every slice is non-reference
                    o.disposable = if o.vcl { o.disposable && non_ref } else { non_ref };
                    o.vcl = true;
                }
            }
            i = k + 3;
        }
        self.keep_tail(i, blocked);
    }

    /// Close the open unit and start the next access unit at `at`: it has no timestamp (the PES
    /// timestamps belong to the first access unit of the packet).
    fn begin_access_unit(&mut self, at: u64) {
        self.close(at);
        let (pos, offset) = self.locate(at).unwrap_or((u64::MAX, 0));
        self.open = Some(Open { start: at, pos, offset, pts: None, dts: None, key: false, disposable: false, vcl: false, picture: None });
    }

    /// Drop the scanned part of the window: keep from a start code waiting for more bytes, or
    /// the last bytes a split start code may begin in.
    fn keep_tail(&mut self, i: usize, blocked: Option<usize>) {
        let len = self.buf.len();
        let resume = blocked.unwrap_or_else(|| i.max(len.saturating_sub(3)));
        self.trim(len - resume.min(len));
    }

    fn scan_mpeg(&mut self, start: u64, data: &[u8]) {
        self.window(start, data);
        let mut i = 0;
        let mut blocked = None;
        while let Some(k) = self.next_start_code(i) {
            // up to 5 bytes after the code are inspected
            if k + 9 > self.buf.len() {
                blocked = Some(k);
                break;
            }
            let at = self.buf_start + k as u64;
            let code = self.buf[k + 3];
            let p = [self.buf[k + 4], self.buf[k + 5], self.buf[k + 6], self.buf[k + 7], self.buf[k + 8]];
            match code {
                0xB3 => {
                    self.settle_pending();
                    self.header_start.get_or_insert(at);
                    self.seq_in_unit = true;
                }
                0xB8 => {
                    self.settle_pending();
                    self.header_start.get_or_insert(at);
                    self.gop = Some((p[3] & 0x40 != 0, p[3] & 0x20 != 0));
                }
                0x00 => {
                    self.settle_pending();
                    let info = PictureInfo {
                        coding_type: (p[1] >> 3) & 7,
                        temporal_reference: ((p[0] as u16) << 2) | (p[1] >> 6) as u16,
                        structure: 3,
                        progressive_frame: true,
                        sequence_header: self.seq_in_unit,
                        gop: self.gop.is_some(),
                        closed_gop: self.gop.is_some_and(|g| g.0),
                        broken_link: self.gop.is_some_and(|g| g.1),
                        ..Default::default()
                    };
                    let unit_start = self.header_start.take().unwrap_or(at);
                    self.seq_in_unit = false;
                    self.gop = None;
                    self.pending = Some(PendingPicture { unit_start, at, info });
                }
                0xB5 if p[0] >> 4 == 8 => {
                    if let Some(pp) = self.pending.as_mut() {
                        pp.info.structure = p[2] & 3;
                        pp.info.top_field_first = p[3] & 0x80 != 0;
                        pp.info.repeat_first_field = p[3] & 0x02 != 0;
                        pp.info.progressive_frame = p[4] & 0x80 != 0;
                    }
                    self.settle_pending();
                }
                0xB5 | 0xB2 => {}
                _ => self.settle_pending(),
            }
            i = k + 3;
        }
        self.keep_tail(i, blocked);
    }

    /// A picture's structure is known: start a unit, or join a first field.
    fn settle_pending(&mut self) {
        let Some(pp) = self.pending.take() else { return };
        let s = pp.info.structure;
        if let Some(first) = self.open_field
            && s != 3
            && first != s
            && s != 0
        {
            self.open_field = None;
            return;
        }
        self.close(pp.unit_start);
        let (pos, offset) = self.locate(pp.unit_start).unwrap_or((u64::MAX, 0));
        let mut o = Open {
            start: pp.unit_start,
            pos,
            offset,
            pts: None,
            dts: None,
            key: matches!(pp.info.coding_type, 1 | 4),
            disposable: pp.info.coding_type == 3,
            vcl: true,
            picture: Some(pp.info),
        };
        if let Some((pts, dts)) = self.timestamps_at(pp.at) {
            o.pts = pts;
            o.dts = dts;
        }
        self.open = Some(o);
        self.open_field = (s == 1 || s == 2).then_some(s);
    }

    fn scan_audio(&mut self, start: u64, data: &[u8], sync: AudioSync) {
        let end = start + data.len() as u64;
        // skip whole chunks inside the current frame
        if let Some(n) = self.next_frame
            && n >= end
        {
            return;
        }
        self.window(start, data);
        loop {
            let at = match self.next_frame {
                Some(n) if n >= self.buf_start => n,
                _ => self.buf_start,
            };
            let i = (at - self.buf_start) as usize;
            if i + 8 > self.buf.len() {
                // not enough bytes for a header yet
                self.trim(self.buf.len().saturating_sub(i));
                return;
            }
            match frame_len(sync, &self.buf[i..]) {
                Some(len) if self.next_frame == Some(at) || self.confirmed(sync, i, len) => {
                    // an E-AC-3 dependent substream or further program joins the access unit of
                    // the independent substream 0 before it, as in MP4 and Matroska
                    if !(sync == AudioSync::Ac3 && self.open.is_some() && eac3_joins_unit(&self.buf[i..])) {
                        self.close(at);
                        let (pos, offset) = self.locate(at).unwrap_or((u64::MAX, 0));
                        let mut o = Open { start: at, pos, offset, pts: None, dts: None, key: true, disposable: false, vcl: false, picture: None };
                        if let Some((pts, dts)) = self.timestamps_at(at) {
                            o.pts = pts;
                            o.dts = dts;
                        }
                        self.open = Some(o);
                    }
                    self.next_frame = Some(at + len as u64);
                    if at + len as u64 >= self.buf_start + self.buf.len() as u64 {
                        self.buf.clear();
                        self.buf_start = at + len as u64;
                        return;
                    }
                }
                _ => {
                    // lost sync: look for the next header from the following byte
                    self.next_frame = None;
                    self.skipped += 1;
                    let next = self.buf[i + 1..].iter().position(|&b| {
                        matches!((sync, b), (AudioSync::Ac3, 0x0B) | (AudioSync::Loas, 0x56) | (AudioSync::Mpa | AudioSync::Adts, 0xFF))
                    });
                    match next {
                        Some(d) => {
                            self.skipped += d as u64;
                            // Keep a cursor into the current window rather than draining its
                            // prefix for every false sync candidate.
                            self.next_frame = Some(at + 1 + d as u64);
                        }
                        None => {
                            self.skipped += (self.buf.len() - i - 1) as u64;
                            // A split header can begin in the final few bytes; compact once.
                            self.trim(7);
                            return;
                        }
                    }
                }
            }
        }
    }

    /// A header found while hunting for sync is accepted when the next frame's header follows
    /// it (or the data ends before it).
    fn confirmed(&self, sync: AudioSync, i: usize, len: usize) -> bool {
        let j = i + len;
        if j + 8 > self.buf.len() {
            return true;
        }
        frame_len(sync, &self.buf[j..]).is_some()
    }

    /// Opus access units: each control header gives the payload size, so the unit starts after
    /// the header and ends where the next header starts.
    fn scan_opus(&mut self, start: u64, data: &[u8]) {
        let end = start + data.len() as u64;
        let Some(next) = self.next_frame else {
            // the rest of the PES packet belongs to the open unit, or is not Opus at all
            if self.open.is_none() {
                self.skipped += data.len() as u64;
            }
            // no later header can refer back to these bytes: `feed` may forget their chunks
            self.buf.clear();
            self.buf_start = end;
            return;
        };
        if next >= end {
            // inside the open unit's payload
            self.buf.clear();
            self.buf_start = end;
            return;
        }
        self.window(start, data);
        while let Some(at) = self.next_frame {
            if at < self.buf_start {
                self.next_frame = None;
                break;
            }
            let i = (at - self.buf_start) as usize;
            if i >= self.buf.len() {
                break;
            }
            match opus_control_header(&self.buf[i..], at, &mut self.opus_scan) {
                OpusHeader::Incomplete => {
                    self.trim(self.buf.len() - i);
                    return;
                }
                OpusHeader::Absent => {
                    // no control header: the access unit runs to the end of the PES packet
                    self.close(at);
                    self.open_opus(at, at, (0, 0));
                    self.next_frame = None;
                }
                OpusHeader::Invalid => {
                    self.close(at);
                    self.skipped += end - at;
                    self.next_frame = None;
                }
                OpusHeader::Found { len, size, trim } => {
                    self.close(at);
                    let payload = at + len as u64;
                    self.open_opus(at, payload, trim);
                    self.next_frame = Some(payload + size);
                }
            }
        }
        self.buf.clear();
        self.buf_start = end;
    }

    /// Open an Opus unit at `start` for the access unit whose header (if any) starts at `at`.
    fn open_opus(&mut self, at: u64, start: u64, trim: (u16, u16)) {
        // a payload that starts with the next chunk is placed when that chunk arrives
        let (pos, offset) = if start < self.es_pos { self.locate(start).unwrap_or((u64::MAX, 0)) } else { (u64::MAX, 0) };
        let mut o = Open { start, pos, offset, pts: None, dts: None, key: true, disposable: false, vcl: false, picture: None };
        if let Some((pts, dts)) = self.timestamps_at(at) {
            o.pts = pts;
            o.dts = dts;
        }
        self.open = Some(o);
        self.open_trim = trim;
    }

    /// MPEG-4 Part 2 video: the unit's first VOP header gives its coding type (ISO/IEC 14496-2
    /// §6.2.5: 0 I, 1 P, 2 B, 3 S).
    fn scan_mpeg4(&mut self, start: u64, data: &[u8]) {
        self.window(start, data);
        if self.open.is_some_and(|o| o.vcl) {
            // the unit's flags are known: keep only where a start code may begin
            self.trim(3);
            return;
        }
        let mut i = 0;
        let mut blocked = None;
        while let Some(k) = self.next_start_code(i) {
            if k + 5 > self.buf.len() {
                blocked = Some(k);
                break;
            }
            let at = self.buf_start + k as u64;
            if self.buf[k + 3] == 0xB6
                && let Some(o) = self.open.as_mut()
                && !o.vcl
                && at >= o.start
            {
                let coding_type = self.buf[k + 4] >> 6;
                o.key = coding_type == 0;
                o.disposable = coding_type == 2;
                o.vcl = true;
            }
            i = k + 3;
        }
        self.keep_tail(i, blocked);
    }

    /// End of stream: close the open unit.
    pub fn finish(&mut self) {
        if self.mode == Mode::Opus {
            self.close_opus(self.es_pos);
            return;
        }
        self.settle_pending();
        let end = match (self.mode, self.next_frame) {
            (Mode::Audio(_), Some(n)) => n.min(self.es_pos),
            _ => self.es_pos,
        };
        if let (Mode::Audio(_), Some(o)) = (self.mode, self.open) {
            // a truncated last frame is dropped
            if self.next_frame.is_some_and(|n| n > self.es_pos) {
                self.skipped += self.es_pos - o.start;
                self.open = None;
            }
        }
        self.close(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_frame_sizes() {
        // MPEG-1 layer II 48 kHz 192 kb/s: 576 bytes; layer III 44.1 kHz 128 kb/s with padding: 418
        assert_eq!(mpa_header(&[0xFF, 0xFD, 0xA4, 0x00]), Some((576, 48_000, 1152)));
        assert_eq!(mpa_header(&[0xFF, 0xFB, 0x92, 0x00]), Some((418, 44_100, 1152)));
        // MPEG-2 layer III 24 kHz 64 kb/s: 192 bytes, 576 samples
        assert_eq!(mpa_header(&[0xFF, 0xF3, 0x84, 0x00]), Some((192, 24_000, 576)));
        // AC-3 448 kb/s at 48 kHz = 1792 bytes; 44.1 kHz 192 kb/s odd code = 836 bytes
        assert_eq!(ac3_frame_bytes(0, 30), Some(1792));
        assert_eq!(ac3_frame_bytes(1, 21), Some(836));
        assert_eq!(ac3_frame_bytes(2, 0), Some(192));
    }

    #[test]
    fn eac3_dependent_substreams_join_the_access_unit() {
        // 16-byte E-AC-3 syncframes (frmsiz 7), 48 kHz six blocks stereo: independent, dependent
        // (7.1 extension), independent, dependent
        let frame = |strmtyp: u8| {
            let mut f = vec![0x0B, 0x77, strmtyp << 6, 7, (3 << 4) | (2 << 1), 16 << 3];
            f.resize(16, 0);
            f
        };
        let es: Vec<u8> = [frame(0), frame(1), frame(0), frame(1)].concat();
        let mut s = Splitter::new(&Codec::Eac3);
        s.feed(0, &es);
        s.finish();
        assert_eq!(s.units.iter().map(|u| u.size).collect::<Vec<_>>(), vec![32, 32]);
        assert_eq!(frame_info(&Codec::Eac3, &es).map(|f| f.samples), Some(1536));
        // AC-3 frames are never joined
        assert!(!eac3_joins_unit(&[0x0B, 0x77, 0x40, 0, 0, 8 << 3]));
    }

    #[test]
    fn ac3_header_prefix_of_six_bytes_is_not_indexed_out_of_bounds() {
        // exactly the six bytes frame_len() guarantees: byte 6 (acmod) is missing
        assert_eq!(frame_info(&Codec::Ac3, &[0x0B, 0x77, 0, 0, 0x1E, 0x40]), None);
        assert_eq!(frame_info(&Codec::Ac3, &[0x0B, 0x77, 0, 0, 0x1E, 0x40, 0]), None);
    }

    #[test]
    fn eac3_reserved_sample_rate_is_refused() {
        // fscod 3 and fscod2 3: no sample rate
        let mut f = vec![0x0B, 0x77, 0, 7, 0xF0, 16 << 3];
        f.resize(16, 0);
        assert_eq!(frame_info(&Codec::Eac3, &f), None);
        f[4] = 0xE0; // fscod2 2: 16 kHz
        assert_eq!(frame_info(&Codec::Eac3, &f).map(|i| i.sample_rate), Some(16_000));
    }

    #[test]
    fn four_byte_eac3_frames_make_no_units() {
        let es: Vec<u8> = [0x0B, 0x77, 0x00, 0x01].repeat(10_000);
        let mut s = Splitter::new(&Codec::Eac3);
        s.feed(0, &es);
        s.finish();
        assert!(s.units.is_empty(), "{} units from 40000 noise bytes", s.units.len());
    }

    #[test]
    fn pes_delimited_modes_do_not_retain_chunks() {
        for codec in [Codec::LpcmBluray, Codec::Unknown(0x99)] {
            let mut s = Splitter::new(&codec);
            for k in 0..5000u64 {
                s.pes_start(Some(k as i64), None);
                s.feed(k * 188, &[0x11; 184]);
            }
            s.finish();
            assert!(s.chunks.len() <= 2, "{codec:?}: {} chunks retained", s.chunks.len());
            assert_eq!(s.units.len(), 5000, "{codec:?}");
            assert!(s.units.iter().enumerate().all(|(k, u)| u.pos == k as u64 * 188 && u.offset == 0), "{codec:?}");
        }
    }

    #[test]
    fn a_long_opus_control_header_is_read_once() {
        // a payload size of 100,000 bytes of 0xFF, cut into transport stream payloads
        let mut s = Splitter::new(&Codec::Opus);
        s.pes_start(Some(0), None);
        let mut es = vec![0x7F, 0xE0];
        es.extend(std::iter::repeat_n(0xFF, 100_000));
        es.push(0);
        for (k, c) in es.chunks(184).enumerate() {
            s.feed(k as u64 * 188, c);
        }
        assert!(s.opus_scan.reads <= es.len() as u64, "{} size bytes read for a {}-byte header", s.opus_scan.reads, es.len());
        assert_eq!(s.next_frame, Some(es.len() as u64 + 255 * 100_000));
    }

    #[test]
    fn opus_chunks_are_forgotten_inside_a_unit_and_after_a_headerless_one() {
        // an access unit whose payload spans 500 chunks, then a PES packet without a control
        // header
        let mut s = Splitter::new(&Codec::Opus);
        s.pes_start(Some(0), None);
        let size = 184 * 500;
        let mut es = vec![0x7F, 0xE0];
        es.extend(std::iter::repeat_n(0xFF, size / 255));
        es.push((size % 255) as u8);
        es.resize(es.len() + size, 0x11);
        let mut pos = 0;
        for (k, c) in es.chunks(184).enumerate() {
            s.feed(pos, c);
            pos += 188;
            assert!(k < 3 || s.chunks.len() <= 2, "chunk {k}: {} chunks retained", s.chunks.len());
        }
        s.pes_start(Some(1800), None);
        for k in 0..500 {
            s.feed(pos, &[0xFC; 184]);
            pos += 188;
            assert!(s.chunks.len() <= 2, "headerless chunk {k}: {} chunks retained", s.chunks.len());
        }
        s.finish();
        assert_eq!(s.units.iter().map(|u| u.size as usize).collect::<Vec<_>>(), [size, 184 * 500]);
    }

    #[test]
    fn unit_table_is_capped() {
        let frame = |k: u8| {
            let mut v = vec![0x0B, 0x77, 0, 7, (3 << 4) | (2 << 1), 16 << 3];
            v.resize(16, k);
            v
        };
        let mut s = Splitter::new(&Codec::Eac3);
        s.max_units = 10;
        let es: Vec<u8> = (0..100).flat_map(frame).collect();
        s.feed(0, &es);
        s.finish();
        assert_eq!(s.units.len(), 10);
        assert!(s.truncated);
    }
}

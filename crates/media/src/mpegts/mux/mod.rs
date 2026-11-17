//! MPEG-2 transport stream writer (ITU-T H.222.0 | ISO/IEC 13818-1).
//!
//! [`Muxer`] writes one program to any [`Write`] as frames arrive:
//!
//! - a program association section and a program map section, each ending with its CRC-32, at the
//!   start, every [`Config::psi_interval`], before every video keyframe (optional), after
//!   [`Muxer::replace_writer`] and after [`Muxer::discontinuity`];
//! - one PES packet per frame for H.264, HEVC, MPEG-1, MPEG-2 and MPEG-4 Part 2 video, AV1, AAC in
//!   ADTS frames (passed in, or built from raw AAC frames), Opus, AC-3, E-AC-3 and MPEG audio
//!   (layers I to III), with PTS and, when it differs, DTS;
//! - the program clock reference on [`Config::pcr_pid`], in the adaptation field of the PCR
//!   stream's frames or in packets of its own, never more than [`Config::pcr_interval`] apart
//!   (at most 100 ms, §2.7.2);
//! - a continuity counter per PID, the random access indicator on keyframes, the discontinuity
//!   indicator after [`Muxer::discontinuity`], and adaptation field stuffing in each frame's last
//!   packet.
//!
//! Timestamps are 90 kHz ticks. They are written modulo 2^33, so a stream may run past the
//! wrap. The PCR follows the largest DTS so far minus [`Config::pcr_delay`].
//!
//! # Arrival order
//!
//! Frames of different streams may arrive in any order, as long as each frame's DTS is no more
//! than [`Config::pcr_delay`] behind the largest DTS written so far (or the largest time given to
//! [`Muxer::advance`]). Within one stream the DTS never goes back, and for video it goes forward
//! with every frame. A caller whose encoders drift further apart than `pcr_delay` (at most
//! [`MAX_PCR_DELAY`], one second) queues frames and writes them in DTS order.
//!
//! # Output and failures
//!
//! Packets are built in a 188-byte buffer and collected in a batch of [`Config::batch_packets`]
//! packets, allocated once. A full batch is handed to the output in one `write` call, repeated
//! only when the output takes part of it; the default of 7 packets (1316 bytes) is one SRT or UDP
//! payload. Unless [`Config::hold_packets`] is set, the
//! rest of the batch is written at the end of every [`Muxer::write_frame`] and [`Muxer::advance`]
//! call, so a live output never waits for the next frame. A file output can set `hold_packets`
//! and a larger batch (for example 348 packets, 65 424 bytes) and then needs no `BufWriter`.
//!
//! The output must block until it has taken each write: an [`io::ErrorKind::WouldBlock`] error
//! is an I/O error like any other. After an I/O error the rest of that batch is dropped, the
//! output may hold part of a packet, and the continuity counters have moved on.
//! [`Stats::packets`] and [`Segment::packets`] count only the whole packets the output took. To
//! recover, give the muxer a new output with
//! [`Muxer::replace_writer`]: the new output starts at a packet boundary with the PAT, the PMT and
//! a PCR, and reads on its own.
//!
//! The names here ([`Codec`], [`Stream`], [`Error`]) are the writer's own and differ from the
//! demuxer's in [`crate::mpegts`].

mod av1;
mod pes;
mod psi;

use std::fmt;
use std::io::{self, Write};

/// Bytes in a transport stream packet.
pub const PACKET_SIZE: usize = 188;
/// Timestamp ticks per second.
pub const TICKS_PER_SECOND: u32 = 90_000;
/// Largest PCR interval, 100 ms in 90 kHz ticks (H.222.0 §2.7.2).
pub const MAX_PCR_INTERVAL: u32 = 9_000;
/// Largest [`Config::pcr_delay`], one second in 90 kHz ticks: data may spend at most one second
/// in the decoder's buffers (H.222.0 §2.4.2.6).
pub const MAX_PCR_DELAY: u32 = 90_000;
/// Most elementary streams in one program.
pub const MAX_STREAMS: usize = 32;
/// Largest number of PCR-only packets, and of PAT and PMT repeats, written to fill one gap in the
/// timeline. A gap that needs more PCRs gets one PCR at its end and is counted in
/// [`Stats::pcr_gaps`].
pub const MAX_PCR_FILL: u32 = 1024;
/// Largest video frame, [`super::MAX_UNIT_BYTES`] (128 MiB): the largest unit the demuxer
/// returns.
pub const MAX_VIDEO_FRAME: usize = super::MAX_UNIT_BYTES as usize;
/// Largest [`Config::batch_packets`].
pub const MAX_BATCH_PACKETS: u16 = 1024;
/// Largest Opus start or end trim, in samples at 48 kHz (a 13-bit field).
pub const MAX_OPUS_TRIM: u16 = 8191;

/// Errors from writing a transport stream.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Writing the output failed.
    Io(io::Error),
    /// The configuration or the stream list is not valid.
    Config(String),
    /// This PID is outside 0x0010 to 0x1FFE.
    BadPid(u16),
    /// This PID is used for two things.
    DuplicatePid(u16),
    /// The program map section would be longer than 1021 bytes.
    PsiTooLarge {
        /// The section_length it would have.
        length: usize,
    },
    /// [`Config::pcr_delay`] is outside 1 to [`MAX_PCR_DELAY`].
    PcrDelay(u32),
    /// There is no stream with this index.
    NoSuchStream(usize),
    /// The frame has no bytes.
    EmptyFrame,
    /// The frame is not in the form its codec needs.
    BadFrame(&'static str),
    /// The frame is larger than its codec allows.
    FrameTooLarge {
        /// Bytes in the frame.
        size: usize,
        /// Most bytes the frame may have.
        limit: usize,
    },
    /// The frame's timestamps are not valid.
    Timestamp(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O: {e}"),
            Error::Config(s) => write!(f, "transport stream settings: {s}"),
            Error::BadPid(pid) => write!(f, "PID 0x{pid:04X} is outside 0x0010 to 0x1FFE"),
            Error::DuplicatePid(pid) => write!(f, "PID 0x{pid:04X} is used twice"),
            Error::PsiTooLarge { length } => write!(f, "the program map section would be {length} bytes long; the limit is 1021"),
            Error::PcrDelay(d) => write!(f, "a PCR delay of {d} ticks is outside 1 to {MAX_PCR_DELAY}"),
            Error::NoSuchStream(i) => write!(f, "no stream {i}"),
            Error::EmptyFrame => write!(f, "the frame is empty"),
            Error::BadFrame(s) => write!(f, "{s}"),
            Error::FrameTooLarge { size, limit } => write!(f, "the frame is {size} bytes; the limit is {limit}"),
            Error::Timestamp(s) => write!(f, "{s}"),
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

/// Result alias for the writer.
pub type Result<T> = std::result::Result<T, Error>;

/// The fields of the AV1 video descriptor (Carriage of AV1 in MPEG-2 TS, §2.2), taken from the
/// stream's sequence header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Av1Config {
    /// seq_profile, 0 to 2.
    pub seq_profile: u8,
    /// seq_level_idx[0], 0 to 31.
    pub seq_level_idx_0: u8,
    /// seq_tier[0].
    pub seq_tier_0: bool,
    /// high_bitdepth.
    pub high_bitdepth: bool,
    /// twelve_bit.
    pub twelve_bit: bool,
    /// mono_chrome.
    pub monochrome: bool,
    /// subsampling_x.
    pub chroma_subsampling_x: bool,
    /// subsampling_y.
    pub chroma_subsampling_y: bool,
    /// chroma_sample_position, 0 to 3.
    pub chroma_sample_position: u8,
    /// 0 SDR, 1 wide color gamut, 2 HDR and wide color gamut, 3 no indication.
    pub hdr_wcg_idc: u8,
}

impl Av1Config {
    /// The fields from the first four bytes of an ISOBMFF `av1C` record (marker and version 1),
    /// with `hdr_wcg_idc` 3 (no indication).
    pub fn from_av1c(record: &[u8]) -> Result<Self> {
        let &[b0, b1, b2, _, ..] = record else { return Err(Error::Config("an av1C record has at least four bytes".into())) };
        if b0 != 0x81 {
            return Err(Error::Config("an av1C record has at least four bytes and starts with marker 1 and version 1".into()));
        }
        let c = Self {
            seq_profile: b1 >> 5,
            seq_level_idx_0: b1 & 0x1F,
            seq_tier_0: b2 & 0x80 != 0,
            high_bitdepth: b2 & 0x40 != 0,
            twelve_bit: b2 & 0x20 != 0,
            monochrome: b2 & 0x10 != 0,
            chroma_subsampling_x: b2 & 0x08 != 0,
            chroma_subsampling_y: b2 & 0x04 != 0,
            chroma_sample_position: b2 & 3,
            hdr_wcg_idc: 3,
        };
        c.check().map_err(Error::Config)?;
        Ok(c)
    }

    fn check(&self) -> std::result::Result<(), String> {
        if self.seq_profile > 2 {
            return Err(format!("AV1 seq_profile {} is not 0 to 2", self.seq_profile));
        }
        if self.seq_level_idx_0 > 31 || self.chroma_sample_position > 3 || self.hdr_wcg_idc > 3 {
            return Err("an AV1 descriptor field is out of range".into());
        }
        Ok(())
    }
}

/// An Opus channel configuration coded in full (channel_config_code 0x81 of the Opus transport
/// draft): the channel mapping family, the stream counts and the channel mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpusMapping {
    /// Output channels, 1 to 8.
    pub channels: u8,
    /// Channel mapping family (RFC 7845 §5.1.1): 0 for one mono or stereo stream, 1 for the
    /// standard surround orders, 255 for an application-defined order. Other families are
    /// refused.
    pub family: u8,
    /// Opus streams in each access unit, 1 to `channels` (1 for family 0).
    pub streams: u8,
    /// Streams that decode to two channels, at most `streams` (`channels - 1` for family 0).
    pub coupled: u8,
    /// For each output channel, the coded channel it is taken from (below `streams + coupled`),
    /// or 255 for silence. Entries past `channels` are ignored. Family 0 carries no mapping in
    /// the descriptor, so its entries must be 0 and 1.
    pub mapping: [u8; 8],
}

impl OpusMapping {
    fn check(&self) -> std::result::Result<(), String> {
        let ch = usize::from(self.channels);
        if !(1..=8).contains(&ch) {
            return Err(format!("{ch} Opus channels; 1 to 8 are supported"));
        }
        if self.family == 0 {
            if ch > 2 || self.streams != 1 || u16::from(self.coupled) + 1 != u16::from(self.channels) {
                return Err("Opus mapping family 0 is one stream of 1 or 2 channels".into());
            }
            // the descriptor carries no mapping for family 0: the channels are in coded order
            if self.mapping[..ch].iter().enumerate().any(|(i, &m)| usize::from(m) != i) {
                return Err("Opus mapping family 0 keeps the channels in coded order (mapping 0, 1)".into());
            }
            return Ok(());
        }
        if !matches!(self.family, 1 | 255) {
            return Err(format!("Opus mapping family {}; 0, 1 and 255 are supported", self.family));
        }
        if self.streams == 0 || self.streams > self.channels || self.coupled > self.streams {
            return Err("Opus streams are 1 to the channel count, and coupled streams at most the stream count".into());
        }
        let coded = self.streams + self.coupled;
        if self.mapping[..ch].iter().any(|&m| m >= coded && m != 255) {
            return Err("an Opus channel mapping entry names a coded channel that does not exist".into());
        }
        Ok(())
    }
}

/// What an elementary stream carries, and the form its frames take.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// H.264 access units in Annex B form, with the parameter sets on keyframes. An access unit
    /// delimiter is added when a frame does not start with one.
    H264,
    /// HEVC access units in Annex B form, with the parameter sets on keyframes. An access unit
    /// delimiter is added when a frame does not start with one.
    Hevc,
    /// MPEG-1 video pictures (ISO/IEC 11172-2), from their first start code.
    Mpeg1Video,
    /// MPEG-2 video pictures, from their first start code.
    Mpeg2Video,
    /// MPEG-4 Part 2 visual frames (ISO/IEC 14496-2), from their first start code.
    Mpeg4Part2,
    /// AV1 temporal units in the low overhead bitstream format (every OBU with its size field),
    /// with a sequence header on keyframes. Each OBU is written after a start code with emulation
    /// prevention. Temporal delimiters, and padding OBUs that end with a zero byte, are left out
    /// (Carriage of AV1 in MPEG-2 TS).
    Av1(Av1Config),
    /// AAC frames that already carry their ADTS headers; one call may pass several frames.
    AacAdts,
    /// Raw AAC frames; each one gets an ADTS header built from these fields.
    Aac {
        /// Audio object type, 1 to 4 (2 is AAC LC).
        object_type: u8,
        /// Sampling frequency index, 0 to 12 (3 is 48 kHz).
        sample_rate_index: u8,
        /// Channel configuration, 1 to 7.
        channels: u8,
    },
    /// Opus access units, one per frame; each gets an Opus control header. For 1 or 2 channels
    /// the access unit is one Opus packet. For 3 to 8 channels it holds the streams of mapping
    /// family 1 in the order the Opus transport draft's table 4-3 gives, all but the last
    /// self-delimited; use [`Codec::OpusMapped`] for any other layout.
    Opus {
        /// Channel count, 1 to 8.
        channels: u8,
    },
    /// Opus access units with a channel configuration given in full.
    OpusMapped(OpusMapping),
    /// AC-3 sync frames.
    Ac3,
    /// E-AC-3 sync frames (bsid 11 to 16).
    Eac3,
    /// MPEG-1 or MPEG-2 audio frames, layers I to III (MP2, MP3).
    MpegAudio {
        /// The stream uses the MPEG-2 low sampling frequencies (16, 22.05 and 24 kHz) or the
        /// lower MPEG 2.5 rates. Each frame's version bits must agree.
        low_sample_rate: bool,
    },
}

impl Codec {
    /// The PMT stream_type.
    pub fn stream_type(self) -> u8 {
        match self {
            Codec::H264 => 0x1B,
            Codec::Hevc => 0x24,
            Codec::Mpeg1Video => 0x01,
            Codec::Mpeg2Video => 0x02,
            Codec::Mpeg4Part2 => 0x10,
            Codec::AacAdts | Codec::Aac { .. } => 0x0F,
            Codec::Av1(_) | Codec::Opus { .. } | Codec::OpusMapped(_) => 0x06,
            Codec::Ac3 => 0x81,
            Codec::Eac3 => 0x87,
            Codec::MpegAudio { low_sample_rate: false } => 0x03,
            Codec::MpegAudio { low_sample_rate: true } => 0x04,
        }
    }

    /// The PES stream_id.
    pub fn stream_id(self) -> u8 {
        match self {
            Codec::H264 | Codec::Hevc | Codec::Mpeg1Video | Codec::Mpeg2Video | Codec::Mpeg4Part2 => 0xE0,
            Codec::AacAdts | Codec::Aac { .. } | Codec::MpegAudio { .. } => 0xC0,
            Codec::Av1(_) | Codec::Opus { .. } | Codec::OpusMapped(_) | Codec::Ac3 | Codec::Eac3 => 0xBD,
        }
    }

    /// The stream carries pictures.
    pub fn is_video(self) -> bool {
        matches!(self, Codec::H264 | Codec::Hevc | Codec::Mpeg1Video | Codec::Mpeg2Video | Codec::Mpeg4Part2 | Codec::Av1(_))
    }

    fn is_opus(self) -> bool {
        matches!(self, Codec::Opus { .. } | Codec::OpusMapped(_))
    }

    /// Raw AAC described by an AudioSpecificConfig (ISO/IEC 14496-3 §1.6.2.1). Configurations
    /// that an ADTS header cannot carry are refused: an object type outside 1 to 4 (SBR and PS
    /// signaled explicitly included), an explicit sampling frequency, channel configuration 0,
    /// and frameLengthFlag 1 (960-sample frames).
    pub fn aac_from_asc(asc: &[u8]) -> Result<Codec> {
        let &[a, b, ..] = asc else { return Err(Error::Config("an AudioSpecificConfig has at least two bytes".into())) };
        let object_type = a >> 3;
        let sample_rate_index = (a & 7) << 1 | b >> 7;
        let channels = (b >> 3) & 0x0F;
        let codec = Codec::Aac { object_type, sample_rate_index, channels };
        codec.check().map_err(Error::Config)?;
        // GASpecificConfig's frameLengthFlag follows the channel configuration (ISO/IEC 14496-3 §4.4.1).
        if b & 0x04 != 0 {
            return Err(Error::Config("AAC frameLengthFlag 1 (960-sample frames) cannot be carried by an ADTS header".into()));
        }
        Ok(codec)
    }

    fn check(self) -> std::result::Result<(), String> {
        match self {
            Codec::Aac { object_type, sample_rate_index, channels } => {
                if !(1..=4).contains(&object_type) {
                    return Err(format!("AAC object type {object_type} is not 1 to 4"));
                }
                if sample_rate_index > 12 {
                    return Err(format!("AAC sampling frequency index {sample_rate_index} is not 0 to 12"));
                }
                if !(1..=7).contains(&channels) {
                    return Err(format!("AAC channel configuration {channels} is not 1 to 7"));
                }
                Ok(())
            }
            Codec::Opus { channels } if !(1..=8).contains(&channels) => Err(format!("{channels} Opus channels; 1 to 8 are supported")),
            Codec::OpusMapped(m) => m.check(),
            Codec::Av1(c) => c.check(),
            _ => Ok(()),
        }
    }
}

/// One elementary stream of the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream {
    /// The PID its packets carry, 0x0010 to 0x1FFE.
    pub pid: u16,
    /// What it carries.
    pub codec: Codec,
    /// ISO 639-2 language code (three lowercase letters), written as a language descriptor.
    pub language: Option<[u8; 3]>,
}

impl Stream {
    /// A stream on `pid` without a language.
    pub fn new(pid: u16, codec: Codec) -> Self {
        Self { pid, codec, language: None }
    }

    /// The same stream with a language code.
    pub fn with_language(mut self, language: [u8; 3]) -> Self {
        self.language = Some(language);
        self
    }
}

/// Program, timing and output settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// transport_stream_id in the program association section.
    pub transport_stream_id: u16,
    /// program_number, not 0.
    pub program_number: u16,
    /// The PID of the program map section, 0x0010 to 0x1FFE.
    pub pmt_pid: u16,
    /// The PID that carries the PCR: one of the streams' PIDs, or a PID of its own.
    /// `None` uses the first video stream, or the first stream when there is no video.
    pub pcr_pid: Option<u16>,
    /// Largest gap between PCRs, 90 kHz ticks, 1 to [`MAX_PCR_INTERVAL`].
    pub pcr_interval: u32,
    /// How far the PCR runs behind the largest DTS so far, 90 kHz ticks, 1 to
    /// [`MAX_PCR_DELAY`]. It is also the largest skew between streams (see the crate docs).
    pub pcr_delay: u32,
    /// How often the PAT and PMT are repeated, 90 kHz ticks, at least 1.
    pub psi_interval: u32,
    /// Repeat the PAT and PMT before every video keyframe, so each keyframe starts a segment that
    /// decodes on its own.
    pub psi_before_keyframes: bool,
    /// Packets collected before one write to the output, 1 to [`MAX_BATCH_PACKETS`].
    pub batch_packets: u16,
    /// Keep a partly filled batch between calls instead of writing it at the end of each
    /// [`Muxer::write_frame`] and [`Muxer::advance`]. For file outputs; [`Muxer::flush`],
    /// [`Muxer::replace_writer`] and [`Muxer::finish`] still write it.
    pub hold_packets: bool,
}

impl Default for Config {
    /// Program 1 with its PMT on PID 0x1000; the PCR at most 40 ms apart and 700 ms behind the
    /// DTS; the PAT and PMT every 100 ms and before keyframes; writes of 7 packets, and the rest
    /// at the end of every call.
    fn default() -> Self {
        Self {
            transport_stream_id: 1,
            program_number: 1,
            pmt_pid: 0x1000,
            pcr_pid: None,
            pcr_interval: 3_600,
            pcr_delay: 63_000,
            psi_interval: 9_000,
            psi_before_keyframes: true,
            batch_packets: 7,
            hold_packets: false,
        }
    }
}

/// One frame to write.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// The coded frame.
    pub data: &'a [u8],
    /// Presentation time, 90 kHz ticks.
    pub pts: i64,
    /// Decoding time, 90 kHz ticks; `None` when it equals `pts`. Never after `pts`, never before
    /// the stream's previous DTS (for video, always after it), and never behind the clock: the
    /// largest DTS so far, or the largest time given to [`Muxer::advance`], minus
    /// [`Config::pcr_delay`].
    pub dts: Option<i64>,
    /// Decoding can start at this frame (an IDR or other random access picture; any audio frame).
    pub key: bool,
}

/// Samples to discard from an Opus access unit, at 48 kHz (the Opus control header's start_trim
/// and end_trim).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpusTrim {
    /// Samples discarded from the start: the encoder's pre-skip, on the first access units of a
    /// stream only, 65 535 in all.
    pub start: u16,
    /// Samples discarded from the end, on the last access unit.
    pub end: u16,
}

/// Counts of what has been written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Transport stream packets the outputs took whole. Packets still in the batch, and packets
    /// dropped after an I/O error, are not counted.
    pub packets: u64,
    /// Frames.
    pub frames: u64,
    /// Times the PAT and PMT were written.
    pub psi: u64,
    /// PCRs, in frames' packets and in packets of their own.
    pub pcrs: u64,
    /// Gaps in the timeline longer than [`MAX_PCR_FILL`] PCR intervals.
    pub pcr_gaps: u64,
}

impl Stats {
    /// Bytes written.
    pub fn bytes(&self) -> u64 {
        self.packets.saturating_mul(PACKET_SIZE as u64)
    }
}

/// What one output received, returned when [`Muxer::replace_writer`] or
/// [`Muxer::replace_writer_continuing`] closes it. An HLS packager reads the segment's duration
/// and independence from it.
#[derive(Debug)]
pub struct Segment<W> {
    /// The output.
    pub writer: W,
    /// Transport stream packets it took whole. After an error it may also hold part of a packet.
    pub packets: u64,
    /// Frames written to it.
    pub frames: u64,
    /// PTS of its first frame.
    pub first_pts: Option<i64>,
    /// Largest PTS of its frames.
    pub last_pts: Option<i64>,
    /// Its first video frame is a keyframe (with no video stream: it holds a frame), so it
    /// decodes on its own when it also starts with the tables.
    pub starts_with_keyframe: bool,
    /// Writing the last packets to it, or flushing it, failed; those packets are lost.
    pub error: Option<io::Error>,
}

struct State {
    pid: u16,
    codec: Codec,
    /// The counter the next packet with a payload carries.
    cc: u8,
    last_dts: Option<i64>,
    /// The next packet carries the discontinuity indicator.
    disc: bool,
    /// Every Opus access unit so far was discarded whole by its start trim.
    opus_lead_in: bool,
    /// Opus samples discarded by start trims so far.
    opus_start_trimmed: u32,
}

impl State {
    /// The counter an adaptation-field-only packet carries: the one before the next payload
    /// packet's (§2.4.3.3), so the counter goes up by one at the next payload packet.
    fn held_cc(&self) -> u8 {
        self.cc.wrapping_sub(1) & 0x0F
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PcrOn {
    Stream(usize),
    Own(u16),
}

/// The output and the batch of packets waiting for it.
struct Sink<W> {
    out: W,
    batch: Vec<u8>,
    cap: usize,
    /// Whole packets the output has taken.
    packets: u64,
    /// A mark that moves with every packet built and every batch dropped after an error, so
    /// the muxer can tell whether given packets are still the last ones the output gets.
    pushed: u64,
}

impl<W: Write> Sink<W> {
    fn push(&mut self, p: &[u8; PACKET_SIZE]) -> io::Result<()> {
        self.pushed += 1;
        self.batch.extend_from_slice(p);
        if self.batch.len() >= self.cap { self.drain() } else { Ok(()) }
    }

    /// Writes the batch, counting the whole packets the output takes. On an error the rest of the
    /// batch is dropped.
    fn drain(&mut self) -> io::Result<()> {
        let mut done = 0;
        let mut r = Ok(());
        while done < self.batch.len() {
            match self.out.write(&self.batch[done..]) {
                Ok(0) => {
                    r = Err(io::Error::from(io::ErrorKind::WriteZero));
                    break;
                }
                Ok(n) => done += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    r = Err(e);
                    break;
                }
            }
        }
        self.packets += (done / PACKET_SIZE) as u64;
        if r.is_err() && done < self.batch.len() {
            self.pushed += 1;
        }
        self.batch.clear();
        r
    }
}

/// What the current output has received.
#[derive(Default)]
struct SegState {
    start_packets: u64,
    frames: u64,
    first_pts: Option<i64>,
    last_pts: Option<i64>,
    first_video_key: Option<bool>,
}

/// The bytes of a frame after its PES header and prefix.
enum Payload<'a> {
    Plain(&'a [u8]),
    Av1(av1::Escaper<'a>),
}

impl Payload<'_> {
    /// Fills `out` as far as the payload goes; returns the bytes written.
    fn fill(&mut self, out: &mut [u8]) -> usize {
        match self {
            Payload::Plain(d) => {
                let n = d.len().min(out.len());
                out[..n].copy_from_slice(&d[..n]);
                *d = &d[n..];
                n
            }
            Payload::Av1(e) => e.fill(out),
        }
    }
}

/// Writes one program as a transport stream.
pub struct Muxer<W: Write> {
    sink: Sink<W>,
    cfg: Config,
    streams: Vec<State>,
    has_video: bool,
    pat: Vec<u8>,
    pmt: Vec<u8>,
    pat_cc: u8,
    pmt_cc: u8,
    pcr_on: PcrOn,
    /// The next PCR on a PID of its own carries the discontinuity indicator.
    own_pcr_disc: bool,
    /// The 27 MHz clock: the largest DTS so far minus the PCR delay.
    clock: Option<i64>,
    last_pcr: Option<i64>,
    last_psi: Option<i64>,
    /// [`Sink::pushed`] right after the last PAT and PMT; `None` once something else may follow
    /// them.
    psi_end: Option<u64>,
    psi_forced: bool,
    /// The next frame or advance writes a PCR whatever the interval (a new writer).
    pcr_forced: bool,
    stats: Stats,
    seg: SegState,
}

fn check_pid(pid: u16) -> Result<()> {
    if (0x0010..=0x1FFE).contains(&pid) { Ok(()) } else { Err(Error::BadPid(pid)) }
}

impl<W: Write> Muxer<W> {
    /// A muxer for `streams` writing to `out`. Nothing is written until the first frame or
    /// [`Muxer::advance`].
    pub fn new(out: W, cfg: Config, streams: &[Stream]) -> Result<Self> {
        if streams.is_empty() || streams.len() > MAX_STREAMS {
            return Err(Error::Config(format!("{} streams; 1 to {MAX_STREAMS} are supported", streams.len())));
        }
        if cfg.program_number == 0 {
            return Err(Error::Config("program number 0 is reserved for the network PID".into()));
        }
        check_pid(cfg.pmt_pid)?;
        if !(1..=MAX_PCR_INTERVAL).contains(&cfg.pcr_interval) {
            return Err(Error::Config(format!("a PCR interval of {} ticks is outside 1 to {MAX_PCR_INTERVAL}", cfg.pcr_interval)));
        }
        if !(1..=MAX_PCR_DELAY).contains(&cfg.pcr_delay) {
            return Err(Error::PcrDelay(cfg.pcr_delay));
        }
        if cfg.psi_interval == 0 {
            return Err(Error::Config("the PAT and PMT interval must be at least 1 tick".into()));
        }
        if !(1..=MAX_BATCH_PACKETS).contains(&cfg.batch_packets) {
            return Err(Error::Config(format!("a batch of {} packets is outside 1 to {MAX_BATCH_PACKETS}", cfg.batch_packets)));
        }
        for (i, s) in streams.iter().enumerate() {
            check_pid(s.pid)?;
            if s.pid == cfg.pmt_pid || streams[..i].iter().any(|t| t.pid == s.pid) {
                return Err(Error::DuplicatePid(s.pid));
            }
            s.codec.check().map_err(Error::Config)?;
            if let Some(l) = s.language
                && !l.iter().all(u8::is_ascii_lowercase)
            {
                return Err(Error::Config("a language code is three lowercase letters".into()));
            }
        }
        let pcr_on = match cfg.pcr_pid {
            Some(pid) => {
                check_pid(pid)?;
                if pid == cfg.pmt_pid {
                    return Err(Error::DuplicatePid(pid));
                }
                streams.iter().position(|s| s.pid == pid).map_or(PcrOn::Own(pid), PcrOn::Stream)
            }
            None => PcrOn::Stream(streams.iter().position(|s| s.codec.is_video()).unwrap_or(0)),
        };
        let pcr_pid = match pcr_on {
            PcrOn::Stream(i) => streams.get(i).map_or(0x1FFF, |s| s.pid),
            PcrOn::Own(pid) => pid,
        };
        let cap = usize::from(cfg.batch_packets) * PACKET_SIZE;
        Ok(Self {
            sink: Sink { out, batch: Vec::with_capacity(cap), cap, packets: 0, pushed: 0 },
            cfg,
            streams: streams
                .iter()
                .map(|s| State { pid: s.pid, codec: s.codec, cc: 0, last_dts: None, disc: false, opus_lead_in: true, opus_start_trimmed: 0 })
                .collect(),
            has_video: streams.iter().any(|s| s.codec.is_video()),
            pat: psi::pat(cfg.transport_stream_id, cfg.program_number, cfg.pmt_pid)?,
            pmt: psi::pmt(cfg.program_number, pcr_pid, streams)?,
            pat_cc: 0,
            pmt_cc: 0,
            pcr_on,
            own_pcr_disc: false,
            clock: None,
            last_pcr: None,
            last_psi: None,
            psi_end: None,
            psi_forced: false,
            pcr_forced: false,
            stats: Stats::default(),
            seg: SegState::default(),
        })
    }

    /// The PID that carries the PCR.
    pub fn pcr_pid(&self) -> u16 {
        match self.pcr_on {
            PcrOn::Stream(i) => self.streams.get(i).map_or(0x1FFF, |s| s.pid),
            PcrOn::Own(pid) => pid,
        }
    }

    /// What has been written so far.
    pub fn stats(&self) -> Stats {
        Stats { packets: self.sink.packets, ..self.stats }
    }

    /// The output.
    pub fn get_ref(&self) -> &W {
        &self.sink.out
    }

    /// The output, for writing between packets. With [`Config::hold_packets`] set, call
    /// [`Muxer::flush`] first so no held packet follows what is written here.
    pub fn get_mut(&mut self) -> &mut W {
        // what the caller writes may separate the last tables from the next keyframe
        self.psi_end = None;
        &mut self.sink.out
    }

    /// Writes the held packets and flushes the output.
    pub fn flush(&mut self) -> Result<()> {
        self.sink.drain()?;
        self.sink.out.flush()?;
        Ok(())
    }

    /// Closes the current output for a new segment and sends the following packets to `out`.
    /// The held packets go to the old output first, and it is flushed. The new output starts with
    /// the PAT and PMT, and the next frame (or [`Muxer::advance`]) writes a PCR; continuity
    /// counters and the clock carry on. To start every segment with a keyframe, call this right
    /// before writing a video keyframe.
    pub fn replace_writer(&mut self, out: W) -> Segment<W> {
        self.swap(out, true)
    }

    /// Like [`Muxer::replace_writer`], but the packet flow is unchanged: no tables or PCR are
    /// added, so the outputs joined end to end hold the same bytes as one output would. For the
    /// parts of a low-latency HLS segment.
    pub fn replace_writer_continuing(&mut self, out: W) -> Segment<W> {
        self.swap(out, false)
    }

    fn swap(&mut self, out: W, restart: bool) -> Segment<W> {
        let error = self.sink.drain().err().or_else(|| self.sink.out.flush().err());
        if restart {
            self.psi_forced = true;
            self.pcr_forced = true;
        }
        let writer = std::mem::replace(&mut self.sink.out, out);
        let seg = std::mem::replace(&mut self.seg, SegState { start_packets: self.sink.packets, ..SegState::default() });
        let starts_with_keyframe = if self.has_video { seg.first_video_key.unwrap_or(false) } else { seg.frames > 0 };
        Segment {
            writer,
            packets: self.sink.packets - seg.start_packets,
            frames: seg.frames,
            first_pts: seg.first_pts,
            last_pts: seg.last_pts,
            starts_with_keyframe,
            error,
        }
    }

    /// Writes the PAT and PMT before the next frame.
    pub fn repeat_psi(&mut self) {
        self.psi_forced = true;
    }

    /// Starts a new time base, for an encoder or source that restarts with timestamps that do not
    /// follow the old ones. The clock and every stream's previous DTS are forgotten, the next
    /// packet of every stream PID and of the PCR PID carries the discontinuity indicator
    /// (§2.4.3.5), and the next frame or [`Muxer::advance`] writes the PAT, the PMT and a PCR of
    /// the new time base first. Continuity counters carry on. An Opus stream may carry a start
    /// trim again, as at its start.
    pub fn discontinuity(&mut self) {
        self.clock = None;
        self.last_pcr = None;
        self.last_psi = None;
        self.psi_forced = true;
        self.pcr_forced = true;
        self.own_pcr_disc = matches!(self.pcr_on, PcrOn::Own(_));
        for s in &mut self.streams {
            s.last_dts = None;
            s.disc = true;
            // a restarted Opus encoder starts with its own pre-skip
            s.opus_lead_in = true;
            s.opus_start_trimmed = 0;
        }
    }

    /// Writes the held packets, flushes and returns the output. A transport stream has no
    /// trailer.
    pub fn finish(mut self) -> Result<W> {
        self.flush()?;
        Ok(self.sink.out)
    }

    /// The 27 MHz clock value for a DTS.
    fn time(&self, dts: i64) -> Result<i64> {
        dts.checked_sub(i64::from(self.cfg.pcr_delay)).and_then(|v| v.checked_mul(300)).ok_or(Error::Timestamp("the timestamp is out of range"))
    }

    /// Writes the batch unless the configuration holds packets between calls.
    fn end_call(&mut self) -> Result<()> {
        if !self.cfg.hold_packets {
            self.sink.drain()?;
        }
        Ok(())
    }

    /// Moves the clock without a frame, writing the PCRs and the PAT and PMT that fall due. A
    /// live output calls this while no frames arrive, so the PCR stays within its interval.
    ///
    /// `time` is on the DTS scale (90 kHz): the PCR sent is `time - pcr_delay`, the clock a
    /// frame with DTS `time` would set. After `advance(t)`, frames whose DTS is below
    /// `t - pcr_delay` are refused. A time behind the clock has no effect.
    pub fn advance(&mut self, time: i64) -> Result<()> {
        let r = self.advance_inner(time);
        let end = self.end_call();
        r.and(end)
    }

    fn advance_inner(&mut self, time: i64) -> Result<()> {
        let t = self.time(time)?;
        let clock = self.clock.map_or(t, |c| c.max(t));
        self.clock = Some(clock);
        self.fill_pcr(clock)?;
        self.psi_if_due(clock, false)?;
        if self.pcr_due(clock) {
            self.pcr_packet(clock)?;
        }
        Ok(())
    }

    /// A PCR packet is due: none yet, a new writer, or the interval has passed.
    fn pcr_due(&self, clock: i64) -> bool {
        self.pcr_forced || self.last_pcr.is_none_or(|l| clock.saturating_sub(l) >= self.limit())
    }

    /// Writes one frame of stream `index` (an index into the list given to [`Muxer::new`]).
    /// The frame is checked before anything is written.
    pub fn write_frame(&mut self, index: usize, frame: &Frame<'_>) -> Result<()> {
        self.write_frame_with_trim(index, frame, OpusTrim::default())
    }

    /// Writes one Opus frame whose control header carries `trim`. A start trim is allowed only
    /// while every earlier access unit of the stream (since the start or the last
    /// [`Muxer::discontinuity`]) was discarded whole, and the trims of one
    /// access unit cannot exceed its duration. A zero trim writes a plain frame, for any codec.
    pub fn write_frame_with_trim(&mut self, index: usize, frame: &Frame<'_>, trim: OpusTrim) -> Result<()> {
        let r = self.write_inner(index, frame, trim);
        let end = self.end_call();
        r.and(end)
    }

    fn write_inner(&mut self, index: usize, frame: &Frame<'_>, trim: OpusTrim) -> Result<()> {
        let st = self.streams.get(index).ok_or(Error::NoSuchStream(index))?;
        if frame.data.is_empty() {
            return Err(Error::EmptyFrame);
        }
        let codec = st.codec;
        let dts = frame.dts.unwrap_or(frame.pts);
        if dts > frame.pts {
            return Err(Error::Timestamp("the decoding time is after the presentation time"));
        }
        if let Some(l) = st.last_dts {
            if dts < l {
                return Err(Error::Timestamp("the decoding time is before the stream's previous one"));
            }
            if dts == l && codec.is_video() {
                return Err(Error::Timestamp("a video frame's decoding time must be after the stream's previous one"));
            }
        }
        let trimmed = trim != OpusTrim::default();
        let mut duration = 0;
        if trimmed {
            if !codec.is_opus() {
                return Err(Error::BadFrame("only Opus frames carry trims"));
            }
            if trim.start > MAX_OPUS_TRIM || trim.end > MAX_OPUS_TRIM {
                return Err(Error::BadFrame("an Opus trim is at most 8191 samples"));
            }
            if trim.start > 0 && (!st.opus_lead_in || st.opus_start_trimmed + u32::from(trim.start) > 0xFFFF) {
                return Err(Error::BadFrame("a start trim is allowed only at the start of an Opus stream, 65 535 samples in all"));
            }
            duration = pes::opus_samples(frame.data).ok_or(Error::BadFrame("the Opus packet's table of contents is incomplete"))?;
            if u32::from(trim.start) + u32::from(trim.end) > duration {
                return Err(Error::BadFrame("the trim is longer than the Opus access unit"));
            }
        }
        let with_dts = dts != frame.pts;
        let header_len = if with_dts { 19 } else { 14 };
        let trim_bytes = if trim.start > 0 { 2 } else { 0 } + if trim.end > 0 { 2 } else { 0 };
        let limit = pes::data_limit(codec, header_len - 6, trim_bytes);
        if frame.data.len() > limit {
            return Err(Error::FrameTooLarge { size: frame.data.len(), limit });
        }
        let mut prefix = [0u8; pes::PREFIX_MAX];
        let prep = pes::prepare(codec, frame.data, frame.key, trim, &mut prefix)?;
        // start codes, escapes and delimiters count toward the reader's limit too
        let encoded = prep.prefix_len.saturating_add(prep.body_len);
        if codec.is_video() && encoded > MAX_VIDEO_FRAME {
            return Err(Error::FrameTooLarge { size: encoded, limit: MAX_VIDEO_FRAME });
        }
        // bytes after PES_packet_length
        let body = (header_len - 6) + prep.prefix_len + prep.body_len;
        let length = match u16::try_from(body) {
            Ok(n) => n,
            Err(_) if codec.is_video() => 0,
            Err(_) => return Err(Error::FrameTooLarge { size: frame.data.len(), limit }),
        };
        let t = self.time(dts)?;
        // the frame must arrive no later than it decodes: its DTS may not be behind the clock
        // that PCRs already sent (the largest DTS so far minus the PCR delay, or an advance)
        if dts.checked_mul(300).is_none_or(|d| self.clock.is_some_and(|c| d < c)) {
            return Err(Error::Timestamp("the decoding time is behind the transport stream clock"));
        }
        let mut header = [0u8; pes::HEADER_MAX];
        pes::header(&mut header, codec.stream_id(), length, frame.pts, with_dts.then_some(dts));

        let clock = self.clock.map_or(t, |c| c.max(t));
        self.clock = Some(clock);
        self.fill_pcr(clock)?;
        // a keyframe needs the tables right in front of it: not again when they are the last
        // packets written, but again when PCRs filling a gap came after them
        let tables_last = self.psi_end == Some(self.sink.pushed);
        self.psi_if_due(clock, frame.key && codec.is_video() && !tables_last)?;
        let mut pcr = None;
        if self.pcr_on == PcrOn::Stream(index) {
            if self.pcr_forced || self.last_pcr.is_none_or(|l| clock > l) {
                pcr = Some(clock);
                self.last_pcr = Some(clock);
                self.pcr_forced = false;
                self.stats.pcrs += 1;
            }
        } else if self.pcr_due(clock) {
            self.pcr_packet(clock)?;
        }
        let payload = match codec {
            Codec::Av1(_) => Payload::Av1(av1::Escaper::new(frame.data)),
            _ => Payload::Plain(frame.data),
        };
        self.pes_packets(index, &header[..header_len], &prefix[..prep.prefix_len], payload, prep.body_len, pcr, frame.key)?;
        if let Some(st) = self.streams.get_mut(index) {
            st.last_dts = Some(dts);
            if codec.is_opus() {
                st.opus_start_trimmed += u32::from(trim.start);
                if !trimmed || u32::from(trim.start) < duration {
                    st.opus_lead_in = false;
                }
            }
        }
        self.stats.frames += 1;
        let seg = &mut self.seg;
        seg.frames += 1;
        seg.first_pts.get_or_insert(frame.pts);
        seg.last_pts = Some(seg.last_pts.map_or(frame.pts, |p| p.max(frame.pts)));
        if codec.is_video() {
            seg.first_video_key.get_or_insert(frame.key);
        }
        Ok(())
    }

    /// The PCR interval at 27 MHz.
    fn limit(&self) -> i64 {
        i64::from(self.cfg.pcr_interval) * 300
    }

    fn psi_if_due(&mut self, clock: i64, keyframe: bool) -> Result<()> {
        let due = self.psi_forced
            || self.last_psi.is_none_or(|l| clock.saturating_sub(l) >= i64::from(self.cfg.psi_interval) * 300)
            || (keyframe && self.cfg.psi_before_keyframes);
        if due {
            section_packets(&mut self.sink, 0, &mut self.pat_cc, &self.pat)?;
            section_packets(&mut self.sink, self.cfg.pmt_pid, &mut self.pmt_cc, &self.pmt)?;
            self.last_psi = Some(clock);
            self.psi_end = Some(self.sink.pushed);
            self.psi_forced = false;
            self.stats.psi += 1;
        }
        Ok(())
    }

    /// Fills the gap up to `clock`: PCR-only packets until the gap to `clock` is within the PCR
    /// interval, and the PAT and PMT wherever they fall due before `clock`, in time order. A new
    /// writer gets the PAT and PMT before anything else.
    fn fill_pcr(&mut self, clock: i64) -> Result<()> {
        let Some(last_pcr) = self.last_pcr else { return Ok(()) };
        if self.psi_forced {
            self.psi_if_due(last_pcr, false)?;
        }
        let limit = self.limit();
        let psi = i64::from(self.cfg.psi_interval) * 300;
        // each kind of packet is capped on its own, so a short PAT and PMT interval cannot use
        // up the PCRs' share
        let (mut pcrs, mut tables) = (0, 0);
        loop {
            let mut pcr_at = self.last_pcr.map(|l| l.saturating_add(limit)).filter(|&t| t < clock);
            if pcrs >= MAX_PCR_FILL && pcr_at.is_some() {
                if pcrs == MAX_PCR_FILL {
                    // counted once: the PCR at `clock` that follows closes the gap
                    self.stats.pcr_gaps += 1;
                    pcrs += 1;
                }
                pcr_at = None;
            }
            let psi_at = self.last_psi.map(|l| l.saturating_add(psi)).filter(|&t| t < clock && tables < MAX_PCR_FILL);
            match (psi_at, pcr_at) {
                (Some(s), Some(p)) if s <= p => {
                    self.psi_if_due(s, false)?;
                    tables += 1;
                }
                (Some(s), None) => {
                    self.psi_if_due(s, false)?;
                    tables += 1;
                }
                (_, Some(p)) => {
                    self.pcr_packet(p)?;
                    pcrs += 1;
                }
                (None, None) => return Ok(()),
            }
        }
    }

    /// A packet on the PCR PID with only an adaptation field holding the PCR.
    fn pcr_packet(&mut self, pcr: i64) -> Result<()> {
        let (pid, cc, disc) = match self.pcr_on {
            PcrOn::Stream(i) => match self.streams.get_mut(i) {
                Some(s) => (s.pid, s.held_cc(), std::mem::take(&mut s.disc)),
                None => (0x1FFF, 0, false),
            },
            PcrOn::Own(pid) => (pid, 0, std::mem::take(&mut self.own_pcr_disc)),
        };
        let mut b = [0xFF; PACKET_SIZE];
        b[..6].copy_from_slice(&[0x47, (pid >> 8) as u8 & 0x1F, pid as u8, 0x20 | cc, 183, 0x10 | if disc { 0x80 } else { 0 }]);
        b[6..12].copy_from_slice(&pes::pcr(pcr));
        self.sink.push(&b)?;
        self.stats.pcrs += 1;
        self.last_pcr = Some(pcr);
        self.pcr_forced = false;
        Ok(())
    }

    /// Splits one PES packet (`header`, `prefix`, then `body_len` bytes of `payload`) into
    /// transport packets.
    #[allow(clippy::too_many_arguments)]
    fn pes_packets(
        &mut self,
        index: usize,
        header: &[u8],
        prefix: &[u8],
        mut payload: Payload<'_>,
        body_len: usize,
        pcr: Option<i64>,
        key: bool,
    ) -> Result<()> {
        let Some(st) = self.streams.get_mut(index) else { return Err(Error::NoSuchStream(index)) };
        let pid = st.pid;
        let mut parts = [header, prefix];
        let mut left = header.len() + prefix.len() + body_len;
        let mut first = true;
        while left > 0 {
            let pcr_here = if first { pcr } else { None };
            let rai = first && key;
            let disc = first && st.disc;
            let fixed = if pcr_here.is_some() || rai || disc { 2 + if pcr_here.is_some() { 6 } else { 0 } } else { 0 };
            let take = left.min(184 - fixed);
            // the adaptation field, its length byte included
            let af = 184 - take;
            let mut b = [0u8; PACKET_SIZE];
            b[0] = 0x47;
            b[1] = if first { 0x40 } else { 0 } | (pid >> 8) as u8 & 0x1F;
            b[2] = pid as u8;
            b[3] = if af > 0 { 0x30 } else { 0x10 } | st.cc;
            st.cc = (st.cc + 1) & 0x0F;
            if af > 0 {
                b[4] = (af - 1) as u8;
                if af > 1 {
                    b[5] = if disc { 0x80 } else { 0 } | if rai { 0x40 } else { 0 } | if pcr_here.is_some() { 0x10 } else { 0 };
                    let mut p = 6;
                    if let Some(v) = pcr_here {
                        b[6..12].copy_from_slice(&pes::pcr(v));
                        p = 12;
                    }
                    b[p..4 + af].fill(0xFF);
                }
            }
            if disc {
                st.disc = false;
            }
            let mut at = 4 + af;
            for part in &mut parts {
                let n = part.len().min(PACKET_SIZE - at);
                b[at..at + n].copy_from_slice(&part[..n]);
                *part = &part[n..];
                at += n;
            }
            if at < PACKET_SIZE {
                payload.fill(&mut b[at..]);
            }
            self.sink.push(&b)?;
            left -= take;
            first = false;
        }
        Ok(())
    }
}

/// Writes a PSI section on `pid`: a pointer field of 0, the section, and 0xFF stuffing.
fn section_packets<W: Write>(sink: &mut Sink<W>, pid: u16, cc: &mut u8, section: &[u8]) -> Result<()> {
    let mut rest = section;
    let mut first = true;
    while first || !rest.is_empty() {
        let mut b = [0xFF; PACKET_SIZE];
        b[0] = 0x47;
        b[1] = if first { 0x40 } else { 0 } | (pid >> 8) as u8 & 0x1F;
        b[2] = pid as u8;
        b[3] = 0x10 | *cc;
        *cc = (*cc + 1) & 0x0F;
        let mut at = 4;
        if first {
            b[4] = 0;
            at = 5;
        }
        let n = rest.len().min(PACKET_SIZE - at);
        b[at..at + n].copy_from_slice(&rest[..n]);
        rest = &rest[n..];
        sink.push(&b)?;
        first = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

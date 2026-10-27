//! The native decode path: craft-media's demuxers (ISO BMFF / QuickTime, Matroska / WebM, Ogg),
//! raw ADTS, MPEG audio and AC-3 streams, and WAV / AIFF PCM, feeding [`TrackDecoder`].
//! [`Native::open`] answers `None` for a stream with no native codec (MP3, FLAC, ALAC, Vorbis,
//! ...) or a container these demuxers do not read; the caller then uses symphonia.
//!
//! Every container is a [`Packets`] source over a [`Src`], read packet by packet, so a decode
//! streams. Timing follows the containers: MP4 edit lists (AAC priming, Opus pre-skip), Matroska
//! `CodecDelay` and per-block `DiscardPadding`, and Ogg Opus pre-skip and end trimming (RFC 7845
//! §4) are applied, so a file decodes to exactly its presentation length.

use super::aiff::{self, AiffParse};
use super::id3v2_len;
use super::source::{Src, io_error};
use super::track::TrackDecoder;
use super::wav::{self, WavParse};
use crate::{AudioBuffer, AudioError, AudioInfo, CodecId, CodecParams, Container, PcmFormat, Result, aac, ac3, mpa};
use craft_media::{isobmff, matroska, ogg};

/// Corrupt packets tolerated before a stream that has produced nothing is an error.
const MAX_ERRORS: usize = 1000;
/// How far into a raw stream a first frame is looked for.
const MAX_SYNC_SCAN: u64 = 64 * 1024;
/// The most frames a packet may declare (as on the symphonia path): decoders allocate a packet's
/// frames up front.
const MAX_PACKET_FRAMES: u64 = 1 << 20;
/// Frames per packet read from a WAV or AIFF file.
const PCM_PACKET_FRAMES: u64 = 4096;

/// A packet's bytes and the frames to drop from the end of its decoded audio (Matroska
/// `DiscardPadding`).
struct Packet {
    data: Vec<u8>,
    trim_end: u64,
}

/// Where a track's packets come from.
enum Packets<'a> {
    Mp4 {
        file: Box<isobmff::Mp4File>,
        src: Src<'a>,
        track: usize,
        next: usize,
        /// Media timescale and output rate, for packet start times.
        timescale: u32,
        rate: u32,
    },
    /// Matroska is read packet by packet, which carries each block's `DiscardPadding`.
    Mkv {
        demux: Box<matroska::Demuxer<Src<'a>>>,
        track: usize,
        rate: u32,
        /// The track's first packet's timestamp (ffmpeg starts AAC tracks before 0), the decoded
        /// timeline's origin.
        first_ns: i64,
        /// How a packet's place on the decoded timeline is found after a seek.
        frames: MkvFrames,
        src: Src<'a>,
        /// A packet read ahead by a seek, returned next.
        pending: Option<Packet>,
    },
    Ogg {
        file: Box<ogg::OggFile>,
        src: Src<'a>,
        stream: usize,
        next: usize,
        /// Output frame of each packet's first decoded sample (the pre-skip negative).
        starts: Vec<i64>,
    },
    /// Frames of a raw stream: `(offset, length, decoded frames)`.
    Raw { frames: Vec<(u64, u32, u32)>, src: Src<'a>, next: usize },
    /// Interleaved PCM in `start..start + frames * frame_bytes` (WAV, AIFF).
    Pcm { src: Src<'a>, start: u64, frames: u64, frame_bytes: u64, next_frame: u64 },
}

impl Packets<'_> {
    /// The next packet of the track, or `None` at its end.
    fn next(&mut self) -> Option<Result<Packet>> {
        let whole = |data: Vec<u8>| Packet { data, trim_end: 0 };
        match self {
            Packets::Mp4 { file, src, track, next, .. } => {
                let i = *next;
                (i < file.tracks.get(*track).map_or(0, |t| t.samples.len())).then(|| {
                    *next += 1;
                    file.read_sample(src, *track, i).map(whole).map_err(mp4_error)
                })
            }
            Packets::Ogg { file, src, stream, next, .. } => {
                let i = *next;
                (i < file.streams.get(*stream).map_or(0, |s| s.packets.len())).then(|| {
                    *next += 1;
                    file.read_packet(src, *stream, i).map(whole).map_err(ogg_error)
                })
            }
            Packets::Raw { frames, src, next } => {
                let &(at, len, _) = frames.get(*next)?;
                *next += 1;
                Some(src.read(at, len as usize).map(whole))
            }
            Packets::Pcm { src, start, frames, frame_bytes, next_frame } => {
                let n = (*frames - *next_frame).min(PCM_PACKET_FRAMES);
                if n == 0 {
                    return None;
                }
                let at = *start + *next_frame * *frame_bytes;
                *next_frame += n;
                // A truncated file holds fewer bytes than its header says: decode what is there.
                Some(src.read_upto(at, usize::try_from(n * *frame_bytes).unwrap_or(0)).map(whole))
            }
            Packets::Mkv { demux, track, rate, pending, .. } => match pending.take() {
                Some(p) => Some(Ok(p)),
                None => mkv_next(demux, *track, *rate).map(|r| r.map(|(p, _)| p)),
            },
        }
    }

    /// Position the source at the latest packet whose decoded audio starts at or before output
    /// frame `frame` (counted from the track's first decoded sample) and return where that packet
    /// starts.
    fn seek(&mut self, frame: u64) -> Result<u64> {
        // The last index whose start is <= frame, given the starts in order.
        fn at_or_before(starts: impl Iterator<Item = u64>, frame: u64) -> (usize, u64) {
            let mut found = (0, 0);
            for (i, s) in starts.enumerate() {
                if s > frame {
                    break;
                }
                found = (i, s);
            }
            found
        }
        match self {
            Packets::Mp4 { file, track, next, timescale, rate, .. } => {
                let samples = file.tracks.get(*track).map(|t| t.samples.as_slice()).unwrap_or_default();
                let first = samples.first().map_or(0, |s| s.dts);
                let starts = samples.iter().map(|s| rescale(u64::try_from(s.dts - first).unwrap_or(0), u64::from(*timescale), u64::from(*rate)));
                let (i, s) = at_or_before(starts, frame);
                *next = i;
                Ok(s)
            }
            Packets::Ogg { next, starts, .. } => {
                let base = starts.first().copied().unwrap_or(0);
                let (i, s) = at_or_before(starts.iter().map(|&s| u64::try_from(s - base).unwrap_or(0)), frame);
                *next = i;
                Ok(s)
            }
            Packets::Raw { frames, next, .. } => {
                let mut acc = 0u64;
                let starts = frames.iter().map(|&(_, _, n)| {
                    let s = acc;
                    acc += u64::from(n);
                    s
                });
                let (i, s) = at_or_before(starts, frame);
                *next = i;
                Ok(s)
            }
            Packets::Pcm { frames, next_frame, .. } => {
                *next_frame = frame.min(*frames);
                Ok(*next_frame)
            }
            Packets::Mkv { demux, track, rate, first_ns, frames, pending, src } => {
                let ns = i64::try_from(rescale(frame, u64::from(*rate), 1_000_000_000)).unwrap_or(i64::MAX).saturating_add(*first_ns);
                let point = demux.seek(*track, ns).map_err(mkv_error)?;
                match mkv_next(demux, *track, *rate) {
                    Some(Ok((p, pts_ns))) => {
                        *pending = Some(p);
                        // Timestamps are rounded to the file's timestamp scale (often 1 ms), so the
                        // packet's index places it exactly where packets have a known length.
                        let by_index = point.sample.and_then(|k| frames.start(src, k));
                        Ok(by_index.unwrap_or_else(|| {
                            rescale_round(u64::try_from(pts_ns.saturating_sub(*first_ns)).unwrap_or(0), 1_000_000_000, u64::from(*rate))
                        }))
                    }
                    Some(Err(e)) => Err(e),
                    None => Ok(frame),
                }
            }
        }
    }
}

/// How the decoded-timeline start of a Matroska packet is found from its index.
enum MkvFrames {
    /// Every packet decodes to this many frames.
    Constant(u64),
    /// PCM: the stored sizes of the track's packets, and the bytes per frame.
    Pcm { sizes: Vec<u32>, frame_bytes: u64 },
    /// Opus: each packet's length is in its first bytes (RFC 6716 §3.1); the packets' places and
    /// sizes, and their starts once read.
    Opus { samples: Vec<(u64, u32)>, starts: Option<Vec<u64>> },
    /// From the timestamps.
    Timestamps,
}

impl MkvFrames {
    /// The decoded-timeline start of packet `k`, when the packets say.
    fn start(&mut self, src: &Src<'_>, k: usize) -> Option<u64> {
        match self {
            MkvFrames::Constant(n) => Some(k as u64 * *n),
            MkvFrames::Pcm { sizes, frame_bytes } => Some(sizes.iter().take(k).map(|&s| u64::from(s)).sum::<u64>() / *frame_bytes),
            MkvFrames::Opus { samples, starts } => {
                let starts = starts.get_or_insert_with(|| {
                    let mut acc = 0u64;
                    samples
                        .iter()
                        .map(|&(at, size)| {
                            let s = acc;
                            let prefix = src.bytes(at, 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).unwrap_or([0, 0]);
                            acc += u64::from(ogg::opus_packet_samples(prefix, size).unwrap_or(0));
                            s
                        })
                        .collect()
                });
                starts.get(k).copied()
            }
            MkvFrames::Timestamps => None,
        }
    }
}

/// The next packet of Matroska track `track` and its timestamp in nanoseconds.
fn mkv_next(demux: &mut matroska::Demuxer<Src<'_>>, track: usize, rate: u32) -> Option<Result<(Packet, i64)>> {
    loop {
        match demux.next_packet() {
            Ok(Some(p)) if p.track == track => {
                let pad = p.discard_padding_ns.and_then(|ns| u64::try_from(ns).ok()).unwrap_or(0);
                let packet = Packet { data: p.data, trim_end: rescale_round(pad, 1_000_000_000, u64::from(rate)) };
                return Some(Ok((packet, p.pts_ns)));
            }
            Ok(Some(_)) => continue,
            Ok(None) => return None,
            Err(e) => return Some(Err(mkv_error(e))),
        }
    }
}

/// Container read errors: the source failing is [`AudioError::Io`], anything else in the file is
/// [`AudioError::Malformed`].
fn mp4_error(e: isobmff::Error) -> AudioError {
    match e {
        isobmff::Error::Io(e) => io_error(e),
        e => AudioError::Malformed(e.to_string()),
    }
}

fn mkv_error(e: matroska::Error) -> AudioError {
    match e {
        matroska::Error::Io(e) => io_error(e),
        e => AudioError::Malformed(e.to_string()),
    }
}

fn ogg_error(e: ogg::Error) -> AudioError {
    match e {
        ogg::Error::Io(e) => io_error(e),
        e => AudioError::Malformed(e.to_string()),
    }
}

/// A track the native path decodes: its packets, its decoder and its container timing.
pub(crate) struct Native<'a> {
    packets: Packets<'a>,
    dec: TrackDecoder,
    info: AudioInfo,
    /// Whether `info.frames` came from the container (else it is 0 until decoded).
    frames_known: bool,
    /// Decoded frames at the start that are not presented (AAC priming, Opus pre-skip).
    skip: u64,
    /// Presented frames, when the container says.
    limit: Option<u64>,
    /// Decoded-timeline frame (from the first decoded sample) of the next decoded sample.
    pos: u64,
    /// The first decoded-timeline frame to present: `skip`, or a seek target.
    from: u64,
    errors: usize,
    decoded: bool,
    /// The first error that said the stream is unsupported, returned if nothing decodes.
    unsupported: Option<AudioError>,
}

impl<'a> Native<'a> {
    /// Open the audio track `track` (the first when `None`) of the file in `src`; `None` when the
    /// native path does not decode it.
    pub(crate) fn open(src: &Src<'a>, detected: Option<Container>, track: Option<usize>) -> Option<Result<Native<'a>>> {
        let head = src.head();
        let at = after_id3(&head);
        let body = head.get(at..).unwrap_or_default();
        // The bytes decide the container when they say; the detection (which may come from the
        // extension) only picks among raw streams.
        match super::detect_container(&head, None).or(detected) {
            Some(Container::Wav) if wav::is_wav(&head) => open_wav(src),
            Some(Container::Aiff) if aiff::is_aiff(&head) => open_aiff(src),
            Some(Container::Ogg) if ogg::sniff(body) => open_ogg(src, track),
            Some(c @ (Container::Mp4 | Container::QuickTime)) if head.get(4..8) == Some(b"ftyp") => open_mp4(src, c, track),
            Some(c @ (Container::Matroska | Container::WebM)) if body.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) => open_mkv(src, c, track),
            Some(Container::Adts) => open_adts(src).map(Ok),
            Some(Container::MpegAudio) => open_mpa(src).map(Ok),
            Some(Container::Ac3) => open_ac3(src).map(Ok),
            None => open_ac3(src).or_else(|| open_adts(src)).or_else(|| open_mpa(src)).map(Ok),
            _ => None,
        }
    }

    /// What the stream is; `frames` is 0 when the container does not say (see `frames_known`).
    pub(crate) fn info(&self) -> &AudioInfo {
        &self.info
    }

    pub(crate) fn frames_known(&self) -> bool {
        self.frames_known
    }

    /// The next block of presented audio, or `None` at the end. A packet that is damaged or cannot
    /// be decoded is replaced by the decoder's concealment (silence of its length), so the audio
    /// after it keeps its timing; a stream where nothing decodes is an error. A failing source is
    /// returned as [`AudioError::Io`] and skips that packet.
    pub(crate) fn next_block(&mut self) -> Result<Option<AudioBuffer>> {
        loop {
            if self.limit.is_some_and(|l| self.pos >= l.saturating_add(self.skip)) {
                return Ok(None);
            }
            let Some(next) = self.packets.next() else {
                // A track that declares no frames (an empty WAV or AIFF) is empty, not broken.
                let empty = self.frames_known && self.info.frames == 0 && self.errors == 0;
                if !self.decoded && !empty {
                    return Err(self.no_audio("stream contains no audio"));
                }
                return Ok(None);
            };
            let (block, trim_end) = match next {
                Ok(p) => match self.dec.decode(&p.data) {
                    Ok(block) => {
                        if block.channels.is_empty() {
                            continue;
                        }
                        self.decoded = true;
                        (block, p.trim_end)
                    }
                    Err(e) => {
                        self.error(e)?;
                        (self.dec.conceal(Some(&p.data)), p.trim_end)
                    }
                },
                // The source failing is not damage in the file: the caller decides whether to retry.
                Err(e @ AudioError::Io(_)) => return Err(e),
                Err(e) => {
                    self.error(e)?;
                    (self.dec.conceal(None), 0)
                }
            };
            if let Some(out) = self.present(block, trim_end) {
                return Ok(Some(out));
            }
        }
    }

    /// The part of a decoded `block` (less its last `trim_end` frames) that is presented.
    fn present(&mut self, mut block: AudioBuffer, trim_end: u64) -> Option<AudioBuffer> {
        let n = (block.frames() as u64).saturating_sub(trim_end);
        let start = self.pos;
        self.pos += n;
        let end = match self.limit {
            Some(l) => self.pos.min(l.saturating_add(self.skip)),
            None => self.pos,
        };
        let first = start.max(self.from);
        if first >= end {
            return None;
        }
        let (a, b) = ((first - start) as usize, (end - start) as usize);
        for c in &mut block.channels {
            c.truncate(b);
            c.drain(..a);
            for s in c.iter_mut().filter(|s| !s.is_finite()) {
                *s = 0.0;
            }
        }
        Some(block)
    }

    /// Note a packet that could not be read or decoded. A stream where nothing decodes is an
    /// error: unsupported when a packet said so, else malformed.
    fn error(&mut self, e: AudioError) -> Result<()> {
        self.errors += 1;
        self.info.damaged += 1;
        log::debug!("concealing a packet that failed: {e}");
        let msg = e.to_string();
        if matches!(e, AudioError::Unsupported(_)) && self.unsupported.is_none() {
            self.unsupported = Some(e);
        }
        if self.errors > MAX_ERRORS && !self.decoded {
            return Err(self.no_audio(&msg));
        }
        Ok(())
    }

    fn no_audio(&mut self, msg: &str) -> AudioError {
        self.unsupported.take().unwrap_or_else(|| AudioError::Malformed(format!("no decodable packets: {msg}")))
    }

    /// Continue from presented frame `frame`: back up the decoder's pre-roll, decode from the
    /// packet before it and drop what comes before `frame`. Returns the frame decoding resumes at
    /// (`frame`, or the end of the stream).
    pub(crate) fn seek(&mut self, frame: u64) -> Result<u64> {
        let frame = match self.limit {
            Some(l) => frame.min(l),
            None => frame,
        };
        let target = frame + self.skip;
        let start = self.packets.seek(target.saturating_sub(self.dec.preroll() as u64))?;
        self.dec.reset();
        self.pos = start;
        self.from = target;
        Ok(frame)
    }
}

/// Where a stream starts: after any ID3v2 tags.
fn after_id3(b: &[u8]) -> usize {
    let mut at = 0;
    while let Some(len) = b.get(at..).and_then(id3v2_len) {
        at = at.saturating_add(len);
    }
    at
}

/// A native track from its parts; `None` when [`TrackDecoder`] does not decode the codec (it is
/// then left to symphonia).
fn native<'a>(
    container: Container,
    packets: Packets<'a>,
    params: &CodecParams,
    lookahead: &[Vec<u8>],
    skip: u64,
    limit: Option<u64>,
    frames: Option<u64>,
) -> Option<Native<'a>> {
    let lookahead: Vec<&[u8]> = lookahead.iter().map(Vec::as_slice).collect();
    let dec = TrackDecoder::new(params, &lookahead).ok()?;
    let mut info = AudioInfo::new(container, dec.codec(), dec.sample_format(), dec.sample_rate(), dec.layout(), frames.unwrap_or(0));
    if dec.channels() == 0 || dec.sample_rate() == 0 {
        info.frames = 0;
    }
    Some(Native {
        packets,
        info,
        frames_known: frames.is_some() && dec.channels() > 0 && dec.sample_rate() > 0,
        dec,
        skip,
        limit,
        pos: 0,
        from: skip,
        errors: 0,
        decoded: false,
        unsupported: None,
    })
}

fn open_wav<'a>(src: &Src<'a>) -> Option<Result<Native<'a>>> {
    let layout = match wav::parse(src) {
        Ok(WavParse::Native(l)) => l,
        Ok(WavParse::Foreign { .. }) => return None,
        Err(e) => return Some(Err(e)),
    };
    let mut n = pcm_track(src, Container::Wav, layout.info.clone(), layout.kind, layout.data_start, layout.data_len);
    if let Some(Ok(n)) = &mut n {
        n.info.bwf = layout.info.bwf;
    }
    n
}

fn open_aiff<'a>(src: &Src<'a>) -> Option<Result<Native<'a>>> {
    match aiff::parse(src) {
        Ok(AiffParse::Native(l)) => pcm_track(src, Container::Aiff, l.info, l.kind, l.data_start, l.data_len),
        Ok(AiffParse::Foreign) => None,
        Err(e) => Some(Err(e)),
    }
}

/// A WAV or AIFF file's samples as a PCM track.
fn pcm_track<'a>(
    src: &Src<'a>,
    container: Container,
    info: AudioInfo,
    kind: super::pcm::PcmKind,
    start: u64,
    len: u64,
) -> Option<Result<Native<'a>>> {
    let channels = usize::from(info.channels);
    let frame_bytes = (kind.bytes() * channels) as u64;
    let (bits, float, big_endian, unsigned) = match kind {
        super::pcm::PcmKind::Int { bytes, big_endian, unsigned } => ((bytes * 8) as u16, false, big_endian, unsigned),
        super::pcm::PcmKind::Float { bytes, big_endian } => ((bytes * 8) as u16, true, big_endian, false),
    };
    let params = CodecParams::new(CodecId::Pcm)
        .with_sample_rate(info.sample_rate)
        .with_channels(info.channels)
        .with_pcm(PcmFormat::new(bits, float, big_endian, !unsigned));
    let frames = len.checked_div(frame_bytes).unwrap_or(0).min(info.frames);
    let packets = Packets::Pcm { src: src.clone(), start, frames, frame_bytes, next_frame: 0 };
    let mut n = native(container, packets, &params, &[], 0, None, Some(frames))?;
    // The file's own description wins: its layout (an extensible mask) and stored sample format.
    n.info = AudioInfo { frames, ..info };
    Some(Ok(n))
}

/// Reads a raw stream in windows, for [`raw_frames`].
struct Scanner<'s, 'a, F> {
    src: &'s Src<'a>,
    total: u64,
    frame_len: F,
    /// The window's start and bytes.
    window: (u64, Vec<u8>),
}

impl<F: Fn(&[u8]) -> Option<(usize, u32)>> Scanner<'_, '_, F> {
    const WINDOW: usize = 1 << 16;
    /// The most header bytes a frame-length function reads.
    const HEADER: u64 = 16;

    /// The length and decoded frames of the frame at `p`, if one starts there and fits.
    fn len_at(&mut self, p: u64) -> Option<(usize, u32)> {
        if p >= self.total {
            return None;
        }
        if p < self.window.0 || p + Self::HEADER > self.window.0 + self.window.1.len() as u64 {
            self.window = (p, self.src.bytes_upto(p, Self::WINDOW));
        }
        let off = usize::try_from(p - self.window.0).ok()?;
        let (n, frames) = self.window.1.get(off..).and_then(&self.frame_len)?;
        (n > 0 && p + n as u64 <= self.total).then_some((n, frames))
    }

    /// Whether a frame at `p` is followed by another (or by the end of the stream).
    fn paired(&mut self, p: u64) -> bool {
        match self.len_at(p) {
            Some((n, _)) => p + n as u64 == self.total || self.len_at(p + n as u64).is_some(),
            None => false,
        }
    }
}

/// Frame lengths of a raw stream of self-delimiting frames from `at`: `frame_len(rest)` gives the
/// length and decoded frames of a frame starting at `rest`, or `None`. The first frame must be
/// followed by a second (or the end of the data), so a stray sync word does not make a stream;
/// damage later is skipped by scanning for the next pair. Reads the stream in windows.
fn raw_frames(src: &Src<'_>, at: u64, frame_len: impl Fn(&[u8]) -> Option<(usize, u32)>) -> Option<Vec<(u64, u32, u32)>> {
    let total = src.size();
    let mut sc = Scanner { src, total, frame_len, window: (u64::MAX, Vec::new()) };
    let first = (at..total.min(at.saturating_add(MAX_SYNC_SCAN))).find(|&p| sc.paired(p))?;
    let mut frames = Vec::new();
    let mut p = first;
    while p < total {
        match sc.len_at(p) {
            Some((n, f)) => {
                frames.push((p, n as u32, f));
                p += n as u64;
            }
            None => match (p + 1..total).find(|&q| sc.paired(q)) {
                Some(q) => p = q,
                None => break,
            },
        }
    }
    Some(frames)
}

/// The first `n` frames of a raw stream.
fn raw_lookahead(src: &Src<'_>, frames: &[(u64, u32, u32)], n: usize) -> Vec<Vec<u8>> {
    frames.iter().take(n).filter_map(|&(at, len, _)| src.bytes(at, len as usize)).collect()
}

/// A raw ADTS AAC stream: its frames' payloads (without the ADTS header) are the packets.
fn open_adts<'a>(src: &Src<'a>) -> Option<Native<'a>> {
    let head = src.head();
    let frames = raw_frames(src, after_id3(&head) as u64, |rest| {
        aac::AdtsHeader::parse(rest).ok().filter(|h| h.frame_length > h.header_length).map(|h| (h.frame_length, aac::FRAME_LEN as u32))
    })?;
    let &(at, _, _) = frames.first()?;
    let h = aac::AdtsHeader::parse(&src.bytes_upto(at, 16)).ok()?;
    let core = h.to_asc().sample_rate;
    let params = CodecParams::new(CodecId::Aac).with_extradata(h.to_asc().to_bytes()).with_sample_rate(core);
    // The payloads: each frame less its own header.
    let payloads: Vec<(u64, u32, u32)> = frames
        .iter()
        .filter_map(|&(p, n, f)| {
            let hl = aac::AdtsHeader::parse(&src.bytes_upto(p, 16)).ok()?.header_length as u32;
            (n > hl).then_some((p + u64::from(hl), n - hl, f))
        })
        .collect();
    let lookahead = raw_lookahead(src, &payloads, super::track::LOOKAHEAD_PACKETS);
    let mut n = native(Container::Adts, Packets::Raw { frames: payloads, src: src.clone(), next: 0 }, &params, &lookahead, 0, None, None)?;
    // HE-AAC decodes twice the core frame length.
    let per_frame = rescale(aac::FRAME_LEN as u64, u64::from(core), u64::from(n.info.sample_rate));
    if let Packets::Raw { frames, .. } = &mut n.packets {
        for f in frames.iter_mut() {
            f.2 = per_frame as u32;
        }
        n.info.frames = frames.len() as u64 * per_frame;
        n.frames_known = true;
    }
    Some(n)
}

/// A raw MPEG audio layer I or II stream.
fn open_mpa<'a>(src: &Src<'a>) -> Option<Native<'a>> {
    let head = src.head();
    let frames = raw_frames(src, after_id3(&head) as u64, |rest| {
        mpa::parse_header(rest).ok().filter(|h| h.layer == 1 || h.layer == 2).map(|h| (h.frame_bytes, h.samples() as u32))
    })?;
    let total = frames.iter().map(|&(_, _, f)| u64::from(f)).sum();
    let lookahead = raw_lookahead(src, &frames, 1);
    native(
        Container::MpegAudio,
        Packets::Raw { frames, src: src.clone(), next: 0 },
        &CodecParams::new(CodecId::Mp2),
        &lookahead,
        0,
        None,
        Some(total),
    )
}

/// A raw AC-3 or E-AC-3 stream; E-AC-3 dependent substreams decode to nothing and count no frames.
fn open_ac3<'a>(src: &Src<'a>) -> Option<Native<'a>> {
    let head = src.head();
    let frames = raw_frames(src, after_id3(&head) as u64, |rest| {
        ac3::parse_header(rest).ok().map(|h| (h.frame_bytes, if h.is_primary() { h.samples() as u32 } else { 0 }))
    })?;
    let total = frames.iter().map(|&(_, _, f)| u64::from(f)).sum();
    let lookahead = raw_lookahead(src, &frames, 1);
    native(Container::Ac3, Packets::Raw { frames, src: src.clone(), next: 0 }, &CodecParams::new(CodecId::Ac3), &lookahead, 0, None, Some(total))
}

/// The `n`-th audio track (the first when `None`) among `kinds`, as an index into them.
fn audio_track(kinds: impl Iterator<Item = bool>, n: Option<usize>) -> Option<usize> {
    kinds.enumerate().filter(|&(_, audio)| audio).map(|(i, _)| i).nth(n.unwrap_or(0))
}

/// An MP4 / QuickTime file's audio track, if its codec is native. Samples that claim to lie
/// beyond the end of the file are refused rather than read leniently.
fn open_mp4<'a>(src: &Src<'a>, container: Container, track: Option<usize>) -> Option<Result<Native<'a>>> {
    let file = isobmff::open(src).ok()?;
    let track = audio_track(file.tracks.iter().map(|t| t.kind == isobmff::TrackKind::Audio), track)?;
    let t = file.tracks.get(track)?;
    // A track with no samples (or sample sizes the demuxer refused) goes to the other path, whose
    // size checks name what is wrong.
    if t.samples.is_empty() {
        return None;
    }
    if t.audio().is_some_and(|a| u64::from(a.samples_per_packet.max(a.const_frames_per_packet)) > MAX_PACKET_FRAMES) {
        return Some(Err(AudioError::Malformed("the file declares packets far longer than any codec uses".into())));
    }
    if t.samples.iter().any(|s| s.offset.checked_add(u64::from(s.size)).is_none_or(|end| end > src.size())) {
        return Some(Err(AudioError::Malformed("MP4 sample is larger than the file".into())));
    }
    let params = t.codec_params();
    let lookahead: Vec<Vec<u8>> =
        (0..t.samples.len().min(super::track::LOOKAHEAD_PACKETS)).filter_map(|i| file.read_sample(src, track, i).ok()).collect();
    let timescale = t.timescale;
    // Probe the decoder for the output rate before the timing is worked out.
    let rate = TrackDecoder::new(&params, &lookahead.iter().map(Vec::as_slice).collect::<Vec<_>>()).ok()?.sample_rate();
    // A leading media edit (AAC priming, Opus pre-skip) is skipped, and the edit's duration is the
    // presentation length. Empty edits and multi-segment lists play the media as it is.
    let (skip, limit) = match t.edits.as_slice() {
        [e] if e.media_time >= 0 && e.media_rate == 0x0001_0000 => {
            let skip = rescale(e.media_time.unsigned_abs(), u64::from(timescale), u64::from(rate));
            let limit = (e.segment_duration > 0).then(|| rescale(e.segment_duration, u64::from(file.timescale), u64::from(rate)));
            (skip, limit)
        }
        _ => (0, None),
    };
    let media: u64 = t.samples.iter().map(|s| u64::from(s.duration)).sum();
    let frames = limit.unwrap_or_else(|| rescale(media, u64::from(timescale), u64::from(rate)).saturating_sub(skip));
    let packets = Packets::Mp4 { file: Box::new(file), src: src.clone(), track, next: 0, timescale, rate };
    native(container, packets, &params, &lookahead, skip, limit, Some(frames)).map(Ok)
}

/// A Matroska / WebM file's audio track, if its codec is native.
fn open_mkv<'a>(src: &Src<'a>, container: Container, track: Option<usize>) -> Option<Result<Native<'a>>> {
    let demux = matroska::Demuxer::new(src.clone()).ok()?;
    let file = demux.file();
    let track = audio_track(file.tracks.iter().map(|t| t.kind == matroska::TrackKind::Audio), track)?;
    let t = file.tracks.get(track)?;
    if !t.frames_readable() {
        return None;
    }
    let params = t.codec_params();
    let lookahead: Vec<Vec<u8>> =
        (0..t.samples.len().min(super::track::LOOKAHEAD_PACKETS)).filter_map(|i| demux.read_sample(track, i).ok()).collect();
    let probe = TrackDecoder::new(&params, &lookahead.iter().map(Vec::as_slice).collect::<Vec<_>>()).ok()?;
    let rate = probe.sample_rate();
    let frames_of = match (probe.frames_per_packet(), params.pcm) {
        (Some(n), _) => MkvFrames::Constant(n as u64),
        (None, Some(pcm)) => {
            let frame_bytes = u64::from(pcm.bits.div_ceil(8)) * u64::from(params.channels.unwrap_or(1).max(1));
            MkvFrames::Pcm { sizes: t.samples.iter().map(|s| s.size).collect(), frame_bytes: frame_bytes.max(1) }
        }
        _ if params.codec == CodecId::Opus => MkvFrames::Opus { samples: t.samples.iter().map(|s| (s.offset, s.size)).collect(), starts: None },
        _ => MkvFrames::Timestamps,
    };
    // `CodecDelay` (AAC priming, Opus pre-skip) is decoded audio that is not presented.
    let skip = rescale_round(t.codec_delay_ns, 1_000_000_000, u64::from(rate));
    // The presentation ends with the last frame (timestamps, with `CodecDelay` already taken off,
    // are in ticks of `timebase` seconds); known only when the last frame has a duration.
    let (num, den) = t.timebase;
    let last = t.samples.iter().max_by_key(|s| s.pts);
    let frames = last.filter(|s| s.duration > 0).and_then(|s| {
        let end = u64::try_from(s.pts.saturating_add(i64::try_from(s.duration).ok()?)).ok()?;
        Some(rescale_round(end.saturating_mul(num), den, u64::from(rate)))
    });
    let mut demux = demux;
    // The first packet's own timestamp is the decoded timeline's origin; read it and rewind.
    let first = mkv_next(&mut demux, track, rate);
    let first_ns = match first {
        Some(Ok((_, pts_ns))) => pts_ns,
        _ => 0,
    };
    demux.seek(track, i64::MIN).ok()?;
    let packets = Packets::Mkv { demux: Box::new(demux), track, rate, first_ns, frames: frames_of, src: src.clone(), pending: None };
    native(container, packets, &params, &lookahead, skip, frames, frames).map(Ok)
}

/// An Ogg file's Opus stream (RFC 7845: pre-skip and end trimming applied).
fn open_ogg<'a>(src: &Src<'a>, track: Option<usize>) -> Option<Result<Native<'a>>> {
    let file = ogg::open(src).ok()?;
    let stream = audio_track(file.streams.iter().map(|s| s.codec != ogg::Codec::Unknown), track)?;
    let s = file.streams.get(stream)?;
    if s.codec != ogg::Codec::Opus {
        return None;
    }
    let params = s.codec_params();
    let head = crate::opus::OpusHead::parse(&params.extradata).ok()?;
    let timing = ogg::OpusTiming::of(s, u32::from(head.pre_skip));
    let limit = u64::try_from(timing.total).ok();
    let skip = u64::from(timing.pre_skip);
    let starts = timing.starts.clone();
    let packets = Packets::Ogg { file: Box::new(file), src: src.clone(), stream, next: 0, starts };
    native(Container::Ogg, packets, &params, &[], skip, limit, limit).map(Ok)
}

/// The audio tracks of `src` the native path reads, described for a track list; `None` when it
/// does not read the container.
pub(crate) fn tracks(src: &Src<'_>, detected: Option<Container>) -> Option<Vec<CodecParams>> {
    let head = src.head();
    match super::detect_container(&head, None).or(detected) {
        Some(Container::Mp4 | Container::QuickTime) => {
            let file = isobmff::open(src).ok()?;
            Some(file.tracks.iter().filter(|t| t.kind == isobmff::TrackKind::Audio).map(isobmff::Track::codec_params).collect())
        }
        Some(Container::Matroska | Container::WebM) => {
            let file = matroska::open(src.clone()).ok()?;
            Some(file.tracks.iter().filter(|t| t.kind == matroska::TrackKind::Audio).map(matroska::Track::codec_params).collect())
        }
        Some(Container::Ogg) => {
            let file = ogg::open(src).ok()?;
            Some(file.streams.iter().filter(|s| s.codec != ogg::Codec::Unknown).map(ogg::Stream::codec_params).collect())
        }
        _ => None,
    }
}

/// `v` in units of `from` per second, in units of `to` per second (truncated).
fn rescale(v: u64, from: u64, to: u64) -> u64 {
    if from == 0 { 0 } else { u64::try_from(u128::from(v) * u128::from(to) / u128::from(from)).unwrap_or(u64::MAX) }
}

/// [`rescale`], rounded to the nearest unit: for durations stored in a coarser unit than samples
/// (Matroska's nanoseconds hold 1024 samples at 48 kHz as 21 333 333 ns).
fn rescale_round(v: u64, from: u64, to: u64) -> u64 {
    if from == 0 { 0 } else { u64::try_from((u128::from(v) * u128::from(to) + u128::from(from) / 2) / u128::from(from)).unwrap_or(u64::MAX) }
}

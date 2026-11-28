//! The native decode path: craft-media's demuxers (ISO BMFF / QuickTime, Matroska / WebM), raw
//! ADTS, AC-3 and MPEG audio streams, and WAV / AIFF PCM, feeding [`TrackDecoder`].
//! [`Native::open`] answers `None` for a stream with no native codec (FLAC, Vorbis in Matroska,
//! ...) or a container these demuxers do not read; the caller then uses symphonia.
//!
//! Every container is a [`Packets`] source over a [`Src`], read packet by packet, so a decode
//! streams. Timing follows the containers: MP4 edit lists (AAC priming, Opus pre-skip), Matroska
//! `CodecDelay` and per-block `DiscardPadding`, Ogg Opus pre-skip and end trimming (RFC 7845 §4)
//! and Ogg Vorbis start and end trimming (Vorbis I §A.2) are applied, and so are an MP3's encoder
//! delay and padding from its LAME tag, so a file decodes to exactly its presentation length.

use super::aiff::{self, AiffParse};
use super::id3v2_len;
use super::source::{Src, io_error};
use super::wav::{self, WavParse};
use crate::track::{TrackDecoder, adts_format_of, adts_frame_count, adts_stream_header};
use crate::{AudioBuffer, AudioError, AudioInfo, CodecId, CodecParams, Container, PcmFormat, Result, aac, ac3, mpa};
use craft_media::{isobmff, matroska, ogg};

/// Corrupt packets tolerated before a stream that has produced nothing is an error.
const MAX_ERRORS: usize = 1000;
/// ID3v2 tags skipped at the start of a stream; real files have one.
const MAX_ID3_TAGS: usize = 16;
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
    /// The packet follows an Ogg granule position jump that [`bound_gaps`] took out of the
    /// timeline: reading it counts one damaged packet.
    jumped: bool,
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
        /// Frames every packet decodes to, when the codec says: then a packet's place on the
        /// decoded timeline is its index, whatever the sample table's durations claim.
        per_packet: Option<u64>,
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
        /// Vorbis, whose first packet after a seek decodes to nothing (its blocks overlap): whether
        /// each packet can prime the decoder, which a packet whose mode can be read does. A seek
        /// starts at the latest such packet before the target's, so the packets between are
        /// concealed at the block sizes the timeline gave them. `None` for Opus.
        primers: Option<Vec<bool>>,
        /// The packets that follow a granule position jump [`bound_gaps`] took out.
        jumps: Vec<bool>,
    },
    /// Frames of a raw stream: `(offset, length, decoded frames)`.
    Raw { frames: Vec<(u64, u32, u32)>, src: Src<'a>, next: usize },
    /// Interleaved PCM in `start..start + frames * frame_bytes` (WAV, AIFF).
    Pcm { src: Src<'a>, start: u64, frames: u64, frame_bytes: u64, next_frame: u64 },
}

impl Packets<'_> {
    /// The next packet of the track, or `None` at its end.
    fn next(&mut self) -> Option<Result<Packet>> {
        let whole = |data: Vec<u8>| Packet { data, trim_end: 0, jumped: false };
        match self {
            Packets::Mp4 { file, src, track, next, .. } => {
                let i = *next;
                (i < file.tracks.get(*track).map_or(0, |t| t.samples.len())).then(|| {
                    *next += 1;
                    file.read_sample(src, *track, i).map(whole).map_err(mp4_error)
                })
            }
            Packets::Ogg { file, src, stream, next, jumps, .. } => {
                let i = *next;
                let jumped = jumps.get(i).copied().unwrap_or(false);
                (i < file.streams.get(*stream).map_or(0, |s| s.packets.len())).then(|| {
                    *next += 1;
                    file.read_packet(src, *stream, i).map(|data| Packet { jumped, ..whole(data) }).map_err(ogg_error)
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

    /// Where the next packet's decoded audio starts on the decoded timeline, for Ogg: a packet
    /// after a lost page is placed by its page's granule position, after a gap. `None` for the
    /// other containers and at the end.
    fn next_start(&self) -> Option<u64> {
        match self {
            Packets::Ogg { next, starts, .. } => {
                let base = starts.first().copied()?;
                starts.get(*next).map(|&s| u64::try_from(s.saturating_sub(base)).unwrap_or(0))
            }
            _ => None,
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
            Packets::Mp4 { file, track, next, timescale, rate, per_packet, .. } => {
                let samples = file.tracks.get(*track).map(|t| t.samples.as_slice()).unwrap_or_default();
                if let Some(n) = per_packet.filter(|&n| n > 0) {
                    let i = (frame / n).min((samples.len() as u64).saturating_sub(1));
                    *next = i as usize;
                    return Ok(i * n);
                }
                let first = samples.first().map_or(0, |s| s.dts);
                let starts =
                    samples.iter().map(|s| rescale(u64::try_from(s.dts.saturating_sub(first)).unwrap_or(0), u64::from(*timescale), u64::from(*rate)));
                let (i, s) = at_or_before(starts, frame);
                *next = i;
                Ok(s)
            }
            Packets::Ogg { next, starts, primers, .. } => {
                let base = starts.first().copied().unwrap_or(0);
                let start = |k: usize| starts.get(k).map_or(0, |&s| u64::try_from(s.saturating_sub(base)).unwrap_or(0));
                let (i, s) = at_or_before((0..starts.len()).map(start), frame);
                let Some(primers) = primers.as_ref().filter(|_| i > 0) else {
                    *next = i;
                    return Ok(s);
                };
                // The packet before primes the decoder. A packet whose mode cannot be read is
                // concealed at the previous block's size, which a reset forgets, so the seek backs
                // up to a packet whose mode can be read, or to the first packet (which has no block
                // before it, as after a reset). Decoding then resumes at the packet after it.
                let p = (0..i).rev().find(|&k| primers.get(k).copied().unwrap_or(false)).unwrap_or(0);
                *next = p;
                Ok(start(p + 1))
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
                        let timed = rescale_round(u64::try_from(pts_ns.saturating_sub(*first_ns)).unwrap_or(0), 1_000_000_000, u64::from(*rate));
                        let start = point.sample.and_then(|k| frames.start(src, k, timed, |i| demux.read_sample(*track, i).ok()));
                        Ok(start.unwrap_or(timed))
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
    /// Every packet decodes to `frames` frames, except where a packet's timestamp is more than
    /// `slack` frames (the timestamps' rounding) from that place: after a short packet. Then the
    /// packets before it are counted with `count` (reading each packet's frames from its header,
    /// into `starts`, the starts of the packets counted so far), or without one the timestamp
    /// places it.
    Constant { frames: u64, slack: u64, count: Option<crate::track::PacketFrames>, starts: Vec<u64> },
    /// PCM: the stored sizes of the track's packets, and the bytes per frame.
    Pcm { sizes: Vec<u32>, frame_bytes: u64 },
    /// E-AC-3, whose packets can differ in length: each packet's length from the headers of its
    /// primary syncframes. The packets' places and stored sizes, the bytes header stripping took
    /// off each one, and the starts of the packets read so far.
    Eac3 { samples: Vec<(u64, u32)>, prefix: Vec<u8>, starts: Vec<u64> },
    /// Opus: each packet's length is in its first bytes (RFC 6716 §3.1); the packets' places and
    /// sizes, and their starts once read.
    Opus { samples: Vec<(u64, u32)>, starts: Option<Vec<u64>> },
    /// From the timestamps.
    Timestamps,
}

impl MkvFrames {
    /// The decoded-timeline start of packet `k`, when the packets say (`src`: the file, for
    /// packet headers; `timed`: the start its timestamp gives; `read`: packet `i`'s data).
    fn start(&mut self, src: &Src<'_>, k: usize, timed: u64, mut read: impl FnMut(usize) -> Option<Vec<u8>>) -> Option<u64> {
        match self {
            MkvFrames::Constant { frames, slack, count, starts } => {
                let even = (k as u64).saturating_mul(*frames);
                if even.abs_diff(timed) <= *slack {
                    return Some(even);
                }
                let count = (*count)?;
                while starts.len() <= k {
                    let last = starts.last().copied().unwrap_or(0);
                    let n = read(starts.len() - 1).and_then(|p| count(&p, *frames)).unwrap_or(*frames);
                    starts.push(last.saturating_add(n));
                }
                starts.get(k).copied()
            }
            MkvFrames::Pcm { sizes, frame_bytes } => Some(sizes.iter().take(k).map(|&s| u64::from(s)).sum::<u64>() / *frame_bytes),
            MkvFrames::Eac3 { samples, prefix, starts } => {
                // Headers are read only as far as packet `k`; a later seek extends the list. A
                // packet that cannot be read leaves the rest unknown (the caller then places the
                // packet by its timestamp) and is tried again on the next seek.
                if k >= samples.len() {
                    return None;
                }
                while starts.len() <= k {
                    let start = match starts.len().checked_sub(1) {
                        None => 0,
                        Some(prev) => {
                            let &(at, size) = samples.get(prev)?;
                            starts.get(prev)? + eac3_packet_frames(src, prefix, at, size)? as u64
                        }
                    };
                    starts.push(start);
                }
                starts.get(k).copied()
            }
            MkvFrames::Opus { samples, starts } => {
                if starts.is_none() {
                    // A read that fails leaves the starts unknown (the caller then places the
                    // packet by its timestamp) and is tried again on the next seek.
                    let mut acc = 0u64;
                    let mut found = Vec::with_capacity(samples.len());
                    for &(at, size) in samples.iter() {
                        let b = src.bytes(at, size.min(2) as usize)?;
                        let prefix = [b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)];
                        found.push(acc);
                        acc += u64::from(ogg::opus_packet_samples(prefix, size).unwrap_or(0));
                    }
                    *starts = Some(found);
                }
                starts.as_ref()?.get(k).copied()
            }
            MkvFrames::Timestamps => None,
        }
    }
}

/// The frames a stored Matroska E-AC-3 packet decodes to, from the headers of its primary
/// syncframes, reading only those headers; `None` when a read fails or a header does not parse.
/// A packet of dependent syncframes alone is zero frames, not a failure.
fn eac3_packet_frames(src: &Src<'_>, prefix: &[u8], at: u64, size: u32) -> Option<usize> {
    // Enough bytes for any AC-3 / E-AC-3 header ahead of the audio blocks.
    const HEADER_BYTES: usize = 32;
    let len = prefix.len() + size as usize;
    let mut frames = 0;
    let mut p = 0;
    while p < len {
        let n = HEADER_BYTES.min(len - p);
        let mut head = prefix.get(p..).unwrap_or_default().iter().copied().take(n).collect::<Vec<u8>>();
        if head.len() < n {
            let stored = (p + head.len() - prefix.len()) as u64;
            head.extend(src.bytes(at + stored, n - head.len())?);
        }
        let h = ac3::parse_header(&head).ok()?;
        if h.is_primary() {
            frames += h.samples();
        }
        p += h.frame_bytes.max(1);
    }
    (len > 0).then_some(frames)
}

/// The next packet of Matroska track `track` and its timestamp in nanoseconds.
fn mkv_next(demux: &mut matroska::Demuxer<Src<'_>>, track: usize, rate: u32) -> Option<Result<(Packet, i64)>> {
    loop {
        match demux.next_packet() {
            Ok(Some(p)) if p.track == track => {
                let pad = p.discard_padding_ns.and_then(|ns| u64::try_from(ns).ok()).unwrap_or(0);
                let packet = Packet { data: p.data, trim_end: rescale_round(pad, 1_000_000_000, u64::from(rate)), jumped: false };
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
    /// Whether a known length of 0 means the stream is empty, so decoding nothing is not an error.
    /// False for Ogg Vorbis, whose length is inferred from the pages' granule positions.
    empty_ok: bool,
    /// Decoded frames at the start that are not presented (AAC priming, Opus pre-skip).
    skip: u64,
    /// Presented frames, when the container says.
    limit: Option<u64>,
    /// Set by [`Native::seek`]: running out of packets afterwards is the end of the stream, even
    /// when nothing was decoded (a seek to the last frame).
    seeked: bool,
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
        let head = match src.head_checked() {
            Ok(head) => head,
            Err(e) => return Some(Err(e)),
        };
        let at = after_id3(src);
        // A tag longer than the head (cover art) hides what follows it, so look there.
        let past_tag = usize::try_from(at).is_ok_and(|at| at >= head.len() && at > 0);
        let sniffed = if past_tag { super::detect_container(&src.bytes_upto(at, 64 * 1024), None) } else { super::detect_container(&head, None) };
        let body = usize::try_from(at).ok().and_then(|at| head.get(at..)).unwrap_or_default();
        // The bytes decide the container when they say; the detection (which may come from the
        // extension) only picks among raw streams.
        // Formats with one track: any other track index is an error, as for a multi-track file.
        let single = |opened: Option<Result<Native<'a>>>| match (track, opened) {
            (Some(n), Some(Ok(_))) if n > 0 => Some(Err(AudioError::Unsupported(format!("no audio track {n}: the file has one")))),
            (_, opened) => opened,
        };
        match sniffed.or(detected) {
            Some(Container::Wav) if wav::is_wav(&head) => single(open_wav(src)),
            Some(Container::Aiff) if aiff::is_aiff(&head) => single(open_aiff(src)),
            Some(Container::Ogg) if ogg::sniff(body) => open_ogg(src, track),
            Some(c @ (Container::Mp4 | Container::QuickTime)) if head.get(4..8) == Some(b"ftyp") => open_mp4(src, c, track),
            Some(c @ (Container::Matroska | Container::WebM)) if body.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) => open_mkv(src, c, track),
            Some(Container::Adts) => single(open_adts(src, at)),
            Some(Container::Ac3) => single(open_ac3(src, at)),
            Some(Container::MpegAudio) => single(open_mpa(src, at)),
            None => single(open_ac3(src, at).or_else(|| open_adts(src, at)).or_else(|| open_mpa(src, at))),
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
                self.check_decoded()?;
                return Ok(None);
            }
            // The frames of a lost Ogg page are silence, so the packets after it keep the places
            // their granule positions give them; a long gap is filled a block at a time, and its
            // part before a seek target is passed over.
            if let Some(s) = self.packets.next_start().filter(|&s| s > self.pos && self.info.channels > 0) {
                if self.pos < self.from {
                    self.pos = s.min(self.from);
                    continue;
                }
                let n = usize::try_from((s - self.pos).min(PCM_PACKET_FRAMES)).unwrap_or(0);
                let block = AudioBuffer { sample_rate: self.info.sample_rate, channels: crate::pcm::silence(usize::from(self.info.channels), n)? };
                if let Some(out) = self.present(block, 0) {
                    return Ok(Some(out));
                }
                continue;
            }
            let Some(next) = self.packets.next() else {
                self.check_decoded()?;
                return Ok(None);
            };
            let (block, trim_end) = match next {
                Ok(p) => {
                    let decoded = self.dec.decode(&p.data);
                    // A packet after a damaged granule position counts once, also when it fails.
                    if p.jumped && decoded.is_ok() {
                        self.error(AudioError::Malformed("granule position jumps past what the file can hold".into()))?;
                    }
                    match decoded {
                        Ok(block) => {
                            // A block of no frames (a first Vorbis packet) is not decoded audio.
                            if block.frames() == 0 {
                                continue;
                            }
                            self.decoded = true;
                            (block, p.trim_end)
                        }
                        Err(e) => {
                            self.error(e)?;
                            (self.dec.conceal(Some(&p.data))?, p.trim_end)
                        }
                    }
                }
                // The source failing is not damage in the file: the caller decides whether to retry.
                Err(e @ AudioError::Io(_)) => return Err(e),
                Err(e) => {
                    self.error(e)?;
                    (self.dec.conceal(None)?, 0)
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

    /// At the end of the stream: an error when no packet decoded, unless the track declares no
    /// frames (an empty WAV or AIFF is empty, not broken) or a seek reached the end.
    fn check_decoded(&mut self) -> Result<()> {
        let empty = self.frames_known && self.empty_ok && self.info.frames == 0 && self.errors == 0;
        let end_seek = self.seeked
            && self.errors == 0
            && (self.empty_ok || self.frames_known && self.info.frames > 0 && self.from >= self.info.frames.saturating_add(self.skip));
        if self.decoded || empty || end_seek {
            return Ok(());
        }
        Err(self.no_audio("stream contains no audio"))
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
        let frame = if self.frames_known { frame.min(self.info.frames) } else { frame };
        self.seeked = true;
        let target = frame.saturating_add(self.skip);
        let start = self.packets.seek(target.saturating_sub(self.dec.preroll() as u64))?;
        self.dec.reset();
        self.pos = start;
        self.from = target;
        Ok(frame)
    }
}

/// Where a stream starts: after any ID3v2 tags, which may be longer than the head of the file.
fn after_id3(src: &Src<'_>) -> u64 {
    let mut at = 0u64;
    for _ in 0..MAX_ID3_TAGS {
        match id3v2_len(&src.bytes_upto(at, 10)) {
            Some(len) => at = at.saturating_add(len as u64),
            None => break,
        }
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
    Some(with_decoder(container, packets, dec, skip, limit, frames))
}

/// A native track from its packets and a decoder made for them.
fn with_decoder(container: Container, packets: Packets<'_>, dec: TrackDecoder, skip: u64, limit: Option<u64>, frames: Option<u64>) -> Native<'_> {
    let mut info = AudioInfo::new(container, dec.codec(), dec.sample_format(), dec.sample_rate(), dec.layout(), frames.unwrap_or(0));
    if dec.channels() == 0 || dec.sample_rate() == 0 {
        info.frames = 0;
    }
    Native {
        packets,
        info,
        frames_known: frames.is_some() && dec.channels() > 0 && dec.sample_rate() > 0,
        empty_ok: true,
        dec,
        skip,
        limit,
        pos: 0,
        from: skip,
        errors: 0,
        decoded: false,
        seeked: false,
        unsupported: None,
    }
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
    kind: crate::pcm::PcmKind,
    start: u64,
    len: u64,
) -> Option<Result<Native<'a>>> {
    let channels = usize::from(info.channels);
    let frame_bytes = (kind.bytes() * channels) as u64;
    let (bits, float, big_endian, unsigned) = match kind {
        crate::pcm::PcmKind::Int { bytes, big_endian, unsigned } => ((bytes * 8) as u16, false, big_endian, unsigned),
        crate::pcm::PcmKind::Float { bytes, big_endian } => ((bytes * 8) as u16, true, big_endian, false),
    };
    let params = CodecParams::new(CodecId::Pcm).with_sample_rate(info.sample_rate).with_channels(info.channels).with_pcm(if float {
        PcmFormat::float(bits, big_endian)
    } else {
        PcmFormat::int(bits, !unsigned, big_endian)
    });
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
    /// The source failed to read: the scan is over.
    failed: Option<AudioError>,
}

impl<T, F: Fn(&[u8]) -> Option<(usize, T)>> Scanner<'_, '_, F> {
    const WINDOW: usize = 1 << 16;
    /// The most header bytes a frame-length function reads.
    const HEADER: u64 = 16;

    /// The length and description of the frame at `p`, if one starts there and fits.
    fn len_at(&mut self, p: u64) -> Option<(usize, T)> {
        if p >= self.total || self.failed.is_some() {
            return None;
        }
        if p < self.window.0 || p + Self::HEADER > self.window.0 + self.window.1.len() as u64 {
            match self.src.read_upto(p, Self::WINDOW) {
                Ok(bytes) => self.window = (p, bytes),
                Err(e) => {
                    self.failed = Some(e);
                    return None;
                }
            }
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
/// length and a description of a frame starting at `rest`, or `None`. The first frame must be
/// followed by a second (or the end of the data), so a stray sync word does not make a stream;
/// damage later is skipped by scanning for the next pair. Reads the stream in windows. `None` when
/// no first frame is found; a source that fails to read is an error, not the end of the stream.
fn raw_frames<T>(src: &Src<'_>, at: u64, frame_len: impl Fn(&[u8]) -> Option<(usize, T)>) -> Result<Option<Vec<(u64, u32, T)>>> {
    let total = src.size();
    let mut sc = Scanner { src, total, frame_len, window: (u64::MAX, Vec::new()), failed: None };
    let first = (at..total.min(at.saturating_add(MAX_SYNC_SCAN))).find(|&p| sc.failed.is_some() || sc.paired(p));
    let Some(first) = first.filter(|_| sc.failed.is_none()) else {
        return sc.failed.map_or(Ok(None), Err);
    };
    let mut frames = Vec::new();
    let mut p = first;
    while p < total {
        match sc.len_at(p) {
            Some((n, f)) => {
                frames.push((p, n as u32, f));
                p += n as u64;
            }
            None => match (p + 1..total).find(|&q| sc.failed.is_some() || sc.paired(q)) {
                Some(q) if sc.failed.is_none() => p = q,
                _ => break,
            },
        }
    }
    sc.failed.map_or(Ok(Some(frames)), Err)
}

/// The first `n` frames of a raw stream.
fn raw_lookahead<T>(src: &Src<'_>, frames: &[(u64, u32, T)], n: usize) -> Result<Vec<Vec<u8>>> {
    frames.iter().take(n).map(|&(at, len, _)| src.read(at, len as usize)).collect()
}

/// A raw ADTS AAC stream from `at`: its frames, headers included, are the packets. The stream's
/// format is the one most of its first frames share, so one damaged header does not decide it.
/// [`TrackDecoder`] conceals the frames that differ from it or have several raw data blocks; a
/// stream that starts with those is not opened here.
fn open_adts<'a>(src: &Src<'a>, at: u64) -> Option<Result<Native<'a>>> {
    let frames =
        match raw_frames(src, at, |rest| aac::AdtsHeader::parse(rest).ok().filter(|h| h.frame_length > h.header_length).map(|h| (h.frame_length, h)))
        {
            Ok(frames) => frames?,
            Err(e) => return Some(Err(e)),
        };
    let first = &frames[..frames.len().min(crate::track::LOOKAHEAD_PACKETS)];
    let headers: Vec<aac::AdtsHeader> = first.iter().map(|&(_, _, h)| h).collect();
    let stream = adts_format_of(&adts_stream_header(&headers)?);
    if headers.iter().filter(|h| adts_format_of(h) == stream && h.raw_data_blocks == 1).count() * 2 <= first.len() {
        return None;
    }
    // Without extradata the configuration comes from the frames themselves.
    let params = CodecParams::new(CodecId::Aac);
    let lookahead = match raw_lookahead(src, &frames, crate::track::LOOKAHEAD_PACKETS) {
        Ok(lookahead) => lookahead,
        Err(e) => return Some(Err(e)),
    };
    // Decoded frames are set below, when the decoder has said what a frame decodes to.
    let packets: Vec<(u64, u32, u32)> = frames.iter().map(|&(p, n, h)| (p, n, u32::from(h.raw_data_blocks))).collect();
    let mut n = native(Container::Adts, Packets::Raw { frames: packets, src: src.clone(), next: 0 }, &params, &lookahead, 0, None, None)?;
    let per_block = n.dec.frames_per_packet()? as u32;
    if let Packets::Raw { frames, .. } = &mut n.packets {
        for f in frames.iter_mut() {
            f.2 *= per_block;
        }
        n.info.frames = frames.iter().map(|f| u64::from(f.2)).sum();
        n.frames_known = true;
    }
    Some(Ok(n))
}

/// The decoder's own delay that a LAME tag's encoder delay leaves out: decoded audio lags the
/// encoder's input by the delay plus these 528 + 1 samples.
#[cfg(feature = "mp3-decode")]
const MP3_DECODER_DELAY: u64 = 529;

/// A raw MPEG audio stream: layer I or II, and layer III with `mp3-decode`. A Xing / Info or
/// VBRI frame at the start carries no audio and is not a packet; its LAME tag's encoder delay
/// and padding trim the decoded audio to what was encoded.
fn open_mpa<'a>(src: &Src<'a>, at: u64) -> Option<Result<Native<'a>>> {
    let layer_ok = |layer: u8| layer == 1 || layer == 2 || (cfg!(feature = "mp3-decode") && layer == 3);
    #[cfg_attr(not(feature = "mp3-decode"), allow(unused_mut))]
    let mut frames =
        match raw_frames(src, at, |rest| mpa::parse_header(rest).ok().filter(|h| layer_ok(h.layer)).map(|h| (h.frame_bytes, h.samples() as u32))) {
            Ok(frames) => frames?,
            Err(e) => return Some(Err(e)),
        };
    #[cfg_attr(not(feature = "mp3-decode"), allow(unused_mut))]
    let mut gapless: Option<(u32, u32)> = None;
    #[cfg(feature = "mp3-decode")]
    if let Some(&(p, n, _)) = frames.first() {
        let first = match src.read(p, n as usize) {
            Ok(first) => first,
            Err(e) => return Some(Err(e)),
        };
        if let Some(tag) = crate::mp3::InfoTag::parse(&first) {
            frames.remove(0);
            gapless = tag.delay_padding;
        }
    }
    let total: u64 = frames.iter().map(|&(_, _, f)| u64::from(f)).sum();
    let (skip, limit) = match gapless {
        #[cfg(feature = "mp3-decode")]
        Some((delay, padding)) => (u64::from(delay) + MP3_DECODER_DELAY, Some(total.saturating_sub(u64::from(delay) + u64::from(padding)))),
        _ => (0, None),
    };
    let lookahead = match raw_lookahead(src, &frames, 1) {
        Ok(lookahead) => lookahead,
        Err(e) => return Some(Err(e)),
    };
    native(
        Container::MpegAudio,
        Packets::Raw { frames, src: src.clone(), next: 0 },
        &CodecParams::new(CodecId::Mp2),
        &lookahead,
        skip,
        limit,
        Some(limit.unwrap_or(total)),
    )
    .map(Ok)
}

/// A raw AC-3 or E-AC-3 stream; E-AC-3 dependent substreams decode to nothing and count no frames.
fn open_ac3<'a>(src: &Src<'a>, at: u64) -> Option<Result<Native<'a>>> {
    let frames = match raw_frames(src, at, |rest| {
        ac3::parse_header(rest).ok().map(|h| (h.frame_bytes, if h.is_primary() { h.samples() as u32 } else { 0 }))
    }) {
        Ok(frames) => frames?,
        Err(e) => return Some(Err(e)),
    };
    let total = frames.iter().map(|&(_, _, f)| u64::from(f)).sum();
    let lookahead = match raw_lookahead(src, &frames, 1) {
        Ok(lookahead) => lookahead,
        Err(e) => return Some(Err(e)),
    };
    native(Container::Ac3, Packets::Raw { frames, src: src.clone(), next: 0 }, &CodecParams::new(CodecId::Ac3), &lookahead, 0, None, Some(total))
        .map(Ok)
}

/// The `n`-th audio track (the first when `None`) among `kinds`, as an index into them.
fn audio_track(kinds: impl Iterator<Item = bool>, n: Option<usize>) -> Option<Result<usize>> {
    let audio: Vec<usize> = kinds.enumerate().filter(|&(_, audio)| audio).map(|(i, _)| i).collect();
    if audio.is_empty() {
        return None;
    }
    let n = n.unwrap_or(0);
    Some(audio.get(n).copied().ok_or_else(|| AudioError::Unsupported(format!("no audio track {n}: the file has {}", audio.len()))))
}

/// An MP4 / QuickTime file's audio track, if its codec is native. Samples that claim to lie
/// beyond the end of the file are refused rather than read leniently.
fn open_mp4<'a>(src: &Src<'a>, container: Container, track: Option<usize>) -> Option<Result<Native<'a>>> {
    let file = match demuxed(isobmff::open(src), mp4_failure)? {
        Ok(file) => file,
        Err(e) => return Some(Err(e)),
    };
    let track = match audio_track(file.tracks.iter().map(|t| t.kind == isobmff::TrackKind::Audio), track)? {
        Ok(t) => t,
        Err(e) => return Some(Err(e)),
    };
    let t = file.tracks.get(track)?;
    // A track with no samples (or sample sizes the demuxer refused) goes to the other path, whose
    // size checks name what is wrong.
    if t.samples.is_empty() {
        return None;
    }
    if t.audio().is_some_and(|a| u64::from(a.samples_per_packet.max(a.const_frames_per_packet)) > MAX_PACKET_FRAMES) {
        return Some(Err(AudioError::Malformed("the file declares packets far longer than any codec uses".into())));
    }
    let size = src.size();
    if t.samples.iter().any(|s| s.offset.checked_add(u64::from(s.size)).is_none_or(|end| end > size)) {
        return Some(Err(AudioError::Malformed("MP4 sample is larger than the file".into())));
    }
    let params = t.codec_params();
    // Keep file-level FLAC on its existing path until this reader can index variable packet
    // lengths and final duration from the frame headers. The packet API has no such timing role.
    if params.codec == CodecId::Flac {
        return None;
    }
    let lookahead: Vec<Vec<u8>> =
        (0..t.samples.len().min(crate::track::LOOKAHEAD_PACKETS)).filter_map(|i| file.read_sample(src, track, i).ok()).collect();
    let timescale = t.timescale;
    // Probe the decoder for the output rate before the timing is worked out.
    let probe = TrackDecoder::new(&params, &lookahead.iter().map(Vec::as_slice).collect::<Vec<_>>()).ok()?;
    let rate = probe.sample_rate();
    // A leading media edit (AAC priming, Opus pre-skip) is skipped, and the edit's duration is the
    // presentation length. Empty edits and multi-segment lists play the media as it is.
    // A leading empty edit (a delay before the track starts, ffmpeg's `-itsoffset` and muxers
    // aligning audio to video) does not change which media is presented: the media edit after it
    // still skips the priming and ends the track. The delay itself is not added as silence.
    let media_edit = match t.edits.as_slice() {
        [e] => Some(e),
        [empty, e] if empty.media_time < 0 => Some(e),
        _ => None,
    };
    let (skip, limit) = match media_edit {
        Some(e) if e.media_time >= 0 && e.media_rate == 0x0001_0000 => {
            let skip = rescale(e.media_time.unsigned_abs(), u64::from(timescale), u64::from(rate));
            let limit = (e.segment_duration > 0).then(|| rescale(e.segment_duration, u64::from(file.timescale), u64::from(rate)));
            (skip, limit)
        }
        _ => (0, None),
    };
    // The samples' durations end the track when no edit does, and an edit cannot outlast the media.
    let media: u64 = t.samples.iter().map(|s| u64::from(s.duration)).sum();
    let mut stored = rescale(media, u64::from(timescale), u64::from(rate)).saturating_sub(skip);
    // Durations only trim the last packet: ones that end anywhere else are wrong (some count a
    // HE-AAC packet's core frames) and can't cut audio off.
    // ADTS-framed samples (not standard) may hold several frames each, and finding out would mean
    // reading every sample: their sample table's timing is kept instead, unless it falls short of
    // the frames the samples hold (durations counting HE-AAC's core frames): short of one frame
    // per sample, or of the frames in the samples already read. Then it can't end the track, and
    // its length is unknown until decoded.
    let adts_counts: Vec<u64> = lookahead.iter().map(|p| adts_frame_count(p) as u64).collect();
    let adts = adts_counts.iter().any(|&c| c > 0);
    let per_packet = probe.frames_per_packet().map(|n| n as u64).filter(|_| !adts);
    let mut length_known = true;
    if let Some(n) = probe.frames_per_packet().map(|n| n as u64) {
        let one_each = (t.samples.len() as u64).saturating_mul(n).saturating_sub(skip);
        // The samples read ahead are the first ones unless one could not be read. The track's last
        // sample is left out: its duration may trim any number of its frames.
        let read_short = adts_counts.len() == t.samples.len().min(crate::track::LOOKAHEAD_PACKETS) && {
            let counted = adts_counts.len().min(t.samples.len() - 1);
            let held = adts_counts[..counted].iter().sum::<u64>().saturating_mul(n);
            let timed: u64 = t.samples[..counted].iter().map(|s| u64::from(s.duration)).sum();
            held.saturating_sub(rescale(timed, u64::from(timescale), u64::from(rate))) >= n
        };
        if !adts && (stored > one_each || one_each - stored >= n) {
            stored = one_each;
        } else if adts && (one_each.saturating_sub(stored) >= n || read_short) {
            stored = 0;
            length_known = limit.is_some();
        }
    }
    let frames = if stored > 0 { limit.map_or(stored, |l| l.min(stored)) } else { limit.unwrap_or(0) };
    let limit = (stored > 0).then_some(frames).or(limit);
    let packets = Packets::Mp4 { file: Box::new(file), src: src.clone(), track, next: 0, timescale, rate, per_packet };
    native(container, packets, &params, &lookahead, skip, limit, length_known.then_some(frames)).map(Ok)
}

/// A Matroska / WebM file's audio track, if its codec is native.
fn open_mkv<'a>(src: &Src<'a>, container: Container, track: Option<usize>) -> Option<Result<Native<'a>>> {
    let demux = match demuxed(matroska::Demuxer::new(src.clone()), mkv_failure)? {
        Ok(demux) => demux,
        Err(e) => return Some(Err(e)),
    };
    let file = demux.file();
    let track = match audio_track(file.tracks.iter().map(|t| t.kind == matroska::TrackKind::Audio), track)? {
        Ok(t) => t,
        Err(e) => return Some(Err(e)),
    };
    let t = file.tracks.get(track)?;
    if !t.frames_readable() {
        return None;
    }
    let params = t.codec_params();
    // Vorbis in Matroska is left to symphonia: its first packet decodes to nothing, which the
    // timestamp-based timing here does not model.
    // The Matroska file API needs FLAC frame counts to undo timestamp rounding on seeks.
    // Keep its existing fallback until it can index those counts.
    if matches!(params.codec, CodecId::Vorbis | CodecId::Flac) {
        return None;
    }
    let lookahead: Vec<Vec<u8>> =
        (0..t.samples.len().min(crate::track::LOOKAHEAD_PACKETS)).filter_map(|i| demux.read_sample(track, i).ok()).collect();
    let probe = TrackDecoder::new(&params, &lookahead.iter().map(Vec::as_slice).collect::<Vec<_>>()).ok()?;
    let rate = probe.sample_rate();
    // Packet starts come from the packets where they can: block timestamps are rounded (to
    // milliseconds by default). ALAC packets are all its frame length except the last, so every
    // start is a multiple of it, unless a short packet came earlier: then the timestamp is further
    // from that multiple than its rounding allows, and the earlier packets' headers are counted.
    let (num, den) = t.timebase;
    let tick = rescale_round(num, den.max(1), u64::from(rate)).saturating_add(1);
    let frames_of = match (probe.full_packet_frames(), params.pcm) {
        _ if probe.codec() == CodecId::Eac3 => {
            MkvFrames::Eac3 { samples: t.samples.iter().map(|s| (s.offset, s.size)).collect(), prefix: t.frame_prefix(0), starts: Vec::new() }
        }
        (Some(n), _) => MkvFrames::Constant { frames: n as u64, slack: tick, count: probe.packet_frames_reader(), starts: vec![0] },
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
    // The decoded timeline starts at the first frame (negative when `CodecDelay` was taken off),
    // and the presentation starts `CodecDelay` later. Both are counted in timestamp ticks, the
    // delay rounded to one, so a file whose first frame is stored at 0 presents from exactly 0
    // although its timestamps are rounded (AAC's 1024-frame priming is 21.33 ms, stored as -21).
    let last = t.samples.iter().max_by_key(|s| s.pts);
    let delay_ticks = i64::try_from(rescale_round(t.codec_delay_ns.saturating_mul(den), num.saturating_mul(1_000_000_000), 1)).unwrap_or(i64::MAX);
    let start = t.samples.iter().map(|s| s.pts).min().unwrap_or(0).saturating_add(delay_ticks);
    let frames = last.and_then(|s| {
        let (block_start, block_duration) =
            final_mkv_block_duration(t.samples.iter().map(|sample| (sample.block_offset, sample.pts, sample.duration)), s.block_offset, s.pts);
        let (end_start, duration) = if block_duration > 0 {
            // BlockDuration applies to the entire block, including every lace. The demuxer
            // divides it among frames, so sum the final block and measure from its first lace.
            (block_start, block_duration)
        } else {
            // ffmpeg 6 writes AAC and E-AC-3 without block durations, and records the end in a tag.
            let n = match &frames_of {
                MkvFrames::Constant { frames: n, .. } => *n,
                MkvFrames::Eac3 { prefix, .. } => eac3_packet_frames(src, prefix, s.offset, s.size)? as u64,
                _ => return None,
            };
            let packet = rescale_round(n.saturating_mul(den), u64::from(rate).saturating_mul(num), 1);
            (s.pts, tagged_last_duration(track_duration_tag(&file.tags, t.uid)?, s.pts, delay_ticks, packet, t.timebase)?)
        };
        let end = u64::try_from(end_start.saturating_add(i64::try_from(duration).ok()?).saturating_sub(start)).ok()?;
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

/// Return the start timestamp and total duration of the laces belonging to the final block.
fn final_mkv_block_duration(samples: impl Iterator<Item = (u64, i64, u64)>, block_offset: u64, last_pts: i64) -> (i64, u64) {
    samples.fold(
        (last_pts, 0u64),
        |(start, duration), (offset, pts, lace_duration)| {
            if offset == block_offset { (start.min(pts), duration.saturating_add(lace_duration)) } else { (start, duration) }
        },
    )
}

#[cfg(test)]
mod final_mkv_block_duration_tests {
    use super::final_mkv_block_duration;

    #[test]
    fn sums_all_lace_durations_from_the_first_lace() {
        let samples = [(10, 0, 64), (20, 100, 24), (20, 124, 24), (30, 200, 0)];
        assert_eq!(final_mkv_block_duration(samples.into_iter(), 20, 124), (100, 48));
    }

    #[test]
    fn unknown_duration_stays_unknown_and_keeps_the_last_timestamp() {
        let samples = [(20, 100, 0), (20, 100, 0)];
        assert_eq!(final_mkv_block_duration(samples.into_iter(), 20, 100), (100, 0));
    }
}

/// The `DURATION` tag of the track with UID `uid`, in nanoseconds. ffmpeg and mkvmerge write it
/// as `HH:MM:SS.nnnnnnnnn`.
fn track_duration_tag(tags: &[matroska::Tag], uid: u64) -> Option<u64> {
    tags.iter()
        .filter(|tag| tag.targets.track_uids.contains(&uid))
        .flat_map(|tag| &tag.simple)
        .find(|simple| simple.name.eq_ignore_ascii_case("DURATION"))
        .and_then(|simple| parse_tag_duration(simple.string.as_deref()?))
}

/// `HH:MM:SS.nnnnnnnnn` in nanoseconds.
fn parse_tag_duration(text: &str) -> Option<u64> {
    let (clock, fraction) = text.split_once('.').unwrap_or((text, ""));
    let mut parts = clock.split(':').map(|p| p.parse::<u64>().ok());
    let (hours, minutes, seconds) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() || fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let nanos = if fraction.is_empty() { 0 } else { fraction.parse::<u64>().ok()? * 10u64.pow(9 - fraction.len() as u32) };
    hours.checked_mul(3600)?.checked_add(minutes.checked_mul(60)?)?.checked_add(seconds)?.checked_mul(1_000_000_000)?.checked_add(nanos)
}

/// The duration in ticks of a track's last packet, when `tag_ns` (which counts `CodecDelay`, as the
/// stored block timestamps do) ends inside that packet. A tag can go stale when a file is edited,
/// so one that ends anywhere else is ignored and can't cut audio off.
fn tagged_last_duration(tag_ns: u64, last_pts: i64, delay_ticks: i64, packet_ticks: u64, (num, den): (u64, u64)) -> Option<u64> {
    let tag_ticks = i64::try_from(rescale_round(tag_ns.saturating_mul(den), num.saturating_mul(1_000_000_000), 1)).ok()?;
    let duration = u64::try_from(tag_ticks.saturating_sub(delay_ticks).saturating_sub(last_pts)).ok()?;
    (duration > 0 && duration <= packet_ticks).then_some(duration)
}

/// An Ogg file's Opus stream (RFC 7845: pre-skip and end trimming applied).
fn open_ogg<'a>(src: &Src<'a>, track: Option<usize>) -> Option<Result<Native<'a>>> {
    let file = match demuxed(ogg::open(src), ogg_failure)? {
        Ok(file) => file,
        Err(e) => return Some(Err(e)),
    };
    let stream = match audio_track(file.streams.iter().map(|s| s.codec.codec_id().is_audio()), track)? {
        Ok(s) => s,
        Err(e) => return Some(Err(e)),
    };
    let s = file.streams.get(stream)?;
    if s.codec == ogg::Codec::Vorbis {
        return open_ogg_vorbis(src, file, stream);
    }
    if s.codec != ogg::Codec::Opus {
        return None;
    }
    let params = s.codec_params();
    let head = match crate::opus::OpusHead::parse(&params.extradata) {
        Ok(head) => head,
        // A feature this decoder lacks is left to the fallback; a broken header is reported.
        Err(crate::opus::Error::Unsupported(_)) => return None,
        Err(e) => return Some(Err(e.into())),
    };
    let mut last_duration = 960;
    let timing = ogg::OpusTiming::of_with_durations(s, u32::from(head.pre_skip), |packet| {
        if let Some(duration) = ogg::opus_packet_samples(packet.prefix, packet.size) {
            last_duration = duration;
        }
        last_duration
    });
    let mut starts = timing.starts.clone();
    let mut total = timing.total;
    let durations: Vec<u64> = timing.durations.iter().map(|&d| u64::from(d)).collect();
    let jumps = bound_gaps(&mut starts, &durations, &mut total, src.size());
    let limit = u64::try_from(total).ok();
    let skip = u64::from(timing.pre_skip);
    let packets = Packets::Ogg { file: Box::new(file), src: src.clone(), stream, next: 0, starts, primers: None, jumps };
    native(Container::Ogg, packets, &params, &[], skip, limit, limit).map(Ok)
}

/// Stream `stream` of `file`, an Ogg Vorbis stream. Packets are read one at a time; their places
/// on the timeline come from their block sizes and the pages' granule positions, with the start
/// and end trimming those positions call for (Vorbis I §A.2).
fn open_ogg_vorbis<'a>(src: &Src<'a>, file: ogg::OggFile, stream: usize) -> Option<Result<Native<'a>>> {
    let s = file.streams.get(stream)?;
    let params = s.codec_params();
    let dec = match TrackDecoder::new(&params, &[]) {
        Ok(dec) => dec,
        // With symphonia, it reads the stream or reports it; without, the header's fault stands.
        Err(_) if cfg!(feature = "symphonia") => return None,
        Err(e) => return Some(Err(e)),
    };
    // A packet whose mode cannot be read counts as its concealment counts it.
    let mut prev: Option<usize> = None;
    let timing = ogg::VorbisTiming::of_with_durations(s, |p| {
        let n = dec.vorbis_block_size(vorbis_head(p), prev);
        let frames = match (prev, n) {
            (Some(a), Some(b)) => u32::try_from(a / 4 + b / 4).unwrap_or(u32::MAX),
            _ => 0,
        };
        prev = n;
        frames
    });
    let first_damaged = timing.first_damaged;
    let durations: Vec<u64> = timing.durations.iter().map(|&d| u64::from(d)).collect();
    // The presented timeline starts at the first sample to play (a stream cut from a longer one
    // starts later than 0); decoded frames before it keep negative places. Leading packets that
    // decode to nothing (the priming packet, on a page of its own or not) take the place of the
    // first packet that decodes to frames.
    let mut starts: Vec<i64> = timing.starts.iter().map(|s| s.saturating_sub(timing.first)).collect();
    if let Some(k) = timing.durations.iter().position(|&d| d > 0) {
        let place = starts[k];
        starts[..k].fill(place);
    }
    let mut total = timing.total;
    let mut jumps = bound_gaps(&mut starts, &durations, &mut total, src.size());
    // a damaged first granule position counts as damage at the first packet
    if let Some(first) = jumps.first_mut() {
        *first |= first_damaged;
    }
    let primers = s.packets.iter().map(|p| dec.vorbis_mode_readable(vorbis_head(p))).collect();
    // Decoded frames before the first page's granule position are not presented.
    let skip = u64::try_from(starts.first().copied().unwrap_or(0).saturating_neg()).unwrap_or(0);
    let limit = u64::try_from(total).ok();
    let packets = Packets::Ogg { file: Box::new(file), src: src.clone(), stream, next: 0, starts, primers: Some(primers), jumps };
    let mut n = with_decoder(Container::Ogg, packets, dec, skip, limit, limit);
    // a stream with no audio packets, or only the first (which decodes to no frames), is an error
    n.empty_ok = false;
    Some(Ok(n))
}

/// The first bytes of a Vorbis packet, which hold its mode number.
fn vorbis_head(p: &ogg::Packet) -> &[u8] {
    p.prefix.get(..(p.size as usize).min(p.prefix.len())).unwrap_or_default()
}

/// Bounds the silence that gaps between Ogg packets' places leave (lost pages, filled by
/// [`Native::next_block`]): `starts` holds each packet's place, `durations` the frames each decodes
/// to and `total` the presented frames, on one timeline. The gaps of a file of `file_bytes` bytes
/// may hold, in all, the frames of one longest packet per byte of the file. A gap that does not
/// fit in what is left is a damaged granule position: the packets from it on move back by the gap
/// to follow the packet before it, and the packet after it is marked in the result, to count as
/// damaged when read. A later packet placed before the end of the one before it (the granule
/// positions come back down after one damaged page) moves forward to that end. A `total` past
/// the last packet's end shrinks by what stays taken off at the end.
fn bound_gaps(starts: &mut [i64], durations: &[u64], total: &mut i64, file_bytes: u64) -> Vec<bool> {
    let longest = durations.iter().copied().max().unwrap_or(0).max(1);
    let mut budget = file_bytes.saturating_mul(longest);
    let mut jumps = vec![false; starts.len()];
    // frames taken off the starts from the last damaged granule position on
    let mut shift = 0i64;
    for k in 1..starts.len().min(durations.len()) {
        let expected = starts[k - 1].saturating_add(i64::try_from(durations[k - 1]).unwrap_or(i64::MAX));
        let mut start = starts[k].saturating_sub(shift);
        if start < expected {
            let back = expected.saturating_sub(start);
            shift = shift.saturating_sub(back).max(0);
            start = expected;
        }
        if let Some(frames) = u64::try_from(start.saturating_sub(expected)).ok().filter(|&f| f > 0) {
            if frames <= budget {
                budget -= frames;
            } else {
                shift = shift.saturating_add(start.saturating_sub(expected));
                start = expected;
                jumps[k] = true;
            }
        }
        starts[k] = start;
    }
    // A total past the last packet's end is on the jumped timeline; one within it (a last page
    // whose granule position came back down) already leaves the jump out.
    let n = starts.len().min(durations.len());
    let end = n.checked_sub(1).map_or(0, |k| starts[k].saturating_add(i64::try_from(durations[k]).unwrap_or(i64::MAX)));
    if shift > 0 && *total > end {
        *total = total.saturating_sub(shift).max(0);
    }
    jumps
}

#[cfg(test)]
mod ogg_timing_tests {
    #[test]
    fn backward_packet_starts_are_clamped_without_removing_real_gaps_or_end_trimming() {
        let durations = [0, 128, 128, 128, 128];
        for last in [384, 512] {
            let mut starts = [0, 0, 128, 0, last];
            let mut total = last + 64;
            let jumps = super::bound_gaps(&mut starts, &durations, &mut total, 1024);
            assert!(starts.windows(2).zip(durations).all(|(pair, duration)| pair[1] >= pair[0] + duration as i64));
            assert_eq!(starts[4], last);
            assert_eq!(total, last + 64);
            assert!(jumps.iter().all(|jump| !jump));
        }
    }
}

/// The audio tracks of `src` the native path reads, described for a track list; `None` when it
/// does not read the container.
pub(crate) fn tracks(src: &Src<'_>, detected: Option<Container>) -> Option<Result<Vec<CodecParams>>> {
    let head = match src.head_checked() {
        Ok(head) => head,
        Err(e) => return Some(Err(e)),
    };
    match super::detect_container(&head, None).or(detected) {
        Some(Container::Mp4 | Container::QuickTime) => demuxed(isobmff::open(src), mp4_failure)
            .map(|r| r.map(|file| file.tracks.iter().filter(|t| t.kind == isobmff::TrackKind::Audio).map(isobmff::Track::codec_params).collect())),
        Some(Container::Matroska | Container::WebM) => demuxed(matroska::open(src.clone()), mkv_failure)
            .map(|r| r.map(|file| file.tracks.iter().filter(|t| t.kind == matroska::TrackKind::Audio).map(matroska::Track::codec_params).collect())),
        Some(Container::Ogg) => demuxed(ogg::open(src), ogg_failure)
            .map(|r| r.map(|file| file.streams.iter().filter(|s| s.codec.codec_id().is_audio()).map(ogg::Stream::codec_params).collect())),
        _ => None,
    }
}

/// What is damaged in `src`, when it is in a container the native demuxers read and they find it
/// truncated or structurally invalid: the error to return when symphonia cannot read it either.
/// Ogg is left out: `ogg::open` skips damaged pages and calls a file with none it can read not
/// Ogg, and symphonia reports a damaged Ogg stream as malformed itself.
#[cfg(feature = "symphonia")]
pub(crate) fn damage(src: &Src<'_>, detected: Option<Container>) -> Option<String> {
    let head = src.head_checked().ok()?;
    let failure = match super::detect_container(&head, None).or(detected) {
        Some(Container::Mp4 | Container::QuickTime) => mp4_failure(isobmff::open(src).err()?),
        Some(Container::Matroska | Container::WebM) => mkv_failure(matroska::open(src.clone()).err()?),
        _ => return None,
    };
    match failure {
        Failure::Damaged(m) => Some(m),
        Failure::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Some(e.to_string()),
        _ => None,
    }
}

/// Why a craft-media demuxer could not read a file.
enum Failure {
    /// The source failed to read.
    Io(std::io::Error),
    /// The file is damaged: truncated, or structurally invalid.
    Damaged(String),
    /// Anything else: not this container, or a feature the demuxer does not read.
    Other,
}

/// A craft-media demuxer's result for the native path: the source's failure to read as
/// [`AudioError::Io`], and `None` when the demuxer does not read the file, so the other path
/// tries it. Without symphonia there is no other path, so a damaged file is
/// [`AudioError::Malformed`]; with it, the file is malformed when symphonia cannot read it either
/// ([`damage`]).
fn demuxed<T, E>(r: std::result::Result<T, E>, why: fn(E) -> Failure) -> Option<Result<T>> {
    let e = match r {
        Ok(v) => return Some(Ok(v)),
        Err(e) => why(e),
    };
    match e {
        Failure::Io(e) if e.kind() != std::io::ErrorKind::UnexpectedEof => Some(Err(AudioError::Io(e))),
        Failure::Io(e) if cfg!(not(feature = "symphonia")) => Some(Err(AudioError::Malformed(e.to_string()))),
        Failure::Damaged(m) if cfg!(not(feature = "symphonia")) => Some(Err(AudioError::Malformed(m))),
        _ => None,
    }
}

fn mp4_failure(e: isobmff::Error) -> Failure {
    match e {
        isobmff::Error::Io(e) => Failure::Io(e),
        e @ (isobmff::Error::Truncated(_) | isobmff::Error::Invalid(_) | isobmff::Error::NoMoov) => Failure::Damaged(e.to_string()),
        _ => Failure::Other,
    }
}

fn mkv_failure(e: matroska::Error) -> Failure {
    match e {
        matroska::Error::Io(e) => Failure::Io(e),
        e @ matroska::Error::Invalid(_) => Failure::Damaged(e.to_string()),
        _ => Failure::Other,
    }
}

fn ogg_failure(e: ogg::Error) -> Failure {
    match e {
        ogg::Error::Io(e) => Failure::Io(e),
        e @ ogg::Error::Invalid(_) => Failure::Damaged(e.to_string()),
        _ => Failure::Other,
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

#[cfg(test)]
mod tag_tests {
    use super::{parse_tag_duration, tagged_last_duration};

    #[test]
    fn tag_durations_parse_and_nonsense_does_not() {
        assert_eq!(parse_tag_duration("00:00:05.021000000"), Some(5_021_000_000));
        assert_eq!(parse_tag_duration("01:02:03.5"), Some(3_723_500_000_000));
        assert_eq!(parse_tag_duration("00:00:05"), Some(5_000_000_000));
        for bad in ["", "5.021", "00:05.0", "00:00:00:05.0", "00:00:0x.0", "00:00:05.-1", "00:00:05.0000000001", "99999999999:00:00.0"] {
            assert_eq!(parse_tag_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_a_tag_ending_inside_the_last_packet_counts() {
        // 5 s of 48 kHz AAC at millisecond ticks: priming 21, last packet stored at 4992 (5013 with
        // the delay), 21 ticks long
        let last = |ms: u64| tagged_last_duration(ms * 1_000_000, 4992, 21, 21, (1, 1000));
        assert_eq!(last(5021), Some(8));
        assert_eq!(last(5034), Some(21));
        assert_eq!(last(5035), None, "past the packet");
        assert_eq!(last(5013), None, "at its start");
        assert_eq!(last(3000), None, "far before it");
    }
}

#[cfg(test)]
mod adts_tests {
    use crate::{AudioError, Container, DecodeOptions, Decoder, decode};
    use craft_media::ByteSource;

    /// A frame of mono silence at sampling frequency index `sf_index`.
    fn frame(sf_index: u8) -> Vec<u8> {
        // ID_SCE, tag 0, an empty channel stream, ID_END
        let au = [0x00, 0x00, 0x00, 0x07];
        [&crate::aac::AdtsHeader::write(2, sf_index, 1, au.len(), 0x7FF).unwrap()[..], &au].concat()
    }

    #[test]
    fn a_stream_decodes_past_damage_between_frames() {
        let good = frame(3).repeat(4);
        let file = [&good[..], &[0x12; 100], &good].concat();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.container, buf.frames()), (Container::Adts, 8 * 1024));
    }

    #[test]
    fn frames_with_several_raw_data_blocks_do_not_lose_audio_silently() {
        let mut one = frame(3);
        one[6] |= 1; // two raw data blocks
        let file = one.repeat(8);
        // The file is refused, not decoded at half its length.
        assert!(decode(&file, Some("aac")).is_err());
    }

    /// Frames of several blocks after the stream has started are concealed at their whole length,
    /// not skipped.
    #[test]
    fn several_raw_data_blocks_mid_stream_keep_the_timing() {
        let mut two = frame(3);
        two[6] |= 1;
        let file = [frame(3).repeat(8), two, frame(3).repeat(2)].concat();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.damaged, info.frames, buf.frames()), (1, 12 * 1024, 12 * 1024));
        assert_eq!(crate::probe(&file, None).unwrap().frames, 12 * 1024);
    }

    /// One flipped bit in the first header does not silence the stream.
    #[test]
    fn a_damaged_first_header_does_not_decide_the_format() {
        let file = [frame(4), frame(3).repeat(9)].concat();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.sample_rate, info.damaged, buf.frames()), (48_000, 1, 10 * 1024));
    }

    #[test]
    fn a_frame_that_changes_the_stream_format_is_damage() {
        let file = [frame(3).repeat(4), frame(4).repeat(3)].concat();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.sample_rate, info.damaged, buf.frames()), (48_000, 3, 7 * 1024));
    }

    #[test]
    fn a_stream_is_found_after_an_id3_tag_longer_than_the_head() {
        let size = 300_000u32;
        let tag = [
            b"ID3\x03\x00\x00".to_vec(),
            [size >> 21, size >> 14, size >> 7, size].iter().map(|v| (v & 0x7F) as u8).collect(),
            vec![0; size as usize],
        ]
        .concat();
        let file = [tag, frame(3).repeat(4)].concat();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.container, buf.frames()), (Container::Adts, 4 * 1024));
    }

    /// Bytes that fail to read past `good`.
    struct Failing {
        bytes: Vec<u8>,
        good: u64,
    }

    impl ByteSource for Failing {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
            if offset + buf.len() as u64 > self.good {
                return Err(std::io::Error::other("the source failed"));
            }
            self.bytes.as_slice().read_at(offset, buf)
        }
    }

    /// A frame that reads in the scan and fails when read again.
    #[test]
    fn a_source_failing_during_the_lookahead_is_an_io_error() {
        struct FailsFrames(Vec<u8>);
        impl ByteSource for FailsFrames {
            fn len(&self) -> u64 {
                self.0.len() as u64
            }
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
                // A whole frame in the middle: the scan reads windows, to the end of the file.
                if buf.len() == 11 && offset + 11 < self.len() {
                    return Err(std::io::Error::other("the source failed"));
                }
                self.0.as_slice().read_at(offset, buf)
            }
        }
        let opened = Decoder::open(FailsFrames(frame(3).repeat(64)), Some("aac"), &DecodeOptions::default());
        assert!(matches!(opened, Err(AudioError::Io(_))), "{:?}", opened.err());
    }

    /// ADTS-framed HE-AAC whose sample durations count core frames is not cut to the durations'
    /// half length.
    #[test]
    fn adts_framed_he_aac_in_mp4_with_core_durations_keeps_its_length() {
        use craft_media::bitstream::BitWriter;
        use craft_media::isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
        // mono silence with SBR data in a fill element (implicit HE-AAC), at 22.05 kHz
        let mut bw = BitWriter::new();
        for (value, bits) in [(0, 7), (0, 22), (6, 3), (1, 4), (13, 4), (0, 4), (7, 3)] {
            bw.write_bits(value, bits);
        }
        let au = bw.finish();
        let frame = [&crate::aac::AdtsHeader::write(2, 7, 1, au.len(), 0x7FF).unwrap()[..], &au].concat();
        let asc = crate::aac::AudioSpecificConfig::lc(7, 1).to_bytes().unwrap();
        let mut w = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let track = w.add_track(TrackConfig::new(SampleEntry::aac(asc, 1, 44_100), 44_100)).unwrap();
        for _ in 0..12 {
            w.write_sample(track, WriteSample { data: &frame, duration: 1024, composition_offset: 0, is_sync: true }).unwrap();
        }
        let file = w.finish().unwrap().into_inner();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.codec, info.sample_rate, buf.frames()), (crate::CodecId::HeAac, 44_100, 12 * 2048));
        assert_eq!(crate::probe(&file, None).unwrap().frames, 12 * 2048);
    }

    /// Samples of two HE-AAC ADTS frames each, timed as two core frames: the durations match one
    /// output frame per sample, and the frames in the samples read ahead show they fall short.
    #[test]
    fn adts_framed_he_aac_in_mp4_with_several_frames_per_sample_keeps_its_length() {
        use craft_media::bitstream::BitWriter;
        use craft_media::isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
        let mut bw = BitWriter::new();
        for (value, bits) in [(0, 7), (0, 22), (6, 3), (1, 4), (13, 4), (0, 4), (7, 3)] {
            bw.write_bits(value, bits);
        }
        let au = bw.finish();
        let frame = [&crate::aac::AdtsHeader::write(2, 7, 1, au.len(), 0x7FF).unwrap()[..], &au].concat().repeat(2);
        let asc = crate::aac::AudioSpecificConfig::lc(7, 1).to_bytes().unwrap();
        let mut w = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let track = w.add_track(TrackConfig::new(SampleEntry::aac(asc, 1, 44_100), 44_100)).unwrap();
        for _ in 0..12 {
            w.write_sample(track, WriteSample { data: &frame, duration: 2048, composition_offset: 0, is_sync: true }).unwrap();
        }
        let file = w.finish().unwrap().into_inner();
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!((info.codec, info.sample_rate, buf.frames()), (crate::CodecId::HeAac, 44_100, 12 * 4096));
        assert_eq!(crate::probe(&file, None).unwrap().frames, 12 * 4096);
    }

    /// A short track's last sample, read ahead, can trim more than one of its frames.
    #[test]
    fn a_short_adts_framed_mp4_track_keeps_a_last_sample_trim_of_over_a_frame() {
        use craft_media::isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
        let asc = crate::aac::AudioSpecificConfig::lc(3, 1).to_bytes().unwrap();
        let mut w = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let track = w.add_track(TrackConfig::new(SampleEntry::aac(asc, 1, 48_000), 48_000)).unwrap();
        for i in 0..4 {
            let duration = if i == 3 { 2048 - 1500 } else { 2048 };
            w.write_sample(track, WriteSample { data: &frame(3).repeat(2), duration, composition_offset: 0, is_sync: true }).unwrap();
        }
        let file = w.finish().unwrap().into_inner();
        let (_, buf) = decode(&file, None).unwrap();
        assert_eq!(buf.frames(), 4 * 2048 - 1500);
        assert_eq!(crate::probe(&file, None).unwrap().frames, 4 * 2048 - 1500);
    }

    /// AAC-LC samples of two ADTS frames each, timed correctly, still end at their durations.
    #[test]
    fn adts_framed_lc_in_mp4_with_several_frames_per_sample_keeps_its_trim() {
        use craft_media::isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
        let asc = crate::aac::AudioSpecificConfig::lc(3, 1).to_bytes().unwrap();
        let mut w = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let track = w.add_track(TrackConfig::new(SampleEntry::aac(asc, 1, 48_000), 48_000)).unwrap();
        for i in 0..12 {
            let duration = if i == 11 { 2048 - 500 } else { 2048 };
            w.write_sample(track, WriteSample { data: &frame(3).repeat(2), duration, composition_offset: 0, is_sync: true }).unwrap();
        }
        let file = w.finish().unwrap().into_inner();
        let (_, buf) = decode(&file, None).unwrap();
        assert_eq!(buf.frames(), 12 * 2048 - 500);
        assert_eq!(crate::probe(&file, None).unwrap().frames, 12 * 2048 - 500);
    }

    /// A sample of several ADTS frames past the lookahead still counts them all.
    #[test]
    fn a_later_mp4_sample_of_several_adts_frames_keeps_its_length() {
        use craft_media::isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
        let asc = crate::aac::AudioSpecificConfig::lc(3, 1).to_bytes().unwrap();
        let mut w = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let track = w.add_track(TrackConfig::new(SampleEntry::aac(asc, 1, 48_000), 48_000)).unwrap();
        for i in 0..12 {
            let (data, duration) = if i == 10 { (frame(3).repeat(2), 2048) } else { (frame(3), 1024) };
            w.write_sample(track, WriteSample { data: &data, duration, composition_offset: 0, is_sync: true }).unwrap();
        }
        let file = w.finish().unwrap().into_inner();
        let (_, buf) = decode(&file, None).unwrap();
        assert_eq!(buf.frames(), 13 * 1024);
    }

    #[test]
    fn a_source_failing_during_the_frame_scan_is_an_io_error() {
        let bytes = frame(3).repeat(100_000);
        // Past the head, so the failure comes during the scan rather than format detection.
        let opened = Decoder::open(Failing { bytes, good: 300_000 }, Some("aac"), &DecodeOptions::default());
        assert!(matches!(opened, Err(AudioError::Io(_))), "{:?}", opened.err());
    }

    /// An escaped (uncompressed) stereo 16-bit ALAC packet of `chunk`, partial when shorter than
    /// `frame_length`.
    fn alac_escaped_stereo(chunk: &[[i16; 2]], frame_length: usize) -> Vec<u8> {
        let mut w = craft_media::bitstream::BitWriter::new();
        let partial = chunk.len() != frame_length;
        for (value, bits) in [(1, 3), (0, 4), (0, 12), (u32::from(partial) << 3 | 1, 4)] {
            w.write_bits(value, bits);
        }
        if partial {
            w.write_bits(chunk.len() as u32, 32);
        }
        for f in chunk {
            w.write_bits(u32::from(f[0] as u16), 16);
            w.write_bits(u32::from(f[1] as u16), 16);
        }
        w.write_bits(7, 3);
        w.finish()
    }

    /// ALAC in Matroska seeks to the exact frame: packet starts come from the frame length, not
    /// from the millisecond timestamps (packet 1 starts at frame 4096, stored as 85 ms = 4080).
    #[test]
    fn alac_in_matroska_seeks_to_the_exact_frame() {
        use craft_media::matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
        let frames: Vec<[i16; 2]> = (0..10_000).map(|i| [i as i16, -(i as i16)]).collect();
        let mut spec = TrackSpec::new(TrackKind::Audio, "A_ALAC");
        spec.codec_private = crate::alac::SpecificConfig::new(48_000, 2, 16, 4096).to_bytes();
        spec.audio = Some((48_000.0, 2, Some(16)));
        let mut m = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![spec], MuxOptions::default()).unwrap();
        for (k, chunk) in frames.chunks(4096).enumerate() {
            let pts_ns = (k * 4096) as i64 * 1_000_000_000 / 48_000;
            let duration_ns = chunk.len() as u64 * 1_000_000_000 / 48_000;
            m.write_frame(0, pts_ns, true, &alac_escaped_stereo(chunk, 4096), Some(duration_ns)).unwrap();
        }
        let file = m.finish().unwrap().into_inner();
        let (info, buf) = decode(&file, Some("mka")).unwrap();
        assert_eq!((info.codec, buf.frames()), (crate::CodecId::Alac, 10_000));
        let mut d = Decoder::from_bytes(&file, Some("mka"), &DecodeOptions::default()).unwrap();
        for target in [5000u64, 4096, 8191, 9999] {
            d.seek(target).unwrap();
            let block = d.next_block().unwrap().unwrap();
            assert_eq!((block.channels[0][0] * 32768.0).round() as i64, target as i64, "seek to {target}");
        }
    }

    /// A short ALAC packet before the end moves every later packet: seeks after it still land on
    /// the exact frame.
    #[test]
    fn alac_in_matroska_seeks_past_a_short_packet() {
        use craft_media::matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
        let frames: Vec<[i16; 2]> = (0..12_000).map(|i| [i as i16, -(i as i16)]).collect();
        let mut spec = TrackSpec::new(TrackKind::Audio, "A_ALAC");
        spec.codec_private = crate::alac::SpecificConfig::new(48_000, 2, 16, 4096).to_bytes();
        spec.audio = Some((48_000.0, 2, Some(16)));
        let mut m = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![spec], MuxOptions::default()).unwrap();
        let mut start = 0;
        for len in [4096, 1000, 4096, 2808] {
            let chunk = &frames[start..start + len];
            let pts_ns = start as i64 * 1_000_000_000 / 48_000;
            m.write_frame(0, pts_ns, true, &alac_escaped_stereo(chunk, 4096), Some(len as u64 * 1_000_000_000 / 48_000)).unwrap();
            start += len;
        }
        let file = m.finish().unwrap().into_inner();
        let mut d = Decoder::from_bytes(&file, Some("mka"), &DecodeOptions::default()).unwrap();
        for target in [6000u64, 5096, 9191, 11_999, 100] {
            d.seek(target).unwrap();
            let block = d.next_block().unwrap().unwrap();
            assert_eq!((block.channels[0][0] * 32768.0).round() as i64, target as i64, "seek to {target}");
        }
    }
}

#[cfg(all(test, feature = "ac3-decode"))]
mod ac3_tests {
    use crate::ac3::tests::single_block_eac3;
    use crate::{DecodeOptions, Decoder};

    /// One-block E-AC-3 syncframes may reuse exponents sent long before a seek target.
    #[test]
    fn a_seek_in_one_block_e_ac3_keeps_exponents_sent_before_the_preroll() {
        // Exponents in the first syncframe only, reused by the 19 after it (256 frames each).
        let file = [single_block_eac3(true), single_block_eac3(false).repeat(19)].concat();
        let mut dec = Decoder::open(file, Some("ac3"), &DecodeOptions::default()).unwrap();
        assert_eq!(dec.seek(3000).unwrap(), 3000);
        let block = dec.next_block().unwrap().unwrap();
        let peak = block.channels[0].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 1e-5, "the target decodes with the first syncframe's exponents, not as silence");
    }
}

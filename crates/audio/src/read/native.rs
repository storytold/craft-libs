//! The home-grown decode path: craft-media's own demuxers (ISO BMFF / QuickTime, Matroska / WebM,
//! Ogg, and raw ADTS, MPEG audio and AC-3 streams) feeding its own codecs (AAC, MP1/MP2, AC-3 and
//! E-AC-3, Opus, PCM). [`decode`] and [`probe`] answer `None` for a stream with no home-grown
//! codec (MP3, FLAC, ALAC, Vorbis, ...) or a container our demuxers do not read; the caller then
//! hands the file to symphonia.
//!
//! Timing follows the containers: MP4 edit lists (AAC priming, Opus pre-skip), Matroska
//! `CodecDelay`, and Ogg Opus pre-skip and end trimming (RFC 7845 §4) are applied, so a file
//! decodes to exactly its presentation length.

use super::id3v2_len;
use super::pcm::{MAX_DECODE_SAMPLES, PcmKind, deinterleave};
use crate::{AudioBuffer, AudioError, AudioInfo, ChannelLayout, CodecId, Container, Result, SampleFormat, aac, ac3, mpa, opus};
use craft_media::{isobmff, matroska, ogg};

/// Corrupt packets tolerated before a stream that has produced nothing is malformed.
const MAX_ERRORS: usize = 1000;
/// How far into a raw stream a first frame is looked for.
const MAX_SYNC_SCAN: usize = 64 * 1024;

/// A home-grown packet decoder producing planar f32 in WAV/SMPTE channel order.
enum Codec {
    Aac { dec: Box<aac::Decoder>, up: Option<aac::Upsample2x>, channel_config: u8 },
    Mpa(Box<mpa::Decoder>),
    Ac3(Box<ac3::Decoder>),
    Opus { dec: Box<opus::Decoder>, order: Option<&'static [usize]> },
    Pcm { kind: PcmKind, channels: usize },
}

/// `channels` reordered by `order` (output channel → decoded channel); unchanged when the counts
/// differ.
fn reorder(mut channels: Vec<Vec<f32>>, order: Option<&[usize]>) -> Vec<Vec<f32>> {
    match order {
        Some(order) if order.len() == channels.len() => order.iter().map(|&c| channels.get_mut(c).map(std::mem::take).unwrap_or_default()).collect(),
        _ => channels,
    }
}

/// The speakers of an AAC channel configuration, in the WAV order [`aac::wav_order`] gives;
/// configuration 7 and program-config layouts keep the decoded order and are unordered.
fn aac_layout(channel_config: u8, channels: usize) -> ChannelLayout {
    match channel_config {
        1 => ChannelLayout::MONO,
        2 => ChannelLayout::STEREO,
        3 => ChannelLayout::SURROUND_3_0,
        4 => ChannelLayout::from_mask(0x107),
        5 => ChannelLayout::from_mask(0x607),
        6 => ChannelLayout::SURROUND_5_1_SIDE,
        11 => ChannelLayout::SURROUND_6_1,
        12 => ChannelLayout::SURROUND_7_1,
        _ => ChannelLayout::unordered(channels),
    }
}

/// The speakers of an Opus stream (RFC 7845 §5.1.1): Vorbis order for mapping family 1, reordered
/// to WAV order by [`opus::wav_order`]; other families have no speaker assignment.
fn opus_layout(head: &opus::OpusHead) -> ChannelLayout {
    match (head.mapping_family, head.channels) {
        (0 | 1, 1) => ChannelLayout::MONO,
        (0 | 1, 2) => ChannelLayout::STEREO,
        (1, 3) => ChannelLayout::SURROUND_3_0,
        (1, 4) => ChannelLayout::QUAD,
        (1, 5) => ChannelLayout::SURROUND_5_0,
        (1, 6) => ChannelLayout::SURROUND_5_1,
        (1, 7) => ChannelLayout::SURROUND_6_1,
        (1, 8) => ChannelLayout::SURROUND_7_1,
        (_, n) => ChannelLayout::unordered(n),
    }
}

/// The speakers of an AC-3 / E-AC-3 program (A/52 Table 5.8) in the decoder's WAV order: fronts,
/// LFE, surrounds. Dual mono (1+1) has no speaker assignment.
fn ac3_layout(h: &ac3::Header) -> ChannelLayout {
    let fronts = match h.acmod & 7 {
        0 => return ChannelLayout::unordered(h.channels()),
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x103,
        5 => 0x107,
        6 => 0x603,
        _ => 0x607,
    };
    ChannelLayout::from_mask(fronts | if h.lfeon { 0x8 } else { 0 })
}

/// MPEG audio: mono, or stereo (also for dual channel and joint stereo).
fn mpa_layout(h: &mpa::Header) -> ChannelLayout {
    if h.channels() == 1 { ChannelLayout::MONO } else { ChannelLayout::STEREO }
}

impl Codec {
    /// AAC for `asc` at the stream's output `rate` (twice the core rate for HE-AAC: the decoded
    /// core is upsampled to it).
    fn aac(asc: &[u8], rate: u32) -> Option<Codec> {
        let dec = aac::Decoder::new(asc).ok()?;
        let up = (dec.sample_rate().checked_mul(2) == Some(rate)).then(|| aac::Upsample2x::new(dec.channels()));
        let channel_config = dec.config().channel_config;
        Some(Codec::Aac { dec: Box::new(dec), up, channel_config })
    }

    /// The codec and layout of an AAC stream decoded at `rate`: HE-AAC when `rate` is twice the
    /// core rate.
    fn aac_info(asc: &[u8], rate: u32) -> (CodecId, ChannelLayout) {
        match aac::Decoder::new(asc) {
            Ok(d) => {
                let codec = if d.sample_rate().checked_mul(2) == Some(rate) { CodecId::HeAac } else { CodecId::Aac };
                (codec, aac_layout(d.config().channel_config, d.channels()))
            }
            Err(_) => (CodecId::Aac, ChannelLayout::unordered(0)),
        }
    }

    fn opus(head: opus::OpusHead) -> Option<Codec> {
        let order = if head.mapping_family == 1 { opus::wav_order(head.channels) } else { None };
        let dec = opus::Decoder::from_head(head, 48_000).ok()?;
        Some(Codec::Opus { dec: Box::new(dec), order })
    }

    /// Decode one packet. A packet may hold several MPEG audio or AC-3 frames (MP4, Matroska);
    /// E-AC-3 dependent substreams decode to no channels and are skipped. A corrupt Opus packet
    /// is concealed like a lost one, which keeps its timing.
    fn decode(&mut self, data: &[u8]) -> std::result::Result<Vec<Vec<f32>>, String> {
        match self {
            Codec::Aac { dec, up, channel_config } => {
                let mut out = dec.decode(data).map_err(|e| e.to_string())?;
                if let Some(up) = up {
                    out = up.process(out);
                }
                Ok(reorder(out, aac::wav_order(*channel_config)))
            }
            Codec::Opus { dec, order } => {
                let out = match dec.decode(Some(data)) {
                    Ok(o) => o,
                    Err(_) => dec.decode(None).map_err(|e| e.to_string())?,
                };
                Ok(reorder(out, *order))
            }
            Codec::Mpa(d) => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let mut p = 0;
                while let Some(rest) = data.get(p..).filter(|r| !r.is_empty()) {
                    let f = match d.decode(rest) {
                        Ok(f) => f,
                        Err(e) if out.is_empty() => return Err(e.to_string()),
                        Err(_) => break,
                    };
                    p += f.header.frame_bytes.max(1);
                    append(&mut out, f.channels);
                }
                Ok(out)
            }
            Codec::Ac3(d) => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let mut p = 0;
                while let Some(rest) = data.get(p..).filter(|r| !r.is_empty()) {
                    let f = match d.decode(rest) {
                        Ok(f) => f,
                        Err(e) if out.is_empty() => return Err(e.to_string()),
                        Err(_) => break,
                    };
                    p += f.header.frame_bytes.max(1);
                    if !f.channels.is_empty() {
                        append(&mut out, f.channels);
                    }
                }
                Ok(out)
            }
            Codec::Pcm { kind, channels } => {
                let frames = (data.len() / (kind.bytes() * *channels).max(1)) as u64;
                deinterleave(data, *kind, *channels, frames).map_err(|e| e.to_string())
            }
        }
    }
}

/// Append `more` to `out` channel by channel; a block whose channel count differs is dropped.
fn append(out: &mut Vec<Vec<f32>>, more: Vec<Vec<f32>>) {
    if out.is_empty() {
        *out = more;
    } else if out.len() == more.len() {
        for (o, c) in out.iter_mut().zip(more) {
            o.extend(c);
        }
    }
}

/// Collects decoded blocks: drops the leading `skip` frames, stops after `limit` frames, keeps
/// count only when `count_only`, and enforces the decode limit.
struct Sink {
    planar: Vec<Vec<f32>>,
    frames: u64,
    skip: u64,
    limit: Option<u64>,
    count_only: bool,
    errors: usize,
}

impl Sink {
    fn new(skip: u64, limit: Option<u64>, count_only: bool) -> Sink {
        Sink { planar: Vec::new(), frames: 0, skip, limit, count_only, errors: 0 }
    }

    fn done(&self) -> bool {
        self.limit.is_some_and(|l| self.frames >= l)
    }

    /// Note a corrupt packet; a stream that never decodes is malformed.
    fn error(&mut self, msg: &str) -> Result<()> {
        self.errors += 1;
        log::debug!("skipping corrupt packet: {msg}");
        if self.errors > MAX_ERRORS && self.frames == 0 {
            return Err(AudioError::Malformed(format!("no decodable packets: {msg}")));
        }
        Ok(())
    }

    /// Add a decoded block, less its last `trim_end` frames (Matroska `DiscardPadding`).
    fn push(&mut self, mut block: Vec<Vec<f32>>, trim_end: u64) -> Result<()> {
        let ch = block.len();
        if ch == 0 || self.done() {
            return Ok(());
        }
        if !self.planar.is_empty() && self.planar.len() != ch {
            return self.error("the channel count changed mid-stream");
        }
        let mut n = (block.iter().map(Vec::len).min().unwrap_or(0) as u64).saturating_sub(trim_end);
        let drop = self.skip.min(n);
        self.skip -= drop;
        n -= drop;
        if let Some(l) = self.limit {
            n = n.min(l.saturating_sub(self.frames));
        }
        if self.planar.is_empty() {
            self.planar = vec![Vec::new(); ch];
        }
        let total = self.frames.saturating_add(n);
        if total.saturating_mul(ch as u64) > MAX_DECODE_SAMPLES {
            return Err(AudioError::TooLarge("decoded audio exceeds the 4 GiB limit".into()));
        }
        self.frames = total;
        if self.count_only {
            return Ok(());
        }
        let (from, to) = (drop as usize, (drop + n) as usize);
        for (dst, src) in self.planar.iter_mut().zip(block.iter_mut()) {
            dst.extend(src.get(from..to).unwrap_or_default().iter().map(|&s| if s.is_finite() { s } else { 0.0 }));
        }
        Ok(())
    }

    fn finish(self, plan: &Plan<'_>) -> Result<(AudioInfo, AudioBuffer)> {
        if self.planar.is_empty() {
            return Err(AudioError::Malformed("stream contains no audio".into()));
        }
        let channels = self.planar.len();
        u16::try_from(channels).map_err(|_| AudioError::Unsupported("too many channels".into()))?;
        // The decoded channel count is authoritative; a layout that disagrees with it is dropped.
        let layout = if plan.layout.channels() == channels { plan.layout } else { ChannelLayout::unordered(channels) };
        let frames = if self.count_only { self.frames } else { self.planar.iter().map(Vec::len).min().unwrap_or(0) as u64 };
        let sample_rate = plan.sample_rate;
        let info = AudioInfo::new(plan.container, plan.codec_id, plan.sample_format, sample_rate, layout, frames);
        let channels = if self.count_only { vec![Vec::new(); channels] } else { self.planar };
        Ok((info, AudioBuffer { sample_rate, channels }))
    }
}

/// A stream the home-grown path decodes: where its packets come from and how to decode them.
struct Plan<'a> {
    codec: Codec,
    packets: Packets<'a>,
    container: Container,
    codec_id: CodecId,
    /// The speakers of the decoded channels, from the codec configuration.
    layout: ChannelLayout,
    sample_format: SampleFormat,
    sample_rate: u32,
    /// Frames (at `sample_rate`) to drop from the start.
    skip: u64,
    /// Frames to keep in all, when the container says.
    limit: Option<u64>,
    /// Presented frames from headers and sample tables, for [`probe`]; `None` when unknown.
    frames: Option<u64>,
}

enum Packets<'a> {
    Mp4 {
        file: Box<isobmff::Mp4File>,
        bytes: &'a [u8],
        track: usize,
        next: usize,
    },
    /// Matroska is read packet by packet, which carries each block's `DiscardPadding`.
    Mkv {
        demux: Box<matroska::Demuxer<&'a [u8]>>,
        track: usize,
        rate: u32,
    },
    Ogg {
        file: ogg::OggFile,
        bytes: &'a [u8],
        stream: usize,
        next: usize,
    },
    /// Frames of a raw stream: `(offset, length)` in `bytes`.
    Raw {
        frames: Vec<(usize, usize)>,
        bytes: &'a [u8],
        next: usize,
    },
}

/// A packet, or why it could not be read.
type NextPacket = std::result::Result<Packet, String>;

/// A packet's bytes and the frames to drop from the end of its decoded audio.
struct Packet {
    data: Vec<u8>,
    trim_end: u64,
}

impl Packets<'_> {
    /// The next packet of the stream, or `None` at its end.
    fn next(&mut self) -> Option<NextPacket> {
        let whole = |data: Vec<u8>| Packet { data, trim_end: 0 };
        match self {
            Packets::Mp4 { file, bytes, track, next } => {
                let i = *next;
                (i < file.tracks.get(*track).map_or(0, |t| t.samples.len())).then(|| {
                    *next += 1;
                    file.read_sample(*bytes, *track, i).map(whole).map_err(|e| e.to_string())
                })
            }
            Packets::Ogg { file, bytes, stream, next } => {
                let i = *next;
                (i < file.streams.get(*stream).map_or(0, |s| s.packets.len())).then(|| {
                    *next += 1;
                    file.read_packet(*bytes, *stream, i).map(whole).map_err(|e| e.to_string())
                })
            }
            Packets::Raw { frames, bytes, next } => {
                let &(at, len) = frames.get(*next)?;
                *next += 1;
                Some(bytes.get(at..at.checked_add(len)?).map(|d| whole(d.to_vec())).ok_or_else(|| "frame out of range".into()))
            }
            Packets::Mkv { demux, track, rate } => loop {
                match demux.next_packet() {
                    Ok(Some(p)) if p.track == *track => {
                        let pad = p.discard_padding_ns.and_then(|ns| u64::try_from(ns).ok()).unwrap_or(0);
                        break Some(Ok(Packet { data: p.data, trim_end: rescale_round(pad, 1_000_000_000, u64::from(*rate)) }));
                    }
                    Ok(Some(_)) => continue,
                    Ok(None) => break None,
                    Err(e) => break Some(Err(e.to_string())),
                }
            },
        }
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

/// Frame lengths of a raw stream of self-delimiting frames from `at`: `frame_len(rest)` gives the
/// length of a frame starting at `rest`, or `None`. The first frame must be followed by a second
/// (or the end of the data), so a stray sync word does not make a stream; damage later is skipped
/// by scanning for the next pair.
fn raw_frames(b: &[u8], at: usize, frame_len: impl Fn(&[u8]) -> Option<usize>) -> Option<Vec<(usize, usize)>> {
    let len_at = |p: usize| b.get(p..).and_then(&frame_len).filter(|&n| n > 0 && p.checked_add(n).is_some_and(|e| e <= b.len()));
    let paired = |p: usize| len_at(p).is_some_and(|n| p + n == b.len() || len_at(p + n).is_some());
    let first = (at..b.len().min(at.saturating_add(MAX_SYNC_SCAN))).find(|&p| paired(p))?;
    let mut frames = Vec::new();
    let mut p = first;
    while p < b.len() {
        match len_at(p) {
            Some(n) => {
                frames.push((p, n));
                p += n;
            }
            None => match (p + 1..b.len()).find(|&q| paired(q)) {
                Some(q) => p = q,
                None => break,
            },
        }
    }
    Some(frames)
}

/// Frames of `kind` (1 = layer I, 2 = layer II) of a raw MPEG audio stream.
fn mpa_len(rest: &[u8]) -> Option<usize> {
    mpa::parse_header(rest).ok().filter(|h| h.layer == 1 || h.layer == 2).map(|h| h.frame_bytes)
}

fn adts_len(rest: &[u8]) -> Option<usize> {
    aac::AdtsHeader::parse(rest).ok().filter(|h| h.frame_length > h.header_length).map(|h| h.frame_length)
}

fn ac3_len(rest: &[u8]) -> Option<usize> {
    ac3::parse_header(rest).ok().map(|h| h.frame_bytes)
}

/// A raw ADTS AAC stream.
fn plan_adts(b: &[u8]) -> Option<Plan<'_>> {
    let frames = raw_frames(b, after_id3(b), adts_len)?;
    let &(at, _) = frames.first()?;
    let h = aac::AdtsHeader::parse(b.get(at..)?).ok()?;
    let asc = h.to_asc().to_bytes();
    // The payloads of the first frames, to find implicitly signalled HE-AAC.
    let units: Vec<&[u8]> = frames.iter().take(8).filter_map(|&(p, n)| b.get(p + h.header_length..p + n)).collect();
    let rate = aac::output_rate(&asc, units.iter().copied())?;
    let payloads: Vec<(usize, usize)> =
        frames.iter().filter_map(|&(p, n)| aac::AdtsHeader::parse(b.get(p..)?).ok().map(|f| (p + f.header_length, n - f.header_length))).collect();
    let per_frame = rescale(aac::FRAME_LEN as u64, u64::from(h.to_asc().sample_rate), u64::from(rate));
    let (codec_id, layout) = Codec::aac_info(&asc, rate);
    Some(Plan {
        codec: Codec::aac(&asc, rate)?,
        codec_id,
        layout,
        frames: Some(payloads.len() as u64 * per_frame),
        packets: Packets::Raw { frames: payloads, bytes: b, next: 0 },
        container: Container::Adts,
        sample_format: SampleFormat::Compressed,
        sample_rate: rate,
        skip: 0,
        limit: None,
    })
}

/// A raw MPEG audio layer I or II stream.
fn plan_mpa(b: &[u8]) -> Option<Plan<'_>> {
    let frames = raw_frames(b, after_id3(b), mpa_len)?;
    let h = mpa::parse_header(b.get(frames.first()?.0..)?).ok()?;
    let total = frames.iter().filter_map(|&(p, _)| mpa::parse_header(b.get(p..)?).ok()).map(|f| f.samples() as u64).sum();
    Some(Plan {
        codec: Codec::Mpa(Box::default()),
        codec_id: if h.layer == 1 { CodecId::Mp1 } else { CodecId::Mp2 },
        layout: mpa_layout(&h),
        frames: Some(total),
        packets: Packets::Raw { frames, bytes: b, next: 0 },
        container: Container::MpegAudio,
        sample_format: SampleFormat::Compressed,
        sample_rate: h.sample_rate,
        skip: 0,
        limit: None,
    })
}

/// A raw AC-3 or E-AC-3 stream.
fn plan_ac3(b: &[u8]) -> Option<Plan<'_>> {
    let frames = raw_frames(b, after_id3(b), ac3_len)?;
    let h = ac3::parse_header(b.get(frames.first()?.0..)?).ok()?;
    let total =
        frames.iter().filter_map(|&(p, _)| ac3::parse_header(b.get(p..)?).ok()).filter(ac3::Header::is_primary).map(|f| f.samples() as u64).sum();
    Some(Plan {
        codec: Codec::Ac3(Box::default()),
        codec_id: if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 },
        layout: ac3_layout(&h),
        frames: Some(total),
        packets: Packets::Raw { frames, bytes: b, next: 0 },
        container: Container::Ac3,
        sample_format: SampleFormat::Compressed,
        sample_rate: h.sample_rate,
        skip: 0,
        limit: None,
    })
}

/// The MPEG audio layer of the first frame of `data`.
fn mpeg_layer(data: &[u8]) -> Option<u8> {
    mpa::parse_header(data).ok().map(|h| h.layer)
}

fn pcm_kind(bits: u16, float: bool, big_endian: bool, unsigned: bool) -> Option<(PcmKind, SampleFormat)> {
    let bytes = usize::from(bits.div_ceil(8));
    match (float, bits) {
        (true, 32) => Some((PcmKind::Float { bytes: 4, big_endian }, SampleFormat::Float32)),
        (true, 64) => Some((PcmKind::Float { bytes: 8, big_endian }, SampleFormat::Float64)),
        (false, 1..=32) => {
            let sf = match bits {
                1..=8 => SampleFormat::Int8,
                9..=16 => SampleFormat::Int16,
                17..=24 => SampleFormat::Int24,
                _ => SampleFormat::Int32,
            };
            Some((PcmKind::Int { bytes, big_endian, unsigned }, sf))
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

/// The most frames a packet may declare (as on the symphonia path): decoders allocate a packet's
/// frames up front.
const MAX_PACKET_FRAMES: u64 = 1 << 20;

/// An MP4 / QuickTime file's first audio track, if its codec is home-grown. Its atom sizes are
/// checked against the file first, as on the symphonia path: a file that lies about them is
/// refused rather than read leniently.
fn plan_mp4(b: &[u8], container: Container) -> Option<Result<Plan<'_>>> {
    if let Err(e) = super::mp4::check(b, 0) {
        return Some(Err(e));
    }
    let file = isobmff::open(b).ok()?;
    let track = file.track_of_kind(isobmff::TrackKind::Audio)?;
    let t = file.tracks.get(track)?;
    if t.audio().is_some_and(|a| u64::from(a.samples_per_packet.max(a.const_frames_per_packet)) > MAX_PACKET_FRAMES) {
        return Some(Err(AudioError::Malformed("the file declares packets far longer than any codec uses".into())));
    }
    let first = file.read_sample(b, track, 0).ok();
    let (codec, rate, sample_format, codec_id, layout) = match t.codec()? {
        isobmff::CodecConfig::Aac(a) => {
            let units: Vec<Vec<u8>> = (0..t.samples.len().min(8)).filter_map(|i| file.read_sample(b, track, i).ok()).collect();
            let rate = aac::output_rate(&a.asc, units.iter().map(Vec::as_slice))?;
            let (id, layout) = Codec::aac_info(&a.asc, rate);
            (Codec::aac(&a.asc, rate)?, rate, SampleFormat::Compressed, id, layout)
        }
        isobmff::CodecConfig::Mp3 if matches!(first.as_deref().and_then(mpeg_layer), Some(1 | 2)) => {
            let h = mpa::parse_header(first.as_deref()?).ok()?;
            let id = if h.layer == 1 { CodecId::Mp1 } else { CodecId::Mp2 };
            (Codec::Mpa(Box::default()), h.sample_rate, SampleFormat::Compressed, id, mpa_layout(&h))
        }
        isobmff::CodecConfig::Ac3 { .. } | isobmff::CodecConfig::Eac3 { .. } => {
            let h = ac3::parse_header(first.as_deref()?).ok()?;
            let id = if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 };
            (Codec::Ac3(Box::default()), h.sample_rate, SampleFormat::Compressed, id, ac3_layout(&h))
        }
        isobmff::CodecConfig::Opus(o) => {
            let head = opus::OpusHead::from_dops(&o.to_bytes()).ok()?;
            let layout = opus_layout(&head);
            (Codec::opus(head)?, 48_000, SampleFormat::Compressed, CodecId::Opus, layout)
        }
        isobmff::CodecConfig::Pcm(p) => {
            let (kind, sf) = pcm_kind(p.bits, p.float, p.big_endian, !p.signed)?;
            let channels = usize::try_from(p.channels).ok().filter(|&c| c > 0)?;
            let rate = p.sample_rate.round() as u32;
            (Codec::Pcm { kind, channels }, rate, sf, CodecId::Pcm, ChannelLayout::default_for(channels))
        }
        _ => return None,
    };
    // A leading media edit (AAC priming, Opus pre-skip) is skipped, and the edit's duration is the
    // presentation length. Empty edits and multi-segment lists play the media as it is.
    let (skip, limit) = match t.edits.as_slice() {
        [e] if e.media_time >= 0 && e.media_rate == 0x0001_0000 => {
            let skip = rescale(e.media_time.unsigned_abs(), u64::from(t.timescale), u64::from(rate));
            let limit = (e.segment_duration > 0).then(|| rescale(e.segment_duration, u64::from(file.timescale), u64::from(rate)));
            (skip, limit)
        }
        _ => (0, None),
    };
    let media: u64 = t.samples.iter().map(|s| u64::from(s.duration)).sum();
    let frames = limit.unwrap_or_else(|| rescale(media, u64::from(t.timescale), u64::from(rate)).saturating_sub(skip));
    Some(Ok(Plan {
        codec,
        packets: Packets::Mp4 { file: Box::new(file), bytes: b, track, next: 0 },
        container,
        codec_id,
        layout,
        sample_format,
        sample_rate: rate,
        skip,
        limit,
        frames: Some(frames),
    }))
}

/// A Matroska / WebM file's first audio track, if its codec is home-grown.
fn plan_mkv(b: &[u8], container: Container) -> Option<Plan<'_>> {
    let demux = matroska::Demuxer::from_slice(b).ok()?;
    let file = demux.file();
    let track = file.track_of_kind(matroska::TrackKind::Audio)?;
    let t = file.tracks.get(track)?;
    if !t.frames_readable() {
        return None;
    }
    let first = demux.read_sample(track, 0).ok();
    let declared = t.audio.as_ref().map_or(0.0, |a| a.sampling_frequency).round() as u32;
    let declared_channels = t.audio.as_ref().and_then(|a| u16::try_from(a.channels).ok()).unwrap_or(0);
    let (codec, rate, sample_format, codec_id, layout) = match &t.codec {
        matroska::Codec::Aac { asc } => {
            let units: Vec<Vec<u8>> = (0..t.samples.len().min(8)).filter_map(|i| demux.read_sample(track, i).ok()).collect();
            let rate = aac::output_rate(asc, units.iter().map(Vec::as_slice))?;
            let (id, layout) = Codec::aac_info(asc, rate);
            (Codec::aac(asc, rate)?, rate, SampleFormat::Compressed, id, layout)
        }
        matroska::Codec::Mp1 | matroska::Codec::Mp2 => {
            let h = mpa::parse_header(first.as_deref()?).ok()?;
            let id = if h.layer == 1 { CodecId::Mp1 } else { CodecId::Mp2 };
            (Codec::Mpa(Box::default()), h.sample_rate, SampleFormat::Compressed, id, mpa_layout(&h))
        }
        matroska::Codec::Ac3 | matroska::Codec::Eac3 => {
            let h = ac3::parse_header(first.as_deref()?).ok()?;
            let id = if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 };
            (Codec::Ac3(Box::default()), h.sample_rate, SampleFormat::Compressed, id, ac3_layout(&h))
        }
        matroska::Codec::Opus { head } => {
            let head = opus::OpusHead::parse(head).ok()?;
            let layout = opus_layout(&head);
            (Codec::opus(head)?, 48_000, SampleFormat::Compressed, CodecId::Opus, layout)
        }
        matroska::Codec::Pcm { float, big_endian, bits } => {
            let (kind, sf) = pcm_kind(*bits, *float, *big_endian, *bits <= 8 && !*float)?;
            let channels = usize::from(declared_channels).max(1);
            (Codec::Pcm { kind, channels }, declared, sf, CodecId::Pcm, ChannelLayout::default_for(channels))
        }
        _ => return None,
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
    Some(Plan {
        codec,
        packets: Packets::Mkv { demux: Box::new(demux), track, rate },
        container,
        codec_id,
        layout,
        sample_format,
        sample_rate: rate,
        skip,
        limit: frames,
        frames,
    })
}

/// An Ogg Opus file's first Opus stream (RFC 7845: pre-skip and end trimming applied).
fn plan_ogg(b: &[u8]) -> Option<Plan<'_>> {
    let file = ogg::open(b).ok()?;
    let stream = file.stream_of(ogg::Codec::Opus)?;
    let s = file.streams.get(stream)?;
    let head = opus::OpusHead::parse(s.headers.first()?).ok()?;
    let timing = ogg::OpusTiming::of(s, u32::from(head.pre_skip));
    let limit = u64::try_from(timing.total).ok();
    let layout = opus_layout(&head);
    Some(Plan {
        codec: Codec::opus(head)?,
        packets: Packets::Ogg { file, bytes: b, stream, next: 0 },
        container: Container::Ogg,
        codec_id: CodecId::Opus,
        layout,
        sample_format: SampleFormat::Compressed,
        sample_rate: 48_000,
        skip: u64::from(timing.pre_skip),
        limit,
        frames: limit,
    })
}

/// The home-grown plan for `b`, by its container and first frames.
fn plan(b: &[u8], detected: Option<Container>) -> Option<Result<Plan<'_>>> {
    let at = after_id3(b);
    let head = b.get(at..).unwrap_or_default();
    // The bytes decide the container when they say; the detection (which may come from the
    // extension) only picks among raw streams.
    match super::detect_container(b, None).or(detected) {
        Some(Container::Ogg) if ogg::sniff(head) => plan_ogg(b).map(Ok),
        Some(c @ (Container::Mp4 | Container::QuickTime)) if b.get(4..8) == Some(b"ftyp") => plan_mp4(b, c),
        Some(c @ (Container::Matroska | Container::WebM)) if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) => plan_mkv(b, c).map(Ok),
        Some(Container::Adts) => plan_adts(b).map(Ok),
        Some(Container::MpegAudio) => plan_mpa(b).map(Ok),
        Some(Container::Ac3) => plan_ac3(b).map(Ok),
        None => plan_ac3(b).or_else(|| plan_adts(b)).or_else(|| plan_mpa(b)).map(Ok),
        _ => None,
    }
}

/// Decode every packet of `plan` into `sink`.
fn run(plan: &mut Plan<'_>, sink: &mut Sink) -> Result<()> {
    while let Some(next) = plan.packets.next() {
        if sink.done() {
            break;
        }
        let packet = match next {
            Ok(p) => p,
            Err(e) => {
                sink.error(&e)?;
                continue;
            }
        };
        match plan.codec.decode(&packet.data) {
            Ok(block) => sink.push(block, packet.trim_end)?,
            Err(e) => sink.error(&e)?,
        }
    }
    Ok(())
}

/// Decode `b` with the home-grown demuxers and codecs; `None` when they do not handle it.
pub(crate) fn decode(b: &[u8], detected: Option<Container>, count_only: bool) -> Option<Result<(AudioInfo, AudioBuffer)>> {
    let mut plan = match plan(b, detected)? {
        Ok(p) => p,
        Err(e) => return Some(Err(e)),
    };
    let mut sink = Sink::new(plan.skip, plan.limit, count_only);
    Some(run(&mut plan, &mut sink).and_then(|()| sink.finish(&plan)))
}

/// Probe `b` with the home-grown demuxers and codecs: channels and length from the frame headers
/// or sample tables, decoding only when those do not say; `None` when they do not handle it.
pub(crate) fn probe(b: &[u8], detected: Option<Container>) -> Option<Result<AudioInfo>> {
    let plan = match plan(b, detected)? {
        Ok(p) => p,
        Err(e) => return Some(Err(e)),
    };
    match plan.frames {
        Some(frames) if plan.layout.channels() > 0 && plan.sample_rate > 0 => {
            Some(Ok(AudioInfo::new(plan.container, plan.codec_id, plan.sample_format, plan.sample_rate, plan.layout, frames)))
        }
        _ => decode(b, detected, true).map(|r| r.map(|(info, _)| info)),
    }
}

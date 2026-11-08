//! [`TrackDecoder`]: one track's packets in, planar `f32` in WAV channel order out, whatever the
//! codec. The file-level [`crate::decode`] and [`crate::Decoder`] use it for every native codec;
//! an app with its own demuxer (MXF, MPEG-TS, a network stream) uses it directly.

#[cfg(feature = "aac-decode")]
use crate::aac;
#[cfg(feature = "ac3-decode")]
use crate::ac3;
#[cfg(feature = "opus-decode")]
use crate::opus;
use crate::pcm::{PcmKind, deinterleave};
use crate::{AudioBuffer, AudioError, ChannelLayout, CodecId, CodecParams, Result, SampleFormat};

/// How many leading packets [`TrackDecoder::new`] looks at.
pub const LOOKAHEAD_PACKETS: usize = 8;

/// Bound packet work even when every syncframe is a skipped dependent programme.
#[cfg(feature = "ac3-decode")]
pub(crate) const MAX_AC3_PACKET_SYNCFRAMES: usize = 4096;

/// The codec-specific decoder behind a [`TrackDecoder`].
enum Inner {
    #[cfg(feature = "aac-decode")]
    Aac {
        dec: Box<aac::Decoder>,
        up: Option<aac::Upsample2x>,
        order: Option<&'static [usize]>,
        /// The format ADTS packets must keep, when their headers gave the configuration.
        adts_format: Option<AdtsFormat>,
    },
    #[cfg(feature = "ac3-decode")]
    Ac3 {
        dec: Box<ac3::Decoder>,
        format: Option<Ac3Format>,
    },
    #[cfg(feature = "opus-decode")]
    Opus {
        dec: Box<opus::Decoder>,
        order: Option<&'static [usize]>,
    },
    Pcm {
        kind: PcmKind,
        channels: usize,
    },
}

/// Decodes the packets of one audio track to planar `f32`, in WAV channel order when
/// [`TrackDecoder::layout`] is ordered.
///
/// It takes the track's [`CodecParams`] (from `isobmff::Track::codec_params`,
/// `matroska::Track::codec_params`, `ogg::Stream::codec_params`, or built by hand for another
/// container) and decodes AAC (LC; HE-AAC as its upsampled LC core), AC-3/E-AC-3, Opus and PCM.
/// Container timing (AAC priming, Opus pre-skip, end trimming) is the caller's: every decoded
/// sample is returned.
///
/// ```no_run
/// # fn demo(params: &craft_audio::CodecParams, packets: &[Vec<u8>]) -> craft_audio::Result<()> {
/// use craft_audio::TrackDecoder;
///
/// let lookahead: Vec<&[u8]> = packets.iter().take(8).map(Vec::as_slice).collect();
/// let mut dec = TrackDecoder::new(params, &lookahead)?;
/// println!("{} at {} Hz, {}", dec.codec().name(), dec.sample_rate(), dec.layout());
/// for p in packets {
///     let block = match dec.decode(p) {
///         Ok(block) => block,
///         Err(_) => dec.conceal(Some(p))?, // keep the timing across a damaged packet
///     };
///     // block.channels: one Vec<f32> per channel
/// }
/// # Ok(()) }
/// ```
pub struct TrackDecoder {
    inner: Inner,
    codec: CodecId,
    sample_rate: u32,
    layout: ChannelLayout,
    sample_format: SampleFormat,
    /// Frames of the last decoded packet, the length of a concealed one when the codec does not
    /// say.
    last_frames: usize,
}

impl std::fmt::Debug for TrackDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackDecoder")
            .field("codec", &self.codec)
            .field("sample_rate", &self.sample_rate)
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl TrackDecoder {
    /// A decoder for the track `params` describes. `lookahead` holds the track's first packets
    /// (up to [`LOOKAHEAD_PACKETS`]; empty when they are not at hand): HE-AAC signalled only
    /// inside the stream is found in them, and AC-3 takes its rate and channel layout from the
    /// first frame. Without them those are taken from `params` and set
    /// from the first decoded packet.
    ///
    /// Streams without codec configuration are accepted where the packets carry it: AAC with
    /// empty `extradata` takes its configuration from the ADTS headers of the lookahead (MPEG-TS,
    /// raw `.aac`), the one most of them share, and ADTS headers are stripped from every packet;
    /// Opus with empty `extradata` and 1 or 2 `channels` (RTP, MPEG-TS) needs no `OpusHead`. Other
    /// missing configuration is [`AudioError::Malformed`].
    ///
    /// Codecs without a native packet decoder yet (MP3, FLAC, ALAC, Vorbis) are
    /// [`AudioError::Unsupported`]; `craft_audio::decode` decodes them through symphonia when the
    /// `symphonia` feature is on.
    pub fn new(params: &CodecParams, lookahead: &[&[u8]]) -> Result<TrackDecoder> {
        let rate = params.sample_rate.unwrap_or(0);
        let channels = usize::from(params.channels.unwrap_or(0));
        let (inner, codec, sample_rate, layout, sample_format) = match params.codec {
            #[cfg(feature = "aac-decode")]
            CodecId::Aac | CodecId::HeAac => {
                // ADTS-framed packets (MPEG-TS, raw `.aac`) carry their configuration in each
                // header: without extradata, the headers of the lookahead give it.
                let payloads: Vec<&[u8]> = lookahead
                    .iter()
                    .take(LOOKAHEAD_PACKETS)
                    .flat_map(|p| adts_frames(p).map_or_else(|| vec![*p], |frames| frames.into_iter().map(|(_, unit)| unit).collect()))
                    .collect();
                let mut adts_format = None;
                let asc = if params.extradata.is_empty() {
                    let headers: Vec<aac::AdtsHeader> = lookahead.iter().filter_map(|p| aac::AdtsHeader::parse(p).ok()).collect();
                    let h = adts_stream_header(&headers).ok_or_else(|| {
                        AudioError::Malformed("AAC track without an AudioSpecificConfig (extradata), and its packets are not ADTS".into())
                    })?;
                    adts_format = Some(adts_format_of(&h));
                    adts_config(&h, &payloads)?
                } else {
                    params.extradata.clone()
                };
                let asc = asc.as_slice();
                let dec = aac::Decoder::new(asc)?;
                let out = aac::output_rate(asc, payloads.iter().copied()).unwrap_or(dec.sample_rate());
                let up = (dec.sample_rate().checked_mul(2) == Some(out)).then(|| aac::Upsample2x::new(dec.channels()));
                let codec = if up.is_some() { CodecId::HeAac } else { CodecId::Aac };
                // Only twice the core rate is upsampled to; any other signalled rate plays the core.
                let out = if up.is_some() { out } else { dec.sample_rate() };
                let cfg = dec.config().channel_config;
                let layout = aac_layout(cfg, dec.channels());
                let order = layout.is_ordered().then(|| aac::wav_order(cfg)).flatten();
                (Inner::Aac { dec: Box::new(dec), up, order, adts_format }, codec, out, layout, SampleFormat::Compressed)
            }
            #[cfg(feature = "ac3-decode")]
            CodecId::Ac3 | CodecId::Eac3 => match lookahead
                .iter()
                .take(LOOKAHEAD_PACKETS)
                .find_map(|p| first_ac3_primary(p).transpose())
                .transpose()?
            {
                Some(h) => {
                    let codec = if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 };
                    (Inner::Ac3 { dec: Box::default(), format: Some(ac3_format(&h)) }, codec, h.sample_rate, ac3_layout(&h), SampleFormat::Compressed)
                }
                None => (
                    Inner::Ac3 { dec: Box::default(), format: None },
                    params.codec,
                    rate,
                    ChannelLayout::unordered(crate::layout::count(channels)),
                    SampleFormat::Compressed,
                ),
            },
            #[cfg(feature = "opus-decode")]
            CodecId::Opus => {
                // Opus over RTP or in MPEG-TS has no OpusHead: mono and stereo need none (RFC 7845
                // mapping family 0), so one is made from the channel count.
                let head = match (params.extradata.is_empty(), channels) {
                    (false, _) => opus::OpusHead::parse(&params.extradata)?,
                    (true, 1 | 2) => opus::OpusHead::parse(&default_opus_head(channels as u8, rate))?,
                    (true, _) => {
                        return Err(AudioError::Malformed("Opus track without an OpusHead (extradata) needs 1 or 2 channels".into()));
                    }
                };
                let layout = opus_layout(&head);
                let order = if head.mapping_family == 1 { opus::wav_order(head.channels) } else { None };
                let dec = opus::Decoder::from_head(head, 48_000)?;
                (Inner::Opus { dec: Box::new(dec), order }, CodecId::Opus, 48_000, layout, SampleFormat::Compressed)
            }
            CodecId::Pcm => {
                let pcm = params.pcm.ok_or_else(|| AudioError::Malformed("PCM track without a sample format".into()))?;
                let (kind, sample_format) = pcm_kind(pcm.bits, pcm.float, pcm.big_endian, !pcm.signed)
                    .ok_or_else(|| AudioError::Unsupported(format!("{}-bit PCM", pcm.bits)))?;
                if channels == 0 || rate == 0 {
                    return Err(AudioError::Malformed("PCM track without channels or a sample rate".into()));
                }
                (Inner::Pcm { kind, channels }, CodecId::Pcm, rate, ChannelLayout::default_for(crate::layout::count(channels)), sample_format)
            }
            other => return Err(unsupported(other)),
        };
        Ok(TrackDecoder { inner, codec, sample_rate, layout, sample_format, last_frames: 0 })
    }

    /// Native container readers must establish AC-3 metadata before caching their timeline.
    #[cfg(feature = "decode-native")]
    pub(crate) fn stream_format_known(&self) -> bool {
        match &self.inner {
            Inner::Ac3 { format, .. } => format.is_some(),
            _ => true,
        }
    }

    /// The codec, refined from the stream: [`CodecId::HeAac`] for AAC with spectral band
    /// replication, `Eac3` for Dolby Digital Plus.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// The rate of the decoded audio (for HE-AAC, twice the AAC-LC core rate).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The speakers of the decoded channels; ordered layouts come in WAV order.
    pub fn layout(&self) -> ChannelLayout {
        self.layout
    }

    /// The number of decoded channels.
    pub fn channels(&self) -> u16 {
        self.layout.channels()
    }

    /// The stored sample format: the PCM depth, or [`SampleFormat::Compressed`].
    pub fn sample_format(&self) -> SampleFormat {
        self.sample_format
    }

    /// Decode one packet. An MP4 or Matroska packet may hold several AC-3 frames;
    /// an E-AC-3 dependent substream decodes to no frames. A packet that fails is an error for
    /// every codec: call [`TrackDecoder::conceal`] for audio of its length, so what follows keeps
    /// its timing.
    pub fn decode(&mut self, packet: &[u8]) -> Result<AudioBuffer> {
        let channels = match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { dec, up, order, adts_format } => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let units = match adts_frames(packet) {
                    Some(frames) => adts_units(frames, *adts_format)?,
                    None => vec![packet],
                };
                // A failed frame leaves the decoder as it was, so the frames before it in the
                // packet must too: their audio is replaced by the packet's concealment.
                let before = (units.len() > 1).then(|| (dec.clone(), up.clone()));
                let decoded = units.into_iter().try_for_each(|unit| {
                    let mut block = dec.decode(unit)?;
                    if let Some(up) = up.as_mut() {
                        block = up.process(block);
                    }
                    append(&mut out, block)
                });
                if let Err(e) = decoded {
                    if let Some(before) = before {
                        (*dec, *up) = before;
                    }
                    return Err(e);
                }
                reorder(out, *order)
            }
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, order } => {
                let out = dec.decode(Some(packet))?;
                reorder(out, *order)
            }
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3 { dec, format } => {
                // Packet delivery is atomic: the caller may retry, or explicitly conceal the
                // failed packet. Neither earlier frames nor metadata escape a later failure.
                let before = dec.clone();
                let mut candidate = *format;
                let mut metadata = None;
                let decoded = (|| {
                    let mut out = Vec::new();
                    let mut rest = packet;
                    let mut syncframes = 0;
                    while !rest.is_empty() {
                        count_ac3_syncframe(&mut syncframes)?;
                        let f = dec.decode(rest)?;
                        rest = rest.get(f.header.frame_bytes..).ok_or_else(|| AudioError::Malformed("truncated AC-3 packet".into()))?;
                        if f.header.is_primary() {
                            let current = ac3_format(&f.header);
                            if candidate.is_some_and(|known| known != current) {
                                return Err(AudioError::Malformed("AC-3 track changes codec, sample rate or speaker layout".into()));
                            }
                            candidate = Some(current);
                            metadata = Some(f.header);
                            append_ac3(&mut out, f.channels, crate::pcm::MAX_DECODE_SAMPLES)?;
                        }
                    }
                    Ok(out)
                })();
                let out = match decoded {
                    Ok(out) => out,
                    Err(e) => {
                        *dec = before;
                        return Err(e);
                    }
                };
                *format = candidate;
                if let Some(h) = metadata {
                    self.codec = if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 };
                    self.sample_rate = h.sample_rate;
                    self.layout = ac3_layout(&h);
                }
                out
            }
            Inner::Pcm { kind, channels } => {
                let frames = (packet.len() / (kind.bytes() * *channels).max(1)) as u64;
                deinterleave(packet, *kind, *channels, frames)?
            }
        };
        if let Some(n) = channels.iter().map(Vec::len).min().filter(|&n| n > 0) {
            self.last_frames = n;
            if channels.len() != usize::from(self.layout.channels()) {
                self.layout = ChannelLayout::unordered(crate::layout::count(channels.len()));
            }
        }
        Ok(AudioBuffer { sample_rate: self.sample_rate, channels })
    }

    /// What stands in for a lost or undecodable packet (`packet` when its bytes are at hand):
    /// Opus's loss concealment, or silence as long as the packet would have been (from its frame
    /// header, the codec's frame length, or the last packet). Empty when the length is unknown.
    /// Fails with [`AudioError::TooLarge`](crate::AudioError::TooLarge) when that silence would pass
    /// the decode limit (2^30 samples, as [`decode`](Self::decode) has) or cannot be allocated,
    /// rather than return less than the packet's length.
    pub fn conceal(&mut self, packet: Option<&[u8]>) -> Result<AudioBuffer> {
        let data = packet.unwrap_or_default();
        let frames = match &mut self.inner {
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, order } => {
                let order = *order;
                if let Ok(out) = dec.decode(None) {
                    return Ok(AudioBuffer { sample_rate: self.sample_rate, channels: reorder(out, order) });
                }
                None
            }
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => {
                let per_unit = aac::FRAME_LEN * if up.is_some() { 2 } else { 1 };
                match adts_frames(data) {
                    Some(f) => Some(f.iter().map(|(h, _)| usize::from(h.raw_data_blocks)).sum::<usize>().min(MAX_ADTS_FRAMES_PER_PACKET) * per_unit),
                    // A packet lost whole: as long as the last one, in a stream of multi-frame
                    // packets as in any other.
                    None if data.is_empty() && self.last_frames > 0 => Some(self.last_frames),
                    None => Some(per_unit),
                }
            }
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3 { dec, .. } => {
                dec.reset();
                let frames = ac3_packet_frames(data)?.unwrap_or(self.last_frames);
                return Ok(AudioBuffer {
                    sample_rate: self.sample_rate,
                    channels: crate::pcm::silence(usize::from(self.layout.channels()), frames)?,
                });
            }
            Inner::Pcm { kind, channels } => Some(data.len() / (kind.bytes() * *channels).max(1)),
        };
        let frames = frames.filter(|&n| n > 0).unwrap_or(self.last_frames);
        Ok(AudioBuffer { sample_rate: self.sample_rate, channels: crate::pcm::silence(usize::from(self.layout.channels()), frames)? })
    }

    /// The frames every packet decodes to, for codecs whose packets all have one length: AAC
    /// (1024, 2048 for HE-AAC; a packet of several ADTS frames decodes to that many times this);
    /// `None` for AC-3/E-AC-3 (variable block/frame counts), Opus and PCM.
    pub fn frames_per_packet(&self) -> Option<usize> {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => Some(aac::FRAME_LEN * if up.is_some() { 2 } else { 1 }),
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3 { .. } => None,
            #[cfg(feature = "opus-decode")]
            Inner::Opus { .. } => None,
            Inner::Pcm { .. } => None,
        }
    }

    /// Frames to decode before a seek target and drop, so the decoder has settled when the target
    /// is reached: two AAC frames, 80 ms of Opus (RFC 7845 §4.6), one AC-3 frame (its filter
    /// bank overlaps one frame), none for PCM.
    pub fn preroll(&self) -> usize {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => 2 * aac::FRAME_LEN * if up.is_some() { 2 } else { 1 },
            #[cfg(feature = "opus-decode")]
            Inner::Opus { .. } => 3840,
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3 { .. } => 1536,
            Inner::Pcm { .. } => 0,
        }
    }

    /// Forget the decoder state, for a seek: the next packet decodes as if it were the first
    /// (codecs with overlap, such as AAC and Opus, need [`TrackDecoder::preroll`] frames to
    /// settle).
    pub fn reset(&mut self) {
        // the packet before a seek says nothing about the length of a packet lost after it
        self.last_frames = 0;
        match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { dec, up, .. } => {
                dec.reset();
                if let Some(up) = up {
                    up.reset();
                }
            }
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3 { dec, .. } => dec.reset(),
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, .. } => dec.reset(),
            Inner::Pcm { .. } => {}
        }
    }
}

fn unsupported(codec: CodecId) -> AudioError {
    let feature = match codec {
        CodecId::Aac | CodecId::HeAac => Some("aac-decode"),
        CodecId::Ac3 | CodecId::Eac3 => Some("ac3-decode"),
        CodecId::Opus => Some("opus-decode"),
        _ => None,
    };
    match feature {
        // A codec this crate decodes, in a build without its feature.
        Some(f) => AudioError::Unsupported(format!("decoding {} needs the `{f}` feature", codec.name())),
        None if cfg!(feature = "symphonia") && matches!(codec, CodecId::Mp3 | CodecId::Flac | CodecId::Alac | CodecId::Vorbis) => {
            AudioError::Unsupported(format!("no native {} packet decoder yet (craft_audio::decode reads it through symphonia)", codec.name()))
        }
        None => AudioError::Unsupported(format!("no native {} packet decoder yet", codec.name())),
    }
}

/// `channels` reordered by `order` (output channel → decoded channel); unchanged when the counts
/// differ.
#[cfg(any(feature = "aac-decode", feature = "opus-decode"))]
fn reorder(mut channels: Vec<Vec<f32>>, order: Option<&[usize]>) -> Vec<Vec<f32>> {
    match order {
        Some(order) if order.len() == channels.len() => order.iter().map(|&c| channels.get_mut(c).map(std::mem::take).unwrap_or_default()).collect(),
        _ => channels,
    }
}

/// Append `more` to `out` channel by channel. A packet whose frames differ in channel count is
/// malformed: one buffer cannot hold them.
#[cfg(feature = "aac-decode")]
fn append(out: &mut Vec<Vec<f32>>, more: Vec<Vec<f32>>) -> Result<()> {
    if out.is_empty() {
        *out = more;
    } else if out.len() == more.len() {
        for (o, c) in out.iter_mut().zip(more) {
            o.extend(c);
        }
    } else {
        return Err(AudioError::Malformed("the frames of a packet change the channel count".into()));
    }
    Ok(())
}

#[cfg(feature = "ac3-decode")]
type Ac3Format = (bool, u32, u8, bool);

#[cfg(feature = "ac3-decode")]
fn ac3_format(h: &ac3::Header) -> Ac3Format {
    (h.is_eac3(), h.sample_rate, h.acmod, h.lfeon)
}

/// Metadata is taken from a primary programme, never a dependent extension's speaker set.
#[cfg(feature = "ac3-decode")]
fn first_ac3_primary(mut packet: &[u8]) -> Result<Option<ac3::Header>> {
    let mut syncframes = 0;
    while !packet.is_empty() {
        count_ac3_syncframe(&mut syncframes)?;
        let Ok(h) = ac3::parse_header(packet) else { return Ok(None) };
        // A truncated header can advertise another format; wait for a complete frame.
        let Some(rest) = packet.get(h.frame_bytes..) else { return Ok(None) };
        if h.is_primary() {
            return Ok(Some(h));
        }
        packet = rest;
    }
    Ok(None)
}

#[cfg(feature = "ac3-decode")]
fn count_ac3_syncframe(count: &mut usize) -> Result<()> {
    if *count == MAX_AC3_PACKET_SYNCFRAMES {
        return Err(AudioError::TooLarge("AC-3 packet contains too many syncframes".into()));
    }
    *count += 1;
    Ok(())
}

/// Sum primary-programme samples, excluding dependent extensions and other programmes.
/// A readable truncated last frame still declares its concealment duration; an unreadable
/// header leaves timing unknown rather than inventing a six-block duration.
#[cfg(feature = "ac3-decode")]
pub(crate) fn ac3_packet_frames(mut packet: &[u8]) -> Result<Option<usize>> {
    if packet.is_empty() {
        return Ok(None);
    }
    let mut frames = 0usize;
    let mut syncframes = 0;
    while !packet.is_empty() {
        count_ac3_syncframe(&mut syncframes)?;
        let Ok(h) = ac3::parse_header(packet) else {
            return Ok(None);
        };
        if h.is_primary() {
            frames = frames
                .checked_add(h.samples())
                .filter(|&n| n as u64 <= crate::pcm::MAX_DECODE_SAMPLES)
                .ok_or_else(|| AudioError::TooLarge("AC-3 packet duration exceeds decode limit".into()))?;
        }
        let Some(rest) = packet.get(h.frame_bytes..) else {
            break;
        };
        packet = rest;
    }
    Ok(Some(frames))
}

/// Validate the complete aggregate before reserving or appending any samples.
#[cfg(feature = "ac3-decode")]
fn append_ac3(out: &mut Vec<Vec<f32>>, more: Vec<Vec<f32>>, limit: u64) -> Result<()> {
    let count = more.len();
    let frames = more.first().map_or(0, Vec::len);
    if (!out.is_empty() && out.len() != count) || more.iter().any(|c| c.len() != frames) {
        return Err(AudioError::Malformed("AC-3 packet has inconsistent channel planes".into()));
    }
    let previous = out.first().map_or(0, Vec::len);
    if out.iter().any(|c| c.len() != previous) {
        return Err(AudioError::Malformed("AC-3 packet has inconsistent channel planes".into()));
    }
    let total = previous
        .checked_add(frames)
        .and_then(|n| n.checked_mul(count))
        .ok_or_else(|| AudioError::TooLarge("AC-3 packet sample count overflow".into()))?;
    if total as u64 > limit {
        return Err(AudioError::TooLarge("AC-3 packet exceeds decode sample limit".into()));
    }
    if out.is_empty() {
        *out = more;
        return Ok(());
    }
    for plane in out.iter_mut() {
        plane.try_reserve(frames).map_err(|_| AudioError::TooLarge("AC-3 packet allocation failed".into()))?;
    }
    for (plane, samples) in out.iter_mut().zip(more) {
        plane.extend(samples);
    }
    Ok(())
}

/// The most ADTS frames a packet may hold: its audio is decoded and held in memory before any
/// decode limit applies. 1024 frames is the 2^20 frames the demuxers allow a packet.
#[cfg(feature = "aac-decode")]
const MAX_ADTS_FRAMES_PER_PACKET: usize = 1024;

/// The header of the format most of `headers` share, the earliest on a tie, so one damaged header
/// does not decide a stream's format.
#[cfg(feature = "aac-decode")]
pub(crate) fn adts_stream_header(headers: &[aac::AdtsHeader]) -> Option<aac::AdtsHeader> {
    headers.iter().rev().max_by_key(|h| headers.iter().filter(|o| adts_format_of(o) == adts_format_of(h)).count()).copied()
}

/// The ADTS frames `packet` holds, 0 when it is not ADTS-framed.
#[cfg(feature = "decode-native")]
pub(crate) fn adts_frame_count(packet: &[u8]) -> usize {
    adts_frames(packet).map_or(0, |f| f.len())
}

/// The configuration of an AAC stream from its ADTS `header`. Channel configuration 0 leaves
/// the layout to a program config element in the stream, looked for in `first_units`.
#[cfg(feature = "aac-decode")]
fn adts_config(header: &aac::AdtsHeader, first_units: &[&[u8]]) -> Result<Vec<u8>> {
    if header.channel_config != 0 {
        return Ok(header.to_asc().to_bytes()?);
    }
    let mut dec = aac::Decoder::from_adts(header)?;
    for unit in first_units {
        let _ = dec.decode(unit);
        if dec.config().pce.is_some() {
            return Ok(dec.config().to_bytes()?);
        }
    }
    Err(AudioError::Malformed("ADTS stream with channel configuration 0, and no program config element in its first packets".into()))
}

/// The whole frames at the start of an ADTS-framed packet, without a damaged or truncated end, or
/// `None` when the packet does not start with one. A raw access unit never looks like one: its first
/// three bits would be 7, ID_END, leaving nothing to decode.
#[cfg(feature = "aac-decode")]
fn adts_frames(packet: &[u8]) -> Option<Vec<(aac::AdtsHeader, &[u8])>> {
    if packet.len() < 7 || packet[0] != 0xFF || packet[1] & 0xF6 != 0xF0 {
        return None;
    }
    // Past the limit, what matters is only that there are too many: `adts_units` refuses them.
    aac::split_adts_upto(packet, MAX_ADTS_FRAMES_PER_PACKET + 1).ok()
}

/// What an ADTS frame says the stream's format is: profile, sampling frequency index and channel
/// configuration.
#[cfg(feature = "aac-decode")]
pub(crate) type AdtsFormat = (u8, u8, u8);

#[cfg(feature = "aac-decode")]
pub(crate) fn adts_format_of(h: &aac::AdtsHeader) -> AdtsFormat {
    (h.object_type, h.sf_index, h.channel_config)
}

/// The raw access units of an ADTS-framed packet. In a stream whose headers gave its `format`,
/// a frame that changes it would play wrongly, so it is malformed; with extradata, that decides the
/// format and headers are not compared with it. Frames with several raw data blocks are
/// unsupported.
#[cfg(feature = "aac-decode")]
fn adts_units(frames: Vec<(aac::AdtsHeader, &[u8])>, format: Option<AdtsFormat>) -> Result<Vec<&[u8]>> {
    if frames.len() > MAX_ADTS_FRAMES_PER_PACKET {
        return Err(AudioError::Malformed(format!("a packet of more than {MAX_ADTS_FRAMES_PER_PACKET} ADTS frames")));
    }
    if frames.iter().any(|(h, _)| h.raw_data_blocks > 1) {
        return Err(AudioError::Unsupported("ADTS frames with several raw data blocks".into()));
    }
    if format.is_some_and(|f| frames.iter().any(|(h, _)| adts_format_of(h) != f)) {
        return Err(AudioError::Malformed("an ADTS frame changes the stream's format".into()));
    }
    Ok(frames.into_iter().map(|(_, unit)| unit).collect())
}

/// A mapping-family-0 `OpusHead` (RFC 7845 §5.1) for `channels` (1 or 2) and no pre-skip.
#[cfg(feature = "opus-decode")]
fn default_opus_head(channels: u8, input_rate: u32) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.extend_from_slice(&[1, channels, 0, 0]);
    h.extend_from_slice(&(if input_rate > 0 { input_rate } else { 48_000 }).to_le_bytes());
    h.extend_from_slice(&[0, 0, 0]);
    h
}

/// The sample layout and stored format of PCM samples, `None` for a depth we do not read.
pub(crate) fn pcm_kind(bits: u16, float: bool, big_endian: bool, unsigned: bool) -> Option<(PcmKind, SampleFormat)> {
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

#[cfg(feature = "aac-decode")]
/// The speakers of an AAC channel configuration, in the WAV order [`aac::wav_order`] gives;
/// configuration 7 and program-config layouts keep the decoded order and are unordered.
pub(crate) fn aac_layout(channel_config: u8, channels: usize) -> ChannelLayout {
    match channel_config {
        1 => ChannelLayout::MONO,
        2 => ChannelLayout::STEREO,
        3 => ChannelLayout::SURROUND_3_0,
        4 => ChannelLayout::from_mask(0x107),
        5 => ChannelLayout::from_mask(0x607),
        6 => ChannelLayout::SURROUND_5_1_SIDE,
        11 => ChannelLayout::SURROUND_6_1,
        12 => ChannelLayout::SURROUND_7_1,
        _ => ChannelLayout::unordered(crate::layout::count(channels)),
    }
}

#[cfg(feature = "opus-decode")]
/// The speakers of an Opus stream (RFC 7845 §5.1.1): Vorbis order for mapping family 1, reordered
/// to WAV order by [`opus::wav_order`]; other families have no speaker assignment.
pub(crate) fn opus_layout(head: &opus::OpusHead) -> ChannelLayout {
    match (head.mapping_family, head.channels) {
        (0 | 1, 1) => ChannelLayout::MONO,
        (0 | 1, 2) => ChannelLayout::STEREO,
        (1, 3) => ChannelLayout::SURROUND_3_0,
        (1, 4) => ChannelLayout::QUAD,
        (1, 5) => ChannelLayout::SURROUND_5_0,
        (1, 6) => ChannelLayout::SURROUND_5_1,
        (1, 7) => ChannelLayout::SURROUND_6_1,
        (1, 8) => ChannelLayout::SURROUND_7_1,
        (_, n) => ChannelLayout::unordered(crate::layout::count(n)),
    }
}

#[cfg(feature = "ac3-decode")]
/// The speakers of an AC-3 / E-AC-3 program (A/52 Table 5.8) in the decoder's WAV order: fronts,
/// LFE, surrounds. Dual mono (1+1) has no speaker assignment.
pub(crate) fn ac3_layout(h: &ac3::Header) -> ChannelLayout {
    let fronts = match h.acmod & 7 {
        0 => return ChannelLayout::unordered(crate::layout::count(h.channels())),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PcmFormat;

    #[test]
    fn pcm_tracks_decode_in_wav_order() {
        let params = CodecParams::new(CodecId::Pcm).with_sample_rate(48_000).with_channels(2).with_pcm(PcmFormat::int(16, true, false));
        let mut dec = TrackDecoder::new(&params, &[]).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.layout()), (CodecId::Pcm, 48_000, ChannelLayout::STEREO));
        let block = dec.decode(&[0x00, 0x40, 0x00, 0xC0]).unwrap();
        assert_eq!(block.channels, vec![vec![0.5], vec![-0.5]]);
        assert_eq!(dec.conceal(None).unwrap().frames(), 1);
        // a reset forgets the last packet's length
        dec.reset();
        assert_eq!(dec.conceal(None).unwrap().frames(), 0);
        // silence past the decode limit is an error, not shorter silence
        dec.last_frames = usize::try_from(crate::pcm::MAX_DECODE_SAMPLES / 2 + 1).unwrap();
        assert!(matches!(dec.conceal(None), Err(AudioError::TooLarge(_))));
    }

    /// One ADTS frame (stereo, 48 kHz, or channel configuration 0 with a PCE) of silence.
    #[cfg(feature = "aac-decode")]
    fn adts_frame(channel_config: u8) -> Vec<u8> {
        use craft_media::bitstream::BitWriter;
        let mut bw = BitWriter::new();
        if channel_config == 0 {
            bw.write_bits(5, 3); // ID_PCE
            aac::ProgramConfig { front: vec![(false, 0), (true, 0)], ..Default::default() }.write(&mut bw).unwrap();
            bw.write_bits(0, 3); // ID_SCE
            bw.write_bits(0, 4);
            bw.write_bits(0, 22); // global_gain, ics_info, no pulse, TNS or gain control data
        }
        bw.write_bits(1, 3); // ID_CPE
        bw.write_bits(0, 5); // tag, common_window
        bw.write_bits(0, 44); // two such channel streams
        bw.write_bits(7, 3); // ID_END
        let au = bw.finish();
        [&aac::AdtsHeader::write(2, 3, channel_config, au.len(), 0x7FF).unwrap()[..], &au].concat()
    }

    #[cfg(feature = "aac-decode")]
    fn aac_params() -> CodecParams {
        CodecParams::new(CodecId::Aac)
    }

    #[cfg(feature = "aac-decode")]
    #[test]
    fn adts_packets_conceal_to_the_length_of_all_their_frames() {
        let frame = adts_frame(2);
        let mut dec = TrackDecoder::new(&aac_params(), &[&frame]).unwrap();
        let packet = frame.repeat(3);
        assert_eq!(dec.decode(&packet).unwrap().frames(), 3072);
        // the middle frame's payload is damaged: its element id is ID_CCE, which is unsupported
        let mut damaged = packet.clone();
        damaged[frame.len() + 7] = 0b0100_0000;
        assert!(dec.decode(&damaged).is_err());
        assert_eq!(dec.conceal(Some(&damaged)).unwrap().frames(), 3072);
        assert_eq!(dec.conceal(Some(&frame)).unwrap().frames(), 1024);
        // a packet lost whole lasts as long as the last one decoded: here, three frames
        assert_eq!(dec.conceal(None).unwrap().frames(), 3072);
    }

    /// Implicitly signalled HE-AAC: an AAC-LC configuration at 24 kHz or less, with SBR data in a
    /// fill element.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn he_aac_is_found_in_the_lookahead_and_upsampled() {
        use craft_media::bitstream::BitWriter;
        let unit = |sbr: bool| {
            let mut bw = BitWriter::new();
            bw.write_bits(0, 7); // ID_SCE, tag 0
            bw.write_bits(0, 22); // an empty channel stream
            if sbr {
                bw.write_bits(6, 3); // ID_FIL
                bw.write_bits(1, 4); // count
                bw.write_bits(13, 4); // EXT_SBR_DATA
                bw.write_bits(0, 4);
            }
            bw.write_bits(7, 3); // ID_END
            bw.finish()
        };
        // 22.05 kHz, mono
        let params = aac_params().with_extradata(vec![0x13, 0x88]);
        let (plain, he) = (unit(false), unit(true));
        let dec = TrackDecoder::new(&params, &[&plain]).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.frames_per_packet()), (CodecId::Aac, 22_050, Some(1024)));
        let mut dec = TrackDecoder::new(&params, &[&plain, &he]).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.frames_per_packet(), dec.preroll()), (CodecId::HeAac, 44_100, Some(2048), 4096));
        assert_eq!(dec.decode(&he).unwrap().frames(), 2048);
        assert_eq!(dec.conceal(None).unwrap().frames(), 2048);
    }

    /// A signalled output rate that is not twice the core's is not upsampled to: the core plays.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn an_sbr_rate_other_than_twice_the_core_plays_the_core() {
        // AOT 5, 24 kHz, mono, extension 44.1 kHz, core AOT 2
        let asc = {
            use craft_media::bitstream::BitWriter;
            let mut bw = BitWriter::new();
            for (value, bits) in [(5, 5), (6, 4), (1, 4), (4, 4), (2, 5), (0, 3)] {
                bw.write_bits(value, bits);
            }
            bw.finish()
        };
        let dec = TrackDecoder::new(&aac_params().with_extradata(asc), &[]).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.frames_per_packet()), (CodecId::Aac, 24_000, Some(1024)));
    }

    #[cfg(feature = "aac-decode")]
    #[test]
    fn adts_frames_of_another_format_or_several_blocks_conceal_at_their_length() {
        let frame = adts_frame(2);
        let mut dec = TrackDecoder::new(&aac_params(), &[&frame]).unwrap();
        // sampling frequency index 4 where the stream has 3
        let mut other = frame.clone();
        other[2] = (other[2] & 0xC3) | (4 << 2);
        assert!(matches!(dec.decode(&other), Err(AudioError::Malformed(_))));
        assert_eq!(dec.conceal(Some(&other)).unwrap().frames(), 1024);
        let mut blocks = frame.clone();
        blocks[6] |= 2; // three raw data blocks
        assert!(matches!(dec.decode(&blocks), Err(AudioError::Unsupported(_))));
        assert_eq!(dec.conceal(Some(&blocks)).unwrap().frames(), 3072);
        // the profile and the channel configuration are part of the format too
        let mut main = frame.clone();
        main[2] &= 0x3F; // profile 0: Main
        assert!(matches!(dec.decode(&main), Err(AudioError::Malformed(_))));
        let mut mono = frame.clone();
        mono[3] = (mono[3] & 0x3F) | (1 << 6);
        assert!(matches!(dec.decode(&mono), Err(AudioError::Malformed(_))));
    }

    /// With extradata, a header's differing fields are not a format change (ADTS in MP4 with a
    /// program config element in its AudioSpecificConfig, or channel configuration 0 in its headers).
    #[cfg(feature = "aac-decode")]
    #[test]
    fn extradata_decides_the_format_of_adts_packets() {
        let frame = adts_frame(0);
        let mut stereo = adts_frame(2);
        let params = aac_params().with_extradata(aac::AudioSpecificConfig::lc(3, 2).to_bytes().unwrap());
        let mut dec = TrackDecoder::new(&params, &[]).unwrap();
        assert_eq!(dec.decode(&stereo).unwrap().channels.len(), 2);
        stereo[3] &= 0x3F; // channel configuration 0 in the header
        stereo[2] &= 0xFE;
        assert_eq!(dec.decode(&stereo).unwrap().channels.len(), 2);
        assert!(dec.decode(&frame).is_err(), "elements the configuration does not have");
    }

    /// One damaged header at the start of a stream does not decide its format.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn the_adts_format_is_the_one_most_of_the_lookahead_shares() {
        let good = adts_frame(2);
        let mut damaged = good.clone();
        damaged[2] = (damaged[2] & 0xC3) | (4 << 2); // sampling frequency index 4 where the rest have 3
        let lookahead: [&[u8]; 4] = [&damaged, &good, &good, &good];
        let mut dec = TrackDecoder::new(&aac_params(), &lookahead).unwrap();
        assert_eq!(dec.sample_rate(), 48_000);
        assert!(dec.decode(&damaged).is_err());
        assert_eq!(dec.decode(&good).unwrap().frames(), 1024);
        // on a tie, the earlier one
        let tied: [&[u8]; 2] = [&good, &damaged];
        assert_eq!(TrackDecoder::new(&aac_params(), &tied).unwrap().sample_rate(), 48_000);
        #[cfg(feature = "decode-native")]
        assert_eq!((adts_frame_count(&good), adts_frame_count(&good.repeat(2)), adts_frame_count(&[0; 8])), (1, 2, 0));
    }

    /// `au` as a stereo ADTS frame at 48 kHz (channel configuration `channel_config`).
    #[cfg(feature = "aac-decode")]
    fn adts_of(au: &[u8], channel_config: u8) -> Vec<u8> {
        [&aac::AdtsHeader::write(2, 3, channel_config, au.len(), 0x7FF).unwrap()[..], au].concat()
    }

    /// An access unit of `pce` and `elements` (element id, tag), all empty, with channel streams of
    /// 22 bits (a CPE's are preceded by its `common_window` bit).
    #[cfg(feature = "aac-decode")]
    fn au_of(pce: Option<&aac::ProgramConfig>, elements: &[(u32, u32)]) -> Vec<u8> {
        use craft_media::bitstream::BitWriter;
        let mut bw = BitWriter::new();
        if let Some(pce) = pce {
            bw.write_bits(5, 3); // ID_PCE
            pce.write(&mut bw).unwrap();
        }
        for &(id, tag) in elements {
            bw.write_bits(id, 3);
            bw.write_bits(tag, 4);
            if id == 1 {
                bw.write_bits(0, 1);
                bw.write_bits(0, 22);
            }
            bw.write_bits(0, 22);
        }
        bw.write_bits(7, 3); // ID_END
        bw.finish()
    }

    #[cfg(feature = "aac-decode")]
    fn stereo_pce() -> aac::ProgramConfig {
        aac::ProgramConfig { front: vec![(true, 0)], ..Default::default() }
    }

    /// HE-AAC whose SBR data first appears in a later frame of a lookahead packet is found.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn sbr_in_a_later_adts_frame_of_a_packet_is_found() {
        use craft_media::bitstream::BitWriter;
        let unit = |sbr: bool| {
            let mut bw = BitWriter::new();
            bw.write_bits(0, 7); // ID_SCE, tag 0
            bw.write_bits(0, 22);
            if sbr {
                bw.write_bits(6, 3); // ID_FIL
                bw.write_bits(1, 4);
                bw.write_bits(13, 4); // EXT_SBR_DATA
                bw.write_bits(0, 4);
            }
            bw.write_bits(7, 3);
            bw.finish()
        };
        let frame = |au: &[u8]| [&aac::AdtsHeader::write(2, 7, 1, au.len(), 0x7FF).unwrap()[..], au].concat();
        let packet = [frame(&unit(false)), frame(&unit(true))].concat();
        let params = aac_params().with_extradata(vec![0x13, 0x88]);
        let dec = TrackDecoder::new(&params, &[&packet]).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate()), (CodecId::HeAac, 44_100));
    }

    /// A packet whose later frame fails leaves the decoder as a failed single frame does.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn a_failed_frame_undoes_the_earlier_frames_of_its_packet() {
        let first = adts_frame(0);
        let mut dec = TrackDecoder::new(&aac_params(), &[&first]).unwrap();
        assert_eq!(dec.decode(&first).unwrap().channels.len(), 3);
        // A valid frame that changes the layout, then one that fails.
        let change = adts_of(&au_of(Some(&stereo_pce()), &[(1, 0)]), 0);
        let bad = adts_of(&au_of(None, &[(2, 0)]), 0); // ID_CCE, unsupported
        assert!(dec.decode(&[change, bad].concat()).is_err());
        // Still three channels: SCE and CPE with no program config of their own.
        let plain = adts_of(&au_of(None, &[(0, 0), (1, 0)]), 0);
        assert_eq!(dec.decode(&plain).unwrap().channels.len(), 3);
    }

    /// Frames of one packet with different channel counts are an error, not a short packet.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn a_channel_count_change_inside_a_packet_is_an_error() {
        let first = adts_frame(0);
        let mut dec = TrackDecoder::new(&aac_params(), &[&first]).unwrap();
        let change = adts_of(&au_of(Some(&stereo_pce()), &[(1, 0)]), 0);
        assert!(matches!(dec.decode(&[first.clone(), change.clone()].concat()), Err(AudioError::Malformed(_))));
        assert_eq!(dec.conceal(Some(&[first, change].concat())).unwrap().frames(), 2048);
    }

    /// Whole frames before a damaged or truncated end of a packet still decode.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn the_whole_frames_before_a_damaged_tail_decode() {
        let frame = adts_frame(2);
        let mut dec = TrackDecoder::new(&aac_params(), &[&frame]).unwrap();
        let cut = [frame.repeat(2), frame[..frame.len() - 3].to_vec()].concat();
        assert_eq!(dec.decode(&cut).unwrap().frames(), 2048);
        let garbage = [frame.repeat(2), vec![0x12; 9]].concat();
        assert_eq!(dec.decode(&garbage).unwrap().frames(), 2048);
    }

    #[cfg(feature = "aac-decode")]
    #[test]
    fn a_packet_of_too_many_adts_frames_is_refused() {
        let frame = adts_frame(2);
        let mut dec = TrackDecoder::new(&aac_params(), &[&frame]).unwrap();
        assert_eq!(dec.decode(&frame.repeat(MAX_ADTS_FRAMES_PER_PACKET)).unwrap().frames(), MAX_ADTS_FRAMES_PER_PACKET * 1024);
        let packet = frame.repeat(MAX_ADTS_FRAMES_PER_PACKET + 1);
        assert!(matches!(dec.decode(&packet), Err(AudioError::Malformed(_))));
        assert_eq!(dec.conceal(Some(&packet)).unwrap().frames(), MAX_ADTS_FRAMES_PER_PACKET * 1024);
    }

    #[cfg(feature = "aac-decode")]
    #[test]
    fn adts_channel_configuration_0_takes_its_layout_from_the_stream() {
        let frame = adts_frame(0);
        let mut dec = TrackDecoder::new(&aac_params(), &[&frame]).unwrap();
        assert_eq!(dec.channels(), 3);
        assert_eq!(dec.decode(&frame).unwrap().channels.len(), 3);
        // no program config element where one is needed
        let bare = aac::AdtsHeader::write(2, 3, 0, 1, 0x7FF).unwrap();
        let bare = [&bare[..], &[0xE0]].concat();
        assert!(matches!(TrackDecoder::new(&aac_params(), &[&bare]), Err(AudioError::Malformed(_))));
    }

    #[test]
    fn codecs_without_a_packet_decoder_are_unsupported() {
        for codec in [CodecId::Flac, CodecId::Vorbis, CodecId::Alac, CodecId::Mp3, CodecId::H264] {
            assert!(matches!(TrackDecoder::new(&CodecParams::new(codec), &[]), Err(AudioError::Unsupported(_))), "{codec:?}");
        }
        // A codec whose feature is off names the feature.
        #[cfg(not(feature = "opus-decode"))]
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Opus), &[]), Err(AudioError::Unsupported(m)) if m.contains("opus-decode")));
        // PCM needs its sample format.
        assert!(TrackDecoder::new(&CodecParams::new(CodecId::Pcm).with_channels(2).with_sample_rate(48_000), &[]).is_err());
    }
}

#[cfg(all(test, feature = "ac3-decode", feature = "ac3-encode", feature = "eac3-encode"))]
#[path = "track_ac3_review_tests.rs"]
mod ac3_review_tests;

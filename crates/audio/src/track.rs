//! [`TrackDecoder`]: one track's packets in, planar `f32` in WAV channel order out, whatever the
//! codec. The file-level [`crate::decode`] and [`crate::Decoder`] use it for every native codec;
//! an app with its own demuxer (MXF, MPEG-TS, a network stream) uses it directly.

#[cfg(feature = "aac-decode")]
use crate::aac;
#[cfg(feature = "ac3-decode")]
use crate::ac3;
#[cfg(feature = "alac-decode")]
use crate::alac;
#[cfg(feature = "mpa-decode")]
use crate::mpa;
#[cfg(feature = "opus-decode")]
use crate::opus;
use crate::pcm::{PcmKind, deinterleave};
use crate::{AudioBuffer, AudioError, ChannelLayout, CodecId, CodecParams, Result, SampleFormat};

/// How many leading packets [`TrackDecoder::new`] looks at.
pub const LOOKAHEAD_PACKETS: usize = 8;

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
    #[cfg(feature = "alac-decode")]
    Alac {
        dec: alac::Decoder,
        order: Option<&'static [usize]>,
        /// 2^-(bit depth - 1): full scale to ±1.
        scale: f64,
    },
    /// The decoder, and what the packets read so far say of the track's packets.
    #[cfg(feature = "ac3-decode")]
    Ac3(Box<ac3::Decoder>, Ac3Packets),
    #[cfg(feature = "opus-decode")]
    Opus {
        dec: Box<opus::Decoder>,
        order: Option<&'static [usize]>,
    },
    #[cfg(feature = "mpa-decode")]
    Mpa(Box<mpa::Decoder>),
    Pcm {
        kind: PcmKind,
        channels: usize,
    },
}

/// Reads the frames a packet holds from its data, given the frames of a full packet.
#[cfg(feature = "decode-native")]
pub(crate) type PacketFrames = fn(&[u8], u64) -> Option<u64>;

/// Decodes the packets of one audio track to planar `f32`, in WAV channel order when
/// [`TrackDecoder::layout`] is ordered.
///
/// It takes the track's [`CodecParams`] (from `isobmff::Track::codec_params`,
/// `matroska::Track::codec_params`, `ogg::Stream::codec_params`, or built by hand for another
/// container) and decodes AAC (LC; HE-AAC as its upsampled LC core), ALAC, Opus, AC-3/E-AC-3,
/// MP1/MP2 and PCM. Container timing (AAC priming, Opus pre-skip, end trimming) is the caller's:
/// every decoded sample is returned.
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
    /// inside the stream is found in them, and AC-3 and MPEG audio take their rate and channel
    /// layout (MPEG audio its layer too) from the first frame. Without them those are taken from
    /// `params` and set from the first decoded packet.
    ///
    /// Streams without codec configuration are accepted where the packets carry it: AAC with
    /// empty `extradata` takes its configuration from the ADTS headers of the lookahead (MPEG-TS,
    /// raw `.aac`), the one most of them share, and ADTS headers are stripped from every packet;
    /// Opus with empty `extradata` and 1 or 2 `channels` (RTP, MPEG-TS) needs no `OpusHead`. Other
    /// missing configuration is [`AudioError::Malformed`].
    ///
    /// ALAC needs its magic cookie in `extradata`.
    ///
    /// Codecs without a native packet decoder yet (MP3, FLAC, Vorbis) are
    /// [`AudioError::Unsupported`]; `craft_audio::decode` decodes them through symphonia when the
    /// `symphonia` feature is on.
    pub fn new(params: &CodecParams, lookahead: &[&[u8]]) -> Result<TrackDecoder> {
        #[cfg(not(feature = "aac-decode"))]
        let _ = lookahead;
        let rate = params.sample_rate.unwrap_or(0);
        let channels = usize::from(params.channels.unwrap_or(0));
        // AC-3 and MPEG audio read their parameters from the first frame; AAC reads the lookahead;
        // Opus needs none.
        #[cfg_attr(not(any(feature = "ac3-decode", feature = "mpa-decode")), allow(unused_variables))]
        let first = lookahead.first().copied().unwrap_or_default();
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
                // a program config element that spells out a configuration has its speakers
                let cfg = aac::speaker_config(dec.config());
                let layout = aac_layout(cfg, dec.channels());
                let order = layout.is_ordered().then(|| aac::wav_order(cfg)).flatten();
                (Inner::Aac { dec: Box::new(dec), up, order, adts_format }, codec, out, layout, SampleFormat::Compressed)
            }
            #[cfg(feature = "alac-decode")]
            CodecId::Alac => {
                if params.extradata.is_empty() {
                    return Err(AudioError::Malformed("ALAC track without a magic cookie (extradata)".into()));
                }
                let dec = alac::Decoder::new(&params.extradata)?;
                let cfg = *dec.config();
                // The cookie's rate is the codec's; the container's stands in when it is 0.
                let rate = if cfg.sample_rate > 0 { cfg.sample_rate } else { rate };
                if rate == 0 {
                    return Err(AudioError::Malformed("ALAC track without a sample rate in its cookie or container".into()));
                }
                let (layout, order) = if cfg.has_standard_layout() {
                    (ChannelLayout::from_mask(alac::channel_mask(cfg.channels)), alac::wav_order(cfg.channels))
                } else {
                    (ChannelLayout::unordered(u16::from(cfg.channels)), None)
                };
                let sample_format = match cfg.bit_depth {
                    1..=8 => SampleFormat::Int8,
                    9..=16 => SampleFormat::Int16,
                    17..=24 => SampleFormat::Int24,
                    _ => SampleFormat::Int32,
                };
                let scale = 2f64.powi(1 - i32::from(cfg.bit_depth));
                (Inner::Alac { dec, order, scale }, CodecId::Alac, rate, layout, sample_format)
            }
            #[cfg(feature = "ac3-decode")]
            CodecId::Ac3 | CodecId::Eac3 => match ac3::parse_header(first) {
                Ok(h) => {
                    let codec = if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 };
                    (Inner::Ac3(Box::default(), Ac3Packets::of(lookahead)), codec, h.sample_rate, ac3_layout(&h), SampleFormat::Compressed)
                }
                Err(_) => (
                    Inner::Ac3(Box::default(), Ac3Packets::default()),
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
            #[cfg(feature = "mpa-decode")]
            CodecId::Mp1 | CodecId::Mp2 | CodecId::Mp3 => match mpa::parse_header(first) {
                Ok(h) if h.layer == 3 => return Err(unsupported(CodecId::Mp3)),
                Ok(h) => {
                    let codec = if h.layer == 1 { CodecId::Mp1 } else { CodecId::Mp2 };
                    (Inner::Mpa(Box::default()), codec, h.sample_rate, mpa_layout(&h), SampleFormat::Compressed)
                }
                // Without a first frame, a container that names layer III is left to symphonia.
                Err(_) if params.codec == CodecId::Mp3 => return Err(unsupported(CodecId::Mp3)),
                Err(_) => (
                    Inner::Mpa(Box::default()),
                    params.codec,
                    rate,
                    ChannelLayout::default_for(crate::layout::count(channels)),
                    SampleFormat::Compressed,
                ),
            },
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

    /// The codec, refined from the stream: [`CodecId::HeAac`] for AAC with spectral band
    /// replication, `Eac3` for Dolby Digital Plus, `Mp1`/`Mp2` by layer.
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

    /// Decode one packet. An MP4 or Matroska packet may hold several AC-3 or MPEG audio frames;
    /// an E-AC-3 dependent substream decodes to no frames. A packet that fails is an error for
    /// every codec: call [`TrackDecoder::conceal`] for audio of its length, so what follows keeps
    /// its timing.
    pub fn decode(&mut self, packet: &[u8]) -> Result<AudioBuffer> {
        // A packet decoded only in part says nothing of how long packets are.
        #[cfg_attr(not(feature = "ac3-decode"), allow(unused_mut))]
        let mut partial = false;
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
            #[cfg(feature = "alac-decode")]
            Inner::Alac { dec, order, scale } => {
                let decoded = dec.decode(packet)?;
                let floats = decoded.into_iter().map(|c| c.into_iter().map(|v| (f64::from(v) * *scale) as f32).collect()).collect();
                reorder(floats, *order)
            }
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(d) => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let mut p = 0;
                while let Some(rest) = packet.get(p..).filter(|r| !r.is_empty()) {
                    let f = match d.decode(rest) {
                        Ok(f) => f,
                        Err(e) if out.is_empty() => return Err(e.into()),
                        Err(_) => break,
                    };
                    p += f.header.frame_bytes.max(1);
                    if self.last_frames == 0 {
                        self.sample_rate = f.header.sample_rate;
                        self.layout = mpa_layout(&f.header);
                    }
                    append(&mut out, f.channels)?;
                }
                out
            }
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(d, shape) => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let mut p = 0;
                while let Some(rest) = packet.get(p..).filter(|r| !r.is_empty()) {
                    let f = match d.decode(rest) {
                        Ok(f) => f,
                        Err(e) if out.is_empty() => return Err(e.into()),
                        Err(_) => {
                            partial = true;
                            break;
                        }
                    };
                    p += f.header.frame_bytes.max(1);
                    shape.short |= f.header.is_primary() && f.header.samples() < AC3_FRAME_SAMPLES;
                    if !f.channels.is_empty() {
                        if self.last_frames == 0 {
                            self.sample_rate = f.header.sample_rate;
                            self.layout = ac3_layout(&f.header);
                        }
                        append(&mut out, f.channels)?;
                    }
                }
                let frames = out.first().map_or(0, Vec::len);
                shape.varies |= !partial && frames > 0 && shape.first > 0 && frames != shape.first;
                out
            }
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, order } => {
                let out = dec.decode(Some(packet))?;
                reorder(out, *order)
            }
            Inner::Pcm { kind, channels } => {
                let frames = (packet.len() / (kind.bytes() * *channels).max(1)) as u64;
                deinterleave(packet, *kind, *channels, frames)?
            }
        };
        if let Some(n) = channels.iter().map(Vec::len).min().filter(|&n| n > 0 && !partial) {
            self.last_frames = n;
            if channels.len() != usize::from(self.layout.channels()) {
                self.layout = ChannelLayout::unordered(crate::layout::count(channels.len()));
            }
        }
        Ok(AudioBuffer { sample_rate: self.sample_rate, channels })
    }

    /// What stands in for a lost or undecodable packet (`packet` when its bytes are at hand):
    /// Opus's loss concealment, or silence, as long as the packet would have been (from its TOC or
    /// frame header, the codec's frame length, or the last packet). Empty when the length is
    /// unknown.
    /// Fails with [`AudioError::TooLarge`](crate::AudioError::TooLarge) when that silence would pass
    /// the decode limit (2^30 samples, as [`decode`](Self::decode) has) or cannot be allocated,
    /// rather than return less than the packet's length.
    pub fn conceal(&mut self, packet: Option<&[u8]>) -> Result<AudioBuffer> {
        let data = packet.unwrap_or_default();
        let frames = match &mut self.inner {
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, order } => {
                let order = *order;
                if let Ok(out) = dec.conceal(packet) {
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
            // A damaged packet whose header survives says its length; otherwise a packet is as
            // long as the last one (a full one at the start).
            #[cfg(feature = "alac-decode")]
            Inner::Alac { dec, .. } => {
                Some(dec.packet_frames(data).unwrap_or(if self.last_frames > 0 { self.last_frames } else { dec.config().frame_length as usize }))
            }
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(..) => Some(ac3_packet_frames(data)),
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(_) => mpa::parse_header(data).ok().map(|h| h.samples()),
            Inner::Pcm { kind, channels } => Some(data.len() / (kind.bytes() * *channels).max(1)),
        };
        let frames = frames.filter(|&n| n > 0).unwrap_or(self.last_frames);
        Ok(AudioBuffer { sample_rate: self.sample_rate, channels: crate::pcm::silence(usize::from(self.layout.channels()), frames)? })
    }

    /// The frames every packet decodes to, for codecs whose packets all have one length: AAC
    /// (1024, 2048 for HE-AAC; a packet of several ADTS frames decodes to that many times this) and
    /// AC-3 / E-AC-3 (the frames of the last decoded packet, else of the first packet's primary
    /// syncframes, else 1536) and MP1/MP2 (one frame per packet); `None` for Opus, PCM and ALAC
    /// (whose last packet is shorter; the container's timing says), and for AC-3 / E-AC-3 whose
    /// packets read so far differ in length.
    pub fn frames_per_packet(&self) -> Option<usize> {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => Some(aac::FRAME_LEN * if up.is_some() { 2 } else { 1 }),
            #[cfg(feature = "alac-decode")]
            Inner::Alac { .. } => None,
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(_, shape) if shape.varies => None,
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(_, shape) => Some(match (self.last_frames, shape.first) {
                (0, 0) => AC3_FRAME_SAMPLES,
                (0, n) | (n, _) => n,
            }),
            #[cfg(feature = "opus-decode")]
            Inner::Opus { .. } => None,
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(_) => Some(if self.codec == CodecId::Mp1 { 384 } else { 1152 }),
            Inner::Pcm { .. } => None,
        }
    }

    /// The frames of a full packet, for codecs whose packets all have that length except ones
    /// at the end of the stream: ALAC's frame length (only the last packet is shorter). Where
    /// [`Self::frames_per_packet`] is `Some`, the same value.
    #[cfg(feature = "decode-native")]
    pub(crate) fn full_packet_frames(&self) -> Option<usize> {
        match &self.inner {
            #[cfg(feature = "alac-decode")]
            Inner::Alac { dec, .. } => Some(dec.config().frame_length as usize),
            _ => self.frames_per_packet(),
        }
    }

    /// How many frames a packet holds, read from the packet alone, for codecs whose packets
    /// say: ALAC's headers (given the frame length).
    #[cfg(feature = "decode-native")]
    pub(crate) fn packet_frames_reader(&self) -> Option<PacketFrames> {
        match &self.inner {
            #[cfg(feature = "alac-decode")]
            Inner::Alac { .. } => Some(crate::alac::header_frames),
            _ => None,
        }
    }

    /// Frames to decode before a seek target and drop, so the decoder has settled when the target
    /// is reached: two AAC frames, 80 ms of Opus (RFC 7845 §4.6), one AC-3 or MPEG audio frame
    /// (their filter banks overlap one frame), none for ALAC (its packets decode independently)
    /// and PCM. E-AC-3 syncframes of fewer than six blocks may reuse exponents from any syncframe
    /// before them, so once one has been read the stream decodes from its start (`usize::MAX`).
    pub fn preroll(&self) -> usize {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => 2 * aac::FRAME_LEN * if up.is_some() { 2 } else { 1 },
            #[cfg(feature = "alac-decode")]
            Inner::Alac { .. } => 0,
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(_, shape) if shape.short => usize::MAX,
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(..) => AC3_FRAME_SAMPLES,
            #[cfg(feature = "opus-decode")]
            Inner::Opus { .. } => 3840,
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(_) => self.last_frames.max(1152),
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
            #[cfg(feature = "alac-decode")]
            Inner::Alac { .. } => {}
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(d, _) => d.reset(),
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, .. } => dec.reset(),
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(d) => d.reset(),
            Inner::Pcm { .. } => {}
        }
    }
}

fn unsupported(codec: CodecId) -> AudioError {
    let feature = match codec {
        CodecId::Aac | CodecId::HeAac => Some("aac-decode"),
        CodecId::Alac => Some("alac-decode"),
        CodecId::Ac3 | CodecId::Eac3 => Some("ac3-decode"),
        CodecId::Opus => Some("opus-decode"),
        CodecId::Mp1 | CodecId::Mp2 => Some("mpa-decode"),
        _ => None,
    };
    match feature {
        // A codec this crate decodes, in a build without its feature.
        Some(f) => AudioError::Unsupported(format!("decoding {} needs the `{f}` feature", codec.name())),
        None if cfg!(feature = "symphonia") && matches!(codec, CodecId::Mp3 | CodecId::Flac | CodecId::Vorbis) => {
            AudioError::Unsupported(format!("no native {} packet decoder yet (craft_audio::decode reads it through symphonia)", codec.name()))
        }
        None => AudioError::Unsupported(format!("no native {} packet decoder yet", codec.name())),
    }
}

/// `channels` reordered by `order` (output channel → decoded channel); unchanged when the counts
/// differ.
#[cfg(any(feature = "aac-decode", feature = "alac-decode", feature = "opus-decode"))]
fn reorder(mut channels: Vec<Vec<f32>>, order: Option<&[usize]>) -> Vec<Vec<f32>> {
    match order {
        Some(order) if order.len() == channels.len() => order.iter().map(|&c| channels.get_mut(c).map(std::mem::take).unwrap_or_default()).collect(),
        _ => channels,
    }
}

/// Append `more` to `out` channel by channel. A packet whose frames differ in channel count is
/// malformed: one buffer cannot hold them.
#[cfg(any(feature = "aac-decode", feature = "ac3-decode", feature = "mpa-decode"))]
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

/// The frames of a six-block (AC-3 or E-AC-3) syncframe.
#[cfg(feature = "ac3-decode")]
const AC3_FRAME_SAMPLES: usize = 1536;

/// What the AC-3 / E-AC-3 packets read so far say of the track's packets.
#[cfg(feature = "ac3-decode")]
#[derive(Clone, Copy, Debug, Default)]
struct Ac3Packets {
    /// The frames the first packet's primary syncframes hold (0 when unknown).
    first: usize,
    /// Packets have decoded to different lengths: a packet's index does not give its position.
    varies: bool,
    /// A primary syncframe of fewer than six blocks (E-AC-3), whose exponents may come from any
    /// syncframe before it.
    short: bool,
}

#[cfg(feature = "ac3-decode")]
impl Ac3Packets {
    fn of(lookahead: &[&[u8]]) -> Ac3Packets {
        let lengths: Vec<usize> = lookahead.iter().map(|p| ac3_packet_frames(p)).filter(|&n| n > 0).collect();
        let short = lookahead.iter().any(|p| ac3_syncframes(p).any(|h| h.is_primary() && h.samples() < AC3_FRAME_SAMPLES));
        Ac3Packets { first: lookahead.first().map_or(0, |p| ac3_packet_frames(p)), varies: lengths.windows(2).any(|w| w[0] != w[1]), short }
    }
}

/// The headers of the syncframes `packet` holds, up to the first that does not parse.
#[cfg(feature = "ac3-decode")]
fn ac3_syncframes(packet: &[u8]) -> impl Iterator<Item = ac3::Header> + '_ {
    let mut p = 0;
    std::iter::from_fn(move || {
        let h = ac3::parse_header(packet.get(p..)?).ok()?;
        p += h.frame_bytes.max(1);
        Some(h)
    })
}

/// The frames an AC-3 / E-AC-3 packet decodes to: those of every primary syncframe in it (an MP4
/// or Matroska packet may hold several, as E-AC-3 frames of fewer than six blocks are grouped),
/// up to the first that does not parse. 0 when none does.
#[cfg(feature = "ac3-decode")]
fn ac3_packet_frames(packet: &[u8]) -> usize {
    ac3_syncframes(packet).filter(ac3::Header::is_primary).map(|h| h.samples()).sum()
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

#[cfg(feature = "mpa-decode")]
/// MPEG audio: mono, or stereo (also for dual channel and joint stereo).
pub(crate) fn mpa_layout(h: &mpa::Header) -> ChannelLayout {
    if h.channels() == 1 { ChannelLayout::MONO } else { ChannelLayout::STEREO }
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

    #[cfg(feature = "opus-decode")]
    #[test]
    fn opus_conceals_a_damaged_packet_at_its_own_length() {
        let params = CodecParams::new(CodecId::Opus).with_sample_rate(48_000).with_channels(2);
        let mut dec = TrackDecoder::new(&params, &[]).unwrap();
        // CELT fullband, 20 ms (config 31); SILK narrowband, 60 ms (config 3); CELT, 10 ms (config 30).
        let celt_20ms = [31 << 3, 0xFF, 0xFE, 0x01];
        let silk_60ms = [3 << 3, 0x12, 0x34];
        let celt_10ms = [30 << 3, 0x12, 0x34];
        assert_eq!(dec.decode(&celt_20ms).unwrap().frames(), 960);
        assert_eq!(dec.conceal(Some(&silk_60ms)).unwrap().frames(), 2880);
        // A packet lost whole is as long as the last one.
        assert_eq!(dec.conceal(None).unwrap().frames(), 2880);
        // The first packet after a reset has no last packet to go by.
        dec.reset();
        assert_eq!(dec.conceal(Some(&celt_10ms)).unwrap().frames(), 480);
        // Bytes whose framing does not parse fall back to the last length.
        assert_eq!(dec.conceal(Some(&[(31 << 3) | 3])).unwrap().frames(), 480);
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
        // its element lists a centre and a front pair, as configuration 3 does: L R C, in WAV order
        assert_eq!(dec.layout(), ChannelLayout::SURROUND_3_0);
        // no program config element where one is needed
        let bare = aac::AdtsHeader::write(2, 3, 0, 1, 0x7FF).unwrap();
        let bare = [&bare[..], &[0xE0]].concat();
        assert!(matches!(TrackDecoder::new(&aac_params(), &[&bare]), Err(AudioError::Malformed(_))));
    }

    #[cfg(feature = "ac3-decode")]
    #[test]
    fn e_ac3_packets_are_as_long_as_all_their_syncframes() {
        use crate::ac3::tests::single_block_eac3;
        let params = CodecParams::new(CodecId::Eac3);
        // One block per packet: 256 frames, known before any packet is decoded.
        let one = single_block_eac3(true);
        let dec = TrackDecoder::new(&params, &[&one]).unwrap();
        assert_eq!((dec.codec(), dec.frames_per_packet()), (CodecId::Eac3, Some(256)));
        // Six one-block syncframes to a packet (as MP4 groups them): 1536.
        let six = [one.clone(), (0..5).flat_map(|_| single_block_eac3(false)).collect()].concat();
        let mut dec = TrackDecoder::new(&params, &[&six]).unwrap();
        assert_eq!(dec.frames_per_packet(), Some(1536));
        assert_eq!(dec.decode(&six).unwrap().channels[0].len(), 1536);
        // A damaged packet conceals at the length of all its syncframes.
        dec.reset();
        assert_eq!(dec.conceal(Some(&six)).unwrap().channels[0].len(), 1536);
        assert_eq!(dec.conceal(Some(&one)).unwrap().channels[0].len(), 256);
    }

    #[cfg(feature = "ac3-decode")]
    #[test]
    fn a_packet_decoded_in_part_keeps_the_length_lost_packets_are_concealed_at() {
        use crate::ac3::tests::single_block_eac3;
        let params = CodecParams::new(CodecId::Eac3);
        let one = single_block_eac3(true);
        let six = [one.clone(), (0..5).flat_map(|_| single_block_eac3(false)).collect()].concat();
        let mut dec = TrackDecoder::new(&params, &[&six, &six]).unwrap();
        assert_eq!(dec.decode(&six).unwrap().channels[0].len(), 1536);
        // The second syncframe is damaged: its first one decodes, the rest of the packet is lost.
        let damaged = [one.clone(), vec![0x0B, 0x77, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]].concat();
        assert_eq!(dec.decode(&damaged).unwrap().channels[0].len(), 256);
        assert_eq!(dec.conceal(None).unwrap().channels[0].len(), 1536, "a lost packet is as long as the last whole one");
        assert_eq!(dec.frames_per_packet(), Some(1536), "a damaged packet does not make the packets' lengths differ");
    }

    #[cfg(feature = "ac3-decode")]
    #[test]
    fn e_ac3_packets_of_different_lengths_or_short_syncframes_are_not_indexed() {
        use crate::ac3::tests::single_block_eac3;
        let params = CodecParams::new(CodecId::Eac3);
        let one = single_block_eac3(true);
        let two = [single_block_eac3(true), single_block_eac3(false)].concat();
        // Packets of 256 and 512 frames: a packet's index does not give its position.
        let dec = TrackDecoder::new(&params, &[&one, &one, &two]).unwrap();
        assert_eq!(dec.frames_per_packet(), None);
        // Known only once decoded: from then on too.
        let mut dec = TrackDecoder::new(&params, &[&one]).unwrap();
        assert_eq!(dec.frames_per_packet(), Some(256));
        dec.decode(&two).unwrap();
        assert_eq!(dec.frames_per_packet(), None);
        // One-block syncframes may reuse exponents from any syncframe before them.
        assert_eq!(TrackDecoder::new(&params, &[&one]).unwrap().preroll(), usize::MAX);
    }

    #[test]
    fn codecs_without_a_packet_decoder_are_unsupported() {
        for codec in [CodecId::Flac, CodecId::Vorbis, CodecId::Mp3, CodecId::H264] {
            assert!(matches!(TrackDecoder::new(&CodecParams::new(codec), &[]), Err(AudioError::Unsupported(_))), "{codec:?}");
        }
        // A codec whose feature is off names the feature.
        #[cfg(not(feature = "aac-decode"))]
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Aac), &[]), Err(AudioError::Unsupported(m)) if m.contains("aac-decode")));
        #[cfg(not(feature = "alac-decode"))]
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Alac), &[]), Err(AudioError::Unsupported(m)) if m.contains("alac-decode")));
        #[cfg(not(feature = "opus-decode"))]
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Opus), &[]), Err(AudioError::Unsupported(m)) if m.contains("opus-decode")));
        // PCM needs its sample format.
        assert!(TrackDecoder::new(&CodecParams::new(CodecId::Pcm).with_channels(2).with_sample_rate(48_000), &[]).is_err());
    }

    #[cfg(feature = "alac-decode")]
    #[test]
    fn alac_needs_a_sample_rate() {
        let cookie = alac::SpecificConfig::new(0, 2, 16, 4096).to_bytes();
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Alac).with_extradata(cookie.clone()), &[]), Err(AudioError::Malformed(_))));
        let dec = TrackDecoder::new(&CodecParams::new(CodecId::Alac).with_extradata(cookie).with_sample_rate(32_000), &[]).unwrap();
        assert_eq!(dec.sample_rate(), 32_000);
    }

    #[cfg(feature = "alac-decode")]
    #[test]
    fn alac_needs_its_cookie() {
        assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Alac), &[]), Err(AudioError::Malformed(_))));
        let bad = CodecParams::new(CodecId::Alac).with_extradata(vec![0; 10]);
        assert!(matches!(TrackDecoder::new(&bad, &[]), Err(AudioError::Malformed(_))));
    }

    /// ALAC 5.1 comes out in WAV order at full scale, and a lost packet is as long as the last.
    #[cfg(all(feature = "alac-decode", feature = "alac-encode"))]
    #[test]
    fn alac_decodes_in_wav_order() {
        // ALAC order C L R Ls Rs LFE: channel i holds the constant i * 1000.
        let input: Vec<Vec<i32>> = (0..6).map(|i| vec![i * 1000; 300]).collect();
        let mut enc = alac::Encoder::new(alac::EncoderConfig::new(48_000, 6, 16).with_frame_length(1024)).unwrap();
        let packet = enc.encode(&input.iter().map(Vec::as_slice).collect::<Vec<_>>()).unwrap();
        let params = CodecParams::new(CodecId::Alac).with_extradata(enc.cookie()).with_sample_rate(44_100);
        let mut dec = TrackDecoder::new(&params, &[]).unwrap();
        // a damaged packet whose header survives is concealed with its own length
        assert_eq!(dec.conceal(Some(&packet[..8])).unwrap().frames(), 300);
        assert_eq!((dec.sample_rate(), dec.layout(), dec.sample_format()), (48_000, ChannelLayout::SURROUND_5_1, SampleFormat::Int16));
        assert_eq!(dec.conceal(None).unwrap().frames(), 1024);
        let out = dec.decode(&packet).unwrap();
        // WAV order L R C LFE Ls Rs
        let firsts: Vec<f32> = out.channels.iter().map(|c| c[0] * 32768.0).collect();
        assert_eq!(firsts, vec![1000.0, 2000.0, 0.0, 5000.0, 3000.0, 4000.0]);
        assert_eq!(dec.conceal(None).unwrap().frames(), 300);
        assert_eq!((dec.frames_per_packet(), dec.preroll()), (None, 0));
    }
}

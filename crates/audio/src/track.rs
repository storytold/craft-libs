//! [`TrackDecoder`]: one track's packets in, planar `f32` in WAV channel order out, whatever the
//! codec. The file-level [`crate::decode`] and [`crate::Decoder`] use it for every native codec;
//! an app with its own demuxer (MXF, MPEG-TS, a network stream) uses it directly.

#[cfg(feature = "aac-decode")]
use crate::aac;
#[cfg(feature = "ac3-decode")]
use crate::ac3;
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
    },
    #[cfg(feature = "mpa-decode")]
    Mpa(Box<mpa::Decoder>),
    #[cfg(feature = "ac3-decode")]
    Ac3(Box<ac3::Decoder>),
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
/// container) and decodes AAC (LC; HE-AAC as its upsampled LC core), MP1/MP2, AC-3/E-AC-3, Opus
/// and PCM. Container timing (AAC priming, Opus pre-skip, end trimming) is the caller's: every
/// decoded sample is returned.
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
///         Err(_) => dec.conceal(Some(p)), // keep the timing across a damaged packet
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
    /// inside the stream is found in them, and MPEG audio and AC-3 take their layer, rate and
    /// channel layout from the first frame. Without them those are taken from `params` and set
    /// from the first decoded packet.
    ///
    /// Codecs without a native packet decoder yet (MP3, FLAC, ALAC, Vorbis) are
    /// [`AudioError::Unsupported`]; [`crate::decode`] decodes them through symphonia.
    pub fn new(params: &CodecParams, lookahead: &[&[u8]]) -> Result<TrackDecoder> {
        let rate = params.sample_rate.unwrap_or(0);
        let channels = usize::from(params.channels.unwrap_or(0));
        // MPEG audio and AC-3 read their parameters from the first frame.
        #[cfg_attr(not(any(feature = "mpa-decode", feature = "ac3-decode")), allow(unused_variables))]
        let first = lookahead.first().copied().unwrap_or_default();
        let (inner, codec, sample_rate, layout, sample_format) = match params.codec {
            #[cfg(feature = "aac-decode")]
            CodecId::Aac | CodecId::HeAac => {
                let asc = params.extradata.as_slice();
                let dec = aac::Decoder::new(asc)?;
                let out = aac::output_rate(asc, lookahead.iter().copied().take(LOOKAHEAD_PACKETS)).unwrap_or(dec.sample_rate());
                let up = (dec.sample_rate().checked_mul(2) == Some(out)).then(|| aac::Upsample2x::new(dec.channels()));
                let codec = if up.is_some() { CodecId::HeAac } else { CodecId::Aac };
                let cfg = dec.config().channel_config;
                let layout = aac_layout(cfg, dec.channels());
                let order = layout.is_ordered().then(|| aac::wav_order(cfg)).flatten();
                (Inner::Aac { dec: Box::new(dec), up, order }, codec, out, layout, SampleFormat::Compressed)
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
            #[cfg(feature = "ac3-decode")]
            CodecId::Ac3 | CodecId::Eac3 => match ac3::parse_header(first) {
                Ok(h) => {
                    let codec = if h.is_eac3() { CodecId::Eac3 } else { CodecId::Ac3 };
                    (Inner::Ac3(Box::default()), codec, h.sample_rate, ac3_layout(&h), SampleFormat::Compressed)
                }
                Err(_) => (
                    Inner::Ac3(Box::default()),
                    params.codec,
                    rate,
                    ChannelLayout::unordered(crate::layout::count(channels)),
                    SampleFormat::Compressed,
                ),
            },
            #[cfg(feature = "opus-decode")]
            CodecId::Opus => {
                let head = opus::OpusHead::parse(&params.extradata)?;
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

    /// The codec, refined from the stream: [`CodecId::HeAac`] for AAC with spectral band
    /// replication, `Mp1`/`Mp2` by layer, `Eac3` for Dolby Digital Plus.
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

    /// Decode one packet. An MP4 or Matroska packet may hold several MPEG audio or AC-3 frames;
    /// an E-AC-3 dependent substream decodes to no frames. A packet that fails is an error for
    /// every codec: call [`TrackDecoder::conceal`] for audio of its length, so what follows keeps
    /// its timing.
    pub fn decode(&mut self, packet: &[u8]) -> Result<AudioBuffer> {
        let channels = match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { dec, up, order } => {
                let mut out = dec.decode(packet)?;
                if let Some(up) = up {
                    out = up.process(out);
                }
                reorder(out, *order)
            }
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, order } => {
                let out = dec.decode(Some(packet))?;
                reorder(out, *order)
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
                    append(&mut out, f.channels);
                }
                out
            }
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(d) => {
                let mut out: Vec<Vec<f32>> = Vec::new();
                let mut p = 0;
                while let Some(rest) = packet.get(p..).filter(|r| !r.is_empty()) {
                    let f = match d.decode(rest) {
                        Ok(f) => f,
                        Err(e) if out.is_empty() => return Err(e.into()),
                        Err(_) => break,
                    };
                    p += f.header.frame_bytes.max(1);
                    if !f.channels.is_empty() {
                        if self.last_frames == 0 {
                            self.sample_rate = f.header.sample_rate;
                            self.layout = ac3_layout(&f.header);
                        }
                        append(&mut out, f.channels);
                    }
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
    pub fn conceal(&mut self, packet: Option<&[u8]>) -> AudioBuffer {
        let data = packet.unwrap_or_default();
        let frames = match &mut self.inner {
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, order } => {
                let order = *order;
                if let Ok(out) = dec.decode(None) {
                    return AudioBuffer { sample_rate: self.sample_rate, channels: reorder(out, order) };
                }
                None
            }
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => Some(aac::FRAME_LEN * if up.is_some() { 2 } else { 1 }),
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(_) => mpa::parse_header(data).ok().map(|h| h.samples()),
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(_) => ac3::parse_header(data).ok().map(|h| h.samples()),
            Inner::Pcm { kind, channels } => Some(data.len() / (kind.bytes() * *channels).max(1)),
        };
        let frames = frames.filter(|&n| n > 0).unwrap_or(self.last_frames);
        AudioBuffer { sample_rate: self.sample_rate, channels: vec![vec![0.0; frames]; usize::from(self.layout.channels())] }
    }

    /// The frames every packet decodes to, for codecs whose packets all have one length: AAC
    /// (1024, 2048 for HE-AAC), MP1/MP2 and AC-3 (one frame per packet); `None` for Opus and PCM.
    pub fn frames_per_packet(&self) -> Option<usize> {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => Some(aac::FRAME_LEN * if up.is_some() { 2 } else { 1 }),
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(_) => Some(if self.codec == CodecId::Mp1 { 384 } else { 1152 }),
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(_) => Some(if self.last_frames > 0 { self.last_frames } else { 1536 }),
            #[cfg(feature = "opus-decode")]
            Inner::Opus { .. } => None,
            Inner::Pcm { .. } => None,
        }
    }

    /// Frames to decode before a seek target and drop, so the decoder has settled when the target
    /// is reached: two AAC frames, 80 ms of Opus (RFC 7845 §4.6), one MPEG audio or AC-3 frame
    /// (their filter banks overlap one frame), none for PCM.
    pub fn preroll(&self) -> usize {
        match &self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { up, .. } => 2 * aac::FRAME_LEN * if up.is_some() { 2 } else { 1 },
            #[cfg(feature = "opus-decode")]
            Inner::Opus { .. } => 3840,
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(_) => self.last_frames.max(1152),
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(_) => 1536,
            Inner::Pcm { .. } => 0,
        }
    }

    /// Forget the decoder state, for a seek: the next packet decodes as if it were the first
    /// (codecs with overlap, such as AAC and Opus, need [`TrackDecoder::preroll`] frames to
    /// settle).
    pub fn reset(&mut self) {
        match &mut self.inner {
            #[cfg(feature = "aac-decode")]
            Inner::Aac { dec, up, .. } => {
                dec.reset();
                if let Some(up) = up {
                    up.reset();
                }
            }
            #[cfg(feature = "mpa-decode")]
            Inner::Mpa(d) => d.reset(),
            #[cfg(feature = "ac3-decode")]
            Inner::Ac3(d) => d.reset(),
            #[cfg(feature = "opus-decode")]
            Inner::Opus { dec, .. } => dec.reset(),
            Inner::Pcm { .. } => {}
        }
    }
}

fn unsupported(codec: CodecId) -> AudioError {
    let feature = match codec {
        CodecId::Aac | CodecId::HeAac => Some("aac-decode"),
        CodecId::Mp1 | CodecId::Mp2 => Some("mpa-decode"),
        CodecId::Ac3 | CodecId::Eac3 => Some("ac3-decode"),
        CodecId::Opus => Some("opus-decode"),
        _ => None,
    };
    match feature {
        // A codec this crate decodes, in a build without its feature.
        Some(f) => AudioError::Unsupported(format!("decoding {} needs the `{f}` feature", codec.name())),
        None => AudioError::Unsupported(format!("no native {} packet decoder yet (craft_audio::decode reads it through symphonia)", codec.name())),
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

/// Append `more` to `out` channel by channel; a block whose channel count differs is dropped.
#[cfg(any(feature = "mpa-decode", feature = "ac3-decode"))]
fn append(out: &mut Vec<Vec<f32>>, more: Vec<Vec<f32>>) {
    if out.is_empty() {
        *out = more;
    } else if out.len() == more.len() {
        for (o, c) in out.iter_mut().zip(more) {
            o.extend(c);
        }
    }
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

#[cfg(feature = "mpa-decode")]
/// MPEG audio: mono, or stereo (also for dual channel and joint stereo).
pub(crate) fn mpa_layout(h: &mpa::Header) -> ChannelLayout {
    if h.channels() == 1 { ChannelLayout::MONO } else { ChannelLayout::STEREO }
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
        assert_eq!(dec.conceal(None).frames(), 1);
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

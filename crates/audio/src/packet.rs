//! [`PacketEncoder`]: one interface over the AAC-LC, ALAC, MP3, AC-3, E-AC-3 and Opus encoders, for
//! a caller that puts the packets in its own container. It takes planar `f32` in WAV channel order,
//! maps the channels onto the codec's order, and returns packets with their timing; the coded
//! formats of [`FileEncoder`](crate::FileEncoder) are written through it.

// Each codec is its own feature: with only some of them built, the other halves of the shared
// code are unused.
#![cfg_attr(
    not(all(
        feature = "aac-encode",
        feature = "alac-encode",
        feature = "mp3-encode",
        feature = "ac3-encode",
        feature = "eac3-encode",
        feature = "opus-encode"
    )),
    allow(dead_code, unused_imports, unused_variables, unused_mut)
)]

use crate::layout::channel_map;
use crate::{AudioError, ChannelLayout, CodecId, CodecParams, EncodeFormat, EncodeOptions, RateControl, Result};

/// Maximum frames handed to a codec per internal call, bounding that call's input and output.
/// `PacketEncoder::encode` collects packets across all chunks, so its returned vector grows with
/// the caller's input block. Also the frames of an ALAC packet.
pub(crate) const CHUNK: usize = 4096;

/// Bits per second per channel when no rate is asked for: AAC and Opus.
const AAC_DEFAULT_BPS_PER_CHANNEL: u32 = 64_000;
const OPUS_DEFAULT_BPS_PER_CHANNEL: u32 = 64_000;
/// Kilobits per second per channel when no rate is asked for: MP3, and AC-3 and E-AC-3 (per
/// full-bandwidth channel).
const DEFAULT_KBPS_PER_CHANNEL: u32 = 96;

/// Opus's layouts by channel count: RFC 7845 mapping family 0 (mono, and stereo as L R).
const OPUS_LAYOUT_MASKS: [&[u32]; 2] = [&[0x4], &[0x3]];

/// One packet from a [`PacketEncoder`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPacket {
    /// The coded bytes: an AAC raw data block (no ADTS header), an ALAC packet, one MP3 frame,
    /// one AC-3 syncframe or an E-AC-3 access unit, one Opus packet.
    pub data: Vec<u8>,
    /// Where the packet's decoded audio starts, in samples at the sample rate from the first
    /// decoded sample of the stream. The encoder's [priming](PacketEncoder::priming) comes first,
    /// so input frame `n` is at `n + priming`.
    pub pts: u64,
    /// The samples per channel the packet decodes to, the last packet's padding included.
    pub duration: u32,
}

/// Encodes audio to packets of one codec, for a muxer of the caller's own: the encode-side
/// counterpart of [`TrackDecoder`](crate::TrackDecoder). Built from a [`CodecId`] and the
/// [`EncodeOptions`] of [`FileEncoder`](crate::FileEncoder), of which it reads the rate control,
/// the layout, and for ALAC the bit depth and dither.
///
/// Input is planar `f32` in WAV channel order, one slice per channel, at any block size; packets
/// come out as they are completed, and [`finish`](Self::finish) returns the rest. What a container
/// needs to describe the track is on the encoder: [`codec_params`](Self::codec_params) (codec,
/// rate, channels and [`extradata`](Self::extradata)), [`layout`](Self::layout),
/// [`priming`](Self::priming), and after `finish` [`padding`](Self::padding).
///
/// | Codec | Feature | Packet | `extradata` | Priming |
/// |---|---|---|---|---|
/// | [`CodecId::Aac`] (LC) | `aac-encode` | 1024 frames | `AudioSpecificConfig` | 1024 |
/// | [`CodecId::Alac`] | `alac-encode` | 4096 frames | the `alac` box / CodecPrivate | 0 |
/// | [`CodecId::Mp3`] | `mp3-encode` | 1152 frames (576 below 32 kHz) | none | 1057 |
/// | [`CodecId::Ac3`], [`CodecId::Eac3`] | `ac3-encode`, `eac3-encode` | 1536 frames | none | 256 |
/// | [`CodecId::Opus`] | `opus-encode` | 960 frames (20 ms) | `OpusHead` | the pre-skip |
///
/// ```
/// # #[cfg(all(feature = "aac-encode", feature = "aac-decode"))] {
/// use craft_audio::{CodecId, EncodeFormat, EncodeOptions, PacketEncoder, TrackDecoder};
///
/// let opts = EncodeOptions::new(EncodeFormat::Aac).with_bitrate(128_000);
/// let mut enc = PacketEncoder::new(CodecId::Aac, &opts, 48_000, 2)?;
/// let block = [vec![0.25f32; 4800], vec![-0.25f32; 4800]]; // 0.1 s, one Vec per channel
/// let mut packets = enc.encode(&block, 4800)?;
/// packets.extend(enc.finish()?);
/// assert_eq!(enc.priming(), 1024);
/// assert_eq!(packets.iter().map(|p| u64::from(p.duration)).sum::<u64>(), 1024 + 4800 + enc.padding());
///
/// // The description a muxer writes is what a decoder reads.
/// let mut dec = TrackDecoder::new(&enc.codec_params(), &[])?;
/// assert_eq!(dec.decode(&packets[0].data)?.frames(), 1024);
/// # }
/// # Ok::<(), craft_audio::AudioError>(())
/// ```
pub struct PacketEncoder {
    codec: CodecId,
    inner: Inner,
    sample_rate: u32,
    /// The speakers of the channels encoded, in WAV order.
    layout: ChannelLayout,
    /// For each codec channel, the caller's channel that feeds it.
    map: Vec<usize>,
    bitrate: Option<u32>,
    /// The integer depth of a lossless codec.
    bit_depth: Option<u8>,
    /// Where the next packet starts.
    pts: u64,
    /// Frames of input taken.
    frames: u64,
    finished: bool,
    /// Set when a call failed: the packets of that block are lost, so the stream has a gap and
    /// every later call is refused.
    failed: bool,
}

impl std::fmt::Debug for PacketEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PacketEncoder")
            .field("codec", &self.codec)
            .field("sample_rate", &self.sample_rate)
            .field("layout", &self.layout)
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

/// A packet encoder of one of the codecs, taking planar `f32` in its own channel order.
enum Inner {
    #[cfg(feature = "aac-encode")]
    Aac(Box<crate::aac::Encoder>),
    #[cfg(feature = "alac-encode")]
    Alac {
        enc: Box<crate::alac::Encoder>,
        quantizers: Vec<crate::write::pcm::Quantizer>,
        /// Quantized samples of the packet being filled, per channel.
        pending: Vec<Vec<i32>>,
    },
    #[cfg(feature = "mp3-encode")]
    Mp3 {
        enc: Box<crate::mp3::Encoder>,
        /// Bytes the encoder returned that do not yet make a whole frame.
        bytes: Vec<u8>,
    },
    #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
    Ac3 {
        enc: Box<crate::ac3::encoder::Encoder>,
        /// The block with every sample finite and within ±1, as the encoder requires.
        planes: Vec<Vec<f32>>,
    },
    #[cfg(feature = "opus-encode")]
    Opus {
        enc: Box<crate::opus::encoder::Encoder>,
        /// Interleaved samples of the frame being filled.
        pending: Vec<f32>,
        /// Frames of the decoded stream the packets so far cover.
        coded: u64,
    },
}

/// The speakers of `mask` as labels: `"L R C LFE Ls Rs"`.
fn labels(mask: u32) -> String {
    ChannelLayout::from_mask(mask).speakers().iter().map(|s| s.label()).collect::<Vec<_>>().join(" ")
}

/// The speakers of the caller's layout, for an error message.
fn given(layout: &ChannelLayout) -> String {
    layout.mask().map_or_else(|| "without speaker assignment".into(), labels)
}

/// [`channel_map`] for a codec with one layout per channel count (the first of `masks`), and
/// that layout; or why the audio does not fit.
fn fixed_map(masks: &[u32], name: &str, order: Option<&[usize]>, layout: &ChannelLayout) -> Result<(Vec<usize>, ChannelLayout)> {
    let Some(&mask) = masks.first() else {
        return Err(AudioError::Unsupported(format!("{name} cannot hold {} channels", layout.channels())));
    };
    let map = channel_map(mask, order, layout).ok_or_else(|| {
        AudioError::Unsupported(format!("{name} holds {} channels as {}; the audio is {}", layout.channels(), labels(mask), given(layout)))
    })?;
    Ok((map, ChannelLayout::from_mask(mask)))
}

/// The encoder's channel order over AAC's layout for the count: [`crate::aac::aac_order`], and for
/// 8 channels configuration 7's C, Lc Rc, L R, Ls Rs, LFE over the WAV order L R C LFE Ls Rs Lc Rc.
#[cfg(feature = "aac-encode")]
pub(super) fn aac_order(channels: usize) -> Option<&'static [usize]> {
    if channels == 8 { Some(&[2, 6, 7, 0, 1, 4, 5, 3]) } else { crate::aac::aac_order(channels) }
}

/// The lowest AAC bit rate used, per channel: 6 kbit/s, and a quarter of the sample rate (256
/// bits a frame). The encoder accepts less, and from about half of this down codes silence; a
/// stream of silence is no answer to a low bit rate (or to kbit/s given as bit/s).
#[cfg(feature = "aac-encode")]
fn aac_floor_per_channel(sample_rate: u32) -> u32 {
    (sample_rate / 4).max(6000)
}

/// The AAC rate control for `rate`: a bit rate is kept within what the stream can carry, a
/// quality of 0–1 spans the encoder's VBR range.
#[cfg(feature = "aac-encode")]
pub(super) fn aac_mode(rate: Option<RateControl>, sample_rate: u32, channels: usize) -> crate::aac::BitrateMode {
    use crate::aac::BitrateMode;
    let (lo, hi) = crate::aac::cbr_range(sample_rate, channels);
    let lo = lo.max(aac_floor_per_channel(sample_rate).saturating_mul(channels as u32)).min(hi.max(lo));
    // An empty range (a rate or channel count the encoder refuses anyway) leaves the rate alone.
    let fit = |bps: u32| if lo <= hi { bps.clamp(lo, hi) } else { bps };
    match rate {
        None => BitrateMode::Cbr(fit(AAC_DEFAULT_BPS_PER_CHANNEL.saturating_mul(channels as u32))),
        Some(RateControl::Bitrate(bps)) => BitrateMode::Cbr(fit(bps)),
        // NaN stays NaN, which the encoder refuses.
        Some(RateControl::Quality(q)) => BitrateMode::Vbr(1.0 + 4.0 * q.clamp(0.0, 1.0)),
    }
}

/// The bit rate in bits per second for a codec with constant bit rates only.
fn constant_bps(name: &str, rate: Option<RateControl>, default: u32) -> Result<u32> {
    match rate {
        None => Ok(default),
        Some(RateControl::Bitrate(bps)) => Ok(bps),
        Some(RateControl::Quality(_)) => {
            Err(AudioError::Unsupported(format!("{name} is encoded at a constant bit rate; set a bit rate, not a quality")))
        }
    }
}

/// The AC-3 or E-AC-3 settings for audio with `layout`, and the channel map onto them: the coding
/// mode whose speakers the layout names, at the nearest bit rate the stream can carry.
#[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
pub(super) fn ac3_config(
    codec: crate::ac3::encoder::Codec,
    sample_rate: u32,
    layout: &ChannelLayout,
    rate: Option<RateControl>,
) -> Result<(crate::ac3::encoder::Config, Vec<usize>)> {
    use crate::ac3::encoder::{Codec, CodingMode, Config};
    use CodingMode::*;
    let name = if codec == Codec::Ac3 { "AC-3" } else { "E-AC-3" };
    let modes = [Mono, Stereo, ThreeFront, StereoSurround, ThreeFrontSurround, StereoTwoSurround, ThreeFrontTwoSurround, Surround71];
    let found = modes.iter().flat_map(|&mode| [false, true].map(|lfe| (mode, lfe))).find_map(|(mode, lfe)| {
        // 7.1 is an E-AC-3 programme with an LFE; the encoder has no other.
        if mode == Surround71 && (codec != Codec::Eac3 || !lfe) {
            return None;
        }
        let config = Config { codec, sample_rate, bitrate_kbps: 0, mode, lfe };
        let map = channel_map(config.channel_layout().mask()?, None, layout)?;
        Some((config, map))
    });
    let Some((mut config, map)) = found else {
        return Err(AudioError::Unsupported(format!("{name} has no coding mode for {} channels that are {}", layout.channels(), given(layout))));
    };
    let full = config.mode.channels() as u32;
    let want = constant_bps(name, rate, DEFAULT_KBPS_PER_CHANNEL * full * 1000)? / 1000;
    // What `Config::validate` asks of the rate: enough for the channels, and for AC-3 an entry of
    // the frame size table, for E-AC-3 a syncframe its 11-bit word count can hold.
    let least = if config.mode == Surround71 { 384 } else { 32 * full };
    config.bitrate_kbps = match codec {
        Codec::Ac3 => {
            let floor = crate::ac3::BITRATES.iter().copied().find(|&r| r >= least).unwrap_or(least);
            crate::ac3::BITRATES.iter().rev().copied().find(|&r| r <= want).filter(|&r| r >= floor).unwrap_or(floor)
        }
        Codec::Eac3 => {
            let most = if config.mode == Surround71 { 1024 } else { (2048 * u64::from(sample_rate) / 96_000).min(1024) as u32 };
            want.clamp(least, most.max(least))
        }
    };
    Ok((config, map))
}

/// The feature a codec's encoder is built with, or `None` for a codec with no encoder here.
fn feature(codec: CodecId) -> Option<&'static str> {
    Some(match codec {
        CodecId::Aac => "aac-encode",
        CodecId::Alac => "alac-encode",
        CodecId::Mp3 => "mp3-encode",
        CodecId::Ac3 => "ac3-encode",
        CodecId::Eac3 => "eac3-encode",
        CodecId::Opus => "opus-encode",
        _ => return None,
    })
}

impl PacketEncoder {
    /// An encoder of `codec` for `channels` channels at `sample_rate`. Of `opts` it reads
    /// `rate_control` and `layout` (as [`FileEncoder`](crate::FileEncoder) does: the speakers of
    /// the caller's channels, mapped onto the codec's layout), and for ALAC `bit_depth` and
    /// `dither`; `format`, `bwf` and `force_rf64` are for files and ignored.
    ///
    /// Opus takes 48 kHz mono or stereo at a constant bit rate (64 kbit/s per channel when none
    /// is asked for); the others take what their [`EncodeFormat`] lists. A codec this crate has
    /// no encoder for, and one whose feature is not built, are [`AudioError::Unsupported`]. A
    /// rate, channel count, layout or rate control the codec cannot take is refused with the error
    /// [`EncodeOptions::check`] gives for the format (`Unsupported` for a layout or rate control).
    pub fn new(codec: CodecId, opts: &EncodeOptions, sample_rate: u32, channels: u16) -> Result<PacketEncoder> {
        let Some(feature) = feature(codec) else {
            return Err(AudioError::Unsupported(format!("no packet encoder for {}", codec.name())));
        };
        let built = [
            (CodecId::Aac, cfg!(feature = "aac-encode")),
            (CodecId::Alac, cfg!(feature = "alac-encode")),
            (CodecId::Mp3, cfg!(feature = "mp3-encode")),
            (CodecId::Ac3, cfg!(feature = "ac3-encode")),
            (CodecId::Eac3, cfg!(feature = "eac3-encode")),
            (CodecId::Opus, cfg!(feature = "opus-encode")),
        ];
        if !built.contains(&(codec, true)) {
            return Err(AudioError::Unsupported(format!("encoding {} needs the `{feature}` feature", codec.name())));
        }
        let n = usize::from(channels);
        // As the file writers word it.
        if channels == 0 {
            return Err(AudioError::Encode("buffer has no channels".into()));
        }
        if sample_rate == 0 {
            return Err(AudioError::Encode("sample rate is 0".into()));
        }
        if let Some(layout) = &opts.layout
            && layout.channels() != channels
        {
            return Err(AudioError::Encode(format!("a {}-channel layout for {channels} channels", layout.channels())));
        }
        let given = opts.layout.unwrap_or_else(|| ChannelLayout::default_for(channels));
        let (inner, map, layout, bitrate, bit_depth) = match codec {
            #[cfg(feature = "aac-encode")]
            CodecId::Aac => {
                let (map, layout) = fixed_map(EncodeFormat::Aac.layout_masks(channels), "AAC", aac_order(n), &given)?;
                let mode = aac_mode(opts.rate_control, sample_rate, n);
                let config = crate::aac::EncoderConfig { mode, ..crate::aac::EncoderConfig::cbr(sample_rate, n, 0) };
                let bitrate = match mode {
                    crate::aac::BitrateMode::Cbr(bps) => Some(bps),
                    crate::aac::BitrateMode::Vbr(_) => None,
                };
                (Inner::Aac(Box::new(crate::aac::Encoder::new(config)?)), map, layout, bitrate, None)
            }
            #[cfg(feature = "alac-encode")]
            CodecId::Alac => {
                let count = u8::try_from(channels).unwrap_or(u8::MAX);
                let (map, layout) = fixed_map(EncodeFormat::Alac.layout_masks(channels), "ALAC", crate::alac::alac_order(count), &given)?;
                let bits = match opts.bit_depth {
                    crate::BitDepth::Int16 => 16,
                    crate::BitDepth::Int24 => 24,
                    crate::BitDepth::Int32 => 32,
                    crate::BitDepth::Float32 => return Err(AudioError::Unsupported("ALAC cannot store 32-bit float samples".into())),
                };
                let config = crate::alac::EncoderConfig::new(sample_rate, count, bits).with_frame_length(CHUNK as u32);
                let enc = crate::alac::Encoder::new(config)?;
                let quantizers = crate::write::pcm::Quantizer::per_channel(u32::from(bits), opts.dither, n);
                (Inner::Alac { enc: Box::new(enc), quantizers, pending: vec![Vec::new(); n] }, map, layout, None, Some(bits))
            }
            #[cfg(feature = "mp3-encode")]
            CodecId::Mp3 => {
                let (map, layout) = fixed_map(EncodeFormat::Mp3.layout_masks(channels), "MP3", None, &given)?;
                let want = constant_bps("MP3", opts.rate_control, DEFAULT_KBPS_PER_CHANNEL * u32::from(channels) * 1000)? / 1000;
                // A sample rate MP3 has not is the encoder's to refuse.
                let kbps = crate::mp3::nearest_bitrate(sample_rate, want).unwrap_or(want);
                let enc = crate::mp3::Encoder::new(crate::mp3::Config::cbr(sample_rate, n, kbps))?;
                (Inner::Mp3 { enc: Box::new(enc), bytes: Vec::new() }, map, layout, Some(kbps.saturating_mul(1000)), None)
            }
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            CodecId::Ac3 | CodecId::Eac3 => {
                let which = if codec == CodecId::Ac3 { crate::ac3::encoder::Codec::Ac3 } else { crate::ac3::encoder::Codec::Eac3 };
                let (config, map) = ac3_config(which, sample_rate, &given, opts.rate_control)?;
                let enc = crate::ac3::encoder::Encoder::new(config)?;
                let layout = enc.channel_layout();
                let bitrate = Some(config.bitrate_kbps.saturating_mul(1000));
                (Inner::Ac3 { enc: Box::new(enc), planes: vec![Vec::new(); n] }, map, layout, bitrate, None)
            }
            #[cfg(feature = "opus-encode")]
            CodecId::Opus => {
                let masks = n.checked_sub(1).and_then(|i| OPUS_LAYOUT_MASKS.get(i)).copied().unwrap_or_default();
                let (map, layout) = fixed_map(masks, "Opus", None, &given)?;
                if sample_rate != 48_000 {
                    return Err(AudioError::Unsupported(format!("Opus is encoded at 48000 Hz, not {sample_rate} Hz")));
                }
                let bps = constant_bps("Opus", opts.rate_control, OPUS_DEFAULT_BPS_PER_CHANNEL * u32::from(channels))?;
                let enc = crate::opus::encoder::Encoder::new(channels as u8, bps)?;
                let bitrate = Some(enc.bitrate());
                (Inner::Opus { enc: Box::new(enc), pending: Vec::new(), coded: 0 }, map, layout, bitrate, None)
            }
            #[allow(unreachable_patterns)]
            _ => return Err(AudioError::Unsupported(format!("no packet encoder for {}", codec.name()))),
        };
        Ok(PacketEncoder { codec, inner, sample_rate, layout, map, bitrate, bit_depth, pts: 0, frames: 0, finished: false, failed: false })
    }

    /// The codec of the packets.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// The sample rate of the audio and of the packets' timing.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The number of channels.
    pub fn channels(&self) -> u16 {
        self.layout.channels()
    }

    /// The speakers of the channels encoded, in WAV order: the codec's layout for the channel
    /// count (for AC-3 and E-AC-3, the coding mode's), which is what a decoder reports. Where the
    /// codec has one surround pair, it is the codec's (AAC's 5.1 has side surrounds, ALAC's back
    /// ones), whichever pair the caller's layout named.
    pub fn layout(&self) -> ChannelLayout {
        self.layout
    }

    /// The constant bit rate in bits per second for all channels together, as
    /// [`RateControl::Bitrate`] describes the rate the stream settles on; `None` for ALAC and
    /// for AAC at a constant quality.
    pub fn bitrate(&self) -> Option<u32> {
        self.bitrate
    }

    /// The bits per sample of a lossless codec (ALAC), `None` for the lossy ones.
    pub fn bit_depth(&self) -> Option<u8> {
        self.bit_depth
    }

    /// The codec configuration a container stores: AAC's `AudioSpecificConfig` (MP4 `esds`
    /// decoder-specific info, Matroska CodecPrivate), ALAC's configuration as MP4's `alac` box
    /// and Matroska's CodecPrivate hold it, Opus's `OpusHead`; empty for MP3, AC-3 and E-AC-3.
    /// ALAC's records the largest packet and the average bit rate so far, so read it after
    /// [`finish`](Self::finish).
    pub fn extradata(&self) -> Vec<u8> {
        match &self.inner {
            #[cfg(feature = "aac-encode")]
            Inner::Aac(enc) => enc.audio_specific_config(),
            #[cfg(feature = "alac-encode")]
            Inner::Alac { enc, .. } => enc.cookie(),
            #[cfg(feature = "opus-encode")]
            Inner::Opus { enc, .. } => enc.opus_head(),
            #[allow(unreachable_patterns)]
            _ => Vec::new(),
        }
    }

    /// The codec, sample rate, channels and [`extradata`](Self::extradata): the description
    /// [`TrackDecoder::new`](crate::TrackDecoder::new) takes, and what a muxer's track needs.
    pub fn codec_params(&self) -> CodecParams {
        CodecParams::new(self.codec).with_sample_rate(self.sample_rate).with_channels(self.channels()).with_extradata(self.extradata())
    }

    /// Decoded samples before the first input frame: the encoder's delay, which a container
    /// skips (an MP4 edit list's media time, Matroska's CodecDelay, Opus's pre-skip, which
    /// `OpusHead` already holds). 0 for ALAC.
    ///
    /// This is the codec's delay and does not depend on the input. An encoder that is given no
    /// input returns no packets, and this still reports the delay.
    pub fn priming(&self) -> u32 {
        match &self.inner {
            #[cfg(feature = "aac-encode")]
            Inner::Aac(enc) => enc.priming_samples(),
            #[cfg(feature = "mp3-encode")]
            Inner::Mp3 { enc, .. } => enc.delay(),
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            Inner::Ac3 { enc, .. } => enc.priming_samples(),
            #[cfg(feature = "opus-encode")]
            Inner::Opus { enc, .. } => u32::from(enc.pre_skip()),
            #[allow(unreachable_patterns)]
            _ => 0,
        }
    }

    /// Frames of input taken so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Decoded samples after the last input frame, which a container trims (the end of an MP4
    /// edit, Matroska's DiscardPadding): the packets' durations add up to
    /// [`priming`](Self::priming) + [`frames`](Self::frames) + this. Complete after
    /// [`finish`](Self::finish).
    ///
    /// The sum holds once there is input. With none, [`finish`](Self::finish) returns no packets,
    /// so the durations add up to 0 and this is 0, while `priming` is still the codec's delay: a
    /// muxer writes an empty track and has no priming to skip.
    pub fn padding(&self) -> u64 {
        self.pts.saturating_sub(u64::from(self.priming()).saturating_add(self.frames))
    }

    /// Encodes the first `frames` frames of `block`, one slice per channel in WAV order
    /// (`&[Vec<f32>]`, `&[&[f32]]`, …), and returns the packets completed. Samples are full scale
    /// at ±1; NaN is taken as 0, and what a codec cannot carry above full scale is clipped.
    ///
    /// Internal codec calls process at most 4096 frames at a time, but this method collects their
    /// packets in one returned vector whose memory use grows with `frames`. For streaming with
    /// bounded memory, pass bounded input blocks and consume each returned batch before encoding
    /// the next block, as [`FileEncoder`](crate::FileEncoder) does for coded formats.
    ///
    /// `block` must have one slice per channel, each at least `frames` long; otherwise nothing is
    /// encoded and the error is [`AudioError::Encode`]. A call after [`finish`](Self::finish) is
    /// an error too. When encoding itself fails, the packets of the block are lost, and every
    /// later call fails rather than return a stream with a gap.
    pub fn encode<C: AsRef<[f32]>>(&mut self, block: &[C], frames: usize) -> Result<Vec<EncodedPacket>> {
        self.usable()?;
        let channels = usize::from(self.channels());
        if block.len() != channels {
            return Err(AudioError::Encode(format!("a block of {} channels for a {channels}-channel stream", block.len())));
        }
        if let Some(short) = block.iter().map(|c| c.as_ref().len()).filter(|&n| n < frames).min() {
            return Err(AudioError::Encode(format!("{frames} frames asked for, but a channel holds {short}")));
        }
        let mut out = Vec::new();
        for at in (0..frames).step_by(CHUNK) {
            let end = frames.min(at + CHUNK);
            let planes: Vec<&[f32]> = self.map.iter().map(|&c| block.get(c).and_then(|p| p.as_ref().get(at..end)).unwrap_or_default()).collect();
            if let Err(e) = self.push(&planes, &mut out) {
                self.failed = true;
                return Err(e);
            }
            self.frames = self.frames.saturating_add((end - at) as u64);
        }
        Ok(out)
    }

    /// Encodes what the encoder still holds, padded with silence to whole packets and as far as
    /// a decoder needs to return every input frame after the priming, and returns the last
    /// packets. Later calls of [`encode`](Self::encode) and `finish` are errors; the description
    /// ([`extradata`](Self::extradata), [`padding`](Self::padding)) stays readable.
    pub fn finish(&mut self) -> Result<Vec<EncodedPacket>> {
        self.usable()?;
        self.finished = true;
        let mut out = Vec::new();
        let done = self.drain(&mut out);
        if done.is_err() {
            self.failed = true;
        }
        done.map(|()| out)
    }

    fn usable(&self) -> Result<()> {
        if self.failed {
            Err(AudioError::Encode("the packet encoder already failed; its stream is incomplete".into()))
        } else if self.finished {
            Err(AudioError::Encode("the packet encoder is finished".into()))
        } else {
            Ok(())
        }
    }

    /// Appends a packet of `duration` decoded samples at the current position.
    fn emit(pts: &mut u64, out: &mut Vec<EncodedPacket>, data: Vec<u8>, duration: u32) {
        out.push(EncodedPacket { data, pts: *pts, duration });
        *pts = pts.saturating_add(u64::from(duration));
    }

    /// Encodes `planes` (one slice per codec channel, equally long, at most [`CHUNK`] frames)
    /// and appends the packets completed.
    fn push(&mut self, planes: &[&[f32]], out: &mut Vec<EncodedPacket>) -> Result<()> {
        let pts = &mut self.pts;
        match &mut self.inner {
            #[cfg(feature = "aac-encode")]
            Inner::Aac(enc) => {
                for data in enc.encode(planes) {
                    Self::emit(pts, out, data, crate::aac::FRAME_LEN as u32);
                }
            }
            #[cfg(feature = "alac-encode")]
            Inner::Alac { enc, quantizers, pending } => {
                let frames = planes.first().map_or(0, |p| p.len());
                let mut at = 0;
                while at < frames {
                    let filled = pending.first().map_or(0, Vec::len);
                    let end = frames.min(at + CHUNK.saturating_sub(filled).max(1));
                    for ((dst, q), src) in pending.iter_mut().zip(quantizers.iter_mut()).zip(planes) {
                        dst.extend(src.get(at..end).unwrap_or_default().iter().map(|&s| q.quantize(s)));
                    }
                    at = end;
                    if pending.first().is_some_and(|p| p.len() >= CHUNK) {
                        alac_packet(enc, pending, pts, out)?;
                    }
                }
            }
            #[cfg(feature = "mp3-encode")]
            Inner::Mp3 { enc, bytes } => {
                bytes.extend(enc.encode(planes));
                mp3_frames(enc, bytes, pts, out)?;
            }
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            Inner::Ac3 { enc, planes: clean } => {
                for (dst, src) in clean.iter_mut().zip(planes) {
                    dst.clear();
                    dst.extend(src.iter().map(|&s| if s.is_nan() { 0.0 } else { s.clamp(-1.0, 1.0) }));
                }
                let refs: Vec<&[f32]> = clean.iter().map(Vec::as_slice).collect();
                for p in enc.push(&refs)? {
                    Self::emit(pts, out, p.data, p.samples);
                }
            }
            #[cfg(feature = "opus-encode")]
            Inner::Opus { enc, pending, coded } => {
                let frames = planes.first().map_or(0, |p| p.len());
                for i in 0..frames {
                    pending.extend(planes.iter().map(|p| p.get(i).copied().map_or(0.0, |s| if s.is_nan() { 0.0 } else { s.clamp(-1.0, 1.0) })));
                    if pending.len() == crate::opus::encoder::Encoder::FRAME_SIZE * planes.len() {
                        opus_packet(enc, pending, coded, pts, out)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Encodes what the codec still holds and appends the last packets.
    fn drain(&mut self, out: &mut Vec<EncodedPacket>) -> Result<()> {
        let pts = &mut self.pts;
        match &mut self.inner {
            #[cfg(feature = "aac-encode")]
            Inner::Aac(enc) => {
                for data in enc.flush() {
                    Self::emit(pts, out, data, crate::aac::FRAME_LEN as u32);
                }
            }
            #[cfg(feature = "alac-encode")]
            Inner::Alac { enc, pending, .. } => {
                if pending.first().is_some_and(|p| !p.is_empty()) {
                    alac_packet(enc, pending, pts, out)?;
                }
            }
            #[cfg(feature = "mp3-encode")]
            Inner::Mp3 { enc, bytes } => {
                bytes.extend(enc.flush());
                mp3_frames(enc, bytes, pts, out)?;
                if !bytes.is_empty() {
                    return Err(AudioError::Encode(format!("the MP3 encoder ended with {} bytes that are not a whole frame", bytes.len())));
                }
            }
            #[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
            Inner::Ac3 { enc, .. } => {
                for p in enc.finish()?.packets {
                    Self::emit(pts, out, p.data, p.samples);
                }
            }
            #[cfg(feature = "opus-encode")]
            Inner::Opus { enc, pending, coded } => {
                // Every input frame decodes after the pre-skip: as many packets as that takes.
                let size = crate::opus::encoder::Encoder::FRAME_SIZE as u64;
                let needed = if self.frames == 0 { 0 } else { (self.frames + u64::from(enc.pre_skip())).div_ceil(size) * size };
                let width = usize::from(enc.channels());
                while *coded < needed {
                    pending.resize(crate::opus::encoder::Encoder::FRAME_SIZE * width, 0.0);
                    opus_packet(enc, pending, coded, pts, out)?;
                }
            }
        }
        Ok(())
    }
}

/// Encodes the pending samples as one ALAC packet and empties them.
#[cfg(feature = "alac-encode")]
fn alac_packet(enc: &mut crate::alac::Encoder, pending: &mut [Vec<i32>], pts: &mut u64, out: &mut Vec<EncodedPacket>) -> Result<()> {
    let refs: Vec<&[i32]> = pending.iter().map(Vec::as_slice).collect();
    let frames = refs.first().map_or(0, |p| p.len()) as u32;
    let data = enc.encode(&refs)?;
    pending.iter_mut().for_each(Vec::clear);
    PacketEncoder::emit(pts, out, data, frames);
    Ok(())
}

/// Encodes one full frame of pending interleaved samples as an Opus packet and empties them.
#[cfg(feature = "opus-encode")]
fn opus_packet(
    enc: &mut crate::opus::encoder::Encoder,
    pending: &mut Vec<f32>,
    coded: &mut u64,
    pts: &mut u64,
    out: &mut Vec<EncodedPacket>,
) -> Result<()> {
    let data = enc.encode_float(pending)?;
    pending.clear();
    let size = crate::opus::encoder::Encoder::FRAME_SIZE as u32;
    *coded += u64::from(size);
    PacketEncoder::emit(pts, out, data, size);
    Ok(())
}

/// Splits the whole MP3 frames off the front of `bytes`. The encoder writes constant-rate frames
/// back to back, so each one's length follows from its header's padding bit.
#[cfg(feature = "mp3-encode")]
fn mp3_frames(enc: &crate::mp3::Encoder, bytes: &mut Vec<u8>, pts: &mut u64, out: &mut Vec<EncodedPacket>) -> Result<()> {
    let samples = enc.frame_samples();
    let config = enc.config();
    // Bytes per frame: samples / 8 × bit rate / sample rate, plus the padding slot.
    let base = (samples as u64 / 8 * u64::from(config.bitrate_kbps) * 1000 / u64::from(config.sample_rate.max(1))) as usize;
    let mut at = 0;
    while let Some(header) = bytes.get(at..at + 4) {
        if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
            return Err(AudioError::Encode("the MP3 encoder returned bytes that are not a frame".into()));
        }
        let len = base + usize::from((header[2] >> 1) & 1);
        let Some(frame) = bytes.get(at..at + len).filter(|_| len > 4) else { break };
        PacketEncoder::emit(pts, out, frame.to_vec(), samples as u32);
        at += len;
    }
    bytes.drain(..at);
    Ok(())
}

#[cfg(test)]
mod tests;

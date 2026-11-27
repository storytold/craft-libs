//! [`TrackEncoder`]: [`VideoFrame`]s in, one video track's packets out, whatever the codec.
//! The encoding counterpart of `TrackDecoder`.

use crate::error::VideoError;
use crate::frame::{ColorTags, VideoFrame};
use craft_media::{CodecId, CodecParams};

/// One encoded video packet produced by [`TrackEncoder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedPacket {
    /// Coded bitstream bytes of the access unit / frame.
    pub data: Vec<u8>,
    /// Presentation timestamp, in the caller's units.
    pub pts: i64,
    /// Decoding timestamp, in the caller's units.
    pub dts: i64,
    /// True when decoding can begin at this packet.
    pub keyframe: bool,
}

/// Rate control mode for [`TrackEncoderConfig`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RateControlMode {
    /// Codec default rate control (constant quality unless `bitrate_kbps` is set).
    #[default]
    Auto,
    /// Constant bitrate targeting `bitrate_kbps`.
    Cbr,
    /// Variable bitrate targeting `bitrate_kbps`.
    Vbr,
    /// Constant quality / quantiser.
    ConstantQuality,
}

/// Configuration for [`TrackEncoder::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackEncoderConfig {
    /// Target video codec.
    pub codec: CodecId,
    /// Coded frame width in luma samples.
    pub width: u32,
    /// Coded frame height in luma samples.
    pub height: u32,
    /// Frame rate numerator (e.g. `30000` or `30`).
    pub fps_num: u32,
    /// Frame rate denominator (e.g. `1001` or `1`).
    pub fps_den: u32,
    /// Optional target bitrate in kilobits per second.
    pub bitrate_kbps: Option<u32>,
    /// Rate control mode.
    pub rate_mode: RateControlMode,
    /// Optional keyframe interval in frames (`1` = all intra).
    pub keyframe_interval: Option<u32>,
    /// Optional maximum consecutive B-frames (`0` = no B-frames).
    pub bframes: Option<u8>,
    /// Optional codec-specific profile name (e.g. `"baseline"`, `"main"`, `"high"`, `"hq"`, `"lt"`, `"proxy"`, `"hqx"`, `"422-10"`, `"422-12"`).
    pub profile: Option<String>,
    /// Signalled colour description (ITU-T H.273 code points).
    pub color: ColorTags,
    /// Emit Annex B start-code NAL units for H.264 instead of length-prefixed (`avcC`) samples.
    pub annex_b: bool,
}

impl TrackEncoderConfig {
    /// Default configuration for `codec` at `width`×`height` and `fps_num`/`fps_den` frames per second.
    pub fn new(codec: CodecId, width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        Self {
            codec,
            width,
            height,
            fps_num,
            fps_den,
            bitrate_kbps: None,
            rate_mode: RateControlMode::Auto,
            keyframe_interval: None,
            bframes: None,
            profile: None,
            color: ColorTags::UNSPECIFIED,
            annex_b: false,
        }
    }

    #[cfg(any(feature = "hevc-encode", feature = "av1-encode", feature = "vp9-encode", feature = "prores", feature = "dnx"))]
    fn reject_unsupported(&self, codec_name: &str, bframes: bool, profile: bool) -> Result<(), VideoError> {
        if !bframes && self.bframes.is_some_and(|b| b > 0) {
            return Err(VideoError::Unsupported(format!("{codec_name} encoder does not support B-frames")));
        }
        if !profile && let Some(p) = &self.profile {
            return Err(VideoError::Unsupported(format!("{codec_name} encoder has no selectable profile (requested `{p}`)")));
        }
        Ok(())
    }
}

enum Inner {
    #[cfg(feature = "h264-encode")]
    H264(Box<crate::h264::Encoder>),
    #[cfg(feature = "hevc-encode")]
    Hevc(Box<crate::hevc::Encoder>),
    #[cfg(feature = "av1-encode")]
    Av1(Box<crate::av1::Encoder>),
    #[cfg(feature = "vp9-encode")]
    Vp9(Box<crate::vp9::Vp9Encoder>),
    #[cfg(feature = "vp8-encode")]
    Vp8 { width: u32, height: u32, params: crate::vp8::Vp8Params },
    #[cfg(feature = "mpeg2-encode")]
    Mpeg2 { enc: Box<crate::mpeg2::encode::Encoder>, width: u32, height: u32 },
    #[cfg(feature = "mpeg4-encode")]
    Mpeg4(Box<crate::mpeg4::Encoder>),
    #[cfg(feature = "prores")]
    ProRes(Box<crate::prores::Encoder>),
    #[cfg(feature = "dnx")]
    Dnx(Box<crate::dnx::Encoder>),
    #[cfg(feature = "apv-encode")]
    Apv(Box<crate::apv::Encoder>),
    #[cfg(feature = "ffv1")]
    Ffv1 { enc: Box<crate::ffv1::Encoder>, width: u32, height: u32 },
    #[cfg(feature = "mjpeg")]
    Mjpeg { enc: crate::mjpeg::Encoder, width: u32, height: u32 },
}

/// A codec-agnostic video track encoder: takes [`VideoFrame`]s in display order (validated by
/// [`crate::frame::encode`]) and returns [`EncodedPacket`]s in decode order, with
/// [`TrackEncoder::codec_params`] describing the track for a container muxer or `TrackDecoder`.
pub struct TrackEncoder {
    inner: Inner,
    codec: CodecId,
    width: u32,
    height: u32,
}

impl std::fmt::Debug for TrackEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackEncoder").field("codec", &self.codec).field("width", &self.width).field("height", &self.height).finish_non_exhaustive()
    }
}

impl TrackEncoder {
    /// Builds an encoder for the track described by `cfg`.
    ///
    /// Returns [`VideoError::Unsupported`] when `cfg.codec` has no encoder in this build or when
    /// `cfg` asks for a profile or B-frame count the codec does not support, and
    /// [`VideoError::Malformed`] when the dimensions or frame rate are invalid for the codec.
    pub fn new(cfg: TrackEncoderConfig) -> Result<Self, VideoError> {
        if cfg.width == 0 || cfg.height == 0 {
            return Err(VideoError::Malformed("frame dimensions must be non-zero".into()));
        }
        if cfg.fps_num == 0 {
            return Err(VideoError::Malformed("frame rate numerator must be non-zero".into()));
        }
        if cfg.fps_den == 0 {
            return Err(VideoError::Malformed("frame rate denominator must be non-zero".into()));
        }
        let fps_num = cfg.fps_num;
        let fps_den = cfg.fps_den;
        let width = cfg.width;
        let height = cfg.height;
        let codec = cfg.codec;
        let _ = (fps_num, fps_den);

        let inner = match codec {
            #[cfg(feature = "h264-encode")]
            CodecId::H264 => {
                let mut hcfg = crate::h264::EncoderConfig::new(width, height, fps_num, fps_den);
                hcfg.format = if cfg.annex_b { crate::h264::PacketFormat::AnnexB } else { crate::h264::PacketFormat::LengthPrefixed };
                hcfg.rate = match (cfg.rate_mode, cfg.bitrate_kbps) {
                    (RateControlMode::ConstantQuality, _) | (_, None) => crate::h264::RateControl::Crf(23.0),
                    (RateControlMode::Vbr, Some(kbps)) => crate::h264::RateControl::Vbr { target_kbps: kbps, max_kbps: kbps.saturating_mul(3) / 2 },
                    (_, Some(kbps)) => crate::h264::RateControl::Cbr { kbps },
                };
                if let Some(interval) = cfg.keyframe_interval {
                    hcfg.keyint = interval.max(1);
                }
                hcfg.profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("high") => crate::h264::Profile::High,
                    Some("main") => crate::h264::Profile::Main,
                    Some("baseline") => crate::h264::Profile::Baseline,
                    Some("high10") => crate::h264::Profile::High10,
                    Some(other) => return Err(VideoError::Unsupported(format!("H.264: unknown profile `{other}`"))),
                };
                if hcfg.profile == crate::h264::Profile::High10 {
                    hcfg.bit_depth = 10;
                }
                if let Some(b) = cfg.bframes {
                    if b > 3 {
                        return Err(VideoError::Unsupported(format!("H.264: supports at most 3 B-frames, requested {b}")));
                    }
                    if b > 0 && hcfg.profile == crate::h264::Profile::Baseline {
                        return Err(VideoError::Unsupported("H.264: the baseline profile has no B-frames".into()));
                    }
                    hcfg.bframes = b;
                }
                if cfg.color != ColorTags::UNSPECIFIED {
                    hcfg.color = crate::h264::ColorConfig {
                        primaries: cfg.color.primaries,
                        transfer: cfg.color.transfer,
                        matrix: cfg.color.matrix,
                        full_range: cfg.color.full_range,
                    };
                }
                Inner::H264(Box::new(crate::h264::Encoder::new(hcfg)?))
            }
            #[cfg(feature = "hevc-encode")]
            CodecId::Hevc => {
                cfg.reject_unsupported("HEVC", false, true)?;
                let mut hcfg = crate::hevc::EncoderConfig::new(width, height, fps_num, fps_den);
                hcfg.profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("main") => crate::hevc::Profile::Main,
                    Some("main10") => crate::hevc::Profile::Main10,
                    Some(other) => return Err(VideoError::Unsupported(format!("HEVC: unknown profile `{other}`"))),
                };
                hcfg.rate = match (cfg.rate_mode, cfg.bitrate_kbps) {
                    (RateControlMode::ConstantQuality, _) | (_, None) => crate::hevc::RateControl::ConstantQp(28),
                    (_, Some(kbps)) => crate::hevc::RateControl::Bitrate { kbps },
                };
                if let Some(interval) = cfg.keyframe_interval {
                    hcfg.keyint = interval.max(1);
                }
                if cfg.color != ColorTags::UNSPECIFIED {
                    hcfg.colour_primaries = cfg.color.primaries;
                    hcfg.transfer_characteristics = cfg.color.transfer;
                    hcfg.matrix_coeffs = cfg.color.matrix;
                    hcfg.full_range = cfg.color.full_range;
                }
                Inner::Hevc(Box::new(crate::hevc::Encoder::new(hcfg)?))
            }
            #[cfg(feature = "av1-encode")]
            CodecId::Av1 => {
                cfg.reject_unsupported("AV1", false, false)?;
                let mut acfg = crate::av1::EncoderConfig::new(width, height, fps_num, fps_den);
                acfg.bit_depth = 8;
                acfg.rate = match (cfg.rate_mode, cfg.bitrate_kbps) {
                    (RateControlMode::ConstantQuality, _) | (_, None) => crate::av1::RateControl::ConstantQ(100),
                    (_, Some(kbps)) => crate::av1::RateControl::Bitrate { kbps },
                };
                if let Some(interval) = cfg.keyframe_interval {
                    acfg.keyint = interval.max(1);
                }
                if cfg.color != ColorTags::UNSPECIFIED {
                    acfg.color_primaries = cfg.color.primaries;
                    acfg.transfer_characteristics = cfg.color.transfer;
                    acfg.matrix_coefficients = cfg.color.matrix;
                    acfg.full_range = cfg.color.full_range;
                }
                Inner::Av1(Box::new(crate::av1::Encoder::new(acfg)?))
            }
            #[cfg(feature = "vp9-encode")]
            CodecId::Vp9 => {
                cfg.reject_unsupported("VP9", false, false)?;
                if !matches!(cfg.color.primaries, 0..=2) || !matches!(cfg.color.transfer, 0..=2) || !matches!(cfg.color.matrix, 0..=2) {
                    return Err(VideoError::Unsupported(format!(
                        "VP9: encoder signals BT.709 only (requested primaries={}, transfer={}, matrix={})",
                        cfg.color.primaries, cfg.color.transfer, cfg.color.matrix
                    )));
                }
                let target_kbps = if cfg.rate_mode == RateControlMode::ConstantQuality { None } else { cfg.bitrate_kbps };
                let vcfg = crate::vp9::EncoderConfig {
                    width,
                    height,
                    quality: 80,
                    full_range: cfg.color.full_range,
                    keyframe_interval: cfg.keyframe_interval.map_or(60, |k| k.max(1)),
                    target_kbps,
                    frame_rate: f64::from(fps_num) / f64::from(fps_den),
                    loop_filter: true,
                };
                Inner::Vp9(Box::new(crate::vp9::Vp9Encoder::new(vcfg)))
            }
            #[cfg(feature = "vp8-encode")]
            CodecId::Vp8 => {
                if let Some(profile) = cfg.profile.as_deref() {
                    return Err(VideoError::Unsupported(format!("VP8: profile `{profile}` cannot be selected")));
                }
                if width > crate::vp8::MAX_SIDE || height > crate::vp8::MAX_SIDE {
                    return Err(VideoError::Malformed(format!("VP8: dimensions {width}x{height} exceed {}", crate::vp8::MAX_SIDE)));
                }
                Inner::Vp8 { width, height, params: crate::vp8::Vp8Params::default() }
            }
            #[cfg(feature = "mpeg2-encode")]
            CodecId::Mpeg2Video => {
                if cfg.color.full_range {
                    return Err(VideoError::Unsupported("MPEG-2: full-range samples cannot be signalled".into()));
                }
                let (m_num, m_den) = nearest_mpeg2_fps(fps_num, fps_den);
                let mut mcfg = crate::mpeg2::encode::EncoderConfig::new(width, height, m_num, m_den);
                mcfg.chroma = crate::mpeg2::ChromaFormat::Yuv420;
                mcfg.aspect = if u64::from(width) * 9 == u64::from(height) * 16 {
                    crate::mpeg2::encode::AspectRatio::Dar16x9
                } else {
                    crate::mpeg2::encode::AspectRatio::Square
                };
                mcfg.gop_size = cfg.keyframe_interval.unwrap_or(12).clamp(1, 1023);
                mcfg.b_pictures = u32::from(cfg.bframes.unwrap_or(0).min(4));
                let kbps = cfg.bitrate_kbps.unwrap_or(6000).clamp(400, 80_000);
                mcfg.rate = match (cfg.rate_mode, cfg.bitrate_kbps) {
                    (RateControlMode::ConstantQuality, _) | (RateControlMode::Auto, None) => crate::mpeg2::encode::RateControl::Quantiser(4),
                    (RateControlMode::Cbr, _) => crate::mpeg2::encode::RateControl::Cbr { kbps },
                    (RateControlMode::Vbr, _) => {
                        crate::mpeg2::encode::RateControl::Vbr { kbps, max_kbps: kbps.saturating_mul(3).saturating_div(2).min(80_000) }
                    }
                    (RateControlMode::Auto, Some(_)) => crate::mpeg2::encode::RateControl::Abr { kbps },
                };
                let enc = crate::mpeg2::encode::Encoder::new(mcfg).map_err(|e| match e {
                    crate::mpeg2::encode::EncodeError::Config(_) => VideoError::Malformed(format!("MPEG-2: {e}")),
                    other => VideoError::from(other),
                })?;
                let enc = Box::new(enc);
                Inner::Mpeg2 { enc, width, height }
            }
            #[cfg(feature = "mpeg4-encode")]
            CodecId::Mpeg4Video => {
                let (m_num, m_den) = reduced_mpeg4_fps(fps_num, fps_den);
                let mut mcfg = crate::mpeg4::EncoderConfig::new(width, height, m_num, m_den);
                mcfg.keyint = cfg.keyframe_interval.unwrap_or(250).max(1);
                mcfg.headers_in_band = true;
                mcfg.rate = match (cfg.rate_mode, cfg.bitrate_kbps) {
                    (RateControlMode::ConstantQuality, _) | (_, None) => crate::mpeg4::RateControl::Quant(4),
                    (_, Some(kbps)) => crate::mpeg4::RateControl::Bitrate { kbps: kbps.max(100) },
                };
                if cfg.color != ColorTags::UNSPECIFIED {
                    mcfg.color = Some(crate::mpeg4::ColorConfig {
                        primaries: cfg.color.primaries,
                        transfer: cfg.color.transfer,
                        matrix: cfg.color.matrix,
                        full_range: cfg.color.full_range,
                    });
                }
                Inner::Mpeg4(Box::new(crate::mpeg4::Encoder::new(mcfg)?))
            }
            #[cfg(feature = "prores")]
            CodecId::ProRes => {
                cfg.reject_unsupported("ProRes", false, true)?;
                if cfg.color.full_range {
                    return Err(VideoError::Unsupported("ProRes: full-range samples cannot be signalled".into()));
                }
                let profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("422") | Some("standard") => crate::prores::Profile::Standard,
                    Some("proxy") => crate::prores::Profile::Proxy,
                    Some("lt") => crate::prores::Profile::Lt,
                    Some("hq") | Some("422hq") => crate::prores::Profile::Hq,
                    Some(other) => {
                        return Err(VideoError::Unsupported(format!("ProRes: profile `{other}` is not available (4:2:2 profiles only)")));
                    }
                };
                let mut pcfg = crate::prores::EncoderConfig::new(profile, width, height);
                if cfg.color != ColorTags::UNSPECIFIED {
                    let h273_or = |v: u8, default: u8| if matches!(v, 0 | 2) { default } else { v };
                    pcfg.color = crate::prores::ColorInfo {
                        primaries: h273_or(cfg.color.primaries, pcfg.color.primaries),
                        transfer: h273_or(cfg.color.transfer, pcfg.color.transfer),
                        matrix: h273_or(cfg.color.matrix, pcfg.color.matrix),
                    };
                }
                Inner::ProRes(Box::new(crate::prores::Encoder::with_config(pcfg)))
            }
            #[cfg(feature = "dnx")]
            CodecId::Dnxhd => {
                cfg.reject_unsupported("DNxHR", false, true)?;
                if cfg.color.full_range {
                    return Err(VideoError::Unsupported("DNxHR: full-range samples cannot be signalled".into()));
                }
                let color_volume = match cfg.color.matrix {
                    0..=2 => crate::dnx::ColorVolume::Bt709,
                    9 => crate::dnx::ColorVolume::Bt2020Ncl,
                    10 => crate::dnx::ColorVolume::Bt2020Cl,
                    other => {
                        return Err(VideoError::Unsupported(format!(
                            "DNxHR: matrix {other} has no DNx colour volume (supported: BT.709=1, BT.2020 NCL=9, BT.2020 CL=10)"
                        )));
                    }
                };
                let profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("hq") => crate::dnx::Profile::Hq,
                    Some("lb") => crate::dnx::Profile::Lb,
                    Some("sq") => crate::dnx::Profile::Sq,
                    Some("hqx") => crate::dnx::Profile::Hqx,
                    Some(other) => {
                        return Err(VideoError::Unsupported(format!("DNxHR: profile `{other}` is not available (LB, SQ, HQ, HQX)")));
                    }
                };
                let mut dcfg = crate::dnx::EncoderConfig::new(profile, width, height);
                dcfg.color_volume = color_volume;
                Inner::Dnx(Box::new(crate::dnx::Encoder::with_config(dcfg)?))
            }
            #[cfg(feature = "apv-encode")]
            CodecId::Apv => {
                let profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("422-10") => crate::apv::Profile::P422_10,
                    Some("422-12") => crate::apv::Profile::P422_12,
                    Some(other) => return Err(VideoError::Unsupported(format!("APV: unsupported profile `{other}`"))),
                };
                let mut acfg = crate::apv::EncoderConfig::new(profile, width, height);
                acfg.chroma = crate::apv::ChromaFormat::Yuv422;
                acfg.bit_depth = if profile == crate::apv::Profile::P422_12 { 12 } else { 10 };
                if cfg.color != ColorTags::UNSPECIFIED {
                    acfg.color = crate::apv::ColorInfo {
                        primaries: cfg.color.primaries,
                        transfer: cfg.color.transfer,
                        matrix: cfg.color.matrix,
                        full_range: cfg.color.full_range,
                    };
                }
                if let (Some(kbps), false) = (cfg.bitrate_kbps, cfg.rate_mode == RateControlMode::ConstantQuality) {
                    let bytes = (u64::from(kbps) * 1000 / 8).saturating_mul(u64::from(fps_den)) / u64::from(fps_num);
                    acfg.target_frame_bytes = bytes.clamp(512, 64_000_000) as u32;
                }
                Inner::Apv(Box::new(crate::apv::Encoder::with_config(acfg)?))
            }
            #[cfg(feature = "ffv1")]
            CodecId::Ffv1 => {
                let len = usize::try_from(width)
                    .ok()
                    .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
                    .ok_or_else(|| VideoError::Malformed("FFV1: dimensions overflow".into()))?;
                // The template holds three planes of 16-bit samples; refuse sizes the decode budget would refuse.
                let template_bytes = len.checked_mul(3 * std::mem::size_of::<u16>());
                if template_bytes.is_none_or(|b| b as u64 > crate::budget::DECODE_MEMORY_BUDGET) {
                    return Err(VideoError::Malformed(format!("FFV1: dimensions {width}x{height} exceed the memory budget")));
                }
                let template = crate::ffv1::Frame {
                    width,
                    height,
                    colorspace: crate::ffv1::Colorspace::Rgb,
                    bit_depth: 8,
                    chroma_shift: (0, 0),
                    planes: vec![vec![0; len]; 3],
                };
                let enc = Box::new(crate::ffv1::Encoder::new(&template, crate::ffv1::EncoderConfig::default())?);
                Inner::Ffv1 { enc, width, height }
            }
            #[cfg(feature = "mjpeg")]
            CodecId::Mjpeg => {
                let enc = crate::mjpeg::Encoder::new(crate::mjpeg::EncoderConfig::default())?;
                Inner::Mjpeg { enc, width, height }
            }
            other => return Err(VideoError::Unsupported(format!("no {} encoder in this build", other.name()))),
        };

        Ok(Self { inner, codec, width, height })
    }

    /// The track's codec.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// Coded luma width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Coded luma height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Track description for container muxers and `TrackDecoder::new`.
    pub fn codec_params(&self) -> CodecParams {
        match &self.inner {
            #[cfg(feature = "h264-encode")]
            Inner::H264(enc) => {
                if matches!(enc.config().format, crate::h264::PacketFormat::AnnexB) {
                    // Annex B packets carry their own parameter sets; an `avcC` record would
                    // describe length-prefixed packets and mislead a decoder.
                    CodecParams::new(CodecId::H264).with_size(self.width, self.height)
                } else {
                    enc.codec_params()
                }
            }
            #[cfg(feature = "hevc-encode")]
            Inner::Hevc(enc) => enc.codec_params(),
            #[cfg(feature = "av1-encode")]
            Inner::Av1(enc) => enc.codec_params(),
            #[cfg(feature = "vp9-encode")]
            Inner::Vp9(enc) => enc.codec_params(),
            #[cfg(feature = "vp8-encode")]
            Inner::Vp8 { width, height, .. } => CodecParams::new(CodecId::Vp8).with_tag(*b"VP80").with_size(*width, *height),
            #[cfg(feature = "mpeg2-encode")]
            Inner::Mpeg2 { enc, width, height, .. } => {
                CodecParams::new(CodecId::Mpeg2Video).with_tag(*b"mp2v").with_size(*width, *height).with_extradata(enc.sequence_header().to_vec())
            }
            #[cfg(feature = "mpeg4-encode")]
            Inner::Mpeg4(enc) => enc.codec_params(),
            #[cfg(feature = "prores")]
            Inner::ProRes(enc) => enc.codec_params(),
            #[cfg(feature = "dnx")]
            Inner::Dnx(enc) => enc.codec_params(),
            #[cfg(feature = "apv-encode")]
            Inner::Apv(enc) => {
                CodecParams::new(CodecId::Apv).with_tag(*b"apv1").with_size(self.width, self.height).with_extradata(enc.decoder_config_record())
            }
            #[cfg(feature = "ffv1")]
            Inner::Ffv1 { enc, width, height } => {
                CodecParams::new(CodecId::Ffv1).with_tag(*b"FFV1").with_size(*width, *height).with_extradata(enc.config_record())
            }
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg { width, height, .. } => CodecParams::new(CodecId::Mjpeg).with_tag(*b"mjpa").with_size(*width, *height),
        }
    }

    /// Encode one [`VideoFrame`] with presentation timestamp `pts`.
    ///
    /// Input frames are validated and unpacked via [`crate::frame::encode`]: their storage layout
    /// is normalized without silently altering colour space, chroma subsampling, range, or bit
    /// depth, returning [`VideoError::Frame`] when the frame format does not match the encoder.
    /// Encoders with B-frame reordering may return zero packets until future frames or
    /// [`TrackEncoder::flush`] arrive.
    pub fn encode(&mut self, frame: &VideoFrame, pts: i64) -> Result<Vec<EncodedPacket>, VideoError> {
        #[cfg(any(feature = "h264-encode", feature = "vp9-encode", feature = "mpeg2-encode", feature = "mpeg4-encode"))]
        use crate::frame::encode::Planar8;
        #[cfg(any(feature = "h264-encode", feature = "hevc-encode", feature = "av1-encode"))]
        use crate::frame::encode::Planar16;
        #[cfg(feature = "vp8-encode")]
        use crate::frame::encode::Rgba8;

        if frame.width() != self.width || frame.height() != self.height {
            return Err(VideoError::Malformed(format!("frame is {}x{}, encoder is {}x{}", frame.width(), frame.height(), self.width, self.height)));
        }
        match &mut self.inner {
            #[cfg(feature = "h264-encode")]
            Inner::H264(enc) => {
                let packets = if enc.config().bit_depth > 8 {
                    let planar = Planar16::try_from(frame)?;
                    let input = planar.h264()?;
                    enc.encode16(&input, pts)?
                } else {
                    let planar = Planar8::try_from(frame)?;
                    let input = planar.h264()?;
                    enc.encode(&input, pts)?
                };
                Ok(packets.into_iter().map(|p| EncodedPacket { data: p.data, pts: p.pts, dts: p.dts, keyframe: p.keyframe }).collect())
            }
            #[cfg(feature = "hevc-encode")]
            Inner::Hevc(enc) => {
                let planar = Planar16::try_from(frame)?;
                let input = planar.hevc()?;
                let pkt = enc.encode(&input)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.keyframe }])
            }
            #[cfg(feature = "av1-encode")]
            Inner::Av1(enc) => {
                let planar = Planar16::try_from(frame)?;
                let input = planar.av1()?;
                let pkt = enc.encode(&input)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.keyframe }])
            }
            #[cfg(feature = "vp9-encode")]
            Inner::Vp9(enc) => {
                let planar = Planar8::try_from(frame)?;
                let (y, u, v) = planar.vp9()?;
                let pkt = enc.encode_frame(y, u, v, false);
                if pkt.is_empty() {
                    return Err(VideoError::Malformed("VP9: input planes shorter than configured frame size".into()));
                }
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.key }])
            }
            #[cfg(feature = "vp8-encode")]
            Inner::Vp8 { width, height, params } => {
                let rgba = Rgba8::try_from(frame)?;
                let pkt = crate::vp8::encode_rgba(&rgba.pixels, *width, *height, params)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "mpeg2-encode")]
            Inner::Mpeg2 { enc, .. } => {
                let planar = Planar8::try_from(frame)?;
                let input = planar.mpeg2()?;
                let packets = enc.encode(&input, pts)?;
                Ok(packets.into_iter().map(|p| EncodedPacket { data: p.data, pts: p.pts, dts: p.dts, keyframe: p.keyframe }).collect())
            }
            #[cfg(feature = "mpeg4-encode")]
            Inner::Mpeg4(enc) => {
                let planar = Planar8::try_from(frame)?;
                let input = planar.mpeg4()?;
                let pkt = enc.encode(&input)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.keyframe }])
            }
            #[cfg(feature = "prores")]
            Inner::ProRes(enc) => {
                let pframe = crate::prores::Frame::try_from(frame)?;
                let data = enc.encode(&pframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "dnx")]
            Inner::Dnx(enc) => {
                let mut dframe = crate::dnx::Frame::try_from(frame)?;
                dframe.cid = enc.config().profile.cid();
                dframe.color_volume = enc.config().color_volume;
                let data = enc.encode(&dframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "apv-encode")]
            Inner::Apv(enc) => {
                let mut aframe = crate::apv::Frame::try_from(frame)?;
                aframe.profile_idc = enc.config().profile.idc();
                let data = enc.encode(&aframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "ffv1")]
            Inner::Ffv1 { enc, .. } => {
                let fframe = crate::ffv1::Frame::try_from(frame)?;
                let data = enc.encode(&fframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg { enc, .. } => {
                let mframe = crate::mjpeg::Frame::try_from(frame)?;
                let data = enc.encode_frame(&mframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
        }
    }

    /// Flush any delayed packets at end of stream.
    pub fn flush(&mut self) -> Result<Vec<EncodedPacket>, VideoError> {
        match &mut self.inner {
            #[cfg(feature = "h264-encode")]
            Inner::H264(enc) => {
                Ok(enc.flush().into_iter().map(|p| EncodedPacket { data: p.data, pts: p.pts, dts: p.dts, keyframe: p.keyframe }).collect())
            }
            #[cfg(feature = "mpeg2-encode")]
            Inner::Mpeg2 { enc, .. } => {
                Ok(enc.flush().into_iter().map(|p| EncodedPacket { data: p.data, pts: p.pts, dts: p.dts, keyframe: p.keyframe }).collect())
            }
            #[cfg(feature = "hevc-encode")]
            Inner::Hevc(_) => Ok(Vec::new()),
            #[cfg(feature = "av1-encode")]
            Inner::Av1(_) => Ok(Vec::new()),
            #[cfg(feature = "vp9-encode")]
            Inner::Vp9(_) => Ok(Vec::new()),
            #[cfg(feature = "vp8-encode")]
            Inner::Vp8 { .. } => Ok(Vec::new()),
            #[cfg(feature = "mpeg4-encode")]
            Inner::Mpeg4(_) => Ok(Vec::new()),
            #[cfg(feature = "prores")]
            Inner::ProRes(_) => Ok(Vec::new()),
            #[cfg(feature = "dnx")]
            Inner::Dnx(_) => Ok(Vec::new()),
            #[cfg(feature = "apv-encode")]
            Inner::Apv(_) => Ok(Vec::new()),
            #[cfg(feature = "ffv1")]
            Inner::Ffv1 { .. } => Ok(Vec::new()),
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg { .. } => Ok(Vec::new()),
        }
    }
}

#[cfg(feature = "mpeg2-encode")]
fn nearest_mpeg2_fps(num: u32, den: u32) -> (u32, u32) {
    const H262_RATES: [(u32, u32); 8] = [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1)];
    let target = f64::from(num.max(1)) / f64::from(den.max(1));
    H262_RATES
        .iter()
        .copied()
        .min_by(|&(n1, d1), &(n2, d2)| {
            let e1 = (f64::from(n1) / f64::from(d1) - target).abs();
            let e2 = (f64::from(n2) / f64::from(d2) - target).abs();
            e1.total_cmp(&e2)
        })
        .unwrap_or((30000, 1001))
}

#[cfg(feature = "mpeg4-encode")]
fn gcd_u32(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd_u32(b, a % b) }
}

#[cfg(feature = "mpeg4-encode")]
fn reduced_mpeg4_fps(fps_num: u32, fps_den: u32) -> (u32, u32) {
    let mut num = fps_num.max(1);
    let mut den = fps_den.max(1);
    let g = gcd_u32(num, den).max(1);
    num /= g;
    den /= g;
    while num > 0xFFFF {
        num = (num / 2).max(1);
        den = (den / 2).max(1);
    }
    (num, den)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{ColorFamily, Plane, Samples, Subsampling};

    fn test_ycbcr_420_frame(width: u32, height: u32) -> VideoFrame {
        let (cw, ch) = Subsampling::S420.chroma_size(width, height);
        let mut y = Vec::with_capacity((width * height) as usize);
        for ry in 0..height {
            for rx in 0..width {
                y.push((16 + ((rx * 3 + ry * 5) % 200)) as u8);
            }
        }
        let cb = vec![120u8; (cw * ch) as usize];
        let cr = vec![140u8; (cw * ch) as usize];
        let planes = vec![
            Plane::packed(width, height, 8, Samples::U8(y)).unwrap(),
            Plane::packed(cw, ch, 8, Samples::U8(cb)).unwrap(),
            Plane::packed(cw, ch, 8, Samples::U8(cr)).unwrap(),
        ];
        VideoFrame::new(width, height, ColorFamily::YCbCr, Subsampling::S420, planes, None).unwrap()
    }

    #[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
    fn test_ycbcr_422_frame(width: u32, height: u32, bit_depth: u8) -> VideoFrame {
        let (cw, ch) = Subsampling::S422.chroma_size(width, height);
        let scale = 1u16 << (bit_depth - 8);
        let y = vec![128 * scale; (width * height) as usize];
        let cb = vec![120 * scale; (cw * ch) as usize];
        let cr = vec![140 * scale; (cw * ch) as usize];
        let planes = vec![
            Plane::packed(width, height, bit_depth, Samples::U16(y)).unwrap(),
            Plane::packed(cw, ch, bit_depth, Samples::U16(cb)).unwrap(),
            Plane::packed(cw, ch, bit_depth, Samples::U16(cr)).unwrap(),
        ];
        VideoFrame::new(width, height, ColorFamily::YCbCr, Subsampling::S422, planes, None).unwrap()
    }

    #[cfg(any(feature = "vp8-encode", feature = "ffv1", feature = "mjpeg", feature = "h264-encode"))]
    fn test_rgb_frame(width: u32, height: u32) -> VideoFrame {
        let n = (width * height) as usize;
        let planes = vec![
            Plane::packed(width, height, 8, Samples::U8(vec![200u8; n])).unwrap(),
            Plane::packed(width, height, 8, Samples::U8(vec![100u8; n])).unwrap(),
            Plane::packed(width, height, 8, Samples::U8(vec![50u8; n])).unwrap(),
        ];
        VideoFrame::new(width, height, ColorFamily::Rgb, Subsampling::S444, planes, None).unwrap()
    }

    fn native_test_frame(codec: CodecId, width: u32, height: u32) -> VideoFrame {
        match codec {
            #[cfg(any(feature = "prores", feature = "apv-encode"))]
            CodecId::ProRes | CodecId::Apv => test_ycbcr_422_frame(width, height, 10),
            #[cfg(feature = "dnx")]
            CodecId::Dnxhd => test_ycbcr_422_frame(width, height, 8),
            #[cfg(any(feature = "vp8-encode", feature = "ffv1", feature = "mjpeg"))]
            CodecId::Vp8 | CodecId::Ffv1 | CodecId::Mjpeg => test_rgb_frame(width, height),
            _ => test_ycbcr_420_frame(width, height),
        }
    }

    #[test]
    fn track_encoder_roundtrips_enabled_codecs() {
        let codecs = [
            #[cfg(all(feature = "h264-encode", feature = "h264-decode"))]
            CodecId::H264,
            #[cfg(all(feature = "hevc-encode", feature = "hevc-decode"))]
            CodecId::Hevc,
            #[cfg(all(feature = "av1-encode", feature = "av1-decode"))]
            CodecId::Av1,
            #[cfg(all(feature = "vp9-encode", feature = "vp9-decode"))]
            CodecId::Vp9,
            #[cfg(all(feature = "vp8-encode", feature = "vp8-decode"))]
            CodecId::Vp8,
            #[cfg(all(feature = "mpeg2-encode", feature = "mpeg2-decode"))]
            CodecId::Mpeg2Video,
            #[cfg(feature = "prores")]
            CodecId::ProRes,
            #[cfg(feature = "dnx")]
            CodecId::Dnxhd,
            #[cfg(all(feature = "apv-encode", feature = "apv-decode"))]
            CodecId::Apv,
            #[cfg(feature = "ffv1")]
            CodecId::Ffv1,
            #[cfg(feature = "mjpeg")]
            CodecId::Mjpeg,
        ];

        for codec in codecs {
            let frame = native_test_frame(codec, 64, 64);
            let cfg = TrackEncoderConfig::new(codec, 64, 64, 25, 1);
            let mut enc = TrackEncoder::new(cfg).unwrap();
            assert_eq!(enc.codec(), codec);
            assert_eq!(enc.width(), 64);
            assert_eq!(enc.height(), 64);

            let mut packets = enc.encode(&frame, 100).unwrap();
            packets.extend(enc.flush().unwrap());
            assert!(!packets.is_empty(), "{codec:?} produced no packets");
            assert!(packets[0].keyframe, "{codec:?} first packet must be a keyframe");

            #[cfg(any(
                feature = "h264-decode",
                feature = "hevc-decode",
                feature = "vp9-decode",
                feature = "av1-decode",
                feature = "mpeg2-decode",
                feature = "prores",
                feature = "dnx",
                feature = "apv-decode",
                feature = "ffv1",
                feature = "vp8-decode",
                feature = "mjpeg"
            ))]
            {
                let params = enc.codec_params();
                let mut dec = crate::TrackDecoder::new(&params).unwrap();
                let mut decoded = Vec::new();
                for pkt in &packets {
                    decoded.extend(dec.decode(&pkt.data, pkt.pts).unwrap());
                }
                decoded.extend(dec.flush().unwrap());
                assert_eq!(decoded.len(), 1, "{codec:?} roundtrip frame count");
                assert_eq!(decoded[0].width(), 64);
                assert_eq!(decoded[0].height(), 64);
            }
        }
    }

    #[cfg(feature = "h264-encode")]
    #[test]
    fn track_encoder_rejects_mismatched_pixel_format_without_implicit_conversion() {
        let rgb = test_rgb_frame(32, 32);
        let mut enc = TrackEncoder::new(TrackEncoderConfig::new(CodecId::H264, 32, 32, 30, 1)).unwrap();
        assert!(matches!(enc.encode(&rgb, 0), Err(VideoError::Frame(_))));
    }

    #[cfg(feature = "mpeg4-encode")]
    #[test]
    fn track_encoder_encodes_mpeg4_video() {
        let frame = test_ycbcr_420_frame(64, 64);
        let mut enc = TrackEncoder::new(TrackEncoderConfig::new(CodecId::Mpeg4Video, 64, 64, 25, 1)).unwrap();
        assert_eq!(enc.codec_params().codec, CodecId::Mpeg4Video);
        assert!(!enc.codec_params().extradata.is_empty());
        let packets = enc.encode(&frame, 0).unwrap();
        assert_eq!(packets.len(), 1);
        assert!(packets[0].keyframe);
    }

    #[cfg(feature = "h264-encode")]
    #[test]
    fn track_encoder_rejects_requests_it_cannot_honour() {
        let mut cfg = TrackEncoderConfig::new(CodecId::H264, 64, 64, 30, 0);
        assert!(matches!(TrackEncoder::new(cfg.clone()), Err(VideoError::Malformed(_))), "zero frame-rate denominator");
        cfg.fps_den = 1;
        cfg.fps_num = 0;
        assert!(matches!(TrackEncoder::new(cfg.clone()), Err(VideoError::Malformed(_))), "zero frame-rate numerator");
        cfg.fps_num = 30;
        cfg.profile = Some("baseline".into());
        cfg.bframes = Some(1);
        assert!(matches!(TrackEncoder::new(cfg.clone()), Err(VideoError::Unsupported(_))), "baseline has no B-frames");
        cfg.bframes = Some(0);
        assert!(TrackEncoder::new(cfg).is_ok(), "baseline without B-frames is fine");
    }

    #[cfg(feature = "mpeg2-encode")]
    #[test]
    fn track_encoder_mpeg2_reports_bad_sizes_without_overflow() {
        for (w, h) in [(63, 64), (500_000_000, 64)] {
            let err = TrackEncoder::new(TrackEncoderConfig::new(CodecId::Mpeg2Video, w, h, 25, 1)).unwrap_err();
            assert!(matches!(err, VideoError::Malformed(_)), "{w}x{h}: {err:?}");
        }
    }

    #[cfg(all(feature = "h264-encode", feature = "h264-decode"))]
    #[test]
    fn annex_b_h264_track_decodes_from_its_own_params() {
        let mut cfg = TrackEncoderConfig::new(CodecId::H264, 64, 64, 25, 1);
        cfg.annex_b = true;
        let mut enc = TrackEncoder::new(cfg).unwrap();
        let mut packets = enc.encode(&test_ycbcr_420_frame(64, 64), 0).unwrap();
        packets.extend(enc.flush().unwrap());
        let mut dec = crate::TrackDecoder::new(&enc.codec_params()).unwrap();
        let mut decoded = dec.decode(&packets[0].data, 0).unwrap();
        decoded.extend(dec.flush().unwrap());
        assert_eq!(decoded.len(), 1);
    }

    #[cfg(feature = "mpeg2-encode")]
    #[test]
    fn mpeg2_rejects_full_range_output() {
        let mut cfg = TrackEncoderConfig::new(CodecId::Mpeg2Video, 64, 64, 25, 1);
        cfg.color.full_range = true;
        assert!(matches!(TrackEncoder::new(cfg), Err(VideoError::Unsupported(_))));
    }

    #[cfg(feature = "vp9-encode")]
    #[test]
    fn vp9_rejects_non_bt709_colour_description() {
        let mut cfg = TrackEncoderConfig::new(CodecId::Vp9, 64, 64, 25, 1);
        cfg.color.matrix = 6;
        assert!(matches!(TrackEncoder::new(cfg), Err(VideoError::Unsupported(_))));
    }

    #[cfg(feature = "prores")]
    #[test]
    fn prores_propagates_colour_matrix_and_rejects_full_range() {
        let mut full = TrackEncoderConfig::new(CodecId::ProRes, 64, 64, 25, 1);
        full.color.full_range = true;
        assert!(matches!(TrackEncoder::new(full), Err(VideoError::Unsupported(_))));

        let mut cfg = TrackEncoderConfig::new(CodecId::ProRes, 64, 64, 25, 1);
        cfg.color = ColorTags { primaries: 6, transfer: 6, matrix: 6, full_range: false };
        let mut enc = TrackEncoder::new(cfg).unwrap();
        let packets = enc.encode(&test_ycbcr_422_frame(64, 64, 10), 0).unwrap();
        let hdr = crate::prores::probe(&packets[0].data).unwrap();
        assert_eq!(hdr.color, crate::prores::ColorInfo { primaries: 6, transfer: 6, matrix: 6 });
    }

    #[cfg(feature = "dnx")]
    #[test]
    fn dnx_propagates_colour_volume_and_rejects_unsupported_matrices() {
        let mut bad = TrackEncoderConfig::new(CodecId::Dnxhd, 64, 64, 25, 1);
        bad.color.matrix = 6;
        assert!(matches!(TrackEncoder::new(bad), Err(VideoError::Unsupported(_))));

        let mut full = TrackEncoderConfig::new(CodecId::Dnxhd, 64, 64, 25, 1);
        full.color.full_range = true;
        assert!(matches!(TrackEncoder::new(full), Err(VideoError::Unsupported(_))));

        let mut cfg = TrackEncoderConfig::new(CodecId::Dnxhd, 64, 64, 25, 1);
        cfg.color.matrix = 9;
        let mut enc = TrackEncoder::new(cfg).unwrap();
        let packets = enc.encode(&test_ycbcr_422_frame(64, 64, 8), 0).unwrap();
        let hdr = crate::dnx::probe(&packets[0].data).unwrap();
        assert_eq!(hdr.color_volume, crate::dnx::ColorVolume::Bt2020Ncl);
    }
}

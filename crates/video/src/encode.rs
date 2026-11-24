//! [`TrackEncoder`]: [`VideoFrame`]s in, one video track's packets out, whatever the codec.
//! The encoding counterpart of [`crate::TrackDecoder`].

use crate::error::VideoError;
use crate::frame::{ColorFamily, ColorTags, Plane, VideoFrame};
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
    Mpeg2 { enc: Box<crate::mpeg2::encode::Encoder>, width: u32, height: u32, fps_den: i64, next_pts: i64 },
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

/// A codec-agnostic video track encoder: takes [`VideoFrame`]s in display order and returns
/// [`EncodedPacket`]s in decode order, with [`TrackEncoder::codec_params`] describing the track for
/// a container muxer or [`crate::TrackDecoder`].
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
        let fps_num = cfg.fps_num.max(1);
        let fps_den = cfg.fps_den.max(1);
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
                    hcfg.bframes = if hcfg.profile == crate::h264::Profile::Baseline { 0 } else { b };
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
                if width > crate::vp8::MAX_SIDE || height > crate::vp8::MAX_SIDE {
                    return Err(VideoError::Malformed(format!("VP8: dimensions {width}x{height} exceed {}", crate::vp8::MAX_SIDE)));
                }
                Inner::Vp8 { width, height, params: crate::vp8::Vp8Params::default() }
            }
            #[cfg(feature = "mpeg2-encode")]
            CodecId::Mpeg2Video => {
                let (m_num, m_den) = nearest_mpeg2_fps(fps_num, fps_den);
                let mut mcfg = crate::mpeg2::encode::EncoderConfig::new(width, height, m_num, m_den);
                mcfg.chroma = crate::mpeg2::ChromaFormat::Yuv420;
                mcfg.aspect =
                    if width * 9 == height * 16 { crate::mpeg2::encode::AspectRatio::Dar16x9 } else { crate::mpeg2::encode::AspectRatio::Square };
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
                let enc = Box::new(crate::mpeg2::encode::Encoder::new(mcfg)?);
                Inner::Mpeg2 { enc, width, height, fps_den: i64::from(m_den), next_pts: 0 }
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
                let profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("422") | Some("standard") => crate::prores::Profile::Standard,
                    Some("proxy") => crate::prores::Profile::Proxy,
                    Some("lt") => crate::prores::Profile::Lt,
                    Some("hq") | Some("422hq") => crate::prores::Profile::Hq,
                    Some(other) => {
                        return Err(VideoError::Unsupported(format!("ProRes: profile `{other}` is not available (4:2:2 profiles only)")));
                    }
                };
                let pcfg = crate::prores::EncoderConfig::new(profile, width, height);
                Inner::ProRes(Box::new(crate::prores::Encoder::with_config(pcfg)))
            }
            #[cfg(feature = "dnx")]
            CodecId::Dnxhd => {
                cfg.reject_unsupported("DNxHR", false, true)?;
                let profile = match cfg.profile.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    None | Some("hq") => crate::dnx::Profile::Hq,
                    Some("lb") => crate::dnx::Profile::Lb,
                    Some("sq") => crate::dnx::Profile::Sq,
                    Some("hqx") => crate::dnx::Profile::Hqx,
                    Some(other) => {
                        return Err(VideoError::Unsupported(format!("DNxHR: profile `{other}` is not available (LB, SQ, HQ, HQX)")));
                    }
                };
                let dcfg = crate::dnx::EncoderConfig::new(profile, width, height);
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
                if let Some(kbps) = cfg.bitrate_kbps {
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

    /// Track description for container muxers and [`crate::TrackDecoder::new`].
    pub fn codec_params(&self) -> CodecParams {
        match &self.inner {
            #[cfg(feature = "h264-encode")]
            Inner::H264(enc) => enc.codec_params(),
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
    /// Frames with colour family or subsampling different from the underlying encoder's native
    /// format are converted automatically. Encoders with B-frame reordering may return zero packets
    /// until future frames or [`TrackEncoder::flush`] arrive.
    pub fn encode(&mut self, frame: &VideoFrame, pts: i64) -> Result<Vec<EncodedPacket>, VideoError> {
        if frame.width() != self.width || frame.height() != self.height {
            return Err(VideoError::Malformed(format!("frame is {}x{}, encoder is {}x{}", frame.width(), frame.height(), self.width, self.height)));
        }
        match &mut self.inner {
            #[cfg(feature = "h264-encode")]
            Inner::H264(enc) => {
                let packets = if enc.config().bit_depth > 8 {
                    let yuv = frame_to_yuv420_u16(frame, enc.config().bit_depth)?;
                    let input = crate::h264::YuvFrame16 { y: &yuv.y, u: &yuv.u, v: &yuv.v, y_stride: yuv.y_stride, uv_stride: yuv.uv_stride };
                    enc.encode16(&input, pts)?
                } else {
                    let yuv = frame_to_yuv420_u8(frame)?;
                    let (ys, cs) = packed_420_strides(frame);
                    let input = crate::h264::YuvFrame { y: &yuv.y, u: &yuv.u, v: &yuv.v, y_stride: ys, uv_stride: cs };
                    enc.encode(&input, pts)?
                };
                Ok(packets.into_iter().map(|p| EncodedPacket { data: p.data, pts: p.pts, dts: p.dts, keyframe: p.keyframe }).collect())
            }
            #[cfg(feature = "hevc-encode")]
            Inner::Hevc(enc) => {
                let depth = enc.config().profile.bit_depth();
                let yuv = frame_to_yuv420_u16(frame, depth)?;
                let input = crate::hevc::Frame { y: &yuv.y, u: &yuv.u, v: &yuv.v, y_stride: yuv.y_stride, uv_stride: yuv.uv_stride };
                let pkt = enc.encode(&input)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.keyframe }])
            }
            #[cfg(feature = "av1-encode")]
            Inner::Av1(enc) => {
                let depth = enc.config().bit_depth;
                let yuv = frame_to_yuv420_u16(frame, depth)?;
                let input = crate::av1::Frame { y: &yuv.y, u: &yuv.u, v: &yuv.v, y_stride: yuv.y_stride, uv_stride: yuv.uv_stride };
                let pkt = enc.encode(&input)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.keyframe }])
            }
            #[cfg(feature = "vp9-encode")]
            Inner::Vp9(enc) => {
                let yuv = frame_to_yuv420_u8(frame)?;
                let pkt = enc.encode_frame(&yuv.y, &yuv.u, &yuv.v, false);
                if pkt.is_empty() {
                    return Err(VideoError::Malformed("VP9: input planes shorter than configured frame size".into()));
                }
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.key }])
            }
            #[cfg(feature = "vp8-encode")]
            Inner::Vp8 { width, height, params } => {
                let rgba = frame_to_rgba8(frame)?;
                let pkt = crate::vp8::encode_rgba(&rgba, *width, *height, params)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "mpeg2-encode")]
            Inner::Mpeg2 { enc, fps_den, next_pts, .. } => {
                let yuv = frame_to_yuv420_u8(frame)?;
                let (ys, cs) = packed_420_strides(frame);
                let input = crate::mpeg2::encode::YuvFrame { y: &yuv.y, cb: &yuv.u, cr: &yuv.v, y_stride: ys, c_stride: cs };
                let use_pts = if *next_pts == 0 { pts } else { (*next_pts).max(pts) };
                *next_pts = use_pts.saturating_add((*fps_den).max(1));
                let packets = enc.encode(&input, use_pts)?;
                Ok(packets.into_iter().map(|p| EncodedPacket { data: p.data, pts: p.pts, dts: p.dts, keyframe: p.keyframe }).collect())
            }
            #[cfg(feature = "mpeg4-encode")]
            Inner::Mpeg4(enc) => {
                let yuv = frame_to_yuv420_u8(frame)?;
                let (ys, cs) = packed_420_strides(frame);
                let input = crate::mpeg4::YuvFrame { y: &yuv.y, u: &yuv.u, v: &yuv.v, y_stride: ys, uv_stride: cs };
                let pkt = enc.encode(&input)?;
                Ok(vec![EncodedPacket { data: pkt.data, pts, dts: pts, keyframe: pkt.keyframe }])
            }
            #[cfg(feature = "prores")]
            Inner::ProRes(enc) => {
                let yuv = frame_to_yuv422_u16(frame, 10)?;
                let pframe = crate::prores::Frame {
                    width: self.width,
                    height: self.height,
                    chroma: crate::prores::ChromaFormat::Yuv422,
                    bit_depth: 10,
                    y: yuv.y,
                    cb: yuv.u,
                    cr: yuv.v,
                    alpha: None,
                    interlace: crate::prores::Interlace::Progressive,
                    color: crate::prores::ColorInfo::default(),
                    aspect_ratio: 0,
                    frame_rate_code: 0,
                };
                let data = enc.encode(&pframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "dnx")]
            Inner::Dnx(enc) => {
                let depth = enc.config().bit_depth;
                let profile = enc.config().profile;
                let yuv = frame_to_yuv422_u16(frame, depth)?;
                let dframe = crate::dnx::Frame {
                    width: self.width,
                    height: self.height,
                    chroma: crate::dnx::ChromaFormat::Yuv422,
                    rgb: false,
                    bit_depth: depth,
                    y: yuv.y,
                    cb: yuv.u,
                    cr: yuv.v,
                    alpha: None,
                    interlace: crate::dnx::Interlace::Progressive,
                    color_volume: crate::dnx::ColorVolume::Bt709,
                    cid: profile.cid(),
                    par: (0, 0),
                };
                let data = enc.encode(&dframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "apv-encode")]
            Inner::Apv(enc) => {
                let depth = enc.config().bit_depth;
                let profile = enc.config().profile;
                let color = enc.config().color;
                let yuv = frame_to_yuv422_u16(frame, depth)?;
                let mut aframe = crate::apv::Frame::new(self.width, self.height, crate::apv::ChromaFormat::Yuv422, depth, false);
                aframe.profile_idc = profile.idc();
                aframe.y = yuv.y;
                aframe.cb = yuv.u;
                aframe.cr = yuv.v;
                aframe.color = color;
                let data = enc.encode(&aframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "ffv1")]
            Inner::Ffv1 { enc, width, height } => {
                let [r, g, b] = frame_to_rgb_planes_u16(frame)?;
                let fframe = crate::ffv1::Frame {
                    width: *width,
                    height: *height,
                    colorspace: crate::ffv1::Colorspace::Rgb,
                    bit_depth: 8,
                    chroma_shift: (0, 0),
                    planes: vec![r, g, b],
                };
                let data = enc.encode(&fframe)?;
                Ok(vec![EncodedPacket { data, pts, dts: pts, keyframe: true }])
            }
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg { enc, width, height } => {
                let rgb = frame_to_rgb8(frame)?;
                let data = enc.encode(*width, *height, 3, &rgb)?;
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

#[cfg(any(feature = "h264-encode", feature = "vp9-encode", feature = "mpeg2-encode", feature = "mpeg4-encode"))]
struct PlanarYuv8 {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

/// Row strides of the packed 4:2:0 planes of `frame`: luma, then chroma.
#[cfg(any(feature = "h264-encode", feature = "mpeg2-encode", feature = "mpeg4-encode"))]
fn packed_420_strides(frame: &VideoFrame) -> (usize, usize) {
    let w = frame.width() as usize;
    (w, w.div_ceil(2))
}

#[cfg(any(feature = "h264-encode", feature = "hevc-encode", feature = "av1-encode"))]
struct PlanarYuv16 {
    y: Vec<u16>,
    u: Vec<u16>,
    v: Vec<u16>,
    y_stride: usize,
    uv_stride: usize,
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
struct PlanarYuv422_16 {
    y: Vec<u16>,
    u: Vec<u16>,
    v: Vec<u16>,
}

/// Read sample `(x, y)` from `plane` scaled to `target_depth` bits (`1..=16`).
fn sample_at_depth(plane: &Plane, x: u32, y: u32, target_depth: u8) -> u16 {
    let sx = x.min(plane.width().saturating_sub(1)) as usize;
    let sy = y.min(plane.height().saturating_sub(1));
    let src_depth = plane.bit_depth().clamp(1, 16);
    let raw = if let Some(row) = plane.row_u8(sy) {
        u32::from(row.get(sx).copied().unwrap_or(0))
    } else if let Some(row) = plane.row_u16(sy) {
        u32::from(row.get(sx).copied().unwrap_or(0))
    } else {
        0
    };
    let max_in = (1u32 << src_depth) - 1;
    let clamped = raw.min(max_in);
    if target_depth == src_depth {
        clamped as u16
    } else if target_depth > src_depth {
        let shift = u32::from(target_depth - src_depth);
        let fill = u32::from(src_depth).saturating_sub(shift);
        ((clamped << shift) | (clamped >> fill)) as u16
    } else {
        let shift = u32::from(src_depth - target_depth);
        let round = 1u32 << (shift - 1);
        let max_out = (1u32 << target_depth) - 1;
        (((clamped + round) >> shift).min(max_out)) as u16
    }
}

fn luma_matrix(color: ColorTags) -> (f64, f64, f64) {
    let (kr, kb) = match color.matrix {
        5 | 6 => (0.299, 0.114),
        9 | 10 => (0.2627, 0.0593),
        _ => (0.2126, 0.0722),
    };
    (kr, 1.0 - kr - kb, kb)
}

#[cfg(any(feature = "h264-encode", feature = "vp9-encode", feature = "mpeg2-encode", feature = "mpeg4-encode"))]
fn frame_to_yuv420_u8(frame: &VideoFrame) -> Result<PlanarYuv8, VideoError> {
    use crate::frame::Subsampling;
    let w = frame.width();
    let h = frame.height();
    let (cw, ch) = Subsampling::S420.chroma_size(w, h);
    let w_usize = w as usize;
    let h_usize = h as usize;
    let cw_usize = cw as usize;
    let ch_usize = ch as usize;

    match frame.family() {
        ColorFamily::YCbCr => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing Y plane".into()))?;
            let pu = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing Cb plane".into()))?;
            let pv = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing Cr plane".into()))?;
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    y.push(sample_at_depth(py, rx, ry, 8) as u8);
                }
            }
            let sub = frame.subsampling();
            let mut u = Vec::with_capacity(cw_usize * ch_usize);
            let mut v = Vec::with_capacity(cw_usize * ch_usize);
            for cy in 0..ch {
                let ly = (cy << 1) >> sub.y;
                for cx in 0..cw {
                    let lx = (cx << 1) >> sub.x;
                    u.push(sample_at_depth(pu, lx, ly, 8) as u8);
                    v.push(sample_at_depth(pv, lx, ly, 8) as u8);
                }
            }
            Ok(PlanarYuv8 { y, u, v })
        }
        ColorFamily::Gray => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing gray plane".into()))?;
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    y.push(sample_at_depth(py, rx, ry, 8) as u8);
                }
            }
            let u = vec![128u8; cw_usize * ch_usize];
            let v = vec![128u8; cw_usize * ch_usize];
            Ok(PlanarYuv8 { y, u, v })
        }
        ColorFamily::Rgb => {
            let pr = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing R plane".into()))?;
            let pg = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing G plane".into()))?;
            let pb = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing B plane".into()))?;
            let (kr, kg, kb) = luma_matrix(frame.color);
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    let r = f64::from(sample_at_depth(pr, rx, ry, 8));
                    let g = f64::from(sample_at_depth(pg, rx, ry, 8));
                    let b = f64::from(sample_at_depth(pb, rx, ry, 8));
                    let luma = kr * r + kg * g + kb * b;
                    let y_val = (16.0 + 219.0 * luma / 255.0).round().clamp(16.0, 235.0) as u8;
                    y.push(y_val);
                }
            }
            let mut u = Vec::with_capacity(cw_usize * ch_usize);
            let mut v = Vec::with_capacity(cw_usize * ch_usize);
            for cy in 0..ch {
                let y0 = cy * 2;
                let y1 = (y0 + 1).min(h.saturating_sub(1));
                for cx in 0..cw {
                    let x0 = cx * 2;
                    let x1 = (x0 + 1).min(w.saturating_sub(1));
                    let mut r_sum = 0.0;
                    let mut g_sum = 0.0;
                    let mut b_sum = 0.0;
                    for (px, py) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                        r_sum += f64::from(sample_at_depth(pr, px, py, 8));
                        g_sum += f64::from(sample_at_depth(pg, px, py, 8));
                        b_sum += f64::from(sample_at_depth(pb, px, py, 8));
                    }
                    let r = r_sum * 0.25;
                    let g = g_sum * 0.25;
                    let b = b_sum * 0.25;
                    let luma = kr * r + kg * g + kb * b;
                    let u_val = (128.0 + 224.0 * (b - luma) / (2.0 * (1.0 - kb) * 255.0)).round().clamp(16.0, 240.0) as u8;
                    let v_val = (128.0 + 224.0 * (r - luma) / (2.0 * (1.0 - kr) * 255.0)).round().clamp(16.0, 240.0) as u8;
                    u.push(u_val);
                    v.push(v_val);
                }
            }
            Ok(PlanarYuv8 { y, u, v })
        }
    }
}

#[cfg(any(feature = "h264-encode", feature = "hevc-encode", feature = "av1-encode"))]
fn frame_to_yuv420_u16(frame: &VideoFrame, target_depth: u8) -> Result<PlanarYuv16, VideoError> {
    use crate::frame::Subsampling;
    let w = frame.width();
    let h = frame.height();
    let (cw, ch) = Subsampling::S420.chroma_size(w, h);
    let w_usize = w as usize;
    let h_usize = h as usize;
    let cw_usize = cw as usize;
    let ch_usize = ch as usize;
    let shift = u32::from(target_depth.saturating_sub(8));
    let neutral = 128u16 << shift;

    match frame.family() {
        ColorFamily::YCbCr => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing Y plane".into()))?;
            let pu = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing Cb plane".into()))?;
            let pv = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing Cr plane".into()))?;
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    y.push(sample_at_depth(py, rx, ry, target_depth));
                }
            }
            let sub = frame.subsampling();
            let mut u = Vec::with_capacity(cw_usize * ch_usize);
            let mut v = Vec::with_capacity(cw_usize * ch_usize);
            for cy in 0..ch {
                let ly = (cy << 1) >> sub.y;
                for cx in 0..cw {
                    let lx = (cx << 1) >> sub.x;
                    u.push(sample_at_depth(pu, lx, ly, target_depth));
                    v.push(sample_at_depth(pv, lx, ly, target_depth));
                }
            }
            Ok(PlanarYuv16 { y, u, v, y_stride: w_usize, uv_stride: cw_usize })
        }
        ColorFamily::Gray => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing gray plane".into()))?;
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    y.push(sample_at_depth(py, rx, ry, target_depth));
                }
            }
            let u = vec![neutral; cw_usize * ch_usize];
            let v = vec![neutral; cw_usize * ch_usize];
            Ok(PlanarYuv16 { y, u, v, y_stride: w_usize, uv_stride: cw_usize })
        }
        ColorFamily::Rgb => {
            let pr = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing R plane".into()))?;
            let pg = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing G plane".into()))?;
            let pb = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing B plane".into()))?;
            let (kr, kg, kb) = luma_matrix(frame.color);
            let max_in = f64::from((1u32 << target_depth) - 1);
            let scale = f64::from(1u32 << shift);
            let y_min = 16.0 * scale;
            let y_max = 235.0 * scale;
            let y_span = 219.0 * scale;
            let c_mid = 128.0 * scale;
            let c_min = 16.0 * scale;
            let c_max = 240.0 * scale;
            let c_span = 224.0 * scale;

            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    let r = f64::from(sample_at_depth(pr, rx, ry, target_depth)) / max_in;
                    let g = f64::from(sample_at_depth(pg, rx, ry, target_depth)) / max_in;
                    let b = f64::from(sample_at_depth(pb, rx, ry, target_depth)) / max_in;
                    let luma = kr * r + kg * g + kb * b;
                    y.push((y_min + y_span * luma).round().clamp(y_min, y_max) as u16);
                }
            }
            let mut u = Vec::with_capacity(cw_usize * ch_usize);
            let mut v = Vec::with_capacity(cw_usize * ch_usize);
            for cy in 0..ch {
                let y0 = cy * 2;
                let y1 = (y0 + 1).min(h.saturating_sub(1));
                for cx in 0..cw {
                    let x0 = cx * 2;
                    let x1 = (x0 + 1).min(w.saturating_sub(1));
                    let mut r_sum = 0.0;
                    let mut g_sum = 0.0;
                    let mut b_sum = 0.0;
                    for (px, py) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                        r_sum += f64::from(sample_at_depth(pr, px, py, target_depth)) / max_in;
                        g_sum += f64::from(sample_at_depth(pg, px, py, target_depth)) / max_in;
                        b_sum += f64::from(sample_at_depth(pb, px, py, target_depth)) / max_in;
                    }
                    let r = r_sum * 0.25;
                    let g = g_sum * 0.25;
                    let b = b_sum * 0.25;
                    let luma = kr * r + kg * g + kb * b;
                    let u_val = (c_mid + c_span * (b - luma) / (2.0 * (1.0 - kb))).round().clamp(c_min, c_max) as u16;
                    let v_val = (c_mid + c_span * (r - luma) / (2.0 * (1.0 - kr))).round().clamp(c_min, c_max) as u16;
                    u.push(u_val);
                    v.push(v_val);
                }
            }
            Ok(PlanarYuv16 { y, u, v, y_stride: w_usize, uv_stride: cw_usize })
        }
    }
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
fn frame_to_yuv422_u16(frame: &VideoFrame, target_depth: u8) -> Result<PlanarYuv422_16, VideoError> {
    use crate::frame::Subsampling;
    let w = frame.width();
    let h = frame.height();
    let (cw, ch) = Subsampling::S422.chroma_size(w, h);
    let w_usize = w as usize;
    let h_usize = h as usize;
    let cw_usize = cw as usize;
    let ch_usize = ch as usize;
    let shift = u32::from(target_depth.saturating_sub(8));
    let neutral = 128u16 << shift;

    match frame.family() {
        ColorFamily::YCbCr => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing Y plane".into()))?;
            let pu = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing Cb plane".into()))?;
            let pv = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing Cr plane".into()))?;
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    y.push(sample_at_depth(py, rx, ry, target_depth));
                }
            }
            let sub = frame.subsampling();
            let mut u = Vec::with_capacity(cw_usize * ch_usize);
            let mut v = Vec::with_capacity(cw_usize * ch_usize);
            for cy in 0..ch {
                let ly = cy >> sub.y;
                for cx in 0..cw {
                    let lx = (cx << 1) >> sub.x;
                    u.push(sample_at_depth(pu, lx, ly, target_depth));
                    v.push(sample_at_depth(pv, lx, ly, target_depth));
                }
            }
            Ok(PlanarYuv422_16 { y, u, v })
        }
        ColorFamily::Gray => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing gray plane".into()))?;
            let mut y = Vec::with_capacity(w_usize * h_usize);
            for ry in 0..h {
                for rx in 0..w {
                    y.push(sample_at_depth(py, rx, ry, target_depth));
                }
            }
            let u = vec![neutral; cw_usize * ch_usize];
            let v = vec![neutral; cw_usize * ch_usize];
            Ok(PlanarYuv422_16 { y, u, v })
        }
        ColorFamily::Rgb => {
            let pr = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing R plane".into()))?;
            let pg = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing G plane".into()))?;
            let pb = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing B plane".into()))?;
            let (kr, kg, kb) = luma_matrix(frame.color);
            let max_in = f64::from((1u32 << target_depth) - 1);
            let scale = f64::from(1u32 << shift);
            let y_min = 16.0 * scale;
            let y_max = 235.0 * scale;
            let y_span = 219.0 * scale;
            let c_mid = 128.0 * scale;
            let c_min = 16.0 * scale;
            let c_max = 240.0 * scale;
            let c_span = 224.0 * scale;

            let mut y = Vec::with_capacity(w_usize * h_usize);
            let mut u = Vec::with_capacity(cw_usize * ch_usize);
            let mut v = Vec::with_capacity(cw_usize * ch_usize);
            for ry in 0..h {
                for rx in 0..w {
                    let r = f64::from(sample_at_depth(pr, rx, ry, target_depth)) / max_in;
                    let g = f64::from(sample_at_depth(pg, rx, ry, target_depth)) / max_in;
                    let b = f64::from(sample_at_depth(pb, rx, ry, target_depth)) / max_in;
                    let luma = kr * r + kg * g + kb * b;
                    y.push((y_min + y_span * luma).round().clamp(y_min, y_max) as u16);
                }
                for cx in 0..cw {
                    let x0 = cx * 2;
                    let x1 = (x0 + 1).min(w.saturating_sub(1));
                    let r =
                        0.5 * (f64::from(sample_at_depth(pr, x0, ry, target_depth)) + f64::from(sample_at_depth(pr, x1, ry, target_depth))) / max_in;
                    let g =
                        0.5 * (f64::from(sample_at_depth(pg, x0, ry, target_depth)) + f64::from(sample_at_depth(pg, x1, ry, target_depth))) / max_in;
                    let b =
                        0.5 * (f64::from(sample_at_depth(pb, x0, ry, target_depth)) + f64::from(sample_at_depth(pb, x1, ry, target_depth))) / max_in;
                    let luma = kr * r + kg * g + kb * b;
                    let u_val = (c_mid + c_span * (b - luma) / (2.0 * (1.0 - kb))).round().clamp(c_min, c_max) as u16;
                    let v_val = (c_mid + c_span * (r - luma) / (2.0 * (1.0 - kr))).round().clamp(c_min, c_max) as u16;
                    u.push(u_val);
                    v.push(v_val);
                }
            }
            Ok(PlanarYuv422_16 { y, u, v })
        }
    }
}

#[cfg(any(feature = "ffv1", feature = "mjpeg", feature = "vp8-encode"))]
fn pixel_rgb8(frame: &VideoFrame, x: u32, y: u32) -> Result<[u8; 3], VideoError> {
    match frame.family() {
        ColorFamily::Rgb => {
            let pr = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing R plane".into()))?;
            let pg = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing G plane".into()))?;
            let pb = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing B plane".into()))?;
            Ok([sample_at_depth(pr, x, y, 8) as u8, sample_at_depth(pg, x, y, 8) as u8, sample_at_depth(pb, x, y, 8) as u8])
        }
        ColorFamily::Gray => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing gray plane".into()))?;
            let v = sample_at_depth(py, x, y, 8) as u8;
            Ok([v, v, v])
        }
        ColorFamily::YCbCr => {
            let py = frame.plane(0).ok_or_else(|| VideoError::Malformed("missing Y plane".into()))?;
            let pu = frame.plane(1).ok_or_else(|| VideoError::Malformed("missing Cb plane".into()))?;
            let pv = frame.plane(2).ok_or_else(|| VideoError::Malformed("missing Cr plane".into()))?;
            let sub = frame.subsampling();
            let y_val = f64::from(sample_at_depth(py, x, y, 8));
            let u_val = f64::from(sample_at_depth(pu, x >> sub.x, y >> sub.y, 8)) - 128.0;
            let v_val = f64::from(sample_at_depth(pv, x >> sub.x, y >> sub.y, 8)) - 128.0;
            let (kr, kg, kb) = luma_matrix(frame.color);
            let y_norm = if frame.color.full_range { y_val } else { (y_val - 16.0) * (255.0 / 219.0) };
            let c_scale = if frame.color.full_range { 1.0 } else { 255.0 / 224.0 };
            let cb = u_val * c_scale;
            let cr = v_val * c_scale;
            let r = y_norm + 2.0 * (1.0 - kr) * cr;
            let b = y_norm + 2.0 * (1.0 - kb) * cb;
            let g = y_norm - (2.0 * kr * (1.0 - kr) / kg) * cr - (2.0 * kb * (1.0 - kb) / kg) * cb;
            Ok([r.round().clamp(0.0, 255.0) as u8, g.round().clamp(0.0, 255.0) as u8, b.round().clamp(0.0, 255.0) as u8])
        }
    }
}

#[cfg(feature = "ffv1")]
fn frame_to_rgb_planes_u16(frame: &VideoFrame) -> Result<[Vec<u16>; 3], VideoError> {
    let w = frame.width();
    let h = frame.height();
    let n = (w as usize) * (h as usize);
    let mut r = Vec::with_capacity(n);
    let mut g = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    for ry in 0..h {
        for rx in 0..w {
            let [rv, gv, bv] = pixel_rgb8(frame, rx, ry)?;
            r.push(u16::from(rv));
            g.push(u16::from(gv));
            b.push(u16::from(bv));
        }
    }
    Ok([r, g, b])
}

#[cfg(feature = "mjpeg")]
fn frame_to_rgb8(frame: &VideoFrame) -> Result<Vec<u8>, VideoError> {
    let w = frame.width();
    let h = frame.height();
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for ry in 0..h {
        for rx in 0..w {
            out.extend_from_slice(&pixel_rgb8(frame, rx, ry)?);
        }
    }
    Ok(out)
}

#[cfg(feature = "vp8-encode")]
fn frame_to_rgba8(frame: &VideoFrame) -> Result<Vec<u8>, VideoError> {
    let w = frame.width();
    let h = frame.height();
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for ry in 0..h {
        for rx in 0..w {
            let [r, g, b] = pixel_rgb8(frame, rx, ry)?;
            let a = frame.alpha().map_or(255, |ap| sample_at_depth(ap, rx, ry, 8) as u8);
            out.extend_from_slice(&[r, g, b, a]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Samples, Subsampling};

    fn test_ycbcr_frame(width: u32, height: u32) -> VideoFrame {
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

    #[test]
    fn track_encoder_roundtrips_enabled_codecs() {
        let frame = test_ycbcr_frame(64, 64);
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
    fn track_encoder_accepts_rgb_and_gray_frames() {
        let width = 32;
        let height = 32;
        let rgb_planes = vec![
            Plane::packed(width, height, 8, Samples::U8(vec![200u8; (width * height) as usize])).unwrap(),
            Plane::packed(width, height, 8, Samples::U8(vec![100u8; (width * height) as usize])).unwrap(),
            Plane::packed(width, height, 8, Samples::U8(vec![50u8; (width * height) as usize])).unwrap(),
        ];
        let rgb_frame = VideoFrame::new(width, height, ColorFamily::Rgb, Subsampling::S444, rgb_planes, None).unwrap();

        let gray_planes = vec![Plane::packed(width, height, 8, Samples::U8(vec![128u8; (width * height) as usize])).unwrap()];
        let gray_frame = VideoFrame::new(width, height, ColorFamily::Gray, Subsampling::S444, gray_planes, None).unwrap();

        let mut enc = TrackEncoder::new(TrackEncoderConfig::new(CodecId::H264, width, height, 30, 1)).unwrap();
        let mut packets = enc.encode(&rgb_frame, 0).unwrap();
        packets.extend(enc.encode(&gray_frame, 1).unwrap());
        packets.extend(enc.flush().unwrap());
        assert_eq!(packets.len(), 2);
    }

    #[cfg(feature = "mpeg4-encode")]
    #[test]
    fn track_encoder_encodes_mpeg4_video() {
        let frame = test_ycbcr_frame(64, 64);
        let mut enc = TrackEncoder::new(TrackEncoderConfig::new(CodecId::Mpeg4Video, 64, 64, 25, 1)).unwrap();
        assert_eq!(enc.codec_params().codec, CodecId::Mpeg4Video);
        assert!(!enc.codec_params().extradata.is_empty());
        let packets = enc.encode(&frame, 0).unwrap();
        assert_eq!(packets.len(), 1);
        assert!(packets[0].keyframe);
    }
}

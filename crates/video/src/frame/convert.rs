//! `TryFrom` from each decoder's own output into [`VideoFrame`]. Each conversion is built with
//! the feature of its decoder.

// Each conversion uses some of these; a build with no decoder feature uses none.
#[allow(unused_imports)]
use super::{ColorFamily, ColorTags, FieldOrder, FrameError, Plane, Samples, Subsampling, VideoFrame};

/// A bit depth a decoder reports as `u32`.
#[cfg(any(feature = "hevc-decode", feature = "vp9-decode"))]
fn depth(bits: u32) -> Result<u8, FrameError> {
    u8::try_from(bits).map_err(|_| FrameError::Plane("bit depth does not fit the sample storage"))
}

/// A sample aspect ratio with 0 meaning not signalled.
#[cfg(any(feature = "h264-decode", feature = "hevc-decode", feature = "mpeg2-decode", feature = "dnx"))]
fn sar(num: u32, den: u32) -> Option<(u32, u32)> {
    (num != 0 && den != 0).then_some((num, den))
}

/// The sample aspect ratio of a ProRes `aspect_ratio_information` code: 1 is square pixels, 2 and
/// 3 give the picture's shape (4:3 and 16:9), 0 and the reserved codes say nothing.
#[cfg(feature = "prores")]
pub(super) fn prores_sar(code: u8, width: u32, height: u32) -> Option<(u32, u32)> {
    let (dar_w, dar_h): (u64, u64) = match code {
        1 => return Some((1, 1)),
        2 => (4, 3),
        3 => (16, 9),
        _ => return None,
    };
    // sar = dar * height / width, in lowest terms. Neither product can overflow `u64`.
    let (mut num, mut den) = (dar_w * u64::from(height), dar_h * u64::from(width));
    let (mut a, mut b) = (num, den);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    if a == 0 {
        return None;
    }
    num /= a;
    den /= a;
    match (u32::try_from(num), u32::try_from(den)) {
        (Ok(n), Ok(d)) if n != 0 && d != 0 => Some((n, d)),
        _ => None,
    }
}

/// ProRes stores `0` for "unspecified" where H.273 uses `2`.
#[cfg(feature = "prores")]
fn h273(code: u8) -> u8 {
    if code == 0 { 2 } else { code }
}

#[cfg(feature = "h264-decode")]
impl TryFrom<crate::h264::decode::Picture> for VideoFrame {
    type Error = FrameError;

    /// An 8-bit 4:2:0 frame.
    fn try_from(p: crate::h264::decode::Picture) -> Result<Self, FrameError> {
        let planes = vec![
            Plane::new(p.width, p.height, p.y_stride, 8, Samples::U8(p.y))?,
            Plane::new(p.chroma_width, p.chroma_height, p.uv_stride, 8, Samples::U8(p.u))?,
            Plane::new(p.chroma_width, p.chroma_height, p.uv_stride, 8, Samples::U8(p.v))?,
        ];
        let mut f = VideoFrame::new(p.width, p.height, ColorFamily::YCbCr, Subsampling::S420, planes, None)?;
        f.color = ColorTags { primaries: p.color.primaries, transfer: p.color.transfer, matrix: p.color.matrix, full_range: p.color.full_range };
        f.pts = Some(p.pts);
        f.sar = sar(p.sar.0.into(), p.sar.1.into());
        f.draft = p.draft;
        Ok(f)
    }
}

#[cfg(feature = "hap")]
impl TryFrom<crate::hap::Image> for VideoFrame {
    type Error = FrameError;

    /// An 8-bit RGB frame with an alpha plane, full range, identity matrix (Hap Q's YCoCg is
    /// already converted to RGB by the decoder). The picture does not say whether the variant has
    /// alpha: Hap and Hap Q give an opaque (255) alpha plane. The decoder returns interleaved RGBA,
    /// so this conversion copies the samples into planes.
    fn try_from(p: crate::hap::Image) -> Result<Self, FrameError> {
        let (w, h) = (p.width, p.height);
        let pixels = usize::try_from(w)
            .ok()
            .zip(usize::try_from(h).ok())
            .and_then(|(w, h)| w.checked_mul(h))
            .ok_or(FrameError::Plane("plane size overflows"))?;
        if pixels.checked_mul(4) != Some(p.rgba.len()) {
            return Err(FrameError::Shape("sample count does not fit the frame"));
        }
        let plane = |c: usize| Plane::packed(w, h, 8, Samples::U8(p.rgba.iter().skip(c).step_by(4).copied().collect()));
        let planes = vec![plane(0)?, plane(1)?, plane(2)?];
        let mut f = VideoFrame::new(w, h, ColorFamily::Rgb, Subsampling::S444, planes, Some(plane(3)?))?;
        f.color = ColorTags { matrix: 0, full_range: true, ..ColorTags::UNSPECIFIED };
        Ok(f)
    }
}

#[cfg(feature = "hevc-decode")]
impl TryFrom<crate::hevc::decode::Picture> for VideoFrame {
    type Error = FrameError;

    /// A 4:2:0 or 4:2:2 frame, 8-bit storage at bit depth 8 and 16-bit above. The chroma planes
    /// carry the stream's chroma bit depth, which may differ from the luma depth.
    fn try_from(p: crate::hevc::decode::Picture) -> Result<Self, FrameError> {
        use crate::hevc::decode::{ChromaFormat, Plane as P};
        let bits = depth(p.bit_depth)?;
        let chroma_bits = depth(p.bit_depth_chroma)?;
        let samples = |pl: P| match pl {
            P::U8(v) => Samples::U8(v),
            P::U16(v) => Samples::U16(v),
        };
        let planes = vec![
            Plane::new(p.width, p.height, p.y_stride, bits, samples(p.y))?,
            Plane::new(p.chroma_width, p.chroma_height, p.uv_stride, chroma_bits, samples(p.u))?,
            Plane::new(p.chroma_width, p.chroma_height, p.uv_stride, chroma_bits, samples(p.v))?,
        ];
        let sub = match p.chroma {
            ChromaFormat::C420 => Subsampling::S420,
            ChromaFormat::C422 => Subsampling::S422,
        };
        let mut f = VideoFrame::new(p.width, p.height, ColorFamily::YCbCr, sub, planes, None)?;
        f.color = ColorTags { primaries: p.color.primaries, transfer: p.color.transfer, matrix: p.color.matrix, full_range: p.color.full_range };
        f.pts = Some(p.pts);
        f.sar = sar(p.sar.0.into(), p.sar.1.into());
        f.draft = p.draft;
        Ok(f)
    }
}

/// VP9 `color_space` as an H.273 matrix: 1 BT.601 (625-line), 2 BT.709, 3 SMPTE 170M,
/// 4 SMPTE 240M, 5 BT.2020 non-constant luminance, 7 RGB.
#[cfg(feature = "vp9-decode")]
pub(super) fn vp9_matrix(color_space: u8) -> u8 {
    match color_space {
        1 => 5,
        2 => 1,
        3 => 6,
        4 => 7,
        5 => 9,
        7 => 0,
        _ => 2,
    }
}

#[cfg(feature = "vp9-decode")]
impl TryFrom<crate::vp9::decode::Picture> for VideoFrame {
    type Error = FrameError;

    /// A YCbCr frame, or an RGB one for `color_space` 7, whose coded G, B, R planes are reordered
    /// to R, G, B. VP9 signals only the matrix, so primaries and transfer are unspecified.
    fn try_from(p: crate::vp9::decode::Picture) -> Result<Self, FrameError> {
        use crate::vp9::decode::Plane as P;
        let bits = depth(p.bit_depth)?;
        let samples = |pl: P| match pl {
            P::U8(v) => Samples::U8(v),
            P::U16(v) => Samples::U16(v),
        };
        let y = Plane::new(p.width, p.height, p.y_stride, bits, samples(p.y))?;
        let u = Plane::new(p.chroma_width, p.chroma_height, p.uv_stride, bits, samples(p.u))?;
        let v = Plane::new(p.chroma_width, p.chroma_height, p.uv_stride, bits, samples(p.v))?;
        let sub = Subsampling { x: u8::from(p.subsampling_x), y: u8::from(p.subsampling_y) };
        let rgb = p.color.color_space == 7;
        let (family, planes) = if rgb { (ColorFamily::Rgb, vec![v, y, u]) } else { (ColorFamily::YCbCr, vec![y, u, v]) };
        let mut f = VideoFrame::new(p.width, p.height, family, sub, planes, None)?;
        f.color = ColorTags { matrix: vp9_matrix(p.color.color_space), full_range: p.color.full_range, ..ColorTags::UNSPECIFIED };
        f.pts = Some(p.pts);
        f.draft = p.draft;
        Ok(f)
    }
}

#[cfg(feature = "av1-decode")]
impl TryFrom<crate::av1::decode::Picture> for VideoFrame {
    type Error = FrameError;

    /// A YCbCr frame, a gray one for a monochrome stream, or an RGB one for a 4:4:4 stream with
    /// the identity matrix (`matrix_coefficients` 0), whose coded G, B, R planes are reordered to
    /// R, G, B; 16-bit storage at every bit depth.
    fn try_from(p: crate::av1::decode::Picture) -> Result<Self, FrameError> {
        let [y, u, v] = p.planes;
        let y = Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(y))?;
        let (family, sub, planes) = if p.mono_chrome {
            (ColorFamily::Gray, Subsampling::S444, vec![y])
        } else {
            let sub = Subsampling { x: p.subsampling_x, y: p.subsampling_y };
            let (cw, ch) = sub.chroma_size(p.width, p.height);
            let u = Plane::packed(cw, ch, p.bit_depth, Samples::U16(u))?;
            let v = Plane::packed(cw, ch, p.bit_depth, Samples::U16(v))?;
            // AV1 requires 4:4:4 with the identity matrix; a subsampled picture tagged with it
            // cannot be RGB and stays as coded.
            if p.matrix_coefficients == 0 && sub == Subsampling::S444 {
                (ColorFamily::Rgb, sub, vec![v, y, u])
            } else {
                (ColorFamily::YCbCr, sub, vec![y, u, v])
            }
        };
        let mut f = VideoFrame::new(p.width, p.height, family, sub, planes, None)?;
        f.color =
            ColorTags { primaries: p.color_primaries, transfer: p.transfer_characteristics, matrix: p.matrix_coefficients, full_range: p.full_range };
        f.pts = Some(p.pts);
        f.draft = p.draft;
        Ok(f)
    }
}

#[cfg(feature = "mjpeg")]
impl TryFrom<crate::mjpeg::Frame> for VideoFrame {
    type Error = FrameError;

    /// A gray or RGB frame, full range (a JPEG's YCbCr is converted to RGB by the decoder, so RGB
    /// frames carry the identity matrix). 8-bit pictures keep 8-bit storage; 12-bit ones are
    /// stored scaled to 16 bits and say so. The decoder returns interleaved samples, so this is
    /// the one conversion that copies them, into planes.
    fn try_from(p: crate::mjpeg::Frame) -> Result<Self, FrameError> {
        use crate::mjpeg::Samples as S;
        let (w, h) = (p.width, p.height);
        let channels = usize::from(p.channels);
        let family = match channels {
            1 => ColorFamily::Gray,
            3 => ColorFamily::Rgb,
            _ => return Err(FrameError::Shape("a Motion JPEG frame has 1 or 3 channels")),
        };
        let pixels = usize::try_from(w)
            .ok()
            .zip(usize::try_from(h).ok())
            .and_then(|(w, h)| w.checked_mul(h))
            .ok_or(FrameError::Plane("plane size overflows"))?;
        let stored = match &p.samples {
            S::U8(v) => v.len(),
            S::U16(v) => v.len(),
        };
        if pixels.checked_mul(channels) != Some(stored) {
            return Err(FrameError::Shape("sample count does not fit the frame"));
        }
        let planes = match p.samples {
            S::U8(v) => (0..channels)
                .map(|c| Plane::packed(w, h, 8, Samples::U8(v.iter().skip(c).step_by(channels).copied().collect())))
                .collect::<Result<Vec<_>, _>>()?,
            S::U16(v) => (0..channels)
                .map(|c| Plane::packed(w, h, 16, Samples::U16(v.iter().skip(c).step_by(channels).copied().collect())))
                .collect::<Result<Vec<_>, _>>()?,
        };
        let mut f = VideoFrame::new(w, h, family, Subsampling::S444, planes, None)?;
        f.color = ColorTags { matrix: if family == ColorFamily::Rgb { 0 } else { 2 }, full_range: true, ..ColorTags::UNSPECIFIED };
        Ok(f)
    }
}

#[cfg(feature = "mpeg2-decode")]
impl TryFrom<crate::mpeg2::Picture> for VideoFrame {
    type Error = FrameError;

    /// An 8-bit frame. MPEG-1 and MPEG-2 have no full-range flag, so the range is limited.
    fn try_from(p: crate::mpeg2::Picture) -> Result<Self, FrameError> {
        use crate::mpeg2::{ChromaFormat, FieldOrder as F};
        let sub = match p.chroma {
            ChromaFormat::Yuv420 => Subsampling::S420,
            ChromaFormat::Yuv422 => Subsampling::S422,
            ChromaFormat::Yuv444 => Subsampling::S444,
        };
        let (cw, ch) = (p.chroma_width(), p.chroma_height());
        let field_order = match p.field_order() {
            None => FieldOrder::Progressive,
            Some(F::TopFirst) => FieldOrder::TopFieldFirst,
            Some(F::BottomFirst) => FieldOrder::BottomFieldFirst,
        };
        let planes = vec![
            Plane::packed(p.width, p.height, 8, Samples::U8(p.y))?,
            Plane::packed(cw, ch, 8, Samples::U8(p.cb))?,
            Plane::packed(cw, ch, 8, Samples::U8(p.cr))?,
        ];
        let mut f = VideoFrame::new(p.width, p.height, ColorFamily::YCbCr, sub, planes, None)?;
        if let Some((primaries, transfer, matrix)) = p.info.colour {
            f.color = ColorTags { primaries, transfer, matrix, full_range: false };
        }
        f.pts = Some(p.pts);
        f.sar = sar(p.info.sar.0, p.info.sar.1);
        f.field_order = field_order;
        Ok(f)
    }
}

#[cfg(feature = "vp8-decode")]
impl TryFrom<crate::vp8::Picture> for VideoFrame {
    type Error = FrameError;

    /// An 8-bit 4:2:0 frame. `color_space` 0 (BT.601 YCbCr) maps to matrix 6; VP8 signals no
    /// primaries, transfer or range.
    fn try_from(p: crate::vp8::Picture) -> Result<Self, FrameError> {
        let planes = vec![
            Plane::packed(p.width, p.height, 8, Samples::U8(p.y))?,
            Plane::packed(p.chroma_width, p.chroma_height, 8, Samples::U8(p.u))?,
            Plane::packed(p.chroma_width, p.chroma_height, 8, Samples::U8(p.v))?,
        ];
        let mut f = VideoFrame::new(p.width, p.height, ColorFamily::YCbCr, Subsampling::S420, planes, None)?;
        if p.color_space == 0 {
            f.color.matrix = 6;
        }
        f.pts = Some(p.pts);
        Ok(f)
    }
}

#[cfg(feature = "rawvideo")]
impl TryFrom<crate::rawvideo::Frame> for VideoFrame {
    type Error = FrameError;

    /// A 4:2:2 YCbCr or an RGB frame at 8 or 10 bits, in 16-bit storage. Raw video carries no colour
    /// description, so YCbCr leaves everything unspecified; RGB gets the identity matrix and full
    /// range, as for every codec.
    fn try_from(p: crate::rawvideo::Frame) -> Result<Self, FrameError> {
        use crate::rawvideo::Kind;
        let (w, h, bits) = (p.width, p.height, p.bit_depth);
        let [a, b, c] = p.planes;
        let (family, sub) = match p.kind {
            Kind::Yuv422 => (ColorFamily::YCbCr, Subsampling::S422),
            Kind::Rgb => (ColorFamily::Rgb, Subsampling::S444),
        };
        let max_val = if bits < 16 { (1u16 << bits) - 1 } else { u16::MAX };
        if a.iter().any(|&s| s > max_val) || b.iter().any(|&s| s > max_val) || c.iter().any(|&s| s > max_val) {
            return Err(FrameError::Plane("sample value exceeds bit depth"));
        }
        let (cw, ch) = sub.chroma_size(w, h);
        let planes = vec![
            Plane::packed(w, h, bits, Samples::U16(a))?,
            Plane::packed(cw, ch, bits, Samples::U16(b))?,
            Plane::packed(cw, ch, bits, Samples::U16(c))?,
        ];
        let mut f = VideoFrame::new(w, h, family, sub, planes, None)?;
        if family == ColorFamily::Rgb {
            f.color = ColorTags { matrix: 0, full_range: true, ..ColorTags::UNSPECIFIED };
        }
        Ok(f)
    }
}

#[cfg(feature = "prores")]
impl TryFrom<crate::prores::Frame> for VideoFrame {
    type Error = FrameError;

    /// A 4:2:2 or 4:4:4 frame with optional alpha, 16-bit storage. The range is limited. The
    /// sample aspect ratio follows from `aspect_ratio` and the frame size.
    fn try_from(p: crate::prores::Frame) -> Result<Self, FrameError> {
        use crate::prores::{ChromaFormat, Interlace};
        let sub = match p.chroma {
            ChromaFormat::Yuv422 => Subsampling::S422,
            ChromaFormat::Yuv444 => Subsampling::S444,
        };
        let (cw, ch) = sub.chroma_size(p.width, p.height);
        let planes = vec![
            Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(p.y))?,
            Plane::packed(cw, ch, p.bit_depth, Samples::U16(p.cb))?,
            Plane::packed(cw, ch, p.bit_depth, Samples::U16(p.cr))?,
        ];
        let alpha = p.alpha.map(|a| Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(a))).transpose()?;
        let mut f = VideoFrame::new(p.width, p.height, ColorFamily::YCbCr, sub, planes, alpha)?;
        f.color = ColorTags { primaries: h273(p.color.primaries), transfer: h273(p.color.transfer), matrix: h273(p.color.matrix), full_range: false };
        f.sar = prores_sar(p.aspect_ratio, p.width, p.height);
        f.field_order = match p.interlace {
            Interlace::Progressive => FieldOrder::Progressive,
            Interlace::TopFieldFirst => FieldOrder::TopFieldFirst,
            Interlace::BottomFieldFirst => FieldOrder::BottomFieldFirst,
        };
        Ok(f)
    }
}

#[cfg(feature = "dnx")]
impl TryFrom<crate::dnx::Frame> for VideoFrame {
    type Error = FrameError;

    /// A YCbCr frame, or an RGB one whose stored G, B, R planes are reordered to R, G, B; optional
    /// alpha, 16-bit storage. The colour volume (`CLV`) gives only the matrix; primaries, transfer
    /// and range are not signalled. An RGB frame is tagged full range, as an RGB frame that does
    /// not signal a range is.
    fn try_from(p: crate::dnx::Frame) -> Result<Self, FrameError> {
        use crate::dnx::{ChromaFormat, ColorVolume, Interlace};
        let sub = match p.chroma {
            ChromaFormat::Yuv420 => Subsampling::S420,
            ChromaFormat::Yuv422 => Subsampling::S422,
            ChromaFormat::Yuv444 => Subsampling::S444,
        };
        let (cw, ch) = sub.chroma_size(p.width, p.height);
        let y = Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(p.y))?;
        let cb = Plane::packed(cw, ch, p.bit_depth, Samples::U16(p.cb))?;
        let cr = Plane::packed(cw, ch, p.bit_depth, Samples::U16(p.cr))?;
        let (family, planes) = if p.rgb { (ColorFamily::Rgb, vec![cr, y, cb]) } else { (ColorFamily::YCbCr, vec![y, cb, cr]) };
        let alpha = p.alpha.map(|a| Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(a))).transpose()?;
        let mut f = VideoFrame::new(p.width, p.height, family, sub, planes, alpha)?;
        f.color.matrix = match p.color_volume {
            _ if p.rgb => 0,
            ColorVolume::Bt709 => 1,
            ColorVolume::Bt2020Ncl => 9,
            ColorVolume::Bt2020Cl => 10,
            ColorVolume::OutOfBand => 2,
        };
        f.sar = sar(p.par.0.into(), p.par.1.into());
        f.field_order = match p.interlace {
            Interlace::Progressive => FieldOrder::Progressive,
            Interlace::TopFieldFirst => FieldOrder::TopFieldFirst,
            Interlace::BottomFieldFirst => FieldOrder::BottomFieldFirst,
        };
        Ok(f)
    }
}

#[cfg(any(feature = "apv-decode", feature = "apv-encode"))]
impl TryFrom<crate::apv::Frame> for VideoFrame {
    type Error = FrameError;

    /// A YCbCr or gray frame, 16-bit storage. 4:4:4:4 is 4:4:4 with its fourth component as alpha.
    fn try_from(p: crate::apv::Frame) -> Result<Self, FrameError> {
        use crate::apv::ChromaFormat;
        let y = Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(p.y))?;
        let (family, sub, planes) = match p.chroma {
            ChromaFormat::Monochrome => (ColorFamily::Gray, Subsampling::S444, vec![y]),
            c => {
                let sub = if c == ChromaFormat::Yuv422 { Subsampling::S422 } else { Subsampling::S444 };
                let (cw, ch) = sub.chroma_size(p.width, p.height);
                let cb = Plane::packed(cw, ch, p.bit_depth, Samples::U16(p.cb))?;
                let cr = Plane::packed(cw, ch, p.bit_depth, Samples::U16(p.cr))?;
                (ColorFamily::YCbCr, sub, vec![y, cb, cr])
            }
        };
        let alpha = p.alpha.map(|a| Plane::packed(p.width, p.height, p.bit_depth, Samples::U16(a))).transpose()?;
        let mut f = VideoFrame::new(p.width, p.height, family, sub, planes, alpha)?;
        f.color = ColorTags { primaries: p.color.primaries, transfer: p.color.transfer, matrix: p.color.matrix, full_range: p.color.full_range };
        Ok(f)
    }
}

#[cfg(feature = "ffv1")]
impl TryFrom<crate::ffv1::Frame> for VideoFrame {
    type Error = FrameError;

    /// A YCbCr, RGB or gray frame with optional transparency, 16-bit storage. FFV1 carries no
    /// colour description; RGB frames are tagged with the identity matrix and full range.
    fn try_from(p: crate::ffv1::Frame) -> Result<Self, FrameError> {
        use crate::ffv1::Colorspace;
        let (w, h, bits) = (p.width, p.height, p.bit_depth);
        let full = |v: Vec<u16>| Plane::packed(w, h, bits, Samples::U16(v));
        let mut it = p.planes.into_iter();
        let count = it.len();
        let mut next = || it.next().ok_or(FrameError::Shape("plane count does not fit the colour family"));
        let (family, sub, planes, alpha) = match (p.colorspace, count) {
            (Colorspace::YCbCr, 1 | 2) => {
                let y = full(next()?)?;
                let a = if count == 2 { Some(full(next()?)?) } else { None };
                (ColorFamily::Gray, Subsampling::S444, vec![y], a)
            }
            (Colorspace::YCbCr, 3 | 4) => {
                let sub = Subsampling { x: p.chroma_shift.0, y: p.chroma_shift.1 };
                let (cw, ch) = sub.chroma_size(w, h);
                let y = full(next()?)?;
                let cb = Plane::packed(cw, ch, bits, Samples::U16(next()?))?;
                let cr = Plane::packed(cw, ch, bits, Samples::U16(next()?))?;
                let a = if count == 4 { Some(full(next()?)?) } else { None };
                (ColorFamily::YCbCr, sub, vec![y, cb, cr], a)
            }
            (Colorspace::Rgb, 3 | 4) => {
                let planes = vec![full(next()?)?, full(next()?)?, full(next()?)?];
                let a = if count == 4 { Some(full(next()?)?) } else { None };
                (ColorFamily::Rgb, Subsampling::S444, planes, a)
            }
            _ => return Err(FrameError::Shape("plane count does not fit the colour family")),
        };
        let mut f = VideoFrame::new(w, h, family, sub, planes, alpha)?;
        if family == ColorFamily::Rgb {
            f.color = ColorTags { matrix: 0, full_range: true, ..ColorTags::UNSPECIFIED };
        }
        Ok(f)
    }
}

#[cfg(feature = "bink-decode")]
impl TryFrom<crate::bink::Picture> for VideoFrame {
    type Error = FrameError;

    /// An 8-bit 4:2:0 frame. Bink signals no colour description, and the picture carries no timing.
    fn try_from(p: crate::bink::Picture) -> Result<Self, FrameError> {
        let planes = vec![
            Plane::packed(p.width, p.height, 8, Samples::U8(p.y))?,
            Plane::packed(p.chroma_width, p.chroma_height, 8, Samples::U8(p.u))?,
            Plane::packed(p.chroma_width, p.chroma_height, 8, Samples::U8(p.v))?,
        ];
        VideoFrame::new(p.width, p.height, ColorFamily::YCbCr, Subsampling::S420, planes, None)
    }
}

#[cfg(feature = "theora-decode")]
impl TryFrom<crate::theora::Picture> for VideoFrame {
    type Error = FrameError;

    fn try_from(p: crate::theora::Picture) -> Result<Self, FrameError> {
        if p.pic_width == 0 || p.pic_height == 0 {
            return Err(FrameError::Shape("picture dimensions must be non-zero"));
        }
        let src_uv_w = match p.pixel_format {
            crate::theora::PixelFormat::Yuv420 | crate::theora::PixelFormat::Yuv422 => (p.width as usize).div_ceil(2),
            crate::theora::PixelFormat::Yuv444 => p.width as usize,
        };
        if p.y_stride == 0 || p.uv_stride == 0 {
            return Err(FrameError::Plane("stride cannot be zero"));
        }
        if p.y_stride < p.width as usize || p.uv_stride < src_uv_w {
            return Err(FrameError::Plane("stride shorter than plane width"));
        }
        let crop_end_x = p.pic_x.checked_add(p.pic_width).ok_or(FrameError::Shape("crop width overflow"))?;
        let crop_end_y = p.pic_y.checked_add(p.pic_height).ok_or(FrameError::Shape("crop height overflow"))?;
        if crop_end_x > p.width || crop_end_y > p.height {
            return Err(FrameError::Shape("crop bounds exceed frame dimensions"));
        }

        // Determine if crop is aligned with chroma subsampling
        let is_aligned = match p.pixel_format {
            crate::theora::PixelFormat::Yuv420 => p.pic_x.is_multiple_of(2) && p.pic_y.is_multiple_of(2) && p.pic_height.is_multiple_of(2),
            crate::theora::PixelFormat::Yuv422 => p.pic_x.is_multiple_of(2),
            crate::theora::PixelFormat::Yuv444 => true,
        };

        let (sub, cw, ch) = if is_aligned {
            match p.pixel_format {
                crate::theora::PixelFormat::Yuv420 => (Subsampling::S420, p.pic_width.div_ceil(2), p.pic_height.div_ceil(2)),
                crate::theora::PixelFormat::Yuv422 => (Subsampling::S422, p.pic_width.div_ceil(2), p.pic_height),
                crate::theora::PixelFormat::Yuv444 => (Subsampling::S444, p.pic_width, p.pic_height),
            }
        } else {
            // Misaligned crop: retain all chroma samples across odd offsets by converting to S444
            (Subsampling::S444, p.pic_width, p.pic_height)
        };

        let w_u64 = p.pic_width as u64;
        let h_u64 = p.pic_height as u64;
        let cw_u64 = cw as u64;
        let ch_u64 = ch as u64;
        let y_bytes = w_u64.checked_mul(h_u64).ok_or(FrameError::Shape("crop size overflow"))?;
        let c_bytes = cw_u64.checked_mul(ch_u64).ok_or(FrameError::Shape("crop uv size overflow"))?;
        let total_bytes = y_bytes.checked_add(c_bytes.checked_mul(2).ok_or(FrameError::Shape("overflow"))?).ok_or(FrameError::Shape("overflow"))?;
        if total_bytes > crate::budget::DECODE_MEMORY_BUDGET {
            return Err(FrameError::Shape("frame size exceeds memory budget"));
        }

        let y_len = usize::try_from(y_bytes).map_err(|_| FrameError::Shape("crop size exceeds usize"))?;
        let uv_len = usize::try_from(c_bytes).map_err(|_| FrameError::Shape("crop uv size exceeds usize"))?;

        let mut y_cropped = Vec::new();
        y_cropped.try_reserve_exact(y_len).map_err(|_| FrameError::Shape("out of memory"))?;
        for row in 0..p.pic_height {
            let py = p.pic_y.checked_add(p.pic_height - 1 - row).ok_or(FrameError::Shape("crop y overflow"))?;
            let start = (py as usize)
                .checked_mul(p.y_stride)
                .and_then(|s| s.checked_add(p.pic_x as usize))
                .ok_or(FrameError::Shape("crop offset overflow"))?;
            let end = start.checked_add(p.pic_width as usize).ok_or(FrameError::Shape("crop slice overflow"))?;
            let slice = p.y.get(start..end).ok_or(FrameError::Plane("buffer shorter than cropped region"))?;
            y_cropped.extend_from_slice(slice);
        }

        let mut u_cropped = Vec::new();
        u_cropped.try_reserve_exact(uv_len).map_err(|_| FrameError::Shape("out of memory"))?;
        let mut v_cropped = Vec::new();
        v_cropped.try_reserve_exact(uv_len).map_err(|_| FrameError::Shape("out of memory"))?;

        if is_aligned {
            let (cx, cy) = match p.pixel_format {
                crate::theora::PixelFormat::Yuv420 => (p.pic_x / 2, p.pic_y / 2),
                crate::theora::PixelFormat::Yuv422 => (p.pic_x / 2, p.pic_y),
                crate::theora::PixelFormat::Yuv444 => (p.pic_x, p.pic_y),
            };
            for row in 0..ch {
                let py = cy.checked_add(ch - 1 - row).ok_or(FrameError::Shape("crop chroma y overflow"))?;
                let start = (py as usize)
                    .checked_mul(p.uv_stride)
                    .and_then(|s| s.checked_add(cx as usize))
                    .ok_or(FrameError::Shape("crop chroma offset overflow"))?;
                let end = start.checked_add(cw as usize).ok_or(FrameError::Shape("crop chroma slice overflow"))?;
                let u_slice = p.u.get(start..end).ok_or(FrameError::Plane("u buffer shorter than cropped region"))?;
                let v_slice = p.v.get(start..end).ok_or(FrameError::Plane("v buffer shorter than cropped region"))?;
                u_cropped.extend_from_slice(u_slice);
                v_cropped.extend_from_slice(v_slice);
            }
        } else {
            // Misaligned crop: extract full-resolution chroma for each pixel
            for row in 0..p.pic_height {
                let py = p.pic_y.checked_add(p.pic_height - 1 - row).ok_or(FrameError::Shape("crop y overflow"))?;
                let cy = match p.pixel_format {
                    crate::theora::PixelFormat::Yuv420 => py / 2,
                    _ => py,
                };
                let row_start = (cy as usize).checked_mul(p.uv_stride).ok_or(FrameError::Shape("chroma stride overflow"))?;
                for col in 0..p.pic_width {
                    let px = p.pic_x.checked_add(col).ok_or(FrameError::Shape("crop x overflow"))?;
                    let cx = match p.pixel_format {
                        crate::theora::PixelFormat::Yuv444 => px,
                        _ => px / 2,
                    };
                    let offset = row_start.checked_add(cx as usize).ok_or(FrameError::Shape("chroma offset overflow"))?;
                    let &u_val = p.u.get(offset).ok_or(FrameError::Plane("u buffer shorter than cropped region"))?;
                    let &v_val = p.v.get(offset).ok_or(FrameError::Plane("v buffer shorter than cropped region"))?;
                    u_cropped.push(u_val);
                    v_cropped.push(v_val);
                }
            }
        }

        let planes = vec![
            Plane::packed(p.pic_width, p.pic_height, 8, Samples::U8(y_cropped))?,
            Plane::packed(cw, ch, 8, Samples::U8(u_cropped))?,
            Plane::packed(cw, ch, 8, Samples::U8(v_cropped))?,
        ];

        let mut f = VideoFrame::new(p.pic_width, p.pic_height, ColorFamily::YCbCr, sub, planes, None)?;
        match p.color_space {
            crate::theora::ColorSpace::Rec470M => {
                f.color.matrix = 6;
                f.color.primaries = 4;
                f.color.transfer = 6;
            }
            crate::theora::ColorSpace::Rec470Bg => {
                f.color.matrix = 5;
                f.color.primaries = 5;
                f.color.transfer = 1;
            }
            _ => {}
        }
        f.pts = p.pts;
        Ok(f)
    }
}

#[cfg(feature = "vmd-decode")]
impl TryFrom<&crate::vmd::Decoder> for VideoFrame {
    type Error = FrameError;

    /// The decoder's current picture, its palette indices looked up in its palette: 8-bit R, G, B
    /// planes at full size (the samples are copied, as VMD stores none of them as planes). VMD
    /// signals no colour description beyond RGB, nor timing.
    fn try_from(d: &crate::vmd::Decoder) -> Result<Self, FrameError> {
        paletted(d.width(), d.height(), d.indices(), d.palette())
    }
}

/// A picture of palette indices, `width` a row, as 8-bit R, G, B planes. Matrix 0 (identity) and
/// full range, the other colour tags unspecified.
#[cfg(feature = "vmd-decode")]
fn paletted(width: usize, height: usize, indices: &[u8], palette: &[[u8; 3]; 256]) -> Result<VideoFrame, FrameError> {
    let too_large = FrameError::Shape("picture size does not fit in u32");
    let (w, h) = (u32::try_from(width).map_err(|_| too_large.clone())?, u32::try_from(height).map_err(|_| too_large)?);
    let n = width.checked_mul(height).ok_or(FrameError::Shape("picture size overflows"))?;
    let indices = indices.get(..n).ok_or(FrameError::Plane("fewer palette indices than pixels"))?;
    let mut planes = [Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)];
    for &i in indices {
        let [r, g, b] = palette[usize::from(i)];
        planes[0].push(r);
        planes[1].push(g);
        planes[2].push(b);
    }
    let [r, g, b] = planes;
    let planes = vec![Plane::packed(w, h, 8, Samples::U8(r))?, Plane::packed(w, h, 8, Samples::U8(g))?, Plane::packed(w, h, 8, Samples::U8(b))?];
    let mut f = VideoFrame::new(w, h, ColorFamily::Rgb, Subsampling::S444, planes, None)?;
    f.color.matrix = 0;
    Ok(f)
}

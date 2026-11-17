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
    /// and range are not signalled.
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

/// A picture of palette indices, `width` a row, as 8-bit R, G, B planes. Matrix 0 (identity), the
/// other colour tags unspecified.
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

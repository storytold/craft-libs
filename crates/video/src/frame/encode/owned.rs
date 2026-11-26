//! Owned codec frames. Their buffers are copied; source samples and metadata are not modified.

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
use super::Subsampling;

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode", feature = "ffv1", feature = "mjpeg"))]
// Each codec feature uses a different subset of the conversion helpers.
#[allow(unused_imports)]
use super::{ColorFamily, Cow, FrameError, Planar8, Planar16, VideoFrame, len, reserve};

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode", feature = "ffv1"))]
fn owned(plane: Cow<'_, [u16]>) -> Result<Vec<u16>, FrameError> {
    match plane {
        Cow::Owned(v) => Ok(v),
        Cow::Borrowed(v) => {
            let mut out = reserve(v.len())?;
            out.extend_from_slice(v);
            Ok(out)
        }
    }
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
struct Components {
    y: Vec<u16>,
    cb: Vec<u16>,
    cr: Vec<u16>,
    alpha: Option<Vec<u16>>,
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv-encode"))]
fn components(f: &VideoFrame) -> Result<Components, FrameError> {
    let p = Planar16::try_from(f)?;
    let mut planes = p.planes.into_iter();
    let y = owned(planes.next().ok_or(FrameError::Shape("missing colour plane"))?)?;
    let cb = planes.next().map(owned).transpose()?.unwrap_or_default();
    let cr = planes.next().map(owned).transpose()?.unwrap_or_default();
    Ok(Components { y, cb, cr, alpha: p.alpha.map(owned).transpose()? })
}

#[cfg(feature = "prores")]
impl TryFrom<&VideoFrame> for crate::prores::Frame {
    type Error = FrameError;
    /// Copy YCbCr 4:2:2/4:4:4 into packed `u16` planes; alpha requires 4:4:4.
    /// Colour and representable aspect ratio are copied. Frame rate is unset; configure the
    /// encoder's colour, aspect, frame rate and alpha options separately (it uses its config).
    fn try_from(f: &VideoFrame) -> Result<Self, FrameError> {
        use crate::prores::{ChromaFormat, ColorInfo, Interlace};
        if f.family != ColorFamily::YCbCr || f.color.full_range {
            return Err(FrameError::Shape("ProRes requires limited-range YCbCr"));
        }
        let chroma = match f.subsampling {
            Subsampling::S422 if f.alpha.is_none() => ChromaFormat::Yuv422,
            Subsampling::S444 => ChromaFormat::Yuv444,
            _ => return Err(FrameError::Shape("ProRes requires 4:2:2 without alpha or 4:4:4")),
        };
        let aspect_ratio = match f.sar {
            None => 0,
            Some((n, d)) if n > 0 && n == d => 1,
            Some((n, d)) if n > 0 && d > 0 && u128::from(n) * u128::from(f.width) * 3 == u128::from(d) * u128::from(f.height) * 4 => 2,
            Some((n, d)) if n > 0 && d > 0 && u128::from(n) * u128::from(f.width) * 9 == u128::from(d) * u128::from(f.height) * 16 => 3,
            _ => return Err(FrameError::Shape("aspect ratio cannot be represented by ProRes")),
        };
        let Components { y, cb, cr, alpha } = components(f)?;
        let code = |v| if v == 2 { 0 } else { v };
        Ok(Self {
            width: f.width,
            height: f.height,
            chroma,
            bit_depth: f.bit_depth(),
            y,
            cb,
            cr,
            alpha,
            interlace: Interlace::Progressive,
            color: ColorInfo { primaries: code(f.color.primaries), transfer: code(f.color.transfer), matrix: code(f.color.matrix) },
            aspect_ratio,
            frame_rate_code: 0,
        })
    }
}

#[cfg(feature = "dnx")]
impl TryFrom<&VideoFrame> for crate::dnx::Frame {
    type Error = FrameError;
    /// Copy YCbCr 4:2:2/4:4:4 or RGB into DNx's Y/Cb/Cr or G/B/R planes. Alpha is unsupported
    /// by this encoder. CID is unset (selected by encoder profile). Colour volume and pixel
    /// aspect are copied where representable; encoder signalling is configured separately.
    fn try_from(f: &VideoFrame) -> Result<Self, FrameError> {
        use crate::dnx::{ChromaFormat, ColorVolume, Interlace};
        if f.alpha.is_some() {
            return Err(FrameError::Shape("DNx encoder does not support alpha"));
        }
        let chroma = match (f.family, f.subsampling) {
            (ColorFamily::YCbCr, Subsampling::S422) => ChromaFormat::Yuv422,
            (ColorFamily::YCbCr | ColorFamily::Rgb, Subsampling::S444) => ChromaFormat::Yuv444,
            _ => return Err(FrameError::Shape("DNx encoder requires 4:2:2 or 4:4:4 YCbCr, or RGB")),
        };
        let par = match f.sar {
            None => (0, 0),
            Some((mut n, mut d)) if n > 0 && d > 0 => {
                let (mut a, mut b) = (n, d);
                while b != 0 {
                    (a, b) = (b, a % b);
                }
                n /= a;
                d /= a;
                (
                    u16::try_from(n).map_err(|_| FrameError::Shape("DNx pixel aspect ratio exceeds u16"))?,
                    u16::try_from(d).map_err(|_| FrameError::Shape("DNx pixel aspect ratio exceeds u16"))?,
                )
            }
            _ => return Err(FrameError::Shape("invalid pixel aspect ratio")),
        };
        let Components { mut y, mut cb, mut cr, alpha } = components(f)?;
        let rgb = f.family == ColorFamily::Rgb;
        if rgb {
            (y, cb, cr) = (cb, cr, y);
        }
        let color_volume = match f.color.matrix {
            1 => ColorVolume::Bt709,
            9 => ColorVolume::Bt2020Ncl,
            10 => ColorVolume::Bt2020Cl,
            _ => ColorVolume::OutOfBand,
        };
        Ok(Self {
            width: f.width,
            height: f.height,
            chroma,
            rgb,
            bit_depth: f.bit_depth(),
            y,
            cb,
            cr,
            alpha,
            interlace: Interlace::Progressive,
            color_volume,
            cid: 0,
            par,
        })
    }
}

#[cfg(feature = "apv-encode")]
impl TryFrom<&VideoFrame> for crate::apv::Frame {
    type Error = FrameError;
    /// Copy gray, 4:2:2 or 4:4:4 YCbCr (alpha only with 4:4:4). Colour tags are copied.
    /// Profile/level/band are unset: the encoder chooses them from its configuration.
    fn try_from(f: &VideoFrame) -> Result<Self, FrameError> {
        use crate::apv::{ChromaFormat, ColorInfo};
        let chroma = match (f.family, f.subsampling, f.alpha.is_some()) {
            (ColorFamily::Gray, _, false) => ChromaFormat::Monochrome,
            (ColorFamily::YCbCr, Subsampling::S422, false) => ChromaFormat::Yuv422,
            (ColorFamily::YCbCr, Subsampling::S444, false) => ChromaFormat::Yuv444,
            (ColorFamily::YCbCr, Subsampling::S444, true) => ChromaFormat::Yuv4444,
            _ => return Err(FrameError::Shape("APV requires gray, 4:2:2 or 4:4:4 YCbCr; alpha requires 4:4:4")),
        };
        let Components { y, cb, cr, alpha } = components(f)?;
        Ok(Self {
            width: f.width,
            height: f.height,
            chroma,
            bit_depth: f.bit_depth(),
            y,
            cb,
            cr,
            alpha,
            color: ColorInfo { primaries: f.color.primaries, transfer: f.color.transfer, matrix: f.color.matrix, full_range: f.color.full_range },
            profile_idc: 0,
            level_idc: 0,
            band_idc: 0,
            mastering_display: None,
            content_light: None,
        })
    }
}

#[cfg(feature = "ffv1")]
impl TryFrom<&VideoFrame> for crate::ffv1::Frame {
    type Error = FrameError;
    /// Copy gray, YCbCr or RGB and optional alpha to FFV1 planes, preserving depth and chroma
    /// shifts. Colour tags, SAR and timestamps have no fields here and remain out of band.
    fn try_from(f: &VideoFrame) -> Result<Self, FrameError> {
        let p = Planar16::try_from(f)?;
        let mut planes = p.planes.into_iter().map(owned).collect::<Result<Vec<_>, _>>()?;
        if let Some(a) = p.alpha {
            planes.push(owned(a)?);
        }
        Ok(Self {
            width: f.width,
            height: f.height,
            colorspace: if f.family == ColorFamily::Rgb { crate::ffv1::Colorspace::Rgb } else { crate::ffv1::Colorspace::YCbCr },
            bit_depth: f.bit_depth(),
            chroma_shift: (f.subsampling.x, f.subsampling.y),
            planes,
        })
    }
}

#[cfg(feature = "mjpeg")]
impl TryFrom<&VideoFrame> for crate::mjpeg::Frame {
    type Error = FrameError;
    /// Interleave 8-bit RGB or gray for the baseline encoder. Alpha is rejected to avoid silently
    /// dropping it. Colour tags, SAR and timestamps must be carried by the caller/container.
    fn try_from(f: &VideoFrame) -> Result<Self, FrameError> {
        let p = Planar8::try_from(f)?;
        let channels = match f.family {
            ColorFamily::Rgb => 3,
            ColorFamily::Gray => 1,
            _ => return Err(FrameError::Shape("Motion JPEG input requires RGB or gray")),
        };
        if p.alpha.is_some() {
            return Err(FrameError::Shape("Motion JPEG encoder does not preserve alpha"));
        }
        let n = len(f.planes.first().ok_or(FrameError::Shape("missing plane"))?)?;
        let mut pixels = reserve(n.checked_mul(channels).ok_or(FrameError::Shape("RGB size overflows"))?)?;
        for i in 0..n {
            for plane in &p.planes {
                pixels.push(*plane.get(i).ok_or(FrameError::Plane("short plane"))?);
            }
        }
        Ok(Self { width: f.width, height: f.height, channels: channels as u8, samples: crate::mjpeg::Samples::U8(pixels), damaged: false })
    }
}

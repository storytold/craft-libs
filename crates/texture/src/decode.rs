//! Decoding PNG, TIFF, Radiance HDR and OpenEXR bytes into float [`Image`] planes, each format
//! behind its own Cargo feature. Integer samples are scaled by their own maximum (255 for 8 bits,
//! 65535 for 16 bits), so a 16-bit file keeps all 65536 levels. Float samples are kept as stored:
//! values above 1, negative values and, in OpenEXR, NaN and infinities. The one exception is a
//! 64-bit TIFF float, which is narrowed to `f32` and loses any precision beyond it.
//!
//! Every function takes the file's bytes and does no I/O. The size of the result is checked
//! against [`Limits`] before its plane is reserved, and the reservation is fallible. The format
//! decoders are bounded by the same limits: their own buffer is at most `max_bytes`, so a decode
//! holds at most two buffers of that size at once. Damaged, cut-short and unsupported files are
//! errors; nothing is filled in.
//!
//! A gray file without alpha gives a [`Kind::Gray`] plane. Every other file gives a
//! [`Kind::Color`] RGBA plane: gray with alpha repeats the gray value in R, G and B, and a file
//! without alpha gets alpha 1. No transfer function is applied: [`Decoded::transfer`] says what
//! the values are, and the caller converts color data (glTF base color and emissive RGB are sRGB)
//! and leaves linear data (normals, metallic-roughness, occlusion) as it is.

#[cfg(any(feature = "png", feature = "tiff"))]
use craft_image_codecs::{ChannelLayout, Cicp, Format, SampleType, f16};

use crate::{Image, ImageError, Kind, Limits};

/// What the RGB values of a decoded plane are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transfer {
    /// Integer code values scaled to `0..=1`, in the file's own encoding. The file states no
    /// transfer function through a PNG `cICP` chunk or an ICC profile; PNG `sRGB` and `gAMA`
    /// chunks are not read. The caller decides by the use of the texture whether to apply the
    /// sRGB decoding.
    Encoded,
    /// Linear values: Radiance HDR, OpenEXR, and PNG with a `cICP` chunk naming linear transfer.
    Linear,
    /// The file describes its color in a way these tags do not cover (an ICC profile, a `cICP`
    /// transfer other than sRGB or linear), or it is a float TIFF, whose transfer the format does
    /// not define.
    Unknown,
}

/// How the color of a decoded plane relates to its alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alpha {
    /// Color is independent of alpha. TIFF associated alpha is converted to this form.
    Straight,
    /// Color is already multiplied by alpha, as OpenEXR stores it. A pixel with alpha 0 can still
    /// carry color (an emitting pixel), and is kept as stored.
    Premultiplied,
}

/// A decoded texture: the plane and what its values mean.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Decoded {
    /// The pixels, top row first.
    pub image: Image,
    pub transfer: Transfer,
    pub alpha: Alpha,
}

/// Why bytes could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The bytes do not start like a file of the format asked for (empty bytes included).
    #[error("the bytes are not a {0} file")]
    WrongFormat(&'static str),
    /// A valid file this decoder does not cover, such as a CMYK or Lab TIFF, or deep OpenEXR.
    #[error("unsupported {format} file: {reason}")]
    Unsupported { format: &'static str, reason: String },
    /// The file is damaged or cut short.
    #[error("damaged {format} file: {reason}")]
    Malformed { format: &'static str, reason: String },
    /// The format decoder refused a size over the limits it was given.
    #[error("{format} file exceeds a limit: {reason}")]
    LimitExceeded { format: &'static str, reason: String },
    /// The plane is over [`Limits`], or its memory could not be reserved.
    #[error(transparent)]
    Image(#[from] ImageError),
}

/// Reserves exactly `len` floats for a `w × h` plane.
fn reserve(len: usize, w: u32, h: u32) -> Result<Vec<f32>, DecodeError> {
    let mut data = Vec::new();
    data.try_reserve_exact(len).map_err(|_| ImageError::OutOfMemory(w, h))?;
    Ok(data)
}

#[cfg(any(feature = "png", feature = "tiff"))]
fn codec_error(format: &'static str, e: craft_image_codecs::CodecError) -> DecodeError {
    use craft_image_codecs::CodecError;
    match e {
        CodecError::UnknownFormat => DecodeError::WrongFormat(format),
        CodecError::Unsupported { reason, .. } => DecodeError::Unsupported { format, reason },
        CodecError::LimitExceeded(reason) => DecodeError::LimitExceeded { format, reason },
        CodecError::Malformed { message, .. } => DecodeError::Malformed { format, reason: message },
        e => DecodeError::Malformed { format, reason: e.to_string() },
    }
}

/// Decodes `bytes` as `format` through `craft-image-codecs`, keeping the stored sample depth.
#[cfg(any(feature = "png", feature = "tiff"))]
fn decode_codec(bytes: &[u8], format: Format, name: &'static str, limits: &Limits) -> Result<Decoded, DecodeError> {
    if craft_image_codecs::detect(bytes) != Some(format) {
        return Err(DecodeError::WrongFormat(name));
    }
    let codec_limits = craft_image_codecs::Limits {
        max_width: limits.max_side,
        max_height: limits.max_side,
        max_pixels: limits.max_pixels,
        max_alloc: limits.max_bytes,
    };
    let opts = craft_image_codecs::DecodeOptions { limits: codec_limits, ..Default::default() };
    let img = craft_image_codecs::decode_as_with(format, bytes, &opts).map_err(|e| codec_error(name, e))?;
    if img.warnings.iter().any(|w| matches!(w, craft_image_codecs::DecodeWarning::Truncated { .. })) {
        return Err(DecodeError::Malformed { format: name, reason: "the image data ends early".into() });
    }
    let image = codec_plane(&img, name, limits)?;
    Ok(Decoded { image, transfer: codec_transfer(&img), alpha: Alpha::Straight })
}

/// The transfer a `craft-image-codecs` image states. An ICC profile makes it unknown, since
/// `craft-image-codecs` relies on the `cICP` only when there is no profile.
#[cfg(any(feature = "png", feature = "tiff"))]
fn codec_transfer(img: &craft_image_codecs::Image) -> Transfer {
    /// The `cICP` transfer code of the sRGB transfer function (ITU-T H.273, IEC 61966-2-1).
    const TRANSFER_SRGB: u16 = 13;
    if img.icc.is_some() {
        return Transfer::Unknown;
    }
    match img.cicp {
        Some(c) if c.transfer == Cicp::TRANSFER_LINEAR => Transfer::Linear,
        Some(c) if c.transfer == TRANSFER_SRGB => Transfer::Encoded,
        Some(_) => Transfer::Unknown,
        None if img.sample_type().is_float() => Transfer::Unknown,
        None => Transfer::Encoded,
    }
}

/// One stored sample as `f32`: integers scaled by their maximum, floats as stored.
#[cfg(any(feature = "png", feature = "tiff"))]
fn sample_f32(sample: SampleType, s: &[u8]) -> f32 {
    match (sample, s) {
        (SampleType::U8, &[a]) => f32::from(a) / 255.0,
        (SampleType::U16, &[a, b]) => f32::from(u16::from_ne_bytes([a, b])) / 65535.0,
        (SampleType::F16, &[a, b]) => f16::from_bits(u16::from_ne_bytes([a, b])).to_f32(),
        (SampleType::F32, &[a, b, c, d]) => f32::from_ne_bytes([a, b, c, d]),
        _ => 0.0,
    }
}

/// Converts a decoded image into one plane, reserved after the limit check of its final kind.
#[cfg(any(feature = "png", feature = "tiff"))]
fn codec_plane(img: &craft_image_codecs::Image, name: &'static str, limits: &Limits) -> Result<Image, DecodeError> {
    let layout = img.layout();
    let kind = match layout {
        ChannelLayout::Gray => Kind::Gray,
        ChannelLayout::GrayA | ChannelLayout::Rgb | ChannelLayout::Rgba => Kind::Color,
        other => return Err(DecodeError::Unsupported { format: name, reason: format!("{other:?} color is not converted to RGB") }),
    };
    let (w, h) = img.dimensions();
    let len = limits.check(w, h, kind)?;
    let sample = img.sample_type();
    let bytes = sample.bytes();
    let pixel_bytes = layout.channels() * bytes;
    if img.data().len() != img.pixel_count().saturating_mul(pixel_bytes) {
        return Err(DecodeError::Malformed { format: name, reason: "the decoded buffer does not match the image size".into() });
    }
    let mut data = reserve(len, w, h)?;
    for px in img.data().chunks_exact(pixel_bytes) {
        let at = |c: usize| sample_f32(sample, px.get(c * bytes..(c + 1) * bytes).unwrap_or_default());
        match layout {
            ChannelLayout::Gray => data.push(at(0)),
            ChannelLayout::GrayA => {
                let g = at(0);
                data.extend([g, g, g, at(1)]);
            }
            ChannelLayout::Rgb => data.extend([at(0), at(1), at(2), 1.0]),
            _ => data.extend([at(0), at(1), at(2), at(3)]),
        }
    }
    Ok(Image::from_data(w, h, kind, data, limits)?)
}

/// Decodes a PNG: 1- to 16-bit gray, gray with alpha, RGB, RGBA and palette images. A 16-bit
/// sample `v` becomes `v / 65535`. The EXIF orientation, when the file has one, is applied.
#[cfg(feature = "png")]
pub fn png(bytes: &[u8], limits: &Limits) -> Result<Decoded, DecodeError> {
    decode_codec(bytes, Format::Png, "PNG", limits)
}

/// Decodes the first page of a TIFF: gray, gray with alpha, RGB, RGBA, palette and YCbCr images,
/// 8- and 16-bit integer samples (wider integers keep their top 16 bits) and 16- and 32-bit float
/// samples (64-bit floats become 32-bit). Associated alpha is converted to straight alpha. CMYK
/// and Lab files are [`DecodeError::Unsupported`], and a file whose image data ends early is
/// [`DecodeError::Malformed`].
#[cfg(feature = "tiff")]
pub fn tiff(bytes: &[u8], limits: &Limits) -> Result<Decoded, DecodeError> {
    decode_codec(bytes, Format::Tiff, "TIFF", limits)
}

/// Decodes a Radiance HDR (RGBE) picture into linear RGB with alpha 1. The `EXPOSURE` and
/// `PRIMARIES` header lines are not applied.
#[cfg(feature = "hdr")]
pub fn hdr(bytes: &[u8], limits: &Limits) -> Result<Decoded, DecodeError> {
    const NAME: &str = "Radiance HDR";
    if !craft_hdr::sniff(bytes) {
        return Err(DecodeError::WrongFormat(NAME));
    }
    let mut hdr_limits = craft_hdr::Limits::default();
    // The picture holds 12 bytes a pixel, less than the 16 of the RGBA plane made from it.
    hdr_limits.max_pixels = limits.max_pixels.min(limits.max_bytes / 16);
    hdr_limits.max_axis = limits.max_side;
    let pic = craft_hdr::decode(bytes, &hdr_limits).map_err(|e| match e {
        craft_hdr::Error::NotHdr => DecodeError::WrongFormat(NAME),
        craft_hdr::Error::Malformed(m) => DecodeError::Malformed { format: NAME, reason: m.into() },
        craft_hdr::Error::Unsupported(m) => DecodeError::Unsupported { format: NAME, reason: m },
        craft_hdr::Error::LimitExceeded(m) => DecodeError::LimitExceeded { format: NAME, reason: m.into() },
    })?;
    let (w, h) = (pic.width, pic.height);
    let len = limits.check(w, h, Kind::Color)?;
    let mut data = reserve(len, w, h)?;
    for &[r, g, b] in pic.rgb.as_chunks::<3>().0 {
        data.extend([r, g, b, 1.0]);
    }
    let image = Image::from_data(w, h, Kind::Color, data, limits)?;
    Ok(Decoded { image, transfer: Transfer::Linear, alpha: Alpha::Straight })
}

/// Decodes the color channels of an OpenEXR file: the part and the channel group that
/// `craft_exr::decode::pick_part` and `craft_exr::channels::color_channels` choose (R, G, B and A,
/// or Y and A, sharing one layer prefix), over the part's data window. A file of exactly one
/// channel, whatever its name (a height or depth map), gives a gray plane. Half and float samples
/// are kept as stored. The result is [`Alpha::Premultiplied`], as OpenEXR stores color. Files
/// with a deep part (even when the color part is flat), subsampled channels, 32-bit unsigned
/// integer color channels and files of several channels without a color group are
/// [`DecodeError::Unsupported`]. The plane's size is checked before any pixel is decoded, and the
/// samples the file declares, every channel of every part at 4 bytes each, must also fit in
/// [`Limits::max_bytes`].
#[cfg(feature = "exr")]
pub fn exr(bytes: &[u8], limits: &Limits) -> Result<Decoded, DecodeError> {
    use craft_exr::channels::{ColorLayout, color_channels};
    use craft_exr::decode::{self as exr_decode, Error, SampleType};

    const NAME: &str = "OpenEXR";
    if bytes.get(..4) != Some(&[0x76, 0x2f, 0x31, 0x01][..]) {
        return Err(DecodeError::WrongFormat(NAME));
    }
    let map = |e: Error| match e {
        Error::Unsupported(m) => DecodeError::Unsupported { format: NAME, reason: m },
        Error::LimitExceeded(m) => DecodeError::LimitExceeded { format: NAME, reason: m },
        Error::Malformed(m) | Error::Encode(m) => DecodeError::Malformed { format: NAME, reason: m },
    };
    let mut exr_limits = exr_decode::Limits::default();
    exr_limits.max_width = limits.max_side;
    exr_limits.max_height = limits.max_side;
    exr_limits.max_pixels = limits.max_pixels;
    // The decoder counts every channel of every part at 4 bytes a sample, which also covers the
    // all-channel blocks it decompresses before the unused channels are dropped.
    exr_limits.max_bytes = limits.max_bytes;
    let parts = exr_decode::parts(bytes, &exr_limits).map_err(map)?;
    let unsupported = |reason: &str| DecodeError::Unsupported { format: NAME, reason: reason.into() };
    let info = exr_decode::pick_part(&parts).and_then(|i| parts.get(i)).ok_or_else(|| unsupported("no part has color channels"))?;
    if parts.iter().any(|p| p.deep) {
        return Err(unsupported("deep data"));
    }
    let (layout, picked) = color_channels(info.channels.iter().map(|c| c.name.as_str())).ok_or_else(|| unsupported("no color channels"))?;
    let mut names = Vec::with_capacity(picked.len());
    for i in picked {
        let c = info.channels.get(i).ok_or_else(|| unsupported("no color channels"))?;
        if c.sample == SampleType::U32 {
            return Err(unsupported("a color channel holds 32-bit unsigned integers"));
        }
        names.push(c.name.as_str());
    }
    let kind = if layout == ColorLayout::Y { Kind::Gray } else { Kind::Color };
    let (w, h) = (info.data_window.width, info.data_window.height);
    let len = limits.check(w, h, kind)?;
    let part = exr_decode::decode_part(bytes, info.index, &|n| names.contains(&n), &exr_limits).map_err(map)?;
    let mut planes = Vec::with_capacity(names.len());
    for n in &names {
        let c = part.channel(n).ok_or_else(|| DecodeError::Malformed { format: NAME, reason: format!("channel {n} was not read") })?;
        planes.push(&c.samples);
    }
    let pixels = len / kind.channels();
    if planes.iter().any(|p| p.len() != pixels) {
        return Err(DecodeError::Malformed { format: NAME, reason: "a channel does not cover the data window".into() });
    }
    let mut data = reserve(len, w, h)?;
    let at = |c: usize, i: usize| planes.get(c).map_or(0.0, |p| p.get_f32(i));
    for i in 0..pixels {
        match layout {
            ColorLayout::Y => data.push(at(0, i)),
            ColorLayout::YA => {
                let y = at(0, i);
                data.extend([y, y, y, at(1, i)]);
            }
            ColorLayout::Rgb => data.extend([at(0, i), at(1, i), at(2, i), 1.0]),
            ColorLayout::Rgba => data.extend([at(0, i), at(1, i), at(2, i), at(3, i)]),
        }
    }
    let image = Image::from_data(w, h, kind, data, limits)?;
    Ok(Decoded { image, transfer: Transfer::Linear, alpha: Alpha::Premultiplied })
}

#[cfg(all(test, any(feature = "png", feature = "tiff")))]
mod tests {
    use super::*;

    #[test]
    fn an_icc_profile_makes_the_transfer_unknown_whatever_the_cicp() {
        let Ok(mut img) = craft_image_codecs::Image::from_u16(1, 1, ChannelLayout::Gray, &[7]) else { panic!("valid 1 x 1 image") };
        assert_eq!(codec_transfer(&img), Transfer::Encoded);
        img.cicp = Some(Cicp { primaries: 1, transfer: Cicp::TRANSFER_LINEAR, matrix: 0, full_range: true });
        assert_eq!(codec_transfer(&img), Transfer::Linear);
        img.icc = Some(vec![0; 128]);
        assert_eq!(codec_transfer(&img), Transfer::Unknown);
        img.cicp = Some(Cicp { primaries: 1, transfer: 13, matrix: 0, full_range: true });
        assert_eq!(codec_transfer(&img), Transfer::Unknown);
        img.icc = None;
        assert_eq!(codec_transfer(&img), Transfer::Encoded);
        img.cicp = Some(Cicp { primaries: 9, transfer: Cicp::TRANSFER_PQ, matrix: 0, full_range: true });
        assert_eq!(codec_transfer(&img), Transfer::Unknown);
    }
}

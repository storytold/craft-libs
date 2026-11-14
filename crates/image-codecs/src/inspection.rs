//! Bounded source-header and tone inspection for raster documents.

use crate::error::CodecError;
use crate::{ChannelLayout, DecodeOptions, Format, Limits, SampleType, decode_with};

/// Return an embedded ICC profile using the selected format decoder.
///
/// This fully decodes the image; callers that already need the pixels should read `Image::meta.icc`
/// from that decode instead of invoking this helper separately.
pub fn embedded_icc(bytes: &[u8]) -> Result<Option<Vec<u8>>, CodecError> {
    Ok(decode_with(bytes, &DecodeOptions::default())?.icc)
}

/// The source pixel model reported by an image container, before decode normalizes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterColorType {
    Bitmap,
    Gray,
    GrayAlpha,
    Indexed,
    Rgb,
    Rgba,
    Cmyk,
    CmykAlpha,
    Lab,
    Other,
}

/// Useful source facts, including palette entries when an indexed container exposes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterInspection {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub color_type: RasterColorType,
    pub palette: Option<Vec<[u8; 3]>>,
    pub transparent_index: Option<u8>,
}

/// A grayscale source suitable for single-swatch tinting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneClass {
    Bitmap,
    Gray,
}

/// Inspect PSD, PNG, JPEG, TIFF, GIF and BMP source headers without expanding pixel data.
pub fn inspect_raster(bytes: &[u8]) -> Result<Option<RasterInspection>, CodecError> {
    if bytes.starts_with(b"8BPS") {
        if bytes.len() < 26 || !matches!(u16::from_be_bytes([bytes[4], bytes[5]]), 1 | 2) {
            return Ok(None);
        }
        let u16_at = |i| u16::from_be_bytes([bytes[i], bytes[i + 1]]);
        let height = u32::from_be_bytes(bytes[14..18].try_into().map_err(|_| CodecError::InvalidImage("short PSD header".into()))?);
        let width = u32::from_be_bytes(bytes[18..22].try_into().map_err(|_| CodecError::InvalidImage("short PSD header".into()))?);
        let color_type = match u16_at(24) {
            0 => RasterColorType::Bitmap,
            1 => RasterColorType::Gray,
            2 => RasterColorType::Indexed,
            3 => RasterColorType::Rgb,
            4 => RasterColorType::Cmyk,
            9 => RasterColorType::Lab,
            _ => RasterColorType::Other,
        };
        return Ok(Some(RasterInspection {
            width,
            height,
            channels: u16_at(12).min(255) as u8,
            bits_per_sample: u16_at(22).min(255) as u8,
            color_type,
            palette: None,
            transparent_index: None,
        }));
    }
    let Some(format) = crate::detect(bytes) else { return Ok(None) };
    let i = match format {
        Format::Png => {
            let rows = crate::PngDecodeRows::new(bytes, &Limits::default())?;
            let h = rows.header();
            let depth = *bytes.get(24).ok_or_else(|| CodecError::malformed(format, "short PNG IHDR"))?;
            let ct = *bytes.get(25).ok_or_else(|| CodecError::malformed(format, "short PNG IHDR"))?;
            let color = match (ct, depth) {
                (0, 1) => RasterColorType::Bitmap,
                (0, _) => RasterColorType::Gray,
                (2, _) => RasterColorType::Rgb,
                (3, _) => RasterColorType::Indexed,
                (4, _) => RasterColorType::GrayAlpha,
                (6, _) => RasterColorType::Rgba,
                _ => RasterColorType::Other,
            };
            let palette = crate::png_palette(bytes)?.map(|p| (p.colors, p.transparent));
            let (palette, transparent_index) = palette.map_or((None, None), |(c, t)| (Some(c), t));
            RasterInspection {
                width: h.width,
                height: h.height,
                channels: match ct {
                    0 | 3 => 1,
                    2 => 3,
                    4 => 2,
                    6 => 4,
                    _ => h.layout.channels().min(255) as u8,
                },
                bits_per_sample: depth,
                color_type: color,
                palette,
                transparent_index,
            }
        }
        Format::Tiff => {
            let info = crate::tiff_info(bytes)?;
            let Some(p) = info.default_page().and_then(|n| info.pages.get(n)) else { return Ok(None) };
            let color = match (p.photometric, p.bits_per_sample) {
                (Some(0 | 1), 1) => RasterColorType::Bitmap,
                (Some(0 | 1), _) => RasterColorType::Gray,
                (Some(2 | 6), _) => RasterColorType::Rgb,
                (Some(3), _) => RasterColorType::Indexed,
                (Some(5), _) => RasterColorType::Cmyk,
                (Some(8), _) => RasterColorType::Lab,
                _ => RasterColorType::Other,
            };
            RasterInspection {
                width: p.width,
                height: p.height,
                channels: p.samples_per_pixel.min(255) as u8,
                bits_per_sample: p.bits_per_sample.min(255) as u8,
                color_type: color,
                palette: None,
                transparent_index: None,
            }
        }
        Format::Jpeg => {
            let Some((w, h, channels)) = crate::codecs::jpeg::scan_metadata(bytes).frame else { return Ok(None) };
            let depth = craft_jpeg::frame(bytes).map_or(8, |f| f.precision);
            let color = match channels {
                1 => RasterColorType::Gray,
                3 => RasterColorType::Rgb,
                4 => RasterColorType::Cmyk,
                _ => RasterColorType::Other,
            };
            RasterInspection { width: w, height: h, channels, bits_per_sample: depth, color_type: color, palette: None, transparent_index: None }
        }
        Format::Bmp | Format::Gif => {
            use image::ImageDecoder;
            let d = crate::codecs::via_image::open(format, bytes, &Limits::default())?;
            let (width, height) = d.dimensions();
            let (channels, bits, color) = match d.original_color_type() {
                image::ExtendedColorType::L1 => (1, 1, RasterColorType::Bitmap),
                image::ExtendedColorType::L2 => (1, 2, RasterColorType::Gray),
                image::ExtendedColorType::L4 => (1, 4, RasterColorType::Gray),
                image::ExtendedColorType::L8 => (1, 8, RasterColorType::Gray),
                image::ExtendedColorType::L16 => (1, 16, RasterColorType::Gray),
                image::ExtendedColorType::La8 => (2, 8, RasterColorType::GrayAlpha),
                image::ExtendedColorType::La16 => (2, 16, RasterColorType::GrayAlpha),
                image::ExtendedColorType::Rgb8 => (3, 8, RasterColorType::Rgb),
                image::ExtendedColorType::Rgb16 => (3, 16, RasterColorType::Rgb),
                image::ExtendedColorType::Rgba8 => (4, 8, RasterColorType::Rgba),
                image::ExtendedColorType::Rgba16 => (4, 16, RasterColorType::Rgba),
                _ => (4, 8, RasterColorType::Other),
            };
            let (palette, transparent_index) = if format == Format::Gif {
                crate::gif_palette(bytes).map_or((None, None), |(p, t)| (Some(p), t))
            } else {
                (crate::bmp_palette(bytes)?, None)
            };
            let (channels, color) = if format == Format::Gif { (1, RasterColorType::Indexed) } else { (channels, color) };
            RasterInspection { width, height, channels, bits_per_sample: bits, color_type: color, palette, transparent_index }
        }
        _ => return Ok(None),
    };
    Ok(Some(i))
}

/// Classify grayscale sources for single-swatch tinting. GIF checks visible decoded pixels and
/// exposes its source palette through [`inspect_raster`]; all other formats use source headers.
pub fn classify_tone(bytes: &[u8]) -> Option<ToneClass> {
    let i = inspect_raster(bytes).ok()??;
    if i.color_type == RasterColorType::Indexed && bytes.starts_with(b"GIF8") {
        i.palette.as_ref()?;
        let decoded = decode_with(bytes, &DecodeOptions::default()).ok()?;
        let rgba = decoded.convert(ChannelLayout::Rgba, SampleType::U8);
        let mut opaque = true;
        let mut two_level = true;
        for p in rgba.data().as_chunks::<4>().0 {
            match p[3] {
                0 => opaque = false,
                255 if p[0] == p[1] && p[1] == p[2] => two_level &= p[0] == 0 || p[0] == 255,
                _ => return None,
            }
        }
        return if two_level {
            Some(ToneClass::Bitmap)
        } else if opaque {
            Some(ToneClass::Gray)
        } else {
            None
        };
    }
    match i.color_type {
        RasterColorType::Bitmap => Some(ToneClass::Bitmap),
        RasterColorType::Gray => Some(ToneClass::Gray),
        _ => None,
    }
}

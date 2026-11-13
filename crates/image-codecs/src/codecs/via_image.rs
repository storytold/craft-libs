//! Formats delegated to the `image` crate (pure-Rust features only):
//! GIF, BMP, TGA, ICO, QOI, Radiance HDR, and AVIF encode (feature `avif`). Targa is written by
//! [`super::tga`] and only read here.

use std::io::{Cursor, Read};

use image::{DynamicImage, ImageDecoder, ImageFormat};

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, SampleType};
use crate::options::{EncodeOptions, Limits};

fn image_format(f: Format) -> Option<ImageFormat> {
    Some(match f {
        Format::Gif => ImageFormat::Gif,
        Format::Bmp => ImageFormat::Bmp,
        Format::Tga => ImageFormat::Tga,
        Format::Ico => ImageFormat::Ico,
        Format::Qoi => ImageFormat::Qoi,
        Format::Hdr => ImageFormat::Hdr,
        Format::Avif => ImageFormat::Avif,
        _ => return None,
    })
}

fn map_err(f: Format, e: image::ImageError) -> CodecError {
    match e {
        image::ImageError::Limits(l) => CodecError::LimitExceeded(l.to_string()),
        image::ImageError::Unsupported(u) => CodecError::unsupported(f, u.to_string()),
        e => CodecError::malformed(f, e),
    }
}

pub(crate) fn bmp_palette(bytes: &[u8]) -> Result<Option<Vec<[u8; 3]>>, CodecError> {
    let decoder = image::codecs::bmp::BmpDecoder::new(Cursor::new(bytes)).map_err(|e| map_err(Format::Bmp, e))?;
    let mut palette = decoder.get_palette().map(|palette| palette.to_vec());
    // The decoder always hands back 256 entries (it pads a short table with black to survive
    // corrupt files); the file's own count is what the document's colour table should hold.
    if let (Some(p), Some(n)) = (palette.as_mut(), bmp_palette_len(bytes)) {
        p.truncate(n.max(1));
    }
    Ok(palette)
}

/// How many colours a palette BMP's header declares: `biClrUsed`, or 2^bits when it is 0.
/// `None` for a header this does not recognise, which leaves the table as decoded.
fn bmp_palette_len(bytes: &[u8]) -> Option<usize> {
    let u16_at = |at: usize| bytes.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    let u32_at = |at: usize| bytes.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let (bit_count, used) = match u32_at(14)? {
        // OS/2 BITMAPCOREHEADER: no colour count, the bit depth sets the table size.
        12 => (u16_at(24)?, 0),
        n if n >= 40 => (u16_at(28)?, u32_at(46)?),
        _ => return None,
    };
    if !(1..=8).contains(&bit_count) {
        return None;
    }
    let max = 1usize << bit_count;
    Some(if used == 0 { max } else { (used as usize).min(max) })
}

/// Writes an 8-bit palette BMP: `indices` row-major, `palette` RGB entries (1..=256). The
/// counterpart of [`bmp_palette`], for Indexed Color documents.
pub(crate) fn encode_bmp_indexed(width: u32, height: u32, indices: &[u8], palette: &[[u8; 3]]) -> Result<Vec<u8>, CodecError> {
    let f = Format::Bmp;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) != indices.len() as u64 {
        return Err(CodecError::InvalidImage("index data does not match the size".into()));
    }
    if palette.is_empty() || palette.len() > 256 {
        return Err(CodecError::InvalidImage("a palette BMP holds 1 to 256 colours".into()));
    }
    if indices.iter().any(|&i| usize::from(i) >= palette.len()) {
        return Err(CodecError::InvalidImage("palette index out of range".into()));
    }
    let mut out = Cursor::new(Vec::new());
    image::codecs::bmp::BmpEncoder::new(&mut out)
        .encode_with_palette(indices, width, height, image::ExtendedColorType::L8, Some(palette))
        .map_err(|e| CodecError::encode(f, e))?;
    Ok(out.into_inner())
}

/// The `image` decoder for `f` with our limits, its header read and its size checked.
fn open<'a>(f: Format, bytes: &'a [u8], limits: &Limits) -> Result<impl ImageDecoder + 'a, CodecError> {
    let fmt = image_format(f).ok_or_else(|| CodecError::unsupported(f, "not handled by image backend"))?;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), fmt);
    let mut il = image::Limits::default();
    il.max_image_width = Some(limits.max_width);
    il.max_image_height = Some(limits.max_height);
    il.max_alloc = Some(limits.max_alloc);
    reader.limits(il);
    if f == Format::Ico {
        // The ICO decoder builds its embedded PNG decoder without limits, which inflates an
        // iCCP profile up to usize::MAX while the reader is constructed, before `il` applies.
        ico_png_profiles_fit(bytes, limits)?;
    }
    let dec = reader.into_decoder().map_err(|e| map_err(f, e))?;
    let (w, h) = dec.dimensions();
    let bpp = dec.color_type().bytes_per_pixel() as u64;
    limits.check_bytes(w, h, bpp)?;
    // `DynamicImage::from_decoder` allocates the whole image before reading a pixel, so a
    // header-only file must not be able to ask for gigabytes (#8597). GIF's LZW tops out near
    // 11000:1 of RGBA output and TGA's and QOI's runs far below; AVIF has already decoded by now.
    // Radiance HDR's old-style RLE repeat markers multiply their counts by 256 each in a row, so
    // no ratio bounds a valid file: only the allocation limit above applies to it.
    let declared = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(bpp);
    match f {
        Format::Avif | Format::Hdr => {}
        Format::Gif => gif_plausible(bytes, w, h, bpp)?,
        // A BMP's RLE escapes (end of line, end of bitmap, delta) leave pixels unwritten, so no
        // ratio bounds a valid file either. The decoder still allocates and clears the whole
        // canvas, so like a GIF's screen only `UNBACKED_CANVAS` of it is free of data.
        Format::Bmp if bmp_is_rle(bytes) => Limits::check_plausible(declared.saturating_sub(UNBACKED_CANVAS), bytes.len(), 1 << 14)?,
        _ => Limits::check_plausible(declared, bytes.len(), 1 << 14)?,
    }
    Ok(dec)
}

/// Canvas bytes a GIF's logical screen or a sparse RLE BMP may declare without data behind
/// them (256 MiB: an 8192x8192 RGBA screen). Past this, the file must back the rest.
const UNBACKED_CANVAS: u64 = 1 << 28;

/// Whether a GIF whose `w`x`h` logical screen decodes to `bpp` bytes a pixel is plausible for
/// its file size. The first frame is what the file must hold: the logical screen around it needs
/// no data, so up to [`UNBACKED_CANVAS`] of screen is free and only the rest must be backed.
/// Shared by the first-frame decode and the animation decode ([`super::gif_frames`]), whose
/// backend also initializes the whole screen before reading the first frame.
pub(crate) fn gif_plausible(bytes: &[u8], w: u32, h: u32, bpp: u64) -> Result<(), CodecError> {
    let declared = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(bpp);
    let declared = match gif_first_frame(bytes) {
        Some((fw, fh)) => u64::from(fw).saturating_mul(u64::from(fh)).saturating_mul(bpp).max(declared.saturating_sub(UNBACKED_CANVAS)),
        None => declared,
    };
    Limits::check_plausible(declared, bytes.len(), 1 << 14)
}

/// Most bytes one embedded iCCP profile of an icon may inflate to: 255 APP2 segments of a JPEG,
/// the largest profile any of our formats carries, and far past any real one.
const MAX_ICO_ICC_BYTES: u64 = 255 * 65_519;
/// Most bytes all of an icon's embedded iCCP profiles may inflate to together while checked.
/// Entries may point into overlapping chunk chains, so this bounds the check's own work.
const MAX_ICO_ICC_TOTAL: u64 = 64 << 20;

/// Refuses an icon whose embedded PNG images carry an iCCP profile larger than the profile cap
/// (or `max_alloc`). The `image` crate's ICO decoder opens the PNG with no limits, and the `png`
/// crate inflates the profile while the decoder is built: a few kilobytes of zlib would ask for
/// gigabytes. Each profile is inflated here into a sink, so the check itself allocates nothing.
fn ico_png_profiles_fit(b: &[u8], limits: &Limits) -> Result<(), CodecError> {
    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let u16_at = |at: usize| b.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    let u32_at = |at: usize| b.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let be32 = |at: usize| b.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
    let cap = MAX_ICO_ICC_BYTES.min(limits.max_alloc);
    let Some(count) = u16_at(4) else { return Ok(()) };
    let mut offsets: Vec<usize> = (0..usize::from(count)).filter_map(|i| u32_at(6 + 16 * i + 12)).filter_map(|o| usize::try_from(o).ok()).collect();
    offsets.sort_unstable();
    offsets.dedup();
    let mut total = 0u64;
    // Chunk offsets already walked by any start: a walk from here repeats the earlier one, so stop.
    let mut visited = std::collections::HashSet::new();
    for start in offsets {
        if b.get(start..start.saturating_add(8)) != Some(&PNG_SIGNATURE[..]) {
            continue;
        }
        // Chunks up to the image data: the png crate parses iCCP only before IDAT.
        let mut at = start + 8;
        while let (Some(len), Some(kind)) = (be32(at), b.get(at.saturating_add(4)..at.saturating_add(8))) {
            if !visited.insert(at) {
                break;
            }
            let data_at = at + 8;
            let Some(data) = usize::try_from(len).ok().and_then(|n| b.get(data_at..data_at.checked_add(n)?)) else { break };
            match kind {
                b"IDAT" | b"IEND" => break,
                b"iCCP" => {
                    // Profile name, NUL, compression method, then the zlib stream.
                    let Some(z) = data.iter().position(|&c| c == 0).and_then(|n| data.get(n + 2..)) else { break };
                    // Count what inflates before a corrupt stream fails too: the png crate
                    // allocates that much before it finds the error.
                    let mut inflated = flate2::read::ZlibDecoder::new(z).take(cap.saturating_add(1));
                    let (mut n, mut buf) = (0u64, [0u8; 8192]);
                    while let Ok(got @ 1..) = inflated.read(&mut buf) {
                        n += got as u64;
                    }
                    total = total.saturating_add(n);
                    if n > cap || total > MAX_ICO_ICC_TOTAL {
                        return Err(CodecError::LimitExceeded(format!("an icon's embedded colour profile inflates past {cap} bytes")));
                    }
                    // The png crate refuses a second iCCP chunk without inflating it.
                    break;
                }
                _ => {}
            }
            at = data_at + data.len() + 4;
        }
    }
    Ok(())
}

pub(crate) fn decode(f: Format, bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let mut dec = open(f, bytes, limits)?;
    let icc = dec.icc_profile().ok().flatten();
    let exif = dec.exif_metadata().ok().flatten();
    let dynimg = DynamicImage::from_decoder(dec).map_err(|e| map_err(f, e))?;
    let mut img = from_dynamic(f, dynimg)?;
    img.icc = icc;
    img.meta.exif = exif;
    if f == Format::Gif {
        img.warnings.extend(gif_more_frames(bytes));
    }
    Ok(img)
}

/// The stored size and EXIF block [`decode`] would find, from the decoder's header alone: the
/// same reader, limits and size check, without decoding pixels. AVIF is refused: its decoder
/// decodes the image as it opens.
pub(crate) fn probe(f: Format, bytes: &[u8], limits: &Limits) -> Result<(u32, u32, Option<Vec<u8>>), CodecError> {
    if f == Format::Avif {
        return Err(CodecError::unsupported(f, "no header-only size probe"));
    }
    let mut dec = open(f, bytes, limits)?;
    let (w, h) = dec.dimensions();
    Ok((w, h, dec.exif_metadata().ok().flatten()))
}

/// Whether a BMP's `biCompression` is RLE8 or RLE4 (only headers of 40 bytes or more have one).
fn bmp_is_rle(b: &[u8]) -> bool {
    let u32_at = |at: usize| b.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    u32_at(14).is_some_and(|n| n >= 40) && matches!(u32_at(30), Some(1 | 2))
}

/// The width and height of a GIF's first image descriptor, if the blocks reach one.
fn gif_first_frame(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = None;
    gif_walk(b, |i| {
        at = Some(i);
        false
    });
    let u16_at = |at: usize| b.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    // Descriptor: separator, left, top, width, height (little-endian u16s), packed fields.
    let i = at?;
    Some((u32::from(u16_at(i + 5)?), u32::from(u16_at(i + 7)?)))
}

/// Counts a GIF's frames (image descriptors) by walking its blocks, without decompressing any.
/// The total is unknown when the blocks end before the trailer or break off.
pub(crate) fn gif_more_frames(b: &[u8]) -> Option<DecodeWarning> {
    let (frames, total) = gif_walk(b, |_| true);
    (frames > 1).then_some(DecodeWarning::MoreFrames { total })
}

/// Whether a GIF holds more than one frame, or None when the walk stops before the trailer
/// after at most one frame (so frames past the break could not be counted).
pub(crate) fn gif_has_more_frames(b: &[u8]) -> Option<bool> {
    let (frames, total) = gif_walk(b, |_| true);
    if frames > 1 { Some(true) } else { total.map(|_| false) }
}

/// Walks a GIF's blocks without decompressing any, handing `frame` the offset of each image
/// descriptor until it returns false. Gives the descriptors seen, and the frame count if the
/// blocks reached the trailer.
fn gif_walk(b: &[u8], mut frame: impl FnMut(usize) -> bool) -> (u32, Option<u32>) {
    // Size of the colour table a packed-fields byte announces.
    let table = |flags: u8| if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    // Skips the data sub-blocks starting at `i` (each length-prefixed, ended by a zero length).
    let skip = |mut i: usize| -> Option<usize> {
        loop {
            let n = usize::from(*b.get(i)?);
            i = i.checked_add(1 + n)?;
            if n == 0 {
                return Some(i);
            }
        }
    };
    // Header (6 bytes), logical screen descriptor (7) and the global colour table.
    let Some(&flags) = b.get(10) else {
        return (0, None);
    };
    let mut i = 13 + table(flags);
    let mut frames = 0u32;
    let total = loop {
        let next = match b.get(i) {
            Some(0x3B) => break Some(frames),
            Some(0x21) => skip(i + 2),
            Some(0x2C) => {
                frames = frames.saturating_add(1);
                if !frame(i) {
                    break None;
                }
                // Descriptor (10 bytes, packed fields last), local colour table, LZW code size.
                b.get(i + 9).and_then(|&flags| skip(i + 11 + table(flags)))
            }
            _ => None,
        };
        match next {
            Some(n) => i = n,
            None => break None,
        }
    };
    (frames, total)
}

pub(crate) fn from_dynamic(f: Format, d: DynamicImage) -> Result<Image, CodecError> {
    let (w, h) = (d.width(), d.height());
    Ok(match d {
        DynamicImage::ImageLuma8(b) => Image::from_u8(w, h, ChannelLayout::Gray, b.into_raw())?,
        DynamicImage::ImageLumaA8(b) => Image::from_u8(w, h, ChannelLayout::GrayA, b.into_raw())?,
        DynamicImage::ImageRgb8(b) => Image::from_u8(w, h, ChannelLayout::Rgb, b.into_raw())?,
        DynamicImage::ImageRgba8(b) => Image::from_u8(w, h, ChannelLayout::Rgba, b.into_raw())?,
        DynamicImage::ImageLuma16(b) => Image::from_u16(w, h, ChannelLayout::Gray, &b.into_raw())?,
        DynamicImage::ImageLumaA16(b) => Image::from_u16(w, h, ChannelLayout::GrayA, &b.into_raw())?,
        DynamicImage::ImageRgb16(b) => Image::from_u16(w, h, ChannelLayout::Rgb, &b.into_raw())?,
        DynamicImage::ImageRgba16(b) => Image::from_u16(w, h, ChannelLayout::Rgba, &b.into_raw())?,
        DynamicImage::ImageRgb32F(b) => Image::from_f32(w, h, ChannelLayout::Rgb, &b.into_raw())?,
        DynamicImage::ImageRgba32F(b) => Image::from_f32(w, h, ChannelLayout::Rgba, &b.into_raw())?,
        other => {
            let b = other.into_rgba32f();
            Image::from_f32(w, h, ChannelLayout::Rgba, &b.into_raw()).map_err(|e| CodecError::malformed(f, e))?
        }
    })
}

fn to_dynamic(img: &Image) -> Result<DynamicImage, CodecError> {
    let (w, h) = img.dimensions();
    let bad = || CodecError::InvalidImage("buffer size mismatch".into());
    Ok(match (img.layout(), img.sample_type()) {
        (ChannelLayout::Gray, SampleType::U8) => DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, img.data().to_vec()).ok_or_else(bad)?),
        (ChannelLayout::GrayA, SampleType::U8) => DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(w, h, img.data().to_vec()).ok_or_else(bad)?),
        (ChannelLayout::Rgb, SampleType::U8) => DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, img.data().to_vec()).ok_or_else(bad)?),
        (ChannelLayout::Rgba, SampleType::U8) => DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, img.data().to_vec()).ok_or_else(bad)?),
        (ChannelLayout::Rgb, SampleType::F32) => {
            DynamicImage::ImageRgb32F(image::Rgb32FImage::from_raw(w, h, img.to_f32_samples().unwrap_or_default()).ok_or_else(bad)?)
        }
        (ChannelLayout::Rgba, SampleType::F32) => {
            DynamicImage::ImageRgba32F(image::Rgba32FImage::from_raw(w, h, img.to_f32_samples().unwrap_or_default()).ok_or_else(bad)?)
        }
        (l, s) => {
            return Err(CodecError::InvalidImage(format!("no image-crate mapping for {l:?} {s:?}")));
        }
    })
}

pub(crate) fn encode(f: Format, src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let fmt = image_format(f).ok_or_else(|| CodecError::unsupported(f, "not handled by image backend"))?;
    if let Some((mw, mh)) = f.max_dimensions()
        && (src.width() > mw || src.height() > mh)
    {
        return Err(CodecError::encode(f, format!("dimensions exceed {mw}x{mh}")));
    }
    let img = src.converted(plan.layout, plan.sample);
    // Targa is written by our encoder: the image crate's RLE packets cross scanlines (#6382).
    if f == Format::Tga {
        return super::tga::encode(&img);
    }
    let dynimg = to_dynamic(&img)?;
    let mut cursor = Cursor::new(Vec::new());
    match f {
        #[cfg(feature = "avif")]
        Format::Avif => {
            let enc = image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut cursor, 8, opts.jpeg_quality.clamp(1, 100));
            dynimg.write_with_encoder(enc).map_err(|e| CodecError::encode(f, e))?;
        }
        _ => {
            let _ = opts;
            dynimg.write_to(&mut cursor, fmt).map_err(|e| CodecError::encode(f, e))?;
        }
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod indexed_tests {
    use super::*;

    #[test]
    fn palette_bmp_roundtrips_its_table_and_pixels() {
        // 3 px wide: rows are padded to 4 bytes, so a wrong stride shows in the second row.
        let pal = [[255, 0, 0], [0, 0, 255], [10, 20, 30]];
        let idx = [0u8, 1, 2, 2, 1, 0];
        let bytes = encode_bmp_indexed(3, 2, &idx, &pal).unwrap();
        assert_eq!(bmp_palette(&bytes).unwrap().as_deref(), Some(&pal[..]), "the table survives, not a 256-entry grey ramp");
        let img = decode(Format::Bmp, &bytes, &Limits::default()).unwrap();
        assert_eq!(img.dimensions(), (3, 2));
        let rgb = img.convert(ChannelLayout::Rgb, SampleType::U8);
        let want: Vec<u8> = idx.iter().flat_map(|&i| pal[usize::from(i)]).collect();
        assert_eq!(rgb.data(), want.as_slice());
    }

    #[test]
    fn palette_len_follows_the_header() {
        let mut bytes = encode_bmp_indexed(2, 1, &[0, 1], &[[1, 2, 3], [4, 5, 6]]).unwrap();
        assert_eq!(bmp_palette_len(&bytes), Some(2));
        // biClrUsed = 0 means the full 2^bits table.
        bytes[46..50].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(bmp_palette_len(&bytes), Some(256));
        // A short or unknown header leaves the decoded table alone instead of panicking.
        assert_eq!(bmp_palette_len(&bytes[..30]), None);
        bytes[14..18].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(bmp_palette_len(&bytes), None);
    }

    /// A 64x48 icon holding one PNG with an iCCP chunk of `profile` bytes (zeros).
    fn icon_with_profile(profile: usize) -> Vec<u8> {
        use std::io::Write;
        let img = Image::from_u8(64, 48, ChannelLayout::Rgba, vec![128; 64 * 48 * 4]).unwrap();
        let png = crate::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&vec![0u8; profile]).unwrap();
        let mut data = b"icc\0\0".to_vec();
        data.extend_from_slice(&z.finish().unwrap());
        let mut crc = flate2::Crc::new();
        crc.update(b"iCCP");
        crc.update(&data);
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"iCCP");
        chunk.extend_from_slice(&data);
        chunk.extend_from_slice(&crc.sum().to_be_bytes());
        // Signature (8) and IHDR (25), then the profile.
        let mut png_icc = png[..33].to_vec();
        png_icc.extend_from_slice(&chunk);
        png_icc.extend_from_slice(&png[33..]);
        let mut ico = vec![0, 0, 1, 0, 1, 0, 64, 48, 0, 0, 1, 0, 32, 0];
        ico.extend_from_slice(&(png_icc.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&png_icc);
        ico
    }

    #[test]
    fn an_icon_profile_inflating_past_the_cap_is_refused_before_the_png_opens() {
        let small = icon_with_profile(4096);
        let img = decode(Format::Ico, &small, &Limits::default()).unwrap();
        assert_eq!(img.dimensions(), (64, 48));
        let huge = icon_with_profile(MAX_ICO_ICC_BYTES as usize + 1);
        assert!(huge.len() < 64 << 10, "{} bytes", huge.len());
        for r in [decode(Format::Ico, &huge, &Limits::default()).map(|_| ()), probe(Format::Ico, &huge, &Limits::default()).map(|_| ())] {
            assert!(matches!(&r, Err(CodecError::LimitExceeded(m)) if m.contains("colour profile")), "{r:?}");
        }
        // max_alloc lowers the cap.
        let tight = Limits { max_alloc: 2048, ..Limits::default() };
        assert!(matches!(decode(Format::Ico, &small, &tight), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn palette_bmp_rejects_bad_input() {
        let pal = [[0, 0, 0], [255, 255, 255]];
        assert!(encode_bmp_indexed(3, 2, &[0; 5], &pal).is_err(), "size mismatch");
        assert!(encode_bmp_indexed(0, 2, &[], &pal).is_err(), "empty");
        assert!(encode_bmp_indexed(2, 1, &[0, 2], &pal).is_err(), "index past the table");
        assert!(encode_bmp_indexed(1, 1, &[0], &[]).is_err(), "no colours");
        assert!(encode_bmp_indexed(1, 1, &[0], &[[0; 3]; 257]).is_err(), "more than 256 colours");
    }
}

//! Device-independent bitmaps (the BLIP DIB: a `BITMAPINFOHEADER`, its colour table and pixels,
//! without a file header) as BMP and PNG files.

use std::io::Write;

use image::ImageEncoder;

use crate::blip::Decode;
use crate::record::{u16_at, u32_at};

/// Largest picture converted to PNG, per side (pixels).
pub(crate) const MAX_SIDE: u32 = 16_384;

/// A device-independent bitmap with the 14-byte file header a `.bmp` needs in front. `None` for a
/// header size no Windows version writes (12 to 124 bytes), or pixels that would start past its
/// end.
pub fn to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let header = u32_at(dib, 0)? as usize;
    if !(12..=124).contains(&header) {
        return None;
    }
    let (bits, compression, used) = if header == 12 {
        (usize::from(u16_at(dib, 10)?), 0, 0)
    } else {
        (usize::from(u16_at(dib, 14)?), u32_at(dib, 16)?, u32_at(dib, 32)? as usize)
    };
    let entry = if header == 12 { 3 } else { 4 };
    // The colour table: up to 2^bits entries (all of them when biClrUsed is 0) for 8 bits or
    // fewer; biClrUsed entries, an optional table, for more.
    let colors = match bits {
        0..=8 if used > 0 => used.min(1 << bits),
        0..=8 => 1 << bits,
        _ => used,
    };
    // With the 40-byte header, BI_BITFIELDS keeps three masks after it, BI_ALPHABITFIELDS four.
    let masks = match (header, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let offset = 14usize.checked_add(header)?.checked_add(colors.checked_mul(entry)?)?.checked_add(masks)?;
    let file_len = 14usize.checked_add(dib.len())?;
    // Pixels starting past the end are no bitmap.
    if offset > file_len {
        return None;
    }
    let total = u32::try_from(file_len).ok()?;
    let mut out = Vec::with_capacity(file_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&u32::try_from(offset).ok()?.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// [`to_bmp`] within `limit` bytes and `work`: the whole BMP file (the DIB and its 14-byte header)
/// is charged before it is made. `None` when it doesn't fit, or isn't a bitmap.
pub fn to_bmp_within(dib: &[u8], limit: usize, work: &mut Decode) -> Option<Vec<u8>> {
    let len = 14usize.checked_add(dib.len())?;
    if len > limit || !work.charge(len, "bitmap") {
        return None;
    }
    to_bmp(dib)
}

/// A DIB as a picture file.
#[derive(Clone, Debug, PartialEq)]
pub enum Dib {
    /// Decoded and encoded as a PNG.
    Png(Vec<u8>),
    /// Kept as a BMP file ([`to_bmp`]), when it couldn't be decoded or its pixels didn't fit the
    /// decode budget: a placeholder of the right size.
    Bmp(Vec<u8>),
}

/// A DIB as a PNG: made a BMP file ([`to_bmp`]), decoded with size limits and re-encoded. When the
/// bitmap can't be decoded, or its declared pixels (4 bytes each) or the PNG don't fit in `work`,
/// the BMP bytes are kept. `None` when not even the BMP fits in `limit` or `work`. Each stage is
/// charged to `work` before it is done: the BMP copy, the decoded pixels, and the PNG as it is
/// written.
pub fn to_png(dib: &[u8], limit: usize, work: &mut Decode) -> Option<Dib> {
    let header = u32_at(dib, 0)? as usize;
    if !(12..=124).contains(&header) {
        return None;
    }
    // The declared size is what decoding allocates, however small the (RLE) data is.
    let (w, h) = if header == 12 {
        (u64::from(u16_at(dib, 4)?), u64::from(u16_at(dib, 6)?))
    } else {
        (u64::from(u32_at(dib, 4)?), u64::from((u32_at(dib, 8)? as i32).unsigned_abs()))
    };
    let cost = w.saturating_mul(h).saturating_mul(4).try_into().unwrap_or(usize::MAX);
    // The BMP copy first: a bitmap whose pixels don't fit still keeps its BMP bytes as a
    // placeholder, if the copy fits.
    let bmp = to_bmp_within(dib, limit, work)?;
    // A bitmap too large a side to be decoded is kept as a BMP without charging its pixels.
    if w > u64::from(MAX_SIDE) || h > u64::from(MAX_SIDE) || !work.charge(cost, "bitmap") {
        return Some(Dib::Bmp(bmp));
    }
    // The PNG may be at most what is left of `work` (and `limit`); what it writes is charged.
    match encode_png(&bmp, image::ImageFormat::Bmp, limit, work, cost) {
        Some(png) => Some(Dib::Png(png)),
        None => Some(Dib::Bmp(bmp)),
    }
}

/// A picture file of `format` decoded (at most [`MAX_SIDE`] pixels a side, and `limit` and
/// 256 MiB of memory) and encoded as a PNG of at most what is then left of `limit` and `work`.
/// The picture is turned as the file says (a TIFF's or JPEG's orientation), and floating-point
/// pixels, which PNG can't hold, become 16-bit ones. Each of those buffers is charged to `work`
/// before it is made: the decoded pixels, less the `prepaid` bytes the caller charged for them
/// already, a TIFF decoder's own buffer, a turned copy, the 16-bit copy, the big-endian copy the
/// PNG encoder makes of 16-bit pixels, and the PNG as it is written (what it wrote before
/// failing too). `None` when it can't be.
pub(crate) fn encode_png(file: &[u8], format: image::ImageFormat, limit: usize, work: &mut Decode, prepaid: usize) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    use image::metadata::Orientation;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(file));
    reader.set_format(format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some((256 << 20).min(limit as u64));
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().ok()?;
    // `ImageReader::decode` holds the pixel buffer to `max_alloc`; decoding through the decoder
    // (to read its orientation first) doesn't, so the buffer is reserved here.
    let pixels = decoder.total_bytes();
    limits.reserve(pixels).ok()?;
    let pixels = usize::try_from(pixels).ok()?;
    let more = pixels.saturating_sub(prepaid);
    if more > 0 && !work.charge(more, "picture") {
        return None;
    }
    // A TIFF is decoded into the decoder's own buffer, in its stored colour type (CMYK's is
    // larger than the RGB it becomes), and then copied into the picture's.
    if format == image::ImageFormat::Tiff {
        let (w, h) = decoder.dimensions();
        let bits = u64::from(decoder.original_color_type().bits_per_pixel());
        let source = usize::try_from(u64::from(w).saturating_mul(bits).div_ceil(8).saturating_mul(u64::from(h))).ok()?;
        if !work.charge(source, "picture") {
            return None;
        }
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    // A quarter-turn makes a new buffer; a flip turns the pixels where they are.
    let turned = matches!(orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH);
    if turned && !work.charge(pixels, "picture") {
        return None;
    }
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    let floats = matches!(img, image::DynamicImage::ImageRgb32F(_) | image::DynamicImage::ImageRgba32F(_));
    if floats && !work.charge(img.as_bytes().len() / 2, "picture") {
        return None;
    }
    let img = match img {
        image::DynamicImage::ImageRgb32F(_) => image::DynamicImage::ImageRgb16(img.to_rgb16()),
        image::DynamicImage::ImageRgba32F(_) => image::DynamicImage::ImageRgba16(img.to_rgba16()),
        other => other,
    };
    // PNG is big-endian: on a little-endian machine the encoder copies 16-bit pixels to swap them.
    let sixteen = matches!(img.color(), image::ColorType::L16 | image::ColorType::La16 | image::ColorType::Rgb16 | image::ColorType::Rgba16);
    if sixteen && cfg!(target_endian = "little") && !work.charge(img.as_bytes().len(), "picture") {
        return None;
    }
    let mut png = PictureWriter { bytes: Vec::new(), limit, work };
    image::codecs::png::PngEncoder::new(&mut png).write_image(img.as_bytes(), img.width(), img.height(), img.color().into()).ok()?;
    Some(png.bytes)
}

/// Cap encoded bytes while the PNG encoder writes, before the buffer can grow: at most `limit`,
/// each write charged to `work` as it is accepted.
struct PictureWriter<'w> {
    bytes: Vec<u8>,
    limit: usize,
    work: &'w mut Decode,
}

impl Write for PictureWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let end = self.bytes.len().checked_add(buf.len()).filter(|&end| end <= self.limit);
        let Some(end) = end.filter(|_| self.work.charge(buf.len(), "picture")) else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "picture exceeds remaining media budget"));
        };
        if end > self.bytes.capacity() {
            let new_capacity = self.bytes.capacity().saturating_mul(2).max(end).min(self.limit).min(end.saturating_add(self.work.left));
            self.bytes.try_reserve_exact(new_capacity - self.bytes.len()).map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

//! Device-independent bitmaps (the BLIP DIB: a `BITMAPINFOHEADER`, its colour table and pixels,
//! without a file header) as PNG files.

use std::io::Write;

use image::ImageEncoder;

use crate::blip::Decode;
use crate::record::{u16_at, u32_at};

/// Largest DIB converted to PNG, per side (pixels).
const MAX_DIB_SIDE: u32 = 16_384;

/// A DIB as a picture file.
#[derive(Clone, Debug, PartialEq)]
pub enum Dib {
    /// Decoded and encoded as a PNG.
    Png(Vec<u8>),
    /// Kept as a BMP file (the DIB with the file header it lacks), when it couldn't be decoded or
    /// its pixels didn't fit the decode budget: a placeholder of the right size.
    Bmp(Vec<u8>),
}

/// A device-independent bitmap (BITMAPINFOHEADER + palette + pixels) as a PNG: the DIB gets
/// the 14-byte BMP file header it lacks, is decoded with size limits and re-encoded. When the
/// bitmap can't be decoded, or its declared pixels (4 bytes each) don't fit in `work`, the
/// BMP bytes are kept. `None` when not even the BMP fits in `limit` or `work`.
pub fn to_png(dib: &[u8], limit: usize, work: &mut Decode) -> Option<Dib> {
    let header = u32_at(dib, 0)? as usize;
    if !(12..=124).contains(&header) {
        return None;
    }
    let (bits, used, compression) =
        if header == 12 { (u16_at(dib, 10)? as usize, 0, 0) } else { (u16_at(dib, 14)? as usize, u32_at(dib, 32)? as usize, u32_at(dib, 16)?) };
    let entry = if header == 12 { 3 } else { 4 };
    let colors = if used > 0 {
        used.min(256)
    } else if bits <= 8 {
        1usize << bits
    } else {
        0
    };
    // BI_BITFIELDS masks follow a 40-byte header.
    let masks = if header == 40 && (compression == 3 || compression == 6) { if compression == 3 { 12 } else { 16 } } else { 0 };
    let off_bits = 14usize.checked_add(header)?.checked_add(colors * entry)?.checked_add(masks)?;
    let file_size = 14usize.checked_add(dib.len()).filter(|&size| size <= limit)?;
    // The declared size is what decoding allocates, however small the (RLE) data is.
    let (w, h) = if header == 12 {
        (u64::from(u16_at(dib, 4)?), u64::from(u16_at(dib, 6)?))
    } else {
        (u64::from(u32_at(dib, 4)?), u64::from((u32_at(dib, 8)? as i32).unsigned_abs()))
    };
    let cost = w.saturating_mul(h).saturating_mul(4).try_into().unwrap_or(usize::MAX);
    // Copying the DIB into a BMP is work too, however few pixels it declares: it is charged
    // before the copy is made. A bitmap whose pixels don't fit still keeps its BMP bytes as a
    // placeholder, if the copy fits.
    let decode = work.charge(cost.max(dib.len()), "bitmap");
    if !decode && !work.charge(dib.len(), "bitmap") {
        return None;
    }
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&u32::try_from(file_size).ok()?.to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&u32::try_from(off_bits).ok()?.to_le_bytes());
    bmp.extend_from_slice(dib);
    if !decode {
        return Some(Dib::Bmp(bmp));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bmp));
    reader.set_format(image::ImageFormat::Bmp);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIB_SIDE);
    limits.max_image_height = Some(MAX_DIB_SIDE);
    limits.max_alloc = Some((256 << 20).min(limit as u64));
    reader.limits(limits);
    let mut png = PictureWriter { bytes: Vec::new(), limit };
    match reader.decode() {
        Ok(img)
            if image::codecs::png::PngEncoder::new(&mut png).write_image(img.as_bytes(), img.width(), img.height(), img.color().into()).is_ok() =>
        {
            Some(Dib::Png(png.bytes))
        }
        _ => Some(Dib::Bmp(bmp)),
    }
}

/// Cap encoded bytes while the PNG encoder writes, before the buffer can grow.
struct PictureWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for PictureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(end) = self.bytes.len().checked_add(buf.len()).filter(|&end| end <= self.limit) else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "picture exceeds remaining media budget"));
        };
        if end > self.bytes.capacity() {
            let new_capacity = self.bytes.capacity().saturating_mul(2).max(end).min(self.limit);
            self.bytes.try_reserve_exact(new_capacity - self.bytes.len()).map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

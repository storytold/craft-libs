//! Device-independent bitmaps (the BLIP DIB: a `BITMAPINFOHEADER`, its colour table and pixels,
//! without a file header) as BMP and PNG files.

use std::io::Write;

use image::ImageEncoder;

use crate::blip::Decode;
use crate::record::{u16_at, u32_at};

/// Largest DIB converted to PNG, per side (pixels).
const MAX_DIB_SIDE: u32 = 16_384;

/// A device-independent bitmap with the 14-byte file header a `.bmp` needs in front. `None` for a
/// header size no Windows version writes (12 to 124 bytes), or pixels that would start past its
/// end.
pub fn to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let offset = pixels_at(dib)?;
    let total = u32::try_from(14usize.checked_add(dib.len())?).ok()?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&u32::try_from(offset).ok()?.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// Where a DIB's pixels start in its BMP file, or `None` when [`to_bmp`] refuses it: checked
/// without copying anything.
fn pixels_at(dib: &[u8]) -> Option<usize> {
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
    (offset <= file_len).then_some(offset)
}

/// [`to_bmp`] within `limit` bytes and `work`: the whole BMP file (the DIB and its 14-byte header)
/// is charged before it is made, once its layout is checked. `None` when it doesn't fit, or isn't
/// a bitmap (then nothing is charged).
pub fn to_bmp_within(dib: &[u8], limit: usize, work: &mut Decode) -> Option<Vec<u8>> {
    let len = 14usize.checked_add(dib.len())?;
    if pixels_at(dib).is_none() || len > limit || !work.charge(len, "bitmap") {
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
    if w > u64::from(MAX_DIB_SIDE) || h > u64::from(MAX_DIB_SIDE) || !work.charge(cost, "bitmap") {
        return Some(Dib::Bmp(bmp));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bmp));
    reader.set_format(image::ImageFormat::Bmp);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIB_SIDE);
    limits.max_image_height = Some(MAX_DIB_SIDE);
    limits.max_alloc = Some((256 << 20).min(limit as u64));
    reader.limits(limits);
    // The PNG may be at most what is left of `work` (and `limit`); what it takes is charged.
    let mut png = PictureWriter { bytes: Vec::new(), limit: limit.min(work.left) };
    match reader.decode() {
        Ok(img)
            if image::codecs::png::PngEncoder::new(&mut png).write_image(img.as_bytes(), img.width(), img.height(), img.color().into()).is_ok()
                && work.charge(png.bytes.len(), "bitmap") =>
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

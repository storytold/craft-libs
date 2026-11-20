//! TIFF pictures as PNG files, for a reader whose apps draw PNG but not TIFF.

use crate::blip::Decode;
use crate::dib::{MAX_SIDE, encode_png};

/// A TIFF file decoded and made a PNG of at most `limit` bytes. Its declared pixels (4 bytes each)
/// are charged to `work` before it is decoded, whether that succeeds or not, and the PNG as it is
/// made; a TIFF whose pixels or PNG don't fit, that is larger than [`MAX_SIDE`] a side, or that
/// can't be decoded gives `None`.
pub fn to_png(tiff: &[u8], limit: usize, work: &mut Decode) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(tiff));
    reader.set_format(image::ImageFormat::Tiff);
    let (w, h) = reader.into_dimensions().ok()?;
    if w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let cost = (u64::from(w) * u64::from(h) * 4).try_into().unwrap_or(usize::MAX);
    if !work.charge(cost.max(tiff.len()), "TIFF") {
        return None;
    }
    encode_png(tiff, image::ImageFormat::Tiff, limit.min(work.left)).filter(|png| work.charge(png.len(), "TIFF"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A TIFF of `w` × `h` red pixels, written by the image crate.
    fn tiff(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 0, 0]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Tiff).unwrap();
        out.into_inner()
    }

    /// A TIFF becomes a PNG of the same picture, its pixels charged first; one whose pixels don't
    /// fit the budget, or that isn't a TIFF, gives nothing.
    #[test]
    fn a_tiff_becomes_a_png_within_the_budget() {
        let file = tiff(3, 2);
        let mut work = Decode::new(1 << 20);
        let png = to_png(&file, 1 << 20, &mut work).unwrap();
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgb8();
        assert_eq!((img.dimensions(), img.get_pixel(0, 0).0), ((3, 2), [255, 0, 0]));
        assert!((1 << 20) - work.left >= file.len().max(3 * 2 * 4) + png.len(), "the pixels and the PNG are charged");
        assert!(to_png(&file, 1 << 20, &mut Decode::new(10)).is_none(), "its pixels don't fit");
        assert!(to_png(b"not a TIFF at all", 1 << 20, &mut Decode::new(1 << 20)).is_none());
    }
}

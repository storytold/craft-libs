//! TIFF pictures as PNG files, for a reader whose apps draw PNG but not TIFF.

use crate::blip::Decode;
use crate::dib::{MAX_SIDE, encode_png};

/// A TIFF file decoded and made a PNG of at most `limit` bytes. Its declared pixels (4 bytes each)
/// are charged to `work` before it is decoded, whether that succeeds or not, then what its
/// pixels really take past that (16-bit and floating-point ones take more), the decoder's own
/// buffer, its turned and 16-bit copies, and the PNG as it is made; a TIFF whose pixels or PNG
/// don't fit, that is larger than 16,384 pixels a side, or that can't be decoded gives `None`.
/// A JPEG-compressed TIFF is also charged, before decoding, the frames its JPEG strips or tiles
/// declare, which the decoder allocates whatever size the TIFF itself declares.
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
    match jpeg_work(tiff, work.left) {
        Some(0) => {}
        Some(jpeg) if work.charge(jpeg, "TIFF") => {}
        _ => return None,
    }
    encode_png(tiff, image::ImageFormat::Tiff, limit, work, cost)
}

/// The work decoding a JPEG-compressed TIFF (Compression 7) does past its own pixels: each strip
/// or tile is a JPEG of its own, copied out of the file after a copy of the shared `JPEGTables`
/// and decoded to a frame of the size its start-of-frame marker declares (in the strip, the
/// tables, or across their join). 0 for a TIFF that isn't
/// JPEG-compressed; `None` when its layout can't be read or the work passes `most`, where the
/// count stops.
fn jpeg_work(file: &[u8], most: usize) -> Option<usize> {
    use ::tiff::decoder::Decoder;
    use ::tiff::tags::Tag;
    let mut d = Decoder::new(std::io::Cursor::new(file)).ok()?;
    if d.find_tag_unsigned::<u16>(Tag::Compression).ok()? != Some(7) {
        return Some(0);
    }
    let tables = match d.find_tag(Tag::JPEGTables).ok()? {
        Some(v) => v.into_u8_vec().ok()?,
        None => Vec::new(),
    };
    let (offsets, counts) = match d.find_tag_unsigned_vec::<u64>(Tag::StripOffsets).ok()? {
        Some(offsets) => (offsets, d.find_tag_unsigned_vec::<u64>(Tag::StripByteCounts).ok()??),
        None => (d.find_tag_unsigned_vec::<u64>(Tag::TileOffsets).ok()??, d.find_tag_unsigned_vec::<u64>(Tag::TileByteCounts).ok()??),
    };
    // The decoder copies the tables, less their end-of-image marker, in front of each strip, less
    // its start-of-image marker: a frame header may straddle the join.
    let head = &tables[..tables.len().saturating_sub(2)];
    let head_frame = frame_bytes(head);
    let mut total = 0usize;
    for (&at, &len) in offsets.iter().zip(&counts) {
        let start = usize::try_from(at).ok()?.min(file.len());
        let end = start.saturating_add(usize::try_from(len).unwrap_or(usize::MAX)).min(file.len());
        let chunk = &file[start..end];
        let frame = frame_bytes(chunk).max(head_frame).max(if tables.is_empty() { 0 } else { join_frame(head, chunk) });
        total = total.saturating_add(tables.len()).saturating_add(chunk.len()).saturating_add(frame);
        if total > most {
            return None;
        }
    }
    Some(total)
}

/// The frame a start-of-frame header straddling the join of `head` (the tables, as the decoder
/// copies them) and a strip declares: the header is ten bytes, so the last nine of `head` and the
/// first nine of the strip after its start-of-image marker hold any that crosses it.
fn join_frame(head: &[u8], chunk: &[u8]) -> usize {
    let tail = chunk.get(2..).unwrap_or_default();
    let mut edge = head[head.len().saturating_sub(9)..].to_vec();
    edge.extend_from_slice(&tail[..tail.len().min(9)]);
    frame_bytes(&edge)
}

/// The bytes of the largest frame a JPEG's start-of-frame markers declare (width × height ×
/// components, two bytes a sample past 8 bits). Every such marker counts, wherever it is: a lenient
/// decoder may find one a strict walk of the segments would skip, and entropy-coded data never
/// holds one (an `FF` there is followed by `00` or a restart marker).
fn frame_bytes(jpeg: &[u8]) -> usize {
    jpeg.windows(10)
        .filter(|m| m[0] == 0xFF && matches!(m[1], 0xC0..=0xCF) && !matches!(m[1], 0xC4 | 0xC8 | 0xCC))
        .map(|m| {
            // FF Cn, the segment length, then the precision, height, width and component count.
            let (height, width) = (usize::from(u16::from_be_bytes([m[5], m[6]])), usize::from(u16::from_be_bytes([m[7], m[8]])));
            let sample = if m[4] > 8 { 2 } else { 1 };
            width.saturating_mul(height).saturating_mul(usize::from(m[9])).saturating_mul(sample)
        })
        .max()
        .unwrap_or(0)
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

    /// Every buffer a TIFF's conversion makes is charged: its declared pixels, the decoder's own
    /// copy of them, and the PNG; for 16-bit pixels also what they take past the 4 bytes declared
    /// and, on a little-endian target, the big-endian copy the PNG encoder makes.
    #[test]
    fn a_tiff_is_charged_the_decoders_buffer_and_the_encoders_copy() {
        let (w, h) = (40usize, 30usize);
        let charged = |img: image::DynamicImage| {
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Tiff).unwrap();
            let mut work = Decode::new(1 << 20);
            let png = to_png(&out.into_inner(), 1 << 20, &mut work).unwrap();
            ((1 << 20) - work.left, png.len())
        };
        let (rgb8, png) = charged(image::DynamicImage::ImageRgb8(image::RgbImage::new(w as u32, h as u32)));
        assert!(rgb8 >= w * h * (4 + 3) + png, "declared pixels, the decoder's buffer, the PNG: {rgb8}");
        let (rgb16, png) = charged(image::DynamicImage::ImageRgb16(image::ImageBuffer::new(w as u32, h as u32)));
        // The encoder swaps 16-bit samples to big-endian in a copy only on a little-endian target.
        let swapped = if cfg!(target_endian = "little") { 6 } else { 0 };
        assert!(rgb16 >= w * h * (6 + 6 + swapped) + png, "the pixels, the decoder's buffer, the swapped copy, the PNG: {rgb16}");
    }

    /// A 1 × 1 RGB TIFF compressed with JPEG, its one strip a JPEG whose frame is declared
    /// `side` × `side` (only its start and frame header).
    fn jpeg_tiff(side: u16) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&sof(side));
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg_tiff_of(&[], &[jpeg])
    }

    /// A start-of-frame header (baseline, 8 bits, three components) declaring `side` × `side`.
    fn sof(side: u16) -> Vec<u8> {
        let [hi, lo] = side.to_be_bytes();
        vec![0xFF, 0xC0, 0, 17, 8, hi, lo, hi, lo, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]
    }

    /// A JPEG-compressed RGB TIFF one pixel wide and a row a strip, with `tables` as its
    /// `JPEGTables` (none when empty) and `strips` as its strips.
    fn jpeg_tiff_of(tables: &[u8], strips: &[Vec<u8>]) -> Vec<u8> {
        let n = strips.len() as u32;
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&[0; 4]);
        let mut offsets = vec![];
        for s in strips {
            offsets.push(out.len() as u32);
            out.extend_from_slice(s);
        }
        let tables_at = out.len() as u32;
        out.extend_from_slice(tables);
        let bits_at = out.len() as u32;
        out.extend_from_slice(&[8, 0, 8, 0, 8, 0]);
        // One strip's offset and count fit in their entries; more go in arrays.
        let array = |out: &mut Vec<u8>, v: &[u32]| {
            let at = out.len() as u32;
            v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
            if v.len() == 1 { v[0] } else { at }
        };
        let counts: Vec<u32> = strips.iter().map(|s| s.len() as u32).collect();
        let (offsets_v, counts_v) = (array(&mut out, &offsets), array(&mut out, &counts));
        let ifd_at = out.len() as u32;
        out[4..8].copy_from_slice(&ifd_at.to_le_bytes());
        let mut tags: Vec<(u16, u16, u32, u32)> = vec![
            (256, 3, 1, 1),
            (257, 3, 1, n),
            (258, 3, 3, bits_at),
            (259, 3, 1, 7),
            (262, 3, 1, 2),
            (273, 4, n, offsets_v),
            (277, 3, 1, 3),
            (278, 3, 1, 1),
            (279, 4, n, counts_v),
            (284, 3, 1, 1),
        ];
        if !tables.is_empty() {
            tags.push((347, 7, tables.len() as u32, tables_at));
        }
        out.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for (tag, kind, count, v) in tags {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    /// A JPEG strip is charged the frame it declares before it is decoded, not the TIFF's own
    /// 1 × 1: with room it is charged at least that, and without room it is refused.
    #[test]
    fn a_tiffs_jpeg_strip_is_charged_its_declared_frame() {
        let file = jpeg_tiff(2048);
        let frame = 2048 * 2048 * 3;
        let mut work = Decode::new(64 << 20);
        let _ = to_png(&file, 1 << 20, &mut work);
        assert!((64 << 20) - work.left >= frame, "{} charged", (64 << 20) - work.left);
        let mut work = Decode::new(frame / 2);
        assert!(to_png(&file, 1 << 20, &mut work).is_none());
        assert!(work.left > frame / 4, "refused before decoding, not charged its frame");
    }

    /// A frame header split between the tables and the strip, which the decoder joins back
    /// together, is charged as one whole header.
    #[test]
    fn a_frame_header_split_across_the_tables_is_charged() {
        let header = sof(2048);
        // The tables end right after the precision byte (then their end marker); the strip
        // starts with its start marker, then the rest of the header.
        let tables = [&[0xFF, 0xD8][..], &header[..5], &[0xFF, 0xD9]].concat();
        let strip = [&[0xFF, 0xD8][..], &header[5..], &[0xFF, 0xD9]].concat();
        let file = jpeg_tiff_of(&tables, &[strip]);
        let mut work = Decode::new(64 << 20);
        let _ = to_png(&file, 1 << 20, &mut work);
        assert!((64 << 20) - work.left >= 2048 * 2048 * 3, "{} charged", (64 << 20) - work.left);
    }

    /// The decoder copies the shared tables for every strip, so each strip is charged them.
    #[test]
    fn the_shared_tables_are_charged_for_every_strip() {
        let tables = [&[0xFF, 0xD8][..], &[0; 1000], &[0xFF, 0xD9]].concat();
        let strips = vec![vec![0xFF, 0xD8, 0xFF, 0xD9]; 64];
        let file = jpeg_tiff_of(&tables, &strips);
        let mut work = Decode::new(1 << 20);
        let _ = to_png(&file, 1 << 20, &mut work);
        assert!((1 << 20) - work.left >= 64 * tables.len(), "{} charged", (1 << 20) - work.left);
    }

    /// A floating-point TIFF's pixels take 12 bytes each, and its 16-bit copy (PNG has no floats)
    /// 6 more: both are charged, not only the 4 a pixel charged before decoding or the file's size.
    #[test]
    fn a_float_tiff_is_charged_its_pixels_and_their_16_bit_copy() {
        let (w, h) = (40, 30);
        let img = image::Rgb32FImage::from_pixel(w, h, image::Rgb([1.0, 0.0, 0.0]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb32F(img).write_to(&mut out, image::ImageFormat::Tiff).unwrap();
        let file = out.into_inner();
        let mut work = Decode::new(1 << 20);
        let png = to_png(&file, 1 << 20, &mut work).unwrap();
        assert!((1 << 20) - work.left >= (w * h * (12 + 6)) as usize + png.len());
    }

    /// A 2 × 3 RGB TIFF, uncompressed, its first pixel red and the rest black, whose Orientation
    /// tag says `orientation`.
    fn oriented_tiff(orientation: u16) -> Vec<u8> {
        let pixels: Vec<u8> = (0..6).flat_map(|i| if i == 0 { [255, 0, 0] } else { [0, 0, 0] }).collect();
        // Header, then the pixels, the three BitsPerSample values and the IFD.
        let (data_at, bits_at) = (8u32, 8 + pixels.len() as u32);
        let ifd_at = bits_at + 6;
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&ifd_at.to_le_bytes());
        out.extend_from_slice(&pixels);
        out.extend_from_slice(&[8, 0, 8, 0, 8, 0]);
        let short = |tag: u16, count: u32, v: u32| (tag, 3u16, count, v);
        let long = |tag: u16, v: u32| (tag, 4u16, 1u32, v);
        let tags = [
            short(256, 1, 2),
            short(257, 1, 3),
            short(258, 3, bits_at),
            short(259, 1, 1),
            short(262, 1, 2),
            long(273, data_at),
            short(274, 1, u32::from(orientation)),
            short(277, 1, 3),
            short(278, 1, 3),
            long(279, pixels.len() as u32),
            short(284, 1, 1),
        ];
        out.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for (tag, kind, count, v) in tags {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    /// A TIFF is turned as its Orientation tag says before it becomes a PNG: turned a quarter
    /// clockwise (6), a 2 × 3 picture is 3 × 2, its first pixel at the top right.
    #[test]
    fn a_tiff_is_turned_as_its_orientation_says() {
        let png = |o: u16| {
            let png = to_png(&oriented_tiff(o), 1 << 20, &mut Decode::new(1 << 20)).unwrap();
            image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgb8()
        };
        let upright = png(1);
        assert_eq!((upright.dimensions(), upright.get_pixel(0, 0).0), ((2, 3), [255, 0, 0]));
        let turned = png(6);
        assert_eq!((turned.dimensions(), turned.get_pixel(2, 0).0), ((3, 2), [255, 0, 0]));
    }

    /// A TIFF of floating-point samples, which PNG can't hold, becomes a 16-bit PNG.
    #[test]
    fn a_floating_point_tiff_becomes_a_png() {
        let img = image::Rgb32FImage::from_pixel(2, 2, image::Rgb([1.0, 0.0, 0.0]));
        let mut file = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb32F(img).write_to(&mut file, image::ImageFormat::Tiff).unwrap();
        let png = to_png(&file.into_inner(), 1 << 20, &mut Decode::new(1 << 20)).expect("a PNG");
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgb8();
        assert_eq!(back.get_pixel(0, 0).0, [255, 0, 0]);
    }
}

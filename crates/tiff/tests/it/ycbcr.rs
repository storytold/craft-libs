//! YCbCr: PhotoCraft's conversion tests and FilmCraft's decoding cases, on files this crate writes.

use craft_tiff::ycbcr::{self, BT601, REFERENCE_BLACK_WHITE, YCBCR_COEFFICIENTS, YCbCr};
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, Tiff, TiffError, TiffWriter, Value, tags};

fn convert(c: &YCbCr, px: [u8; 3]) -> [u8; 3] {
    let mut d = px.to_vec();
    c.convert_u8(&mut d, 3);
    [d[0], d[1], d[2]]
}

#[test]
fn jfif_conversion_matches_known_colours() {
    let c = YCbCr::defaults(255);
    assert_eq!(convert(&c, [0, 128, 128]), [0, 0, 0]);
    assert_eq!(convert(&c, [255, 128, 128]), [255, 255, 255]);
    assert_eq!(convert(&c, [128, 128, 128]), [128, 128, 128]);
    // Pure red, green and blue as JFIF encodes them.
    assert_eq!(convert(&c, [76, 85, 255]), [254, 0, 0]);
    assert_eq!(convert(&c, [150, 44, 21]), [0, 255, 1]);
    assert_eq!(convert(&c, [29, 255, 107]), [0, 0, 254]);
}

#[test]
fn sixteen_bit_and_extra_samples() {
    let c = YCbCr::defaults(65535);
    let mut d = [65535u16, 32768, 32768, 1234];
    c.convert_u16(&mut d, 4);
    assert_eq!(d, [65535, 65535, 65535, 1234]);
}

#[test]
fn studio_range_reference_black_white() {
    // Y 16–235, chroma 128 ± 112 (ITU-R BT.601 studio range).
    let c = YCbCr { luma: BT601, reference_black_white: [16.0, 235.0, 128.0, 240.0, 128.0, 240.0] };
    assert_eq!(convert(&c, [16, 128, 128]), [0, 0, 0]);
    assert_eq!(convert(&c, [235, 128, 128]), [255, 255, 255]);
}

#[test]
fn weights_that_do_not_sum_to_one_keep_grays_neutral() {
    let c = YCbCr { luma: [0.4, 0.4, 0.4], reference_black_white: YCbCr::defaults(255).reference_black_white };
    for y in [0u8, 64, 128, 200, 255] {
        assert_eq!(convert(&c, [y, 128, 128]), [y, y, y]);
    }
}

#[test]
fn odd_buffers_do_not_panic() {
    let c = YCbCr::defaults(255);
    let mut d = vec![1u8, 2];
    c.convert_u8(&mut d, 3);
    c.convert_u8(&mut d, 0);
    c.convert_u8(&mut d, 2);
    let mut w = vec![1u16];
    c.convert_u16(&mut w, 0);
    assert_eq!((d, w), (vec![1, 2], vec![1]));
}

fn ifd_with(entries: &[(u16, Value)]) -> craft_tiff::Ifd {
    // A TIFF directory cannot be empty: give it a size.
    let mut b = IfdBuilder::new().with(tags::IMAGE_WIDTH, Value::Long(vec![1]));
    for (t, v) in entries {
        b.set(*t, v.clone());
    }
    let bytes = TiffWriter::new(ByteOrder::Little, false).write(&[b]).unwrap();
    Tiff::parse(&bytes).unwrap().ifds.remove(0)
}

#[test]
fn tags_are_read_and_bad_ones_fall_back() {
    let d = YCbCr::defaults(255);
    assert_eq!(YCbCr::from_ifd(&ifd_with(&[]), 255), d);
    let rbw = Value::Rational(vec![(16, 1), (235, 1), (128, 1), (240, 1), (128, 1), (240, 1)]);
    let c = YCbCr::from_ifd(
        &ifd_with(&[(YCBCR_COEFFICIENTS, Value::Rational(vec![(2126, 10000), (7152, 10000), (722, 10000)])), (REFERENCE_BLACK_WHITE, rbw.clone())]),
        255,
    );
    assert_eq!((c.luma, c.reference_black_white), ([0.2126, 0.7152, 0.0722], [16.0, 235.0, 128.0, 240.0, 128.0, 240.0]));
    // JPEG-compressed data ignores both tags.
    assert_eq!(YCbCr::from_ifd(&ifd_with(&[(tags::COMPRESSION, Value::Short(vec![7])), (REFERENCE_BLACK_WHITE, rbw)]), 255), d);
    // A weight of 0 or 1, too few values, or a black equal to its white: that tag's default.
    let bad = ifd_with(&[
        (YCBCR_COEFFICIENTS, Value::Rational(vec![(0, 1), (1, 2), (1, 2)])),
        (REFERENCE_BLACK_WHITE, Value::Rational(vec![(16, 1), (16, 1), (128, 1), (240, 1), (128, 1), (240, 1)])),
    ]);
    assert_eq!(YCbCr::from_ifd(&bad, 255), d);
    assert_eq!(YCbCr::from_ifd(&ifd_with(&[(YCBCR_COEFFICIENTS, Value::Rational(vec![(1, 2)]))]), 255).luma, BT601);
}

/// A YCbCr TIFF of `w` × `h` at `sub` with these data units, in `strips` strips, uncompressed or
/// PackBits (one literal packet per 128 bytes).
fn file(w: u32, h: u32, sub: (u16, u16), units: &[u8], packbits: bool, extra: &[(u16, Value)]) -> Vec<u8> {
    let data = if packbits {
        units.chunks(128).flat_map(|c| std::iter::once((c.len() - 1) as u8).chain(c.iter().copied())).collect()
    } else {
        units.to_vec()
    };
    let mut b = IfdBuilder::new();
    b.set(tags::IMAGE_WIDTH, Value::Long(vec![w]))
        .set(tags::IMAGE_LENGTH, Value::Long(vec![h]))
        .set(tags::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]))
        .set(tags::COMPRESSION, Value::Short(vec![if packbits { 32773 } else { 1 }]))
        .set(tags::PHOTOMETRIC, Value::Short(vec![6]))
        .set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![3]))
        .set(tags::YCBCR_SUBSAMPLING, Value::Short(vec![sub.0, sub.1]))
        .set_image(ImageData::Strips { rows_per_strip: h, strips: vec![data] });
    for (t, v) in extra {
        b.set(*t, v.clone());
    }
    TiffWriter::new(ByteOrder::Big, false).write(&[b]).unwrap()
}

fn decode(bytes: &[u8]) -> craft_tiff::Result<Vec<u8>> {
    ycbcr::decode_rgb8(bytes, &Tiff::parse(bytes).unwrap().ifds[0], 1 << 28)
}

#[test]
fn data_units_expand_with_their_chroma() {
    // 3 × 2 at 2 × 1: two units a row, the second half outside the image.
    let units = [10, 20, 128, 128, 30, 40, 128, 128, 50, 60, 128, 128, 70, 80, 128, 128];
    let px = ycbcr::expand_units(&units, 3, 2, (2, 1)).unwrap();
    assert_eq!(px, [10, 128, 128, 20, 128, 128, 30, 128, 128, 50, 128, 128, 60, 128, 128, 70, 128, 128]);
    assert_eq!(ycbcr::units_len(3, 2, (2, 1)), Some(16));
    assert_eq!(ycbcr::units_len(5, 5, (4, 2)), Some(2 * 3 * 10));
    assert_eq!(ycbcr::expand_units(&units[..15], 3, 2, (2, 1)), None);
    // 2 × 2 at 2 × 2: four luma samples (row by row), one chroma pair for all of them.
    assert_eq!(ycbcr::expand_units(&[1, 2, 3, 4, 9, 8], 2, 2, (2, 2)).unwrap(), [1, 9, 8, 2, 9, 8, 3, 9, 8, 4, 9, 8]);
}

#[test]
fn subsampled_files_decode_uncompressed_and_packbits() {
    // 4 × 2 at 2 × 2: grey, white, black and red units (red as JFIF codes it, one shared chroma).
    let units = [128, 128, 128, 128, 128, 128, 255, 255, 255, 255, 128, 128];
    for packbits in [false, true] {
        let rgb = decode(&file(4, 2, (2, 2), &units, packbits, &[])).unwrap();
        assert_eq!(rgb, [128, 128, 128, 128, 128, 128, 255, 255, 255, 255, 255, 255].repeat(2), "packbits {packbits}");
    }
    let red = decode(&file(1, 1, (1, 1), &[76, 85, 255], false, &[])).unwrap();
    assert_eq!(red, [254, 0, 0]);
    // The file's ReferenceBlackWhite applies.
    let studio = Value::Rational(vec![(16, 1), (235, 1), (128, 1), (240, 1), (128, 1), (240, 1)]);
    assert_eq!(decode(&file(1, 1, (1, 1), &[235, 128, 128], false, &[(REFERENCE_BLACK_WHITE, studio)])).unwrap(), [255, 255, 255]);
    // No subsampling tag means 2 × 2.
    assert_eq!(ycbcr::subsampling(&ifd_with(&[])), Some((2, 2)));
}

#[test]
fn what_it_does_not_decode_is_an_error() {
    let units = [128u8; 6];
    let ok = file(2, 2, (2, 2), &units, false, &[]);
    assert!(decode(&ok).is_ok());
    let with = |extra: &[(u16, Value)]| decode(&file(2, 2, (2, 2), &units, false, extra));
    for (tag, v) in [
        (tags::PHOTOMETRIC, Value::Short(vec![2])),
        (tags::SAMPLES_PER_PIXEL, Value::Short(vec![4])),
        (tags::BITS_PER_SAMPLE, Value::Short(vec![16, 16, 16])),
        (tags::PLANAR_CONFIGURATION, Value::Short(vec![2])),
        (tags::COMPRESSION, Value::Short(vec![5])),
        (tags::YCBCR_SUBSAMPLING, Value::Short(vec![3, 1])),
        (tags::TILE_OFFSETS, Value::Long(vec![8])),
    ] {
        assert!(matches!(with(&[(tag, v.clone())]), Err(TiffError::Invalid(_))), "{tag} {v:?}");
    }
}

#[test]
fn limits_and_hostile_files() {
    let units = [128u8; 6];
    let ok = file(2, 2, (2, 2), &units, false, &[]);
    let ifd = Tiff::parse(&ok).unwrap().ifds.remove(0);
    assert!(ycbcr::decode_rgb8(&ok, &ifd, 4).is_ok());
    assert_eq!(ycbcr::decode_rgb8(&ok, &ifd, 3), Err(TiffError::Limit("pixel count")));
    // A small file claiming a large image is refused before the data units are allocated.
    for packbits in [false, true] {
        let big = file(16384, 16384, (1, 1), &units, packbits, &[]);
        assert!(matches!(decode(&big), Err(TiffError::Truncated(_))), "packbits {packbits}");
    }
    // Data shorter than the image, and a strip outside the file.
    assert!(matches!(decode(&file(4, 4, (2, 2), &units, false, &[])), Err(TiffError::Truncated(_))));
    let mut moved = Tiff::parse(&ok).unwrap().ifds.remove(0);
    for e in &mut moved.entries {
        if e.tag == tags::STRIP_OFFSETS {
            e.value = Value::Long(vec![u32::MAX]);
        }
    }
    assert_eq!(ycbcr::decode_rgb8(&ok, &moved, 1 << 20), Err(TiffError::Truncated(u64::from(u32::MAX))));
    let mut far = ok.clone();
    // Every truncation and byte change either decodes or errors.
    for n in 0..ok.len() {
        if let Ok(t) = Tiff::parse(&ok[..n])
            && let Some(ifd) = t.ifds.first()
        {
            let _ = ycbcr::decode_rgb8(&ok[..n], ifd, 1 << 20);
        }
    }
    for i in 0..ok.len() {
        for v in [0u8, 1, 0x80, 0xFF] {
            far.clone_from(&ok);
            far[i] = v;
            if let Ok(t) = Tiff::parse(&far)
                && let Some(ifd) = t.ifds.first()
                && let Ok(rgb) = ycbcr::decode_rgb8(&far, ifd, 1 << 20)
            {
                assert_eq!(rgb.len() as u64, u64::from(ifd.u32(tags::IMAGE_WIDTH).unwrap()) * u64::from(ifd.u32(tags::IMAGE_LENGTH).unwrap()) * 3);
            }
        }
    }
}

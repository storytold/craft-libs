//! An exported file's EXIF names the pixel size it is written at, not the camera
//! sensor's. `exif_with_pixel_size` rewrites PixelXDimension and PixelYDimension in place.

use std::borrow::Cow;

use craft_metadata::edit::exif_with_pixel_size;

const PIXEL_X: u16 = 40962;
const PIXEL_Y: u16 = 40963;
const SHORT: u16 = 3;
const LONG: u16 = 4;

/// A classic TIFF block: IFD0 = Make + the Exif IFD pointer, the Exif IFD = `exif` entries
/// `(tag, type, value)` holding one inline SHORT or LONG each. `big` = big-endian.
fn block(big: bool, exif: &[(u16, u16, u32)]) -> Vec<u8> {
    let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut v = if big { b"MM\0*".to_vec() } else { b"II*\0".to_vec() };
    v.extend(u32b(8));
    // IFD0: two entries, then no next IFD; the Exif IFD follows at 8 + 2 + 24 + 4 = 38.
    v.extend(u16b(2));
    v.extend(u16b(0x010F));
    v.extend(u16b(2));
    v.extend(u32b(4));
    v.extend(b"Sony");
    v.extend(u16b(34665));
    v.extend(u16b(LONG));
    v.extend(u32b(1));
    v.extend(u32b(38));
    v.extend(u32b(0));
    v.extend(u16b(exif.len() as u16));
    for &(tag, ty, value) in exif {
        v.extend(u16b(tag));
        v.extend(u16b(ty));
        v.extend(u32b(1));
        if ty == SHORT {
            v.extend(u16b(value as u16));
            v.extend([0, 0]);
        } else {
            v.extend(u32b(value));
        }
    }
    v.extend(u32b(0));
    v
}

#[test]
fn the_sensor_size_becomes_the_written_size_in_either_byte_order() {
    for big in [false, true] {
        let src = block(big, &[(33434, LONG, 7), (PIXEL_X, LONG, 8192), (PIXEL_Y, LONG, 5464)]);
        let out = exif_with_pixel_size(&src, (900, 600));
        // Exactly what an encoder would write for a 900 × 600 image; nothing else changes.
        assert_eq!(out.as_ref(), block(big, &[(33434, LONG, 7), (PIXEL_X, LONG, 900), (PIXEL_Y, LONG, 600)]), "big-endian {big}");
    }
}

/// A little-endian BigTIFF block: IFD0 holds only the Exif IFD pointer (an IFD8), the Exif IFD
/// `exif` entries `(tag, type, value)`, each one value in the entry's 8-byte field.
fn big_tiff(exif: &[(u16, u16, u32)]) -> Vec<u8> {
    let mut v = b"II+\0".to_vec();
    v.extend(8u16.to_le_bytes());
    v.extend(0u16.to_le_bytes());
    v.extend(16u64.to_le_bytes());
    // IFD0 at 16: count, one 20-byte entry, next IFD; the Exif IFD follows at 16 + 8 + 20 + 8 = 52.
    v.extend(1u64.to_le_bytes());
    v.extend(34665u16.to_le_bytes());
    v.extend(18u16.to_le_bytes());
    v.extend(1u64.to_le_bytes());
    v.extend(52u64.to_le_bytes());
    v.extend(0u64.to_le_bytes());
    v.extend((exif.len() as u64).to_le_bytes());
    for &(tag, ty, value) in exif {
        v.extend(tag.to_le_bytes());
        v.extend(ty.to_le_bytes());
        v.extend(1u64.to_le_bytes());
        let mut field = [0u8; 8];
        if ty == SHORT {
            field[..2].copy_from_slice(&(value as u16).to_le_bytes());
        } else {
            field[..4].copy_from_slice(&value.to_le_bytes());
        }
        v.extend(field);
    }
    v.extend(0u64.to_le_bytes());
    v
}

#[test]
fn a_big_tiff_block_is_rewritten_too() {
    let src = big_tiff(&[(PIXEL_X, SHORT, 6000), (33434, LONG, 7), (PIXEL_Y, LONG, 4000)]);
    assert_eq!(exif_with_pixel_size(&src, (90, 60)).as_ref(), big_tiff(&[(PIXEL_X, LONG, 90), (33434, LONG, 7), (PIXEL_Y, LONG, 60)]));
}

#[test]
fn a_short_dimension_is_written_as_a_long_so_a_large_size_fits() {
    let src = block(false, &[(PIXEL_X, SHORT, 6000), (PIXEL_Y, SHORT, 4000)]);
    assert_eq!(exif_with_pixel_size(&src, (70_000, 300)).as_ref(), block(false, &[(PIXEL_X, LONG, 70_000), (PIXEL_Y, LONG, 300)]));
}

#[test]
fn the_jpeg_exif_prefix_is_kept() {
    let src = [b"Exif\0\0".as_slice(), &block(true, &[(PIXEL_X, LONG, 10), (PIXEL_Y, LONG, 10)])].concat();
    let want = [b"Exif\0\0".as_slice(), &block(true, &[(PIXEL_X, LONG, 4), (PIXEL_Y, LONG, 3)])].concat();
    assert_eq!(exif_with_pixel_size(&src, (4, 3)).as_ref(), want);
}

#[test]
fn nothing_to_change_borrows_the_block() {
    let right = block(false, &[(PIXEL_X, SHORT, 4), (PIXEL_Y, LONG, 3)]);
    assert!(matches!(exif_with_pixel_size(&right, (4, 3)), Cow::Borrowed(_)));
    let without = block(false, &[(33434, LONG, 7)]);
    assert!(matches!(exif_with_pixel_size(&without, (4, 3)), Cow::Borrowed(_)));
}

#[test]
fn malformed_blocks_are_left_as_they_are() {
    let src = block(false, &[(PIXEL_X, LONG, 8192), (PIXEL_Y, LONG, 5464)]);
    // Every truncation up to the Exif IFD's next-IFD pointer (which this never reads), and
    // garbage, comes back unchanged without panicking.
    for n in 0..src.len() - 4 {
        assert_eq!(exif_with_pixel_size(&src[..n], (1, 1)).as_ref(), &src[..n], "truncated to {n}");
    }
    assert!(matches!(exif_with_pixel_size(b"not a tiff at all", (1, 1)), Cow::Borrowed(_)));
    // Dimensions with two values each aren't ones this rewrites (entries at 40 and 52).
    let mut two = src.clone();
    for e in [40, 52] {
        two[e + 4..e + 8].copy_from_slice(&2u32.to_le_bytes());
    }
    assert!(matches!(exif_with_pixel_size(&two, (1, 1)), Cow::Borrowed(_)));
    // An Exif IFD pointer with no value or several (its count at 26) points nowhere reliable.
    for n in [0u32, 2] {
        let mut odd = src.clone();
        odd[26..30].copy_from_slice(&n.to_le_bytes());
        assert!(matches!(exif_with_pixel_size(&odd, (1, 1)), Cow::Borrowed(_)), "pointer count {n}");
    }
}

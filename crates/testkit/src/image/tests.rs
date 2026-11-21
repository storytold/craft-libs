use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::{
    BoundedSink, DecodedPng, GoldenOptions, Gray8, ImageError, PngLimits, Rgba8, Tolerance, check_golden, check_golden_with, compare_gray,
    compare_rgba, decode_png, diff_gray, diff_image, diff_rgba, encode_gray_png, encode_png, is_bless_value, psnr, read_png, write_gray_png,
    write_png, write_stream,
};

/// A variable no test run sets, so `check_golden` compares instead of blessing.
const NO_BLESS: &str = "CRAFT_TESTKIT_SELF_TEST_BLESS";

fn ramp(w: u32, h: u32, alpha: u8) -> Vec<u8> {
    (0..w * h).flat_map(|i| [(i % w * 255 / w) as u8, (i / w * 255 / h) as u8, 77, alpha]).collect()
}

fn rgba(w: u32, h: u32, px: &[u8]) -> Rgba8<'_> {
    Rgba8::new(w, h, px).expect("sized")
}

fn gray(w: usize, h: usize, fill: u8) -> Vec<u8> {
    vec![fill; w * h]
}

fn g(w: usize, h: usize, px: &[u8]) -> Gray8<'_> {
    Gray8::new(w as u32, h as u32, px).expect("sized")
}

fn neighborhood(channel: u8) -> Tolerance {
    Tolerance { max_abs: None, channel, neighborhood: true, ..Tolerance::EXACT }
}

/// FilmCraft's render tolerance.
fn render() -> Tolerance {
    Tolerance { min_psnr: Some(45.0), max_abs: Some(12), max_p99: Some(2), ..Tolerance::EXACT }
}

/// AniCraft's default tolerance.
fn per_pixel() -> Tolerance {
    Tolerance { max_abs: None, channel: 3, max_bad_fraction: Some(0.002), ..Tolerance::EXACT }
}

// PNG files written by hand from the W3C PNG specification, so the reader is checked against
// the format rather than against the encoder it ships with.

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() { vec![&[]] } else { data.chunks(65_535).collect() };
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend(adler32(data).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend(crc32(&body).to_be_bytes());
}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// A PNG with the given header, extra chunks before the image data, and filtered scanlines.
fn png_file(width: u32, height: u32, depth: u8, color: u8, interlace: u8, extra: &[(&[u8; 4], &[u8])], scanlines: &[u8]) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([depth, color, 0, 0, interlace]);
    chunk(&mut out, b"IHDR", &ihdr);
    for (kind, data) in extra {
        chunk(&mut out, kind, data);
    }
    chunk(&mut out, b"IDAT", &zlib_stored(scanlines));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Adam7 pass origins and steps: (x0, y0, dx, dy).
const ADAM7: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

/// Scanlines of an 8-bit gray image in Adam7 order, each with filter type 0.
fn adam7_gray(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (x0, y0, dx, dy) in ADAM7 {
        let cols: Vec<usize> = (x0..w).step_by(dx).collect();
        if cols.is_empty() {
            continue;
        }
        for y in (y0..h).step_by(dy) {
            out.push(0);
            out.extend(cols.iter().map(|&x| px[y * w + x]));
        }
    }
    out
}

fn decode(bytes: &[u8]) -> DecodedPng {
    decode_png(bytes, PngLimits::DEFAULT).expect("decodes")
}

#[test]
fn fixture_crc_matches_the_specification() {
    // Every PNG ends with this IEND chunk, whose CRC the specification's sample code yields.
    assert_eq!(crc32(b"IEND"), 0xAE42_6082);
}

// Views.

#[test]
fn invalid_view_length_is_error() {
    assert!(matches!(Rgba8::new(2, 2, &[0; 15]), Err(ImageError::InvalidBufferLength { expected: 16, actual: 15 })));
    assert!(matches!(Rgba8::new(2, 2, &[0; 17]), Err(ImageError::InvalidBufferLength { expected: 16, actual: 17 })));
    assert!(Rgba8::new(2, 2, &[0; 16]).is_ok());
}

#[test]
fn rgba_length_overflow_is_error() {
    for result in [Rgba8::new(u32::MAX, u32::MAX, &[]), Rgba8::new(u32::MAX, u32::MAX, &[0; 4])] {
        assert!(matches!(result, Err(ImageError::InvalidDimensions { .. } | ImageError::InvalidBufferLength { .. })), "{result:?}");
    }
}

#[test]
fn a_buffer_that_is_not_the_stated_size_is_rejected() {
    let buf = gray(4, 4, 0);
    assert!(Gray8::new(4, 4, &buf).is_ok());
    assert!(Gray8::new(5, 4, &buf).is_err());
    assert!(Gray8::new(4, 5, &buf).is_err());
    assert!(Gray8::new(u32::MAX, u32::MAX, &buf).is_err(), "the area must not overflow into a match");
    // A zero-sized image has no samples for p99 or PSNR, and PNG does not allow one: it is an error.
    assert!(matches!(Gray8::new(0, 0, &[]), Err(ImageError::InvalidDimensions { width: 0, height: 0 })));
}

/// A zero-width image of enormous height is rejected at once, without walking its rows.
#[test]
fn an_image_without_pixels_compares_at_once() {
    assert!(matches!(Gray8::new(0, u32::MAX, &[]), Err(ImageError::InvalidDimensions { .. })));
    assert!(matches!(Rgba8::new(u32::MAX, 0, &[]), Err(ImageError::InvalidDimensions { .. })));
}

// Statistics.

#[test]
fn diff_metrics() {
    let a = ramp(10, 10, 255);
    assert_eq!(diff_rgba(rgba(10, 10, &a), rgba(10, 10, &a)).unwrap().psnr, f64::INFINITY);
    let mut b = a.clone();
    b[0] = b[0].wrapping_add(10); // one channel of one pixel
    let d = diff_rgba(rgba(10, 10, &a), rgba(10, 10, &b)).unwrap();
    assert_eq!((d.max_abs, d.p99, d.differing_pixels, d.total_pixels), (10, 0, 1, 100));
    assert!((d.mean_abs - 10.0 / 400.0).abs() < 1e-12);
    let mse: f64 = 100.0 / 400.0;
    assert!((d.psnr - 10.0 * (255.0f64 * 255.0 / mse).log10()).abs() < 1e-9);
    assert!(compare_rgba(rgba(10, 10, &a), rgba(10, 10, &b), render()).unwrap().passed());
    assert!(!compare_rgba(rgba(10, 10, &a), rgba(10, 10, &b), Tolerance::EXACT).unwrap().passed());
    let taller = ramp(10, 11, 255);
    assert!(diff_rgba(rgba(10, 10, &a), rgba(10, 11, &taller)).is_err());
}

#[test]
fn p99_nearest_rank_small_samples() {
    // One pixel: rank ceil(0.99) = 1.
    let (a, b) = ([0u8], [7u8]);
    assert_eq!(diff_gray(g(1, 1, &a), g(1, 1, &b)).unwrap().p99, 7);
    // 100 pixels: rank 99. One changed pixel sits at position 100, two reach position 99.
    let base = gray(100, 1, 0);
    let mut one = base.clone();
    one[0] = 10;
    assert_eq!(diff_gray(g(100, 1, &base), g(100, 1, &one)).unwrap().p99, 0);
    let mut two = one.clone();
    two[1] = 10;
    assert_eq!(diff_gray(g(100, 1, &base), g(100, 1, &two)).unwrap().p99, 10);
    // 101 pixels: rank ceil(99.99) = 100.
    let base = gray(101, 1, 0);
    let mut one = base.clone();
    one[0] = 10;
    assert_eq!(diff_gray(g(101, 1, &base), g(101, 1, &one)).unwrap().p99, 0);
    let mut two = one.clone();
    two[1] = 10;
    assert_eq!(diff_gray(g(101, 1, &base), g(101, 1, &two)).unwrap().p99, 10);
}

#[test]
fn psnr_published_two_sample_value() {
    // MSE = 255² / 2, so PSNR = 10 log10(2).
    let value = psnr(&[0u8, 0], &[255u8, 0], 255.0).unwrap();
    assert!((value - 3.010_299_956_6).abs() < 1e-9, "{value}");
}

#[test]
fn psnr_identical_is_infinite() {
    assert_eq!(psnr(&[1u8, 2, 3], &[1u8, 2, 3], 255.0).unwrap(), f64::INFINITY);
    assert_eq!(psnr(&[1000u16], &[1000u16], 1023.0).unwrap(), f64::INFINITY);
}

#[test]
fn psnr_rejects_unequal_empty_and_bad_peak() {
    assert!(matches!(psnr(&[0u8, 1], &[0u8], 255.0), Err(ImageError::PlaneLengthMismatch { a: 2, b: 1 })));
    assert!(matches!(psnr::<u8>(&[], &[], 255.0), Err(ImageError::EmptyPlane)));
    for peak in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(psnr(&[0u8], &[1u8], peak), Err(ImageError::InvalidPeak(_))), "{peak}");
    }
}

#[test]
fn psnr_u16_peak() {
    let value = psnr(&[0u16, 0], &[1023u16, 0], 1023.0).unwrap();
    assert!((value - 10.0 * 2f64.log10()).abs() < 1e-9, "{value}");
    assert!(psnr(&[0u16], &[65535u16], 65535.0).unwrap().abs() < 1e-12);
    // peak² does not fit in f64 here, but the result does: 20 × 200 dB.
    let value = psnr(&[0u8], &[1u8], 1e200).unwrap();
    assert!((value - 4000.0).abs() < 1e-9, "{value}");
}

#[test]
fn diff_image_rejects_size_mismatch() {
    let a = [10u8, 20, 30, 255, 0, 0, 0, 255];
    let b = [11u8, 20, 30, 250, 0, 100, 0, 255];
    let d = diff_image(rgba(2, 1, &a), rgba(2, 1, &b)).unwrap();
    // Red differs by 1 (8 after scaling), alpha by 5 (40), which shows in every color channel.
    assert_eq!(d, vec![40, 40, 40, 255, 0, 255, 0, 255]);
    assert!(matches!(diff_image(rgba(2, 1, &a), rgba(1, 2, &b)), Err(ImageError::SizeMismatch { .. })));
}

// Tolerances and the neighborhood.

#[test]
fn identical_images_do_not_differ() {
    let buf = gray(16, 16, 128);
    let v = compare_gray(g(16, 16, &buf), g(16, 16, &buf), neighborhood(0)).expect("same size");
    assert_eq!(v.bad_pixels, 0);
    assert_eq!(v.stats.max_abs, 0);
    assert_eq!(v.stats.differing_pixels, 0);
    assert_eq!(v.stats.total_pixels, 256);
}

/// A one-pixel shift of a stem is forgiven, because every moved pixel finds its match next door.
#[test]
fn a_one_pixel_shift_is_forgiven() {
    let (w, h) = (9usize, 9usize);
    let mut left = gray(w, h, 255);
    let mut right = gray(w, h, 255);
    for y in 0..h {
        left[y * w + 4] = 0;
        right[y * w + 5] = 0;
    }
    let v = compare_gray(g(w, h, &left), g(w, h, &right), neighborhood(8)).expect("same size");
    assert_eq!(v.bad_pixels, 0, "a one-pixel shift must not count as a difference");
    // The aligned difference still shows the shift.
    assert_eq!(v.stats.max_abs, 255);
}

/// Content that is absent is not forgiven, however tolerant the neighborhood is.
#[test]
fn missing_content_is_not_forgiven() {
    let (w, h) = (16usize, 16usize);
    let blank = gray(w, h, 255);
    let mut drawn = blank.clone();
    for y in 5..11 {
        for x in 5..11 {
            drawn[y * w + x] = 0;
        }
    }
    let v = compare_gray(g(w, h, &blank), g(w, h, &drawn), neighborhood(8)).expect("same size");
    // Blank against drawn forgives the block's border and counts its 4x4 interior; drawn
    // against blank counts all 36 black pixels. The larger count is reported.
    assert_eq!(v.bad_pixels, 36, "every pixel of a missing block must be counted");
}

/// A shift of more than one pixel is beyond the 3x3 window and is reported.
#[test]
fn a_shift_wider_than_the_window_is_reported() {
    let (w, h) = (16usize, 16usize);
    let mut left = gray(w, h, 255);
    let mut right = gray(w, h, 255);
    for y in 0..h {
        for x in 4..8 {
            left[y * w + x] = 0;
        }
        for x in 11..15 {
            right[y * w + x] = 0;
        }
    }
    assert!(compare_gray(g(w, h, &left), g(w, h, &right), neighborhood(8)).expect("same size").bad_pixels > 0);
}

#[test]
fn tolerance_absorbs_small_differences_only() {
    let flat = gray(8, 8, 100);
    let near = gray(8, 8, 105);
    let far = gray(8, 8, 200);
    let a = g(8, 8, &flat);
    assert_eq!(compare_gray(a, g(8, 8, &near), neighborhood(8)).expect("ok").bad_pixels, 0);
    assert_eq!(compare_gray(a, g(8, 8, &far), neighborhood(8)).expect("ok").bad_pixels, 64);
    // The tolerance is inclusive at the boundary.
    assert_eq!(compare_gray(a, g(8, 8, &near), neighborhood(5)).expect("ok").bad_pixels, 0);
    assert_eq!(compare_gray(a, g(8, 8, &near), neighborhood(4)).expect("ok").bad_pixels, 64);
}

/// Comparison is symmetric: neither argument order flatters one image.
#[test]
fn comparison_is_symmetric() {
    let (w, h) = (12usize, 12usize);
    let mut thin = gray(w, h, 255);
    let mut thick = gray(w, h, 255);
    for y in 0..h {
        thin[y * w + 6] = 0;
        for x in 4..9 {
            thick[y * w + x] = 0;
        }
    }
    let forward = compare_gray(g(w, h, &thin), g(w, h, &thick), neighborhood(8)).expect("ok");
    let backward = compare_gray(g(w, h, &thick), g(w, h, &thin), neighborhood(8)).expect("ok");
    assert_eq!(forward, backward, "argument order must not change the verdict");
    assert!(forward.bad_pixels > 0, "a 1px stem against a 5px bar is a real difference");
}

#[test]
fn neighborhood_clips_at_edges() {
    // Only the edge pixels differ, and their windows stop at the image border.
    let (a, b) = ([0u8, 255, 255], [255u8, 255, 0]);
    assert_eq!(compare_gray(g(3, 1, &a), g(3, 1, &b), neighborhood(0)).unwrap().bad_pixels, 1);
    let (a, b) = ([0u8], [1u8]);
    assert_eq!(compare_gray(g(1, 1, &a), g(1, 1, &b), neighborhood(0)).unwrap().bad_pixels, 1);
    assert_eq!(compare_gray(g(1, 1, &a), g(1, 1, &b), neighborhood(1)).unwrap().bad_pixels, 0);
    // RGBA pixels match only when every channel is within the tolerance.
    let a = [0u8, 0, 0, 255, 9, 9, 9, 255];
    let b = [0u8, 0, 0, 200, 9, 9, 9, 200];
    assert_eq!(compare_rgba(rgba(2, 1, &a), rgba(2, 1, &b), neighborhood(54)).unwrap().bad_pixels, 2);
    assert_eq!(compare_rgba(rgba(2, 1, &a), rgba(2, 1, &b), neighborhood(55)).unwrap().bad_pixels, 0);
}

#[test]
fn bad_pixel_fraction_is_inclusive() {
    let base = gray(1000, 1, 0);
    let mut off = base.clone();
    off[0] = 9;
    off[1] = 9;
    let t = Tolerance { max_abs: None, channel: 3, max_bad_fraction: Some(0.002), ..Tolerance::EXACT };
    let v = compare_gray(g(1000, 1, &base), g(1000, 1, &off), t).unwrap();
    assert_eq!((v.bad_pixels, v.allowed_bad_pixels), (2, Some(2)));
    assert!(v.passed());
    off[2] = 4;
    let v = compare_gray(g(1000, 1, &base), g(1000, 1, &off), t).unwrap();
    assert_eq!(v.bad_pixels, 3);
    assert!(!v.passed());
    // 0.29 × 100 is 28.999999999999996 in f64; 29 of 100 pixels is still 29%.
    let base = gray(100, 1, 0);
    let mut off = base.clone();
    off[..29].fill(9);
    let t = Tolerance { max_bad_fraction: Some(0.29), ..t };
    let v = compare_gray(g(100, 1, &base), g(100, 1, &off), t).unwrap();
    assert_eq!((v.bad_pixels, v.allowed_bad_pixels), (29, Some(29)));
    assert!(v.passed());
}

#[test]
fn invalid_tolerance_is_error() {
    let a = [0u8];
    for t in [
        Tolerance { max_bad_fraction: Some(1.5), ..Tolerance::EXACT },
        Tolerance { max_bad_fraction: Some(-0.1), ..Tolerance::EXACT },
        Tolerance { max_bad_fraction: Some(f64::NAN), ..Tolerance::EXACT },
        Tolerance { min_psnr: Some(f64::NAN), ..Tolerance::EXACT },
    ] {
        assert!(matches!(compare_gray(g(1, 1, &a), g(1, 1, &a), t), Err(ImageError::InvalidTolerance(_))), "{t:?}");
    }
}

#[test]
fn different_sizes_are_an_error_not_a_score() {
    let small = gray(4, 4, 0);
    let large = gray(8, 8, 0);
    let err = compare_gray(g(4, 4, &small), g(8, 8, &large), Tolerance::EXACT).expect_err("mismatch");
    assert!(matches!(err, ImageError::SizeMismatch { a: (4, 4), b: (8, 8) }), "{err:?}");
    assert!(err.to_string().contains("4x4"), "{err}");
}

// PNG.

#[test]
fn png_round_trip_rgb_and_rgba() {
    for alpha in [255, 128] {
        let px = ramp(37, 21, alpha);
        let png = encode_png(rgba(37, 21, &px), PngLimits::DEFAULT).unwrap();
        assert_eq!(decode(&png), DecodedPng::Rgba { width: 37, height: 21, pixels: px });
    }
}

#[test]
fn gray_png_round_trip() {
    let px: Vec<u8> = (0..60).map(|i| (i * 4) as u8).collect();
    let png = encode_gray_png(g(10, 6, &px), PngLimits::DEFAULT).unwrap();
    assert_eq!(decode(&png), DecodedPng::Gray { width: 10, height: 6, pixels: px });
}

#[test]
fn encoded_header_matches_the_png_specification() {
    let opaque = ramp(3, 2, 255);
    let clear = ramp(3, 2, 7);
    let shades = [1u8, 2, 3, 4, 5, 6];
    for (png, color_type) in [
        (encode_png(rgba(3, 2, &opaque), PngLimits::DEFAULT).unwrap(), 2u8),
        (encode_png(rgba(3, 2, &clear), PngLimits::DEFAULT).unwrap(), 6),
        (encode_gray_png(g(3, 2, &shades), PngLimits::DEFAULT).unwrap(), 0),
    ] {
        assert_eq!(png[..8], SIGNATURE);
        assert_eq!(png[8..16], [0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        // Width, height, bit depth, color type, compression, filter and interlace methods.
        assert_eq!(png[16..29], [0, 0, 0, 3, 0, 0, 0, 2, 8, color_type, 0, 0, 0]);
        assert_eq!(png[29..33], crc32(&png[12..29]).to_be_bytes());
    }
}

#[test]
fn png_normalizes_supported_color_types() {
    // Gray, 1 bit: 1 scales to 255.
    let png = png_file(8, 1, 1, 0, 0, &[], &[0, 0b1010_0000]);
    assert_eq!(decode(&png), DecodedPng::Gray { width: 8, height: 1, pixels: vec![255, 0, 255, 0, 0, 0, 0, 0] });
    // Gray, 2 bits: steps of 85.
    let png = png_file(4, 1, 2, 0, 0, &[], &[0, 0b00_01_10_11]);
    assert_eq!(decode(&png), DecodedPng::Gray { width: 4, height: 1, pixels: vec![0, 85, 170, 255] });
    // Gray with alpha.
    let png = png_file(2, 1, 8, 4, 0, &[], &[0, 10, 20, 30, 40]);
    assert_eq!(decode(&png), DecodedPng::Rgba { width: 2, height: 1, pixels: vec![10, 10, 10, 20, 30, 30, 30, 40] });
    // RGB.
    let png = png_file(1, 1, 8, 2, 0, &[], &[0, 1, 2, 3]);
    assert_eq!(decode(&png), DecodedPng::Rgba { width: 1, height: 1, pixels: vec![1, 2, 3, 255] });
    // RGBA.
    let png = png_file(1, 1, 8, 6, 0, &[], &[0, 1, 2, 3, 4]);
    assert_eq!(decode(&png), DecodedPng::Rgba { width: 1, height: 1, pixels: vec![1, 2, 3, 4] });
    // Palette with a tRNS entry for its first color.
    let png = png_file(2, 1, 8, 3, 0, &[(b"PLTE", &[255, 0, 0, 0, 0, 255]), (b"tRNS", &[128])], &[0, 0, 1]);
    assert_eq!(decode(&png), DecodedPng::Rgba { width: 2, height: 1, pixels: vec![255, 0, 0, 128, 0, 0, 255, 255] });
    // Gray with a tRNS value of 7.
    let png = png_file(2, 1, 8, 0, 0, &[(b"tRNS", &[0, 7])], &[0, 7, 9]);
    assert_eq!(decode(&png), DecodedPng::Rgba { width: 2, height: 1, pixels: vec![7, 7, 7, 0, 9, 9, 9, 255] });
}

#[test]
fn png_16bit_strips_to_8bit() {
    let png = png_file(1, 2, 16, 0, 0, &[], &[0, 0x12, 0x34, 0, 0xAB, 0xCD]);
    assert_eq!(decode(&png), DecodedPng::Gray { width: 1, height: 2, pixels: vec![0x12, 0xAB] });
    let png = png_file(1, 1, 16, 6, 0, &[], &[0, 1, 2, 3, 4, 5, 6, 0xFF, 0]);
    assert_eq!(decode(&png), DecodedPng::Rgba { width: 1, height: 1, pixels: vec![1, 3, 5, 255] });
}

#[test]
fn interlaced_png_decodes_complete_frame() {
    let (w, h) = (5usize, 5usize);
    let px: Vec<u8> = (0..w * h).map(|i| (i * 10) as u8).collect();
    let png = png_file(5, 5, 8, 0, 1, &[], &adam7_gray(w, h, &px));
    assert_eq!(decode(&png), DecodedPng::Gray { width: 5, height: 5, pixels: px });
}

#[test]
fn wrong_signature_and_truncated_png_are_errors() {
    let px = ramp(4, 4, 255);
    let png = encode_png(rgba(4, 4, &px), PngLimits::DEFAULT).unwrap();
    let mut wrong = png.clone();
    wrong[1] = b'Q';
    assert!(matches!(decode_png(&wrong, PngLimits::DEFAULT), Err(ImageError::Decode(_))));
    assert!(matches!(decode_png(&[], PngLimits::DEFAULT), Err(ImageError::Decode(_))));
    // Cut inside the header, and just after the zlib header of the image data.
    let idat = png.windows(4).position(|w| w == b"IDAT").expect("an IDAT chunk");
    for len in [20, idat + 6] {
        assert!(matches!(decode_png(&png[..len], PngLimits::DEFAULT), Err(ImageError::Decode(_))), "{len}");
    }
}

#[test]
fn png_without_a_valid_end_is_an_error() {
    let png = png_file(2, 1, 8, 0, 0, &[], &[0, 1, 2]);
    assert!(decode_png(&png, PngLimits::DEFAULT).is_ok());
    // No IEND chunk.
    let cut = &png[..png.len() - 12];
    assert!(matches!(decode_png(cut, PngLimits::DEFAULT), Err(ImageError::Decode(_))));
    // An IEND chunk with a wrong CRC.
    let mut bad_crc = png.clone();
    if let Some(last) = bad_crc.last_mut() {
        *last ^= 0xFF;
    }
    assert!(matches!(decode_png(&bad_crc, PngLimits::DEFAULT), Err(ImageError::Decode(_))));
}

#[test]
fn png_limits_reject_header_before_allocation() {
    // The image data is a few bytes, so only the header can trigger the error.
    let wide = png_file(1_000_000, 1, 8, 0, 0, &[], &[0]);
    assert!(matches!(decode_png(&wide, PngLimits::DEFAULT), Err(ImageError::LimitExceeded(_))));
    let tall = png_file(1, 1_000_000, 8, 0, 0, &[], &[0]);
    assert!(matches!(decode_png(&tall, PngLimits::DEFAULT), Err(ImageError::LimitExceeded(_))));
    // 16,384 × 16,384 RGBA is 1 GiB decoded, over the 256 MiB default.
    let big = png_file(16_384, 16_384, 8, 6, 0, &[], &[0]);
    assert!(matches!(decode_png(&big, PngLimits::DEFAULT), Err(ImageError::LimitExceeded(_))));
    let narrow = PngLimits { max_width: 3, ..PngLimits::DEFAULT };
    let px = ramp(4, 1, 255);
    assert!(matches!(encode_png(rgba(4, 1, &px), narrow), Err(ImageError::LimitExceeded(_))));
}

#[test]
fn png_input_byte_limit() {
    let px = ramp(4, 4, 255);
    let png = encode_png(rgba(4, 4, &px), PngLimits::DEFAULT).unwrap();
    let exact = PngLimits { max_input_bytes: png.len(), ..PngLimits::DEFAULT };
    assert!(decode_png(&png, exact).is_ok());
    let short = PngLimits { max_input_bytes: png.len() - 1, ..PngLimits::DEFAULT };
    assert!(matches!(decode_png(&png, short), Err(ImageError::LimitExceeded(_))));
    assert!(matches!(encode_png(rgba(4, 4, &px), short), Err(ImageError::LimitExceeded(_))));
}

#[test]
fn png_decoded_byte_limit() {
    // 4x4 RGB: the decoder's output is 48 bytes, and 64 once converted to RGBA.
    let scanlines: Vec<u8> = (0..4).flat_map(|_| std::iter::once(0).chain(std::iter::repeat_n(9, 12))).collect();
    let png = png_file(4, 4, 8, 2, 0, &[], &scanlines);
    let with = |max_decoded_bytes| decode_png(&png, PngLimits { max_decoded_bytes, ..PngLimits::DEFAULT });
    assert!(with(64).is_ok());
    assert!(matches!(with(50), Err(ImageError::LimitExceeded(_))), "checked after conversion");
    assert!(matches!(with(47), Err(ImageError::LimitExceeded(_))), "checked before allocation");
}

#[test]
fn encoding_refuses_an_image_that_would_not_read_back() {
    // An image is only written when the same limits can decode it again.
    let px = ramp(4, 4, 255);
    let fits = PngLimits { max_decoded_bytes: 64, ..PngLimits::DEFAULT };
    let png = encode_png(rgba(4, 4, &px), fits).unwrap();
    assert!(decode_png(&png, fits).is_ok());
    let small = PngLimits { max_decoded_bytes: 63, ..PngLimits::DEFAULT };
    assert!(matches!(encode_png(rgba(4, 4, &px), small), Err(ImageError::LimitExceeded(_))));
    let shades = gray(4, 4, 9);
    assert!(encode_gray_png(g(4, 4, &shades), PngLimits { max_decoded_bytes: 16, ..PngLimits::DEFAULT }).is_ok());
    assert!(matches!(
        encode_gray_png(g(4, 4, &shades), PngLimits { max_decoded_bytes: 15, ..PngLimits::DEFAULT }),
        Err(ImageError::LimitExceeded(_))
    ));
}

#[test]
fn encoding_stops_at_the_byte_limit() {
    // Noise does not compress, so the encoder would write about 64 KiB without the bound.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let px: Vec<u8> = (0..128 * 128 * 4)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 56) as u8
        })
        .collect();
    let tiny = PngLimits { max_input_bytes: 100, ..PngLimits::DEFAULT };
    let err = encode_png(rgba(128, 128, &px), tiny).unwrap_err();
    assert!(matches!(err, ImageError::LimitExceeded(_)), "{err:?}");
    assert!(encode_png(rgba(128, 128, &px), PngLimits::DEFAULT).unwrap().len() > 100);
    let mut sink = BoundedSink { out: Vec::new(), max: 100, exceeded: false };
    assert!(write_stream(&mut sink, 128, 128, png::ColorType::Rgba, &px).is_err());
    assert!(sink.exceeded && sink.out.len() <= 100, "{} bytes held", sink.out.len());
}

#[test]
fn write_error_is_result() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    let px = ramp(2, 2, 255);
    assert!(matches!(write_png(&file.join("x.png"), rgba(2, 2, &px), PngLimits::DEFAULT), Err(ImageError::Io { .. })));
    assert!(matches!(read_png(&dir.path().join("none.png"), PngLimits::DEFAULT), Err(ImageError::Io { .. })));
}

// Golden checks.

fn options<'a>(failures: &'a Path, name: &'a str) -> GoldenOptions<'a> {
    GoldenOptions { bless_var: NO_BLESS, ..GoldenOptions::new(failures, name) }
}

fn mismatch(result: Result<super::Verdict, ImageError>) -> Box<super::GoldenMismatch> {
    match result {
        Err(ImageError::GoldenMismatch(m)) => m,
        other => panic!("expected a mismatch, got {other:?}"),
    }
}

#[test]
fn check_golden_accepts_within_tolerance_and_reports_failures() {
    let dir = tempfile::tempdir().unwrap();
    let failures = dir.path().join("failures");
    let path = dir.path().join("ramp.png");
    let img = ramp(32, 16, 255);
    write_png(&path, rgba(32, 16, &img), PngLimits::DEFAULT).unwrap();
    assert!(check_golden(&path, rgba(32, 16, &img), Tolerance::EXACT, options(&failures, "ramp")).is_ok());
    let mut near = img.clone();
    near[5] += 1;
    assert!(check_golden(&path, rgba(32, 16, &near), render(), options(&failures, "ramp")).is_ok());
    let mut far = img.clone();
    far.iter_mut().step_by(4).for_each(|v| *v = 255 - *v);
    let e = check_golden(&path, rgba(32, 16, &far), render(), options(&failures, "ramp")).unwrap_err();
    assert!(e.to_string().contains("PSNR"), "{e}");
    let missing = check_golden(&dir.path().join("missing.png"), rgba(32, 16, &img), render(), options(&failures, "ramp")).unwrap_err();
    assert!(matches!(missing, ImageError::MissingReference { .. }) && missing.to_string().contains(NO_BLESS), "{missing}");
}

#[test]
fn the_check_passes_within_the_tolerance_and_fails_past_it() {
    let dir = tempfile::tempdir().unwrap();
    let failures = dir.path().join("failures");
    let golden = dir.path().join("g.png");
    let (w, h) = (20u32, 10u32);
    let base: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 100, 200, 255]).collect();
    write_png(&golden, rgba(w, h, &base), PngLimits::DEFAULT).unwrap();
    assert!(check_golden(&golden, rgba(w, h, &base), per_pixel(), options(&failures, "self-test-same")).is_ok());
    // Small differences everywhere are within the channel tolerance.
    let near: Vec<u8> = base.iter().map(|c| c.saturating_sub(2)).collect();
    assert!(check_golden(&golden, rgba(w, h, &near), per_pixel(), options(&failures, "self-test-near")).is_ok());
    // A whole row changed is not.
    let mut off = base.clone();
    for px in off.as_chunks_mut::<4>().0.iter_mut().take(w as usize) {
        px[1] = 0;
    }
    let m = mismatch(check_golden(&golden, rgba(w, h, &off), per_pixel(), options(&failures, "self-test-off")));
    assert_eq!(m.verdict.bad_pixels, 20);
    assert!(m.actual.exists() && m.diff.exists());
    assert!(m.diff.ends_with("self-test-off.diff.png"), "{}", m.diff.display());
    // A missing golden fails and says how to make one.
    let err = check_golden(&dir.path().join("none.png"), rgba(w, h, &base), per_pixel(), options(&failures, "self-test-missing")).unwrap_err();
    assert!(err.to_string().contains(&format!("{NO_BLESS}=1")), "{err}");
}

#[test]
fn bless_requires_exact_one() {
    assert!(is_bless_value(Some(OsStr::new("1"))));
    for value in ["", "0", "true", "yes", "on", " 1", "1 ", "01", "11"] {
        assert!(!is_bless_value(Some(OsStr::new(value))), "{value:?}");
    }
    assert!(!is_bless_value(None));
}

#[test]
fn missing_reference_fails_without_bless() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("refs").join("new.png");
    let px = ramp(4, 4, 255);
    let err = check_golden_with(&reference, rgba(4, 4, &px), Tolerance::EXACT, options(dir.path(), "new"), false).unwrap_err();
    assert!(matches!(err, ImageError::MissingReference { .. }), "{err:?}");
    assert!(!reference.exists(), "a normal run must not create a reference");
}

#[test]
fn bless_creates_missing_reference() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("refs").join("new.png");
    let px = ramp(4, 4, 200);
    check_golden_with(&reference, rgba(4, 4, &px), Tolerance::EXACT, options(dir.path(), "new"), true).unwrap();
    assert_eq!(read_png(&reference, PngLimits::DEFAULT).unwrap(), DecodedPng::Rgba { width: 4, height: 4, pixels: px });
}

#[test]
fn oversize_bless_leaves_reference_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("r.png");
    let old = ramp(8, 8, 255);
    write_png(&reference, rgba(8, 8, &old), PngLimits::DEFAULT).unwrap();
    let before = std::fs::read(&reference).unwrap();
    let new = ramp(8, 8, 9);
    let small = GoldenOptions { limits: PngLimits { max_input_bytes: 10, ..PngLimits::DEFAULT }, ..options(dir.path(), "r") };
    let err = check_golden_with(&reference, rgba(8, 8, &new), Tolerance::EXACT, small, true).unwrap_err();
    assert!(matches!(err, ImageError::LimitExceeded(_)), "{err:?}");
    assert_eq!(std::fs::read(&reference).unwrap(), before);
}

#[test]
fn gray_reference_expansion_respects_the_decoded_limit() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("g.png");
    let shades = gray(4, 4, 9);
    write_gray_png(&reference, g(4, 4, &shades), PngLimits::DEFAULT).unwrap();
    let px: Vec<u8> = shades.iter().flat_map(|&v| [v, v, v, 255]).collect();
    assert!(check_golden(&reference, rgba(4, 4, &px), Tolerance::EXACT, options(dir.path(), "g")).is_ok());
    // The 16-byte gray reference fits, but its 64-byte RGBA form does not.
    let small = GoldenOptions { limits: PngLimits { max_decoded_bytes: 16, ..PngLimits::DEFAULT }, ..options(dir.path(), "g") };
    let err = check_golden(&reference, rgba(4, 4, &px), Tolerance::EXACT, small).unwrap_err();
    assert!(matches!(err, ImageError::LimitExceeded(_)), "{err:?}");
    let wide = ramp(8, 2, 255);
    let err = check_golden(&reference, rgba(8, 2, &wide), Tolerance::EXACT, options(dir.path(), "g")).unwrap_err();
    assert!(matches!(err, ImageError::SizeMismatch { a: (4, 4), b: (8, 2) }), "{err:?}");
}

#[test]
fn blessing_replaces_a_reference_whole() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("r.png");
    let old = ramp(4, 4, 255);
    write_png(&reference, rgba(4, 4, &old), PngLimits::DEFAULT).unwrap();
    let new = ramp(4, 4, 99);
    check_golden_with(&reference, rgba(4, 4, &new), Tolerance::EXACT, options(dir.path(), "r"), true).unwrap();
    assert_eq!(read_png(&reference, PngLimits::DEFAULT).unwrap(), DecodedPng::Rgba { width: 4, height: 4, pixels: new });
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("r.png")], "no temporary file is left behind");
}

#[test]
fn failure_name_rejects_path_components() {
    let dir = tempfile::tempdir().unwrap();
    let failures = dir.path().join("failures");
    let reference = dir.path().join("r.png");
    let px = ramp(2, 2, 255);
    write_png(&reference, rgba(2, 2, &px), PngLimits::DEFAULT).unwrap();
    let other = ramp(2, 2, 0);
    let long = "a".repeat(101);
    for name in ["", "..", "../escape", "a/b", "a\\b", ".hidden", "C:x", "a b", "x\0", "trailing.", "CON", "nul.x", "com1", "LPT9.png", long.as_str()]
    {
        let err = check_golden(&reference, rgba(2, 2, &other), Tolerance::EXACT, options(&failures, name)).unwrap_err();
        assert!(matches!(err, ImageError::InvalidName(_)), "{name:?}: {err:?}");
    }
    assert!(!failures.exists() && !dir.path().join("escape.actual.png").exists());
    for name in ["ok-name_1.v2", "_x", "com0", "console"] {
        assert!(
            matches!(check_golden(&reference, rgba(2, 2, &other), Tolerance::EXACT, options(&failures, name)), Err(ImageError::GoldenMismatch(_))),
            "{name:?}"
        );
    }
    for var in ["", "A=B", "A\0B"] {
        let bad = GoldenOptions { bless_var: var, ..options(&failures, "ok") };
        assert!(matches!(check_golden(&reference, rgba(2, 2, &px), Tolerance::EXACT, bad), Err(ImageError::InvalidName(_))), "{var:?}");
    }
}

#[test]
fn failure_artifacts_stay_in_folder() {
    let dir = tempfile::tempdir().unwrap();
    let failures = dir.path().join("deep").join("failures");
    let reference = dir.path().join("r.png");
    let px = ramp(3, 3, 255);
    write_png(&reference, rgba(3, 3, &px), PngLimits::DEFAULT).unwrap();
    let other = ramp(3, 3, 1);
    let m = mismatch(check_golden(&reference, rgba(3, 3, &other), Tolerance::EXACT, options(&failures, "case")));
    assert_eq!(m.actual.parent(), Some(failures.as_path()));
    assert_eq!(m.diff.parent(), Some(failures.as_path()));
    assert_eq!(read_png(&m.actual, PngLimits::DEFAULT).unwrap(), DecodedPng::Rgba { width: 3, height: 3, pixels: other });
    let diff = read_png(&m.diff, PngLimits::DEFAULT).unwrap();
    assert_eq!((diff.width(), diff.height()), (3, 3));
}

#[test]
fn failure_artifacts_do_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("r.png");
    let px = ramp(3, 3, 255);
    write_png(&reference, rgba(3, 3, &px), PngLimits::DEFAULT).unwrap();
    let first_img = ramp(3, 3, 1);
    let second_img = ramp(3, 3, 2);
    let first = mismatch(check_golden(&reference, rgba(3, 3, &first_img), Tolerance::EXACT, options(dir.path(), "same")));
    let kept = std::fs::read(&first.actual).unwrap();
    let second = mismatch(check_golden(&reference, rgba(3, 3, &second_img), Tolerance::EXACT, options(dir.path(), "same")));
    assert_ne!(first.actual, second.actual);
    assert_ne!(first.diff, second.diff);
    assert!(second.actual.ends_with("same-1.actual.png"), "{}", second.actual.display());
    assert_eq!(std::fs::read(&first.actual).unwrap(), kept);
}

#[test]
fn parallel_failures_get_distinct_paths() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("r.png");
    let px = ramp(3, 3, 255);
    write_png(&reference, rgba(3, 3, &px), PngLimits::DEFAULT).unwrap();
    let other = ramp(3, 3, 1);
    let paths: Vec<PathBuf> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| mismatch(check_golden(&reference, rgba(3, 3, &other), Tolerance::EXACT, options(dir.path(), "race")))))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| {
                let m = h.join().expect("thread");
                [m.actual, m.diff]
            })
            .collect()
    });
    let distinct: HashSet<&PathBuf> = paths.iter().collect();
    assert_eq!(distinct.len(), 16);
    assert!(paths.iter().all(|p| p.exists()));
}

#[test]
fn assert_similar_returns_the_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let reference = dir.path().join("r.png");
    let px = ramp(3, 3, 255);
    write_png(&reference, rgba(3, 3, &px), PngLimits::DEFAULT).unwrap();
    let verdict = crate::assert_similar!(&reference, rgba(3, 3, &px), Tolerance::EXACT, options(dir.path(), "ok"));
    assert!(verdict.passed());
}

#[test]
#[should_panic(expected = "no reference image")]
fn assert_similar_fails_the_test() {
    let dir = tempfile::tempdir().unwrap();
    let px = ramp(3, 3, 255);
    crate::assert_similar!(&dir.path().join("none.png"), rgba(3, 3, &px), Tolerance::EXACT, options(dir.path(), "none"));
}

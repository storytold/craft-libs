//! Lossy WebP (our VP8 encoder) checked against an independent decoder (`image-webp`): the
//! files decode, look like the input (PSNR by quality), get smaller as quality drops and are far
//! smaller than lossless on photographic content; alpha and metadata travel in the container;
//! odd sizes and the limits behave.

use crate::common;
use common::*;
use craft_image_codecs::*;

fn lossy(q: u8) -> EncodeOptions {
    EncodeOptions { webp_lossless: false, webp_quality: q, ..Default::default() }
}

/// A smooth photograph-like image: gradients with a little grain.
fn photo(w: u32, h: u32, seed: u64) -> Image {
    synth(w, h, ChannelLayout::Rgb, SampleType::U8, seed, 0.03)
}

#[test]
fn decodes_with_the_right_size_and_looks_like_the_input() {
    let img = photo(97, 61, 3);
    for (q, min_psnr) in [(100u8, 40.0), (90, 36.0), (75, 33.0), (50, 30.0), (20, 26.0)] {
        let bytes = encode(&img, Format::WebP, &lossy(q)).unwrap();
        let back = decode(&bytes).unwrap_or_else(|e| panic!("q{q}: {e}"));
        assert_eq!(back.dimensions(), (97, 61));
        assert_eq!(back.layout(), ChannelLayout::Rgb);
        let p = psnr(&img, &back);
        assert!(p >= min_psnr, "quality {q}: PSNR {p:.1} dB below {min_psnr}");
    }
}

#[test]
fn smaller_at_lower_quality_and_much_smaller_than_lossless_on_photos() {
    let img = synth(256, 192, ChannelLayout::Rgb, SampleType::U8, 9, 0.08);
    let lossless = encode(&img, Format::WebP, &EncodeOptions::default()).unwrap().len();
    let sizes: Vec<usize> = [95u8, 80, 60, 40, 20].iter().map(|&q| encode(&img, Format::WebP, &lossy(q)).unwrap().len()).collect();
    for w in sizes.windows(2) {
        assert!(w[1] <= w[0], "sizes must not grow as quality drops: {sizes:?}");
    }
    assert!(sizes[1] * 4 < lossless, "q80 {} vs lossless {lossless}", sizes[1]);
    assert!(sizes[4] * 10 < lossless, "q20 {} vs lossless {lossless}", sizes[4]);
}

#[test]
fn flat_areas_cost_almost_nothing() {
    // A single colour: every macroblock is skipped; the file is dominated by the header.
    let img = Image::from_u8(640, 480, ChannelLayout::Rgb, vec![200u8; 640 * 480 * 3]).unwrap();
    let bytes = encode(&img, Format::WebP, &lossy(80)).unwrap();
    assert!(bytes.len() < 1200, "{} bytes", bytes.len());
    let back = decode(&bytes).unwrap();
    assert!(max_abs_diff(&img, &back) <= 3.0 / 255.0, "flat colour reproduced within rounding");
}

#[test]
fn alpha_is_kept_exactly_and_metadata_travels() {
    let mut img = synth(40, 30, ChannelLayout::Rgba, SampleType::U8, 5, 0.05);
    img.icc = Some(sample_icc(400));
    img.meta.exif = Some(sample_exif());
    img.meta.xmp = Some(SAMPLE_XMP.to_owned());
    let bytes = encode(&img, Format::WebP, &lossy(85)).unwrap();
    assert_eq!(&bytes[12..16], b"VP8X", "extended container");
    let back = decode(&bytes).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Rgba);
    for (a, b) in img.data().as_chunks::<4>().0.iter().zip(back.data().as_chunks::<4>().0) {
        assert_eq!(a[3], b[3], "alpha is stored losslessly");
    }
    assert!(psnr(&img, &back) > 30.0);
    assert_eq!(back.icc, img.icc);
    assert!(back.meta.xmp.is_some());
    assert!(back.meta.exif.is_some());
    // Fully opaque alpha is dropped rather than stored.
    let mut data = synth(40, 30, ChannelLayout::Rgba, SampleType::U8, 5, 0.0).data().to_vec();
    for px in data.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    let opaque = Image::from_u8(40, 30, ChannelLayout::Rgba, data).unwrap();
    let bytes = encode(&opaque, Format::WebP, &EncodeOptions { embed_icc: false, embed_metadata: false, ..lossy(85) }).unwrap();
    assert_eq!(&bytes[12..16], b"VP8 ");
    assert_eq!(decode(&bytes).unwrap().layout(), ChannelLayout::Rgb);
}

#[test]
fn gray_and_odd_sizes_and_tiny_images() {
    for (w, h) in [(1, 1), (2, 3), (17, 9), (33, 1), (1, 47), (16, 16), (31, 15)] {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            // Little noise: at these sizes the 2 × 2 chroma average alone would otherwise dominate.
            let img = synth(w, h, layout, SampleType::U8, u64::from(w * 100 + h), 0.02);
            let bytes = encode(&img, Format::WebP, &lossy(80)).unwrap_or_else(|e| panic!("{w}x{h} {layout:?}: {e}"));
            let back = decode(&bytes).unwrap_or_else(|e| panic!("{w}x{h} {layout:?}: {e}"));
            assert_eq!(back.dimensions(), (w, h));
            // Below 8 px the 2 × 2 chroma average of random saturated pixels is the whole error.
            if w >= 8 && h >= 8 {
                let p = psnr(&img.convert(back.layout(), SampleType::U8), &back);
                assert!(p > 24.0, "{w}x{h} {layout:?}: {p:.1} dB");
            }
        }
    }
}

#[test]
fn sixteen_bit_input_is_quantised_to_eight() {
    let img = synth(24, 24, ChannelLayout::Rgb, SampleType::U16, 2, 0.02);
    let bytes = encode(&img, Format::WebP, &lossy(90)).unwrap();
    let back = decode(&bytes).unwrap();
    assert!(psnr(&img.convert(ChannelLayout::Rgb, SampleType::U8), &back) > 34.0);
}

#[test]
fn dimension_limit_is_an_error_not_a_panic() {
    let img = Image::from_u8(16384, 1, ChannelLayout::Rgb, vec![0u8; 16384 * 3]).unwrap();
    let e = encode(&img, Format::WebP, &lossy(50)).unwrap_err();
    assert!(matches!(e, CodecError::Encode { .. }), "{e}");
    let img = Image::from_u8(16383, 1, ChannelLayout::Rgb, vec![0u8; 16383 * 3]).unwrap();
    let bytes = encode(&img, Format::WebP, &lossy(50)).unwrap();
    assert_eq!(decode(&bytes).unwrap().dimensions(), (16383, 1));
}

#[test]
fn hard_edges_and_text_like_content_keep_their_structure() {
    // Black text-like strokes on white: B_PRED sub-block modes keep edges sharp enough that the
    // thresholded result matches the source almost everywhere.
    let (w, h) = (128u32, 64u32);
    let mut data = vec![255u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let stroke = (x % 11 < 2 && y > 8 && y < 56) || (y % 13 < 2 && x > 4 && x < 124);
            if stroke {
                let i = ((y * w + x) * 3) as usize;
                data[i..i + 3].fill(0);
            }
        }
    }
    let img = Image::from_u8(w, h, ChannelLayout::Rgb, data).unwrap();
    let bytes = encode(&img, Format::WebP, &lossy(90)).unwrap();
    let back = decode(&bytes).unwrap();
    let wrong = img.data().as_chunks::<3>().0.iter().zip(back.data().as_chunks::<3>().0).filter(|(a, b)| (a[0] < 128) != (b[0] < 128)).count();
    let total = (w * h) as usize;
    assert!(wrong * 100 < total * 2, "{wrong} of {total} pixels flipped side of mid-gray");
    assert!(psnr(&img, &back) > 30.0, "{:.1}", psnr(&img, &back));
}

/// Integer-only test content, so the bytes do not depend on floating point: gradients with
/// xorshift grain, a flat band, and thin dark strokes, so the encoder picks both the whole-block
/// modes and the sub-block modes.
fn mixed(w: u32, h: u32, seed: u32) -> Image {
    let mut s = seed | 1;
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let stroke = (x % 23 < 2 && y % 40 > 6) || (y % 29 < 2 && x % 50 > 4);
            let flat = y * 4 / h.max(1) == 2;
            for c in 0..3u32 {
                let v = if stroke {
                    20
                } else if flat {
                    180 + c * 10
                } else {
                    ((x * (c + 1) * 3 + y * (3 - c) * 2) / 3 + (s >> (8 * c)) % 9) % 256
                };
                data.push(v as u8);
            }
        }
    }
    Image::from_u8(w, h, ChannelLayout::Rgb, data).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn encoder_output_bytes_are_unchanged() {
    // SHA-256 of each lossy WebP file as the encoder wrote it before the B_PRED trial stopped
    // copying the luma plane for every macroblock. Each file must also decode with `image-webp`.
    let cases: [(u32, u32, u8, &str); 10] = [
        (1, 1, 80, "039d134b25d490c799df0819880b52dd2650799b91a556fa8ec0a789c19abde9"),
        (2, 3, 80, "f6bba710b503a8b53c12a7704b8aba6e448d18ca7990e2f01b4e5e1870768c82"),
        (17, 9, 90, "e723e08ef9a8bf811f6dfbe034bcb4e4cdc9c18304750f1a1a6f29d6767549b2"),
        (31, 15, 50, "c3d15c6228e9fb1aae8fad4e6cb212eb8fee03b967b422d277ed0f6c6a745f40"),
        (33, 47, 75, "40b09871821c938e9d12d3bb24be36dffeb1d109b86654e9ab5013e75d9fc1a5"),
        (100, 75, 10, "1a0bc0e7dab44a41597fed103f195babe1db9e7e83afe703acd14dd7e444eafa"),
        (100, 75, 95, "d1088a31aeb0697ac6d153a6849cadfaf6d26be9166b55d0be8815aaafbac49a"),
        (257, 129, 80, "39f4229783ae45952e1000f6e8fd26f24dca8dcda0c19d5836eef37881a7e294"),
        (640, 480, 75, "7dedafa23f4f003c84f3898a77b2a3ffdeb59d0c42d0ae2a0470a6270cb19589"),
        (640, 480, 30, "ff32e3ee3c3bdf7b0e50815aabc84449bdeb771b5e6c5e5d5758e5e0fdcb8554"),
    ];
    let mut actual = Vec::new();
    for (w, h, q, _) in cases {
        let img = mixed(w, h, w * 31 + h);
        let bytes = encode(&img, Format::WebP, &lossy(q)).unwrap();
        let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(&bytes[..])).unwrap_or_else(|e| panic!("{w}x{h} q{q}: {e}"));
        assert_eq!(dec.dimensions(), (w, h));
        let mut rgb = vec![0u8; dec.output_buffer_size().unwrap()];
        dec.read_image(&mut rgb).unwrap_or_else(|e| panic!("{w}x{h} q{q}: {e}"));
        assert_eq!(rgb.len(), (w * h * 3) as usize);
        actual.push(sha256_hex(&bytes));
    }
    let expected: Vec<&str> = cases.iter().map(|c| c.3).collect();
    assert_eq!(actual, expected);
}

/// Encode timings for squares of growing size. Ignored by default; run in release:
///
/// ```sh
/// cargo test --release -p craft-image-codecs --test it -- webp_lossy::encode_time_by_size --ignored --nocapture
/// ```
#[test]
#[ignore]
fn encode_time_by_size() {
    for side in [512u32, 1024, 2048, 4096] {
        let img = mixed(side, side, 7);
        let start = std::time::Instant::now();
        let bytes = encode(&img, Format::WebP, &lossy(75)).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        println!("{side}x{side}: {ms:.0} ms, {:.1} ns per pixel, {} bytes", ms * 1e6 / f64::from(side * side), bytes.len());
    }
}

//! YCbCr TIFFs (PhotometricInterpretation 6), as scanners write them: JPEG-compressed with 2×2
//! chroma subsampling, and uncompressed with ReferenceBlackWhite and YCbCrCoefficients (#10316).
//! Every file here is hand-built by `common::tiffgen`.

use crate::common::sample_icc;
use crate::common::tiffgen::*;
use craft_image_codecs::*;

const COMPRESSION: u16 = 259;
const SUBSAMPLING: u16 = 530;
const YCBCR_COEFFICIENTS: u16 = 529;
const REFERENCE_BLACK_WHITE: u16 = 532;
const ICC_PROFILE: u16 = 34675;

/// A directory whose Compression is `c`.
fn compressed(mut d: Dir, c: u64) -> Dir {
    d.entries.retain(|e| e.0 != COMPRESSION);
    d.tag(COMPRESSION, SHORT, &[c])
}

/// A little-endian RATIONAL value for `tiffgen`.
fn rational(num: u32, den: u32) -> u64 {
    u64::from(num) | (u64::from(den) << 32)
}

/// The image's pixels as RGB bytes.
fn rgb(img: &Image) -> Vec<u8> {
    img.to_rgba8().as_chunks::<4>().0.iter().flat_map(|[r, g, b, _]| [*r, *g, *b]).collect()
}

/// A smooth colour test card, `w`×`h` RGB.
fn card(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            v.extend([(x * 255 / (w - 1)) as u8, (y * 255 / (h - 1)) as u8, (255 - x * 255 / (w - 1)) as u8]);
        }
    }
    v
}

/// A 4:2:0 baseline JPEG stream of `rgb`.
fn jpeg(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let img = Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, rgb.to_vec()).unwrap();
    encode(&img, Format::Jpeg, &EncodeOptions { jpeg_chroma_subsampling: true, ..Default::default() }).unwrap()
}

#[test]
fn jpeg_compressed_ycbcr_with_subsampled_chroma_reads_as_rgb() {
    let (w, h) = (32, 24);
    let stream = jpeg(&card(w, h), w, h);
    // The stream really is subsampled: the luma component samples 2×2 in the frame header.
    let sof = stream.windows(2).position(|m| m == [0xFF, 0xC0]).expect("baseline frame header");
    assert_eq!(stream[sof + 11], 0x22, "luma sampling factors");
    let dir = compressed(strips(w, h, 8, 3, 6, h, &stream), 7).tag(SUBSAMPLING, SHORT, &[2, 2]);
    let file = build(true, false, &[dir], &[0]).bytes;

    let img = decode(&file).unwrap();
    assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (w, h, ChannelLayout::Rgb, SampleType::U8));
    // The oracle: the same JPEG stream decoded to RGB by an independent decoder.
    let want = image::load_from_memory_with_format(&stream, image::ImageFormat::Jpeg).unwrap().to_rgb8().into_raw();
    let worst = rgb(&img).iter().zip(&want).map(|(g, o)| g.abs_diff(*o)).max().unwrap();
    assert!(worst <= 2, "largest difference from the oracle: {worst}");
}

#[test]
fn uncompressed_ycbcr_uses_the_default_conversion() {
    // Black, white, mid gray and JFIF's pure red, without subsampling.
    let px = [0, 128, 128, 255, 128, 128, 128, 128, 128, 76, 85, 255];
    let dir = strips(4, 1, 8, 3, 6, 1, &px).tag(SUBSAMPLING, SHORT, &[1, 1]);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(img.layout(), ChannelLayout::Rgb);
    assert_eq!(rgb(&img), [0, 0, 0, 255, 255, 255, 128, 128, 128, 254, 0, 0]);
}

#[test]
fn reference_black_white_sets_the_code_range() {
    // Studio range: Y 16–235, chroma 128 ± 112.
    let rbw = [rational(16, 1), rational(235, 1), rational(128, 1), rational(240, 1), rational(128, 1), rational(240, 1)];
    let dir = strips(2, 1, 8, 3, 6, 1, &[16, 128, 128, 235, 128, 128]).tag(SUBSAMPLING, SHORT, &[1, 1]).tag(REFERENCE_BLACK_WHITE, RATIONAL, &rbw);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(rgb(&img), [0, 0, 0, 255, 255, 255]);
}

#[test]
fn weights_that_do_not_sum_to_one_keep_grays_neutral() {
    // 0.25 each (sum 0.75): TIFF 6.0's full inverse would scale green's Y by (1 − Lr − Lb) / Lg
    // = 2 and give mid gray G 255 (clamped). Like libtiff, green keeps Y, so gray stays gray, and
    // the weights still set the chroma terms: R = Y + 1.5·Cr, B = Y + 1.5·Cb.
    let weights = [rational(1, 4), rational(1, 4), rational(1, 4)];
    let dir = strips(2, 1, 8, 3, 6, 1, &[128, 128, 128, 128, 64, 192]).tag(SUBSAMPLING, SHORT, &[1, 1]).tag(YCBCR_COEFFICIENTS, RATIONAL, &weights);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    // Green for the second pixel: 128 − (Lb·1.5/Lg)·Cb − (Lr·1.5/Lg)·Cr = 128 + 96 − 96.
    assert_eq!(rgb(&img), [128, 128, 128, 224, 128, 32]);
}

#[test]
fn ycbcr_coefficients_set_the_luma_weights() {
    // Weights 0.25, 0.5, 0.25 with Cb −64 and Cr +64: R = Y + 1.5·Cr, B = Y + 1.5·Cb, and G keeps Y
    // (BT.601 would give R 218 and B 15).
    let weights = [rational(1, 4), rational(1, 2), rational(1, 4)];
    let dir = strips(1, 1, 8, 3, 6, 1, &[128, 64, 192]).tag(SUBSAMPLING, SHORT, &[1, 1]).tag(YCBCR_COEFFICIENTS, RATIONAL, &weights);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(rgb(&img), [224, 128, 32]);
}

#[test]
fn ycbcr_tags_with_the_wrong_count_use_the_defaults() {
    // Four weights and seven black/white codes: both tags are malformed, so BT.601 and full range
    // apply (the weights alone would give 224, 128, 32).
    let weights = [rational(1, 4), rational(1, 2), rational(1, 4), rational(1, 4)];
    let rbw = [rational(16, 1), rational(235, 1), rational(128, 1), rational(240, 1), rational(128, 1), rational(240, 1), rational(0, 1)];
    let dir = strips(1, 1, 8, 3, 6, 1, &[128, 64, 192]).tag(SUBSAMPLING, SHORT, &[1, 1]).tag(YCBCR_COEFFICIENTS, RATIONAL, &weights).tag(
        REFERENCE_BLACK_WHITE,
        RATIONAL,
        &rbw,
    );
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!(rgb(&img), [218, 104, 15]);
}

#[test]
fn sixteen_bit_ycbcr_reads_as_rgb() {
    let px: Vec<u8> = [65535u16, 32768, 32768, 0, 32768, 32768].iter().flat_map(|v| v.to_le_bytes()).collect();
    let dir = strips(2, 1, 16, 3, 6, 1, &px).tag(SUBSAMPLING, SHORT, &[1, 1]);
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!((img.layout(), img.sample_type()), (ChannelLayout::Rgb, SampleType::U16));
    let n = img.to_normalized();
    assert!(n[..3].iter().all(|&v| v == 1.0) && n[3..].iter().all(|&v| v == 0.0), "{n:?}");
}

#[test]
#[cfg(target_pointer_width = "64")]
fn sixteen_bit_ycbcr_bands_match_whole_decode_with_a_missing_last_strip() {
    // On 32-bit targets max_alloc also bounds the whole image, so a small allocation cap
    // cannot force these tiny strips into separate output groups there.
    let (w, h, rows) = (4u32, 5u32, 2u32);
    let pixels = [[0u16, 32768, 32768], [65535, 32768, 32768], [19595, 21710, 65535], [32768, 40960, 24576]];
    let samples: Vec<u16> = (0..w * h).flat_map(|i| pixels[i as usize % pixels.len()]).collect();
    let weights = [rational(1, 4), rational(1, 2), rational(1, 4)];
    let icc = sample_icc(512);
    for little in [true, false] {
        for missing in [false, true] {
            let dir = strips(w, h, 16, 3, 6, rows, &u16s(&samples, little))
                .tag(SUBSAMPLING, SHORT, &[1, 1])
                .tag(YCBCR_COEFFICIENTS, RATIONAL, &weights)
                .bytes(ICC_PROFILE, UNDEFINED, &icc);
            let mut file = build(little, false, &[dir], &[0]);
            if missing {
                let past = file.bytes.len() as u64 + 4096;
                patch_value(&mut file, little, false, 0, 273, 2, past);
            }
            let want = decode_with(&file.bytes, &DecodeOptions { keep_orientation: true, ..Default::default() }).unwrap();
            let limits = Limits { max_alloc: u64::from(w * rows * 3 * 2), ..Limits::banded() };
            let mut bands = TiffBands::open(&file.bytes, None, &limits).unwrap();
            assert_eq!((bands.dimensions(), bands.layout(), bands.sample_type()), ((w, h), ChannelLayout::Rgb, SampleType::U16));
            let mut got = Image::new(w, h, bands.layout(), bands.sample_type()).unwrap();
            let mut positions = Vec::new();
            bands
                .decode(|x, y, group: &Image| -> Result<(), CodecError> {
                    positions.push((x, y, group.height()));
                    assert_eq!((x, group.width(), group.layout(), group.sample_type()), (0, w, ChannelLayout::Rgb, SampleType::U16));
                    let at = y as usize * w as usize * 3 * 2;
                    got.data_mut()[at..at + group.data().len()].copy_from_slice(group.data());
                    Ok(())
                })
                .unwrap();
            assert_eq!(positions, [(0, 0, 2), (0, 2, 2), (0, 4, 1)], "one output group per strip, including the short last strip");
            assert_eq!(got.data(), want.data(), "little endian: {little}, missing last strip: {missing}");
            assert_eq!(bands.icc, want.icc);
            assert_eq!(bands.meta, want.meta);
            assert_eq!(bands.warnings, want.warnings);
            assert_eq!(bands.warnings.iter().any(|w| matches!(w, DecodeWarning::Truncated { .. })), missing);
        }
    }
}

#[test]
fn unsupported_ycbcr_files_fail_cleanly() {
    // Four samples (an extra sample next to YCbCr).
    let four = strips(1, 1, 8, 4, 6, 1, &[128, 128, 128, 255]).tag(SUBSAMPLING, SHORT, &[1, 1]);
    assert!(decode(&build(true, false, &[four], &[0]).bytes).is_err());
    // Subsampled chroma without JPEG compression (the default subsampling is 2×2).
    let sub = strips(2, 2, 8, 3, 6, 2, &[128; 12]);
    assert!(decode(&build(true, false, &[sub], &[0]).bytes).is_err());
    // A JPEG strip that is not JPEG data.
    let junk = compressed(strips(16, 16, 8, 3, 6, 16, &[0x5A; 64]), 7).tag(SUBSAMPLING, SHORT, &[2, 2]);
    assert!(decode(&build(true, false, &[junk], &[0]).bytes).is_err());
    // Coefficients and ReferenceBlackWhite that would divide by zero fall back to the defaults.
    let bad = strips(1, 1, 8, 3, 6, 1, &[255, 128, 128])
        .tag(SUBSAMPLING, SHORT, &[1, 1])
        .tag(YCBCR_COEFFICIENTS, RATIONAL, &[rational(0, 1), rational(0, 0), rational(5, 1)])
        .tag(REFERENCE_BLACK_WHITE, RATIONAL, &[rational(7, 1); 6]);
    let img = decode(&build(true, false, &[bad], &[0]).bytes).unwrap();
    assert_eq!(&img.to_rgba8()[..3], &[255, 255, 255]);
}

#[test]
fn tiled_jpeg_ycbcr_keeps_its_icc_profile() {
    // Two 16×16 tiles, each its own 4:2:0 JPEG stream, and an embedded profile, like a scanner's
    // tiled output.
    let (w, h, t) = (32u32, 16u32, 16u32);
    let px = card(w, h);
    let tile = |tx: u32| -> Vec<u8> {
        (0..t)
            .flat_map(|y| {
                let at = ((y * w + tx * t) * 3) as usize;
                px[at..at + (t * 3) as usize].to_vec()
            })
            .collect()
    };
    let streams = [jpeg(&tile(0), t, t), jpeg(&tile(1), t, t)];
    let icc = sample_icc(512);
    let dir = Dir { entries: base(w, h, 8, 3, 6), chunks: streams.to_vec(), tiled: true };
    let dir = compressed(dir, 7).tag(322, SHORT, &[u64::from(t)]).tag(323, SHORT, &[u64::from(t)]).tag(SUBSAMPLING, SHORT, &[2, 2]).bytes(
        ICC_PROFILE,
        UNDEFINED,
        &icc,
    );
    let img = decode(&build(true, false, &[dir], &[0]).bytes).unwrap();
    assert_eq!((img.width(), img.height(), img.layout()), (w, h, ChannelLayout::Rgb));
    assert_eq!(img.icc.as_deref(), Some(icc.as_slice()), "the embedded profile is kept");
    // The oracle: each tile's stream decoded by an independent decoder, placed side by side.
    let tiles: Vec<Vec<u8>> =
        streams.iter().map(|s| image::load_from_memory_with_format(s, image::ImageFormat::Jpeg).unwrap().to_rgb8().into_raw()).collect();
    let row = (t * 3) as usize;
    let want: Vec<u8> = (0..t as usize).flat_map(|y| [&tiles[0][y * row..(y + 1) * row], &tiles[1][y * row..(y + 1) * row]].concat()).collect();
    let worst = rgb(&img).iter().zip(&want).map(|(g, o)| g.abs_diff(*o)).max().unwrap();
    assert!(worst <= 2, "largest difference from the oracle: {worst}");
}

#[test]
fn big_endian_sixteen_bit_ycbcr_reads_as_rgb() {
    // White, black, and red as full-range 16-bit YCbCr (Y 0.299·65535, Cb 32768 − 0.1687·65535, Cr max).
    let px = [65535u16, 32768, 32768, 0, 32768, 32768, 19595, 21710, 65535];
    let dir = strips(3, 1, 16, 3, 6, 1, &u16s(&px, false)).tag(SUBSAMPLING, SHORT, &[1, 1]);
    let file = build(false, false, &[dir], &[0]).bytes;
    assert_eq!(&file[..2], b"MM");
    let img = decode(&file).unwrap();
    assert_eq!((img.layout(), img.sample_type()), (ChannelLayout::Rgb, SampleType::U16));
    let n = img.to_normalized();
    let want = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    assert!(n.iter().zip(want).all(|(g, w)| (g - w).abs() < 2.0 / 255.0), "{n:?}");
}

#[test]
fn planar_ycbcr_reads_as_rgb_in_both_byte_orders() {
    // The pixels of the default-conversion test, one plane per component.
    let px = [0u8, 128, 128, 255, 128, 128, 128, 128, 128, 76, 85, 255];
    for little in [true, false] {
        let dir = Dir { entries: base(4, 1, 8, 3, 6), chunks: planar_strips(&px, 4, 1, 3, 1, 1), tiled: false }
            .tag(278, LONG, &[1])
            .tag(284, SHORT, &[2])
            .tag(SUBSAMPLING, SHORT, &[1, 1]);
        let img = decode(&build(little, false, &[dir], &[0]).bytes).unwrap();
        assert_eq!(rgb(&img), [0, 0, 0, 255, 255, 255, 128, 128, 128, 254, 0, 0], "little endian: {little}");
    }
}

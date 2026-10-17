//! The BMP readers against files made offline by a differential BMP generator (the files are the
//! fixtures; the generator is not part of this repository). Each `x.bmp` has an
//! `x.rgba`: the picture Pillow (or, where Pillow is known to be wrong, the generator's model) decodes,
//! top row first. 1, 4, 8, 24 and 32 bits, uncompressed and RLE, both row orders, info/V4/OS-2 headers,
//! palettes of every kind, and extra bytes before the pixels.

use craft_scan::{bmp, bmp_size, bw};
use std::path::PathBuf;

fn fixtures() -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("bmp");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "bmp")
            && let (Ok(bytes), Ok(rgba)) = (std::fs::read(&path), std::fs::read(path.with_extension("rgba")))
        {
            out.push((path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), bytes, rgba));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    u8::try_from((299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) + 500) / 1000).unwrap_or(255)
}

#[test]
fn the_fixture_set_is_there_and_small() {
    let all = fixtures();
    assert!(all.len() >= 100, "{} fixtures", all.len());
    assert!(all.iter().map(|(_, b, r)| b.len() + r.len()).sum::<usize>() < 200_000);
}

#[test]
fn to_rgba_matches_the_oracle_picture() {
    for (name, bytes, want) in fixtures() {
        let (w, h, rgba) = bmp::to_rgba(&bytes).unwrap_or_else(|| panic!("{name}: no picture"));
        assert_eq!(bmp_size(&bytes), Some((w, h)), "{name}: size");
        assert_eq!(rgba, want, "{name}: pixels");
    }
}

#[test]
fn the_luminance_reader_matches_the_oracle_and_leaves_one_bit_alone() {
    for (name, bytes, want) in fixtures() {
        let got = bw::bmp_luma(&bytes);
        let info = bmp::parse(&bytes).unwrap_or_else(|| panic!("{name}: header"));
        if info.bits == 1 || info.compression != bmp::Compression::Rgb {
            assert!(got.is_none(), "{name}: not the own reader's form");
            continue;
        }
        let (w, h, bits, pixels) = got.unwrap_or_else(|| panic!("{name}: no luminance"));
        assert_eq!((w, h, bits), (info.width, info.height, u32::from(info.bits)), "{name}");
        let expect: Vec<u8> = want.chunks(4).map(|p| luma(p[0], p[1], p[2])).collect();
        assert_eq!(pixels, expect, "{name}: luminance");
    }
}

#[test]
fn thresholding_matches_the_oracle_and_keeps_the_picture_upright() {
    for (name, bytes, want) in fixtures() {
        let info = bmp::parse(&bytes).unwrap_or_else(|| panic!("{name}: header"));
        for threshold in [1u8, 77, 128, 200, 255] {
            match bw::threshold_to_one_bit(&bytes, threshold, 200) {
                bw::Thresholded::AlreadyOneBit => assert_eq!(info.bits, 1, "{name}"),
                bw::Thresholded::Converted(one) => {
                    assert!(info.bits > 1, "{name}: a 1-bit page was converted");
                    assert_eq!((one.width, one.height, one.from_bits), (info.width, info.height, u32::from(info.bits)), "{name}");
                    let out = bmp::parse(&one.bytes).unwrap_or_else(|| panic!("{name}: output header"));
                    assert_eq!((out.bits, out.top_down), (1, false), "{name}: bottom-up 1-bit");
                    let (_, _, rgba) = bmp::to_rgba(&one.bytes).unwrap_or_else(|| panic!("{name}: output picture"));
                    for (i, px) in rgba.chunks(4).enumerate() {
                        let ink = luma(want[i * 4], want[i * 4 + 1], want[i * 4 + 2]) < threshold;
                        assert_eq!(px[0] == 0, ink, "{name}: pixel {i} at threshold {threshold}");
                    }
                }
                bw::Thresholded::Unreadable => panic!("{name}: unreadable"),
            }
        }
    }
}

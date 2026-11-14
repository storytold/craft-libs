//! TIFF "Save Image Pyramid" (#8827): reduced-resolution levels after the full image, each half
//! the size of the one before, that every reader skips when it opens the file.

use craft_image_codecs::*;

fn pyramid() -> EncodeOptions {
    EncodeOptions { tiff_pyramid: true, ..Default::default() }
}

/// An image of `layout`/`sample` where every sample is `v` (on the 0–1 scale).
fn solid(w: u32, h: u32, layout: ChannelLayout, sample: SampleType, v: f32) -> Image {
    let n = w as usize * h as usize * layout.channels();
    let data: Vec<u8> = match sample {
        SampleType::U8 => vec![(v * 255.0).round() as u8; n],
        SampleType::U16 => (0..n).flat_map(|_| ((v * 65535.0).round() as u16).to_ne_bytes()).collect(),
        SampleType::F32 => (0..n).flat_map(|_| v.to_ne_bytes()).collect(),
        SampleType::F16 => (0..n).flat_map(|_| f16::from_f32(v).to_bits().to_ne_bytes()).collect(),
    };
    Image::from_raw(w, h, layout, sample, data).unwrap()
}

fn sizes(bytes: &[u8]) -> Vec<(TiffPageKind, u32, u32)> {
    tiff_info(bytes).unwrap().pages.iter().map(|p| (p.kind, p.width, p.height)).collect()
}

#[test]
fn levels_halve_down_to_the_minimum() {
    use TiffPageKind::*;
    let img = solid(300, 200, ChannelLayout::Rgb, SampleType::U8, 0.5);
    let bytes = encode(&img, Format::Tiff, &pyramid()).unwrap();
    // 300 → 150 → 75 → 38: the first level no longer than 64 px is the last.
    assert_eq!(sizes(&bytes), [(Page, 300, 200), (ReducedResolution, 150, 100), (ReducedResolution, 75, 50), (ReducedResolution, 38, 25)]);
    // Opening the file opens the full image, and the levels are not extra pages.
    let back = decode(&bytes).unwrap();
    assert_eq!(back.dimensions(), (300, 200));
    assert_eq!(back.data(), img.data());
    assert!(!back.warnings.iter().any(|w| matches!(w, DecodeWarning::MorePages { .. })), "{:?}", back.warnings);
}

#[test]
fn off_by_default_and_none_for_small_images() {
    let img = solid(300, 200, ChannelLayout::Rgb, SampleType::U8, 0.5);
    assert_eq!(sizes(&encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap()).len(), 1);
    let small = solid(64, 10, ChannelLayout::Rgb, SampleType::U8, 0.5);
    assert_eq!(sizes(&encode(&small, Format::Tiff, &pyramid()).unwrap()).len(), 1);
    // A 1-px-wide strip still halves its long side.
    let strip = solid(1, 200, ChannelLayout::Gray, SampleType::U8, 0.5);
    let s = sizes(&encode(&strip, Format::Tiff, &pyramid()).unwrap());
    assert_eq!(s.iter().map(|&(_, w, h)| (w, h)).collect::<Vec<_>>(), [(1, 200), (1, 100), (1, 50)]);
}

#[test]
fn every_depth_layout_and_compression() {
    let layouts = [
        ChannelLayout::Gray,
        ChannelLayout::GrayA,
        ChannelLayout::Rgb,
        ChannelLayout::Rgba,
        ChannelLayout::Cmyk,
        ChannelLayout::CmykA,
        ChannelLayout::Lab,
    ];
    for layout in layouts {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for compression in [TiffCompression::None, TiffCompression::Lzw, TiffCompression::Deflate, TiffCompression::PackBits] {
                let img = solid(120, 70, layout, sample, 0.6);
                let opts = EncodeOptions { tiff_compression: compression, ..pyramid() };
                let bytes = encode(&img, Format::Tiff, &opts).unwrap_or_else(|e| panic!("{layout:?} {sample:?} {compression:?}: {e}"));
                let info = tiff_info(&bytes).unwrap();
                assert_eq!(info.pages.len(), 2, "{layout:?} {sample:?} {compression:?}");
                let level = decode_tiff_page(&bytes, 1, &DecodeOptions::default()).unwrap();
                assert_eq!(level.dimensions(), (60, 35));
                assert_eq!(level.layout(), decode(&bytes).unwrap().layout(), "{layout:?} {sample:?}");
                // A solid image stays the same colour at every level.
                let full = decode(&bytes).unwrap().convert(level.layout(), SampleType::F32);
                let level = level.convert(level.layout(), SampleType::F32);
                let (a, b) = (full.to_f32_samples().unwrap(), level.to_f32_samples().unwrap());
                for (c, v) in b.iter().take(level.layout().channels()).enumerate() {
                    assert!((v - a[c]).abs() < 1e-3, "{layout:?} {sample:?} channel {c}: {v} vs {}", a[c]);
                }
            }
        }
    }
}

/// A 128×128 image of 2×2 `tile`s (top left, top right, bottom left, bottom right; alpha last),
/// and the first pixel of its first reduced level.
fn first_level_pixel(layout: ChannelLayout, tile: [&[u8]; 4]) -> Vec<u8> {
    let mut data = Vec::new();
    for _ in 0..64 {
        for x in 0..128 {
            data.extend_from_slice(tile[x % 2]);
        }
        for x in 0..128 {
            data.extend_from_slice(tile[2 + x % 2]);
        }
    }
    let img = Image::from_raw(128, 128, layout, SampleType::U8, data).unwrap();
    let bytes = encode(&img, Format::Tiff, &pyramid()).unwrap();
    let level = decode_tiff_page(&bytes, 1, &DecodeOptions::default()).unwrap();
    assert_eq!(level.dimensions(), (64, 64), "{layout:?}");
    assert_eq!(level.layout(), layout);
    level.data()[..layout.channels()].to_vec()
}

#[test]
fn colour_is_weighted_by_alpha() {
    // One opaque pixel and three transparent ones of another colour: the level keeps the opaque
    // colour at a quarter alpha. Unweighted, the transparent colour would bleed in (pink, not red).
    assert_eq!(
        first_level_pixel(ChannelLayout::Rgba, [&[255, 0, 0, 255], &[255, 255, 255, 0], &[255, 255, 255, 0], &[255, 255, 255, 0]]),
        [255, 0, 0, 64]
    );
    // Gray and CMYK reach the same path with other channel counts.
    assert_eq!(first_level_pixel(ChannelLayout::GrayA, [&[40, 255], &[255, 0], &[255, 0], &[255, 0]]), [40, 64]);
    assert_eq!(first_level_pixel(ChannelLayout::CmykA, [&[200, 10, 30, 90, 255], &[0; 5], &[0; 5], &[0; 5]]), [200, 10, 30, 90, 64]);
}

#[test]
fn colour_is_weighted_between_translucent_pixels() {
    // Two half-transparent pixels of different colours (red, blue) and two clear ones: the level
    // averages the two by their alpha (purple), where copying either pixel would give red or blue.
    let tile: [&[u8]; 4] = [&[255, 0, 0, 128], &[0, 0, 255, 128], &[0; 4], &[0; 4]];
    assert_eq!(first_level_pixel(ChannelLayout::Rgba, tile), [128, 0, 128, 64]);
}

#[test]
fn layered_and_tagged_data_stay_on_the_full_image() {
    let mut img = solid(200, 100, ChannelLayout::Rgb, SampleType::U8, 0.25);
    img.icc = Some(photocraft_cms_free_profile());
    img.meta.photoshop_layers = Some(vec![1, 2, 3, 4]);
    img.meta.extra_samples = vec![ExtraSample { kind: 0, data: vec![9; 200 * 100] }];
    let bytes = encode(&img, Format::Tiff, &pyramid()).unwrap();
    let info = tiff_info(&bytes).unwrap();
    assert_eq!(info.pages.len(), 3);
    // Only the full image carries the layers and the extra samples.
    assert_eq!(info.pages[0].samples_per_pixel, 4);
    assert!(info.pages[1..].iter().all(|p| p.samples_per_pixel == 3));
    assert_eq!(tiff_photoshop_tags(&bytes).1, Some(&[1u8, 2, 3, 4][..]));
    let level = decode_tiff_page(&bytes, 1, &DecodeOptions::default()).unwrap();
    assert_eq!(level.icc, img.icc, "levels keep the profile so they show the same colours");
    assert!(level.meta.extra_samples.is_empty());
}

/// Any bytes a decoder keeps as the ICC profile: the codec stores it without parsing.
fn photocraft_cms_free_profile() -> Vec<u8> {
    (0..=255u8).collect()
}

#[test]
fn levels_average_exactly_at_every_depth() {
    // One 2×2 block of gray values 0, 0.25, 0.5, 1 averages to 0.4375; a 2×2 block with half
    // transparent pixels keeps only the opaque colour and averages the alpha.
    for sample in [SampleType::U16, SampleType::F32] {
        let vals = [0.0f32, 0.25, 0.5, 1.0];
        let mut img = solid(130, 2, ChannelLayout::Gray, sample, 0.0);
        let w = 130;
        for (i, v) in vals.iter().enumerate() {
            let (x, y) = (i % 2, i / 2);
            let at = (y * w + x) * sample.bytes();
            let bytes: Vec<u8> = match sample {
                SampleType::U16 => ((v * 65535.0).round() as u16).to_ne_bytes().to_vec(),
                _ => v.to_ne_bytes().to_vec(),
            };
            img.data_mut()[at..at + bytes.len()].copy_from_slice(&bytes);
        }
        let bytes = encode(&img, Format::Tiff, &pyramid()).unwrap();
        let level = decode_tiff_page(&bytes, 1, &DecodeOptions::default()).unwrap().convert(ChannelLayout::Gray, SampleType::F32);
        let v = level.to_f32_samples().unwrap()[0];
        assert!((v - 0.4375).abs() < 1e-4, "{sample:?}: {v}");
    }
    let mut data = Vec::new();
    for _ in 0..2 {
        for x in 0..130u32 {
            // RGBA16: opaque blue on even columns, transparent red on odd.
            let px: [u16; 4] = if x % 2 == 0 { [0, 0, 65535, 65535] } else { [65535, 0, 0, 0] };
            data.extend(px.iter().flat_map(|v| v.to_ne_bytes()));
        }
    }
    let img = Image::from_raw(130, 2, ChannelLayout::Rgba, SampleType::U16, data).unwrap();
    let bytes = encode(&img, Format::Tiff, &pyramid()).unwrap();
    let level = decode_tiff_page(&bytes, 1, &DecodeOptions::default()).unwrap();
    assert_eq!(level.to_u16_samples().unwrap()[..4], [0, 0, 65535, 32768]);
}

#[test]
fn bigtiff_files_carry_the_pyramid_too() {
    let img = solid(300, 200, ChannelLayout::Rgba, SampleType::U8, 0.5);
    let bytes = encode(&img, Format::Tiff, &EncodeOptions { tiff_bigtiff: true, ..pyramid() }).unwrap();
    let info = tiff_info(&bytes).unwrap();
    assert!(info.big_tiff);
    assert_eq!(info.pages.iter().map(|p| (p.kind, p.width)).collect::<Vec<_>>().len(), 4);
    assert_eq!(decode_tiff_page(&bytes, 3, &DecodeOptions::default()).unwrap().dimensions(), (38, 25));
}

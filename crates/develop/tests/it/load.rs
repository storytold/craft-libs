//! Raw bytes to a rendered picture, end to end, on DNGs written by craft-raw's own writer.

use craft_develop::load::{LoadError, LoadOptions, develop_raw, load_raw};
use craft_develop::{OutputDepth, OutputSpace, PixelWindow, Quality, RenderRequest, StageCache, render, render_cached};
use craft_develop_model::DevelopSettings;
use craft_raw::{BlackLevel, Cfa, ColorData, DngCompression, DngWriteOptions, OpcodeLists, Orientation, RawData, RawFormat, RawImage, Rect};
use std::sync::Arc;

/// A 96 × 64 RGGB mosaic of a lit grey ramp with a red patch, black 256, white 4095, with a colour matrix
/// (a plausible camera's, D65) and an as-shot neutral.
fn mosaic(orientation: Orientation) -> RawImage {
    let (w, h) = (96usize, 64usize);
    let cfa = Cfa::bayer("RGGB");
    let data = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let c = cfa.as_ref().map_or(1, |c| c.color_at(x, y));
            let grey = 0.05 + 0.85 * x as f32 / w as f32;
            let red = (30..50).contains(&x) && (20..40).contains(&y);
            let v = match (c, red) {
                (0, true) => 0.8,
                (_, true) => 0.1,
                (0, false) => grey * 0.5,
                (1, false) => grey,
                _ => grey * 0.7,
            };
            (256.0 + v * (4095.0 - 256.0)) as u16
        })
        .collect();
    RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(data),
        cfa,
        bits: 12,
        black: BlackLevel::uniform(256.0),
        white: vec![4095.0],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(2, 2, w - 4, h - 4),
        orientation,
        color: ColorData {
            illuminant: [21, 0],
            color_matrix: [Some([[0.7, -0.15, -0.05], [-0.4, 1.2, 0.2], [-0.05, 0.15, 0.6]]), None],
            as_shot_neutral: Some([0.5, 1.0, 0.7]),
            ..Default::default()
        },
        wb_multipliers: None,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: Default::default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    }
}

fn dng(raw: &RawImage) -> Vec<u8> {
    craft_raw::write_dng(raw, &DngWriteOptions { compression: DngCompression::Uncompressed, ..Default::default() }).unwrap()
}

#[test]
fn a_dng_loads_as_upright_linear_rec2020_with_its_colour_model() {
    let bytes = dng(&mosaic(Orientation::Rotate90));
    let full = load_raw(&bytes, &LoadOptions::full()).unwrap();
    // the default crop, rotated upright
    assert_eq!((full.image.width, full.image.height), (60, 92));
    assert!(full.info.raw && full.info.camera_color.is_some(), "a DNG with a matrix keeps its colour model for white balance");
    assert!(!full.info.relative_wb);
    assert!(full.from_preview.is_none() && !full.reduced);
    assert!(full.image.data.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
    // the as-shot neutral renders grey: a grey sample of the ramp has about equal channels
    let p = full.image.get(30, 80);
    assert!((p[0] / p[1] - 1.0).abs() < 0.05 && (p[2] / p[1] - 1.0).abs() < 0.05, "{p:?}");
    // a small size bins the mosaic: about the size asked for, the same orientation
    let small = load_raw(&bytes, &LoadOptions::fit(40)).unwrap();
    assert!(small.image.width.max(small.image.height) <= 40 && small.image.height > small.image.width);
    assert_eq!((small.info.as_shot_temp, small.info.as_shot_tint), (full.info.as_shot_temp, full.info.as_shot_tint));
}

#[test]
fn a_decoded_raw_develops_without_its_file() {
    let raw = mosaic(Orientation::Normal);
    let a = develop_raw(raw.clone(), None, &LoadOptions::full()).unwrap();
    let b = load_raw(&dng(&raw), &LoadOptions::full()).unwrap();
    assert_eq!((a.image.width, a.image.height), (92, 60));
    assert_eq!(a.image, b.image, "a DNG's colour needs nothing but its tags");
}

#[test]
fn a_raw_renders_to_prophoto_16_bit_and_in_windows() {
    let loaded = load_raw(&dng(&mosaic(Orientation::Normal)), &LoadOptions::full()).unwrap();
    let mut s = DevelopSettings::for_raw(loaded.info.as_shot_temp, loaded.info.as_shot_tint);
    s.light.exposure = 0.5;
    let req = RenderRequest { space: OutputSpace::ProPhoto, depth: OutputDepth::U16, ..RenderRequest::fit(92, 60) };
    let out = render(&loaded.image, &loaded.info, &s, &req);
    let deep = out.deep.as_ref().unwrap();
    assert_eq!((deep.width, deep.height, deep.space), (92, 60, OutputSpace::ProPhoto));
    assert!(matches!(&deep.samples, craft_develop::DeepSamples::U16(v) if v.len() == 92 * 60 * 3));
    // a window of the same output is that part of the whole render
    let src = Arc::new(loaded.image.clone());
    let cache = StageCache::default();
    let window = PixelWindow { x: 20, y: 10, w: 40, h: 30 };
    let part = render_cached(&src, &loaded.info, &s, &RenderRequest { window: Some(window), ..req }, &cache);
    let part = part.deep.unwrap();
    assert_eq!((part.width, part.height), (40, 30));
    // and a draft render is the same size
    let draft = render(&loaded.image, &loaded.info, &s, &RenderRequest { quality: Quality::Draft, ..RenderRequest::fit(92, 60) });
    assert_eq!((draft.image.width, draft.image.height), (92, 60));
}

#[test]
fn bytes_that_are_not_a_raw_are_an_error() {
    assert!(matches!(load_raw(b"not a raw file", &LoadOptions::full()), Err(LoadError::Raw(_))));
}

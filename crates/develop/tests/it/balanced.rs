//! Original synthetic RAW scenes from storytold/lightcraft#1463, generated in these tests.

use craft_develop::raster::Rgb32f;
use craft_develop::sensor::{self, Method, RawDevelopExt, highlight};
use craft_raw::opcodes::{Opcode, OpcodeLists};
use craft_raw::{BlackLevel, Cfa, ColorData, Orientation, RawData, RawFormat, RawImage, Rect};

/// Daylight-like as-shot multipliers (a Sony body's are about 2.6 : 1 : 1.85).
const DAYLIGHT: [f32; 3] = [2.5, 1.0, 1.6];
const CLIP: f32 = 0.99;

/// A floating-point raw (black 0, white 1) whose mosaic samples `scene` (white-balanced RGB) through `cfa`, divided
/// by `wb` as a sensor records it, and clipped at the white level.
fn raw_of(w: usize, h: usize, cfa: &Cfa, wb: [f32; 3], scene: impl Fn(usize, usize) -> [f32; 3]) -> RawImage {
    let data = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let c = cfa.color_at(x, y) as usize;
            (scene(x, y)[c] / wb[c]).min(1.0)
        })
        .collect();
    RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::F32(data),
        cfa: Some(cfa.clone()),
        bits: 16,
        black: BlackLevel::uniform(0.0),
        white: vec![1.0],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::Normal,
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: true,
        pixel_aspect: 1.0,
        lens_metadata: Default::default(),
        opcodes: OpcodeLists::default(),
        linearization_table: None,
        camera: Default::default(),
        cinema: Default::default(),
        level_sources: Default::default(),
        notices: Default::default(),
    }
}

/// What the decode does after the demosaic, up to the colour matrix: clipped highlights rebuilt, white balance.
fn finish(mut img: Rgb32f, wb: [f32; 3]) -> Rgb32f {
    highlight::reconstruct(&mut img, wb, CLIP);
    finish_wb(img, wb)
}

fn layouts() -> Vec<(String, Cfa, Method)> {
    let mut v = Vec::new();
    for name in ["RGGB", "BGGR", "GRBG", "GBRG"] {
        for m in [Method::Ahd, Method::Ppg] {
            v.push((format!("{name} {m:?}"), Cfa::bayer(name).unwrap(), m));
        }
    }
    v.push(("X-Trans directional".into(), Cfa::xtrans(), Method::Ahd));
    v.push(("X-Trans directional (shifted)".into(), Cfa::xtrans().shifted(2, 3), Method::Ahd));
    v
}

#[test]
fn balanced_demosaic_returns_constant_colour_on_the_sensor_scale() {
    let colours = [[0.3f32, 0.55, 0.8], [0.02, 0.9, 0.4], [0.7, 0.7, 0.7]];
    let wbs = [DAYLIGHT, [1.3, 1.0, 3.1], [2.0, 1.2, 1.5], [1.0; 3]];
    let mut cases = layouts();
    for name in ["RGGB", "GBRG"] {
        cases.push((format!("{name} Bilinear"), Cfa::bayer(name).unwrap(), Method::Bilinear));
    }
    for (name, cfa, method) in &cases {
        for colour in colours {
            for wb in wbs {
                // the colour as camera RGB, i.e. what the mosaic holds
                let raw = raw_of(23, 19, cfa, [1.0; 3], |_, _| colour);
                let out = raw.develop_balanced(*method, wb).unwrap();
                for p in &out.data {
                    for c in 0..3 {
                        assert!((p[c] - colour[c]).abs() <= 1e-6 * colour[c].max(1.0), "{name} wb {wb:?}: {p:?} for {colour:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn every_sample_keeps_its_own_value() {
    let cfa = Cfa::bayer("GRBG").unwrap();
    let raw = raw_of(40, 32, &cfa, [1.0; 3], |x, y| {
        let v = 0.05 + 0.9 * (((x * 7 + y * 13) % 17) as f32 / 17.0);
        [v * 0.6, v, v * 0.8]
    });
    let n = raw.normalized().unwrap();
    for m in [Method::Ahd, Method::Ppg] {
        let out = raw.develop_balanced(m, DAYLIGHT).unwrap();
        for y in 0..32 {
            for x in 0..40 {
                let (c, s) = (cfa.color_at(x, y) as usize, n.data[y * 40 + x]);
                assert!((out.get(x, y)[c] - s).abs() <= 1e-6, "{m:?} ({x}, {y})");
            }
        }
    }
}

#[test]
fn unsupported_white_balance_inputs_keep_the_plain_develop_result() {
    let cfa = Cfa::bayer("RGGB").unwrap();
    let raw = raw_of(30, 22, &cfa, DAYLIGHT, |x, y| {
        let v = if (x / 5 + y / 4) % 2 == 0 { 0.08 } else { 0.9 };
        [v, v * 0.9, v * 1.1]
    });
    for method in [Method::Ahd, Method::Ppg] {
        let plain = raw.develop(method).unwrap();
        // multipliers count relative to green: equal ones are no white balance at all
        // (and finite multipliers whose ratios would overflow the scaled mosaic or the division back, review of #1328)
        for wb in [
            [1.0; 3],
            [2.0; 3],
            [f32::NAN, 1.0, 1.0],
            [0.0, 1.0, 1.5],
            [2.0, -1.0, 1.5],
            [f32::INFINITY, 1.0, 1.0],
            [f32::MAX, 1.0, 1.0],
            [1e-40, 1.0, 1.0],
            [f32::MIN_POSITIVE, 1.0, 1.0],
            [1e20, 1.0, 1e-20],
            [f32::MAX, f32::MIN_POSITIVE, 1.0],
        ] {
            assert_eq!(raw.develop_balanced(method, wb).unwrap(), plain, "{method:?} {wb:?}");
        }
        // Strong finite gains still balance when the samples fit.
        for wb in [[16.0, 1.0, 1.0 / 16.0], [2.0, 4.0, 3.0]] {
            let out = raw.develop_balanced(method, wb).unwrap();
            assert_ne!(out, plain, "{method:?} {wb:?}");
            assert!(out.data.iter().flatten().all(|v| v.is_finite()), "{method:?} {wb:?}");
        }
    }
    // bilinear interpolates each colour on its own: nothing to balance
    assert_eq!(raw.develop_balanced(Method::Bilinear, DAYLIGHT).unwrap(), raw.develop(Method::Bilinear).unwrap());
    // already demosaiced and monochrome data are left alone
    let mut mono = raw.clone();
    mono.cfa = None;
    assert_eq!(mono.develop_balanced(Method::Ahd, DAYLIGHT).unwrap(), mono.develop(Method::Ahd).unwrap());

    let mut linear = mono;
    linear.cpp = 3;
    linear.data = RawData::F32([0.2, 0.6, 0.8].repeat(linear.width * linear.height));
    assert_eq!(linear.develop_balanced(Method::Ahd, DAYLIGHT).unwrap(), linear.develop(Method::Ahd).unwrap());
}

/// What the lens makes of a sharp `scene` (white-balanced level, sampled at sub-pixel positions): each pixel the
/// mean of 4 × 4 points of its area, then a blur of about one pixel (binomial [1 4 6 4 1] / 16 each way: a good lens
/// and the sensor's microlenses). Any sharper and a single sample holds detail no demosaic can place.
fn photograph(w: usize, h: usize, scene: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let area: Vec<f32> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            (0..16).map(|k| scene(x + ((k % 4) as f32 + 0.5) / 4.0, y + ((k / 4) as f32 + 0.5) / 4.0)).sum::<f32>() / 16.0
        })
        .collect();
    let blur = |v: &[f32], dx: isize, dy: isize| -> Vec<f32> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                let at = |k: isize| v[(y + k * dy).clamp(0, h as isize - 1) as usize * w + (x + k * dx).clamp(0, w as isize - 1) as usize];
                (at(-2) + 4.0 * at(-1) + 6.0 * at(0) + 4.0 * at(1) + at(2)) / 16.0
            })
            .collect()
    };
    blur(&blur(&area, 1, 0), 0, 1)
}

/// Issue #1328: on a dark neutral surface, neutral specular points of 1 to 3 px bright enough to clip green (the
/// brightest clip every channel). Around them, the pixels with nothing clipped must stay neutral after highlight
/// reconstruction and white balance: no channel below half the pixel's mean, none negative. Demosaiced from the
/// unbalanced mosaic, red (or blue) beside a point fell to about zero or below: green and cyan rims, on 16 to 38 %
/// of these pixels.
#[test]
fn specular_points_have_no_coloured_rims() {
    let (w, h) = (96usize, 72usize);
    // (x, y, size, white-balanced level): green clips at 1, blue at 1.6, red at 2.5
    let mut points = Vec::new();
    for (i, level) in [2.0f32, 4.0, 8.0].into_iter().enumerate() {
        for (j, size) in [1.0f32, 2.0, 3.0].into_iter().enumerate() {
            for k in 0..2 {
                points.push((8.0 + j as f32 * 30.0 + k as f32 * 12.6, 9.0 + i as f32 * 22.0 + k as f32 * 7.3, size, level));
            }
        }
    }
    let light =
        photograph(w, h, |x, y| points.iter().find(|&&(px, py, s, _)| (px..px + s).contains(&x) && (py..py + s).contains(&y)).map_or(0.04, |p| p.3));
    let near = |x: usize, y: usize| {
        let (x, y) = (x as f32, y as f32);
        points.iter().any(|&(px, py, s, _)| x + 3.0 > px && x < px + s + 2.0 && y + 3.0 > py && y < py + s + 2.0)
    };
    for (name, cfa, method) in layouts() {
        let raw = raw_of(w, h, &cfa, DAYLIGHT, |x, y| [light[y * w + x]; 3]);
        // what the scene is about: green clipped at most points, every channel at the brightest
        let n = raw.normalized().unwrap();
        let clipped = |c: u8| (0..w * h).filter(|&i| cfa.color_at(i % w, i / w) == c && n.data[i] >= 1.0).count();
        assert!(clipped(1) >= 20 && clipped(0) >= 2 && clipped(2) >= 4, "{name}: clipped R G B {} {} {}", clipped(0), clipped(1), clipped(2));
        let developed = raw.develop_balanced(method, DAYLIGHT).unwrap();
        let unclipped: Vec<bool> = developed.data.iter().map(|p| p.iter().all(|&v| v < CLIP)).collect();
        let out = finish(developed, DAYLIGHT);
        // X-Trans: its directional weights already keep colour differences from crossing an edge, and it has the
        // same few pale pixels (a channel at 0.3 to 0.5 of the mean) either way
        let floor = if cfa.is_bayer() { 0.5 } else { 0.25 };
        let (mut rim, mut bad) = (0, Vec::new());
        for y in 0..h {
            for x in 0..w {
                if !near(x, y) || !unclipped[y * w + x] {
                    continue;
                }
                rim += 1;
                let p = out.get(x, y);
                let mean = (p[0] + p[1] + p[2]) / 3.0;
                if p.iter().any(|&v| v < 0.0 || v < floor * mean) {
                    bad.push((x, y, p));
                }
            }
        }
        assert!(rim > 0, "{name}: {rim} rim pixels");
        assert!(bad.is_empty(), "{name}: {} of {rim} rim pixels coloured, e.g. {:?}", bad.len(), &bad[..bad.len().min(4)]);
    }
}

/// Issue #1328: sharp neutral white-on-dark edges with nothing clipped (straight, diagonal, round, a 1 px line).
/// The pixels along them must stay nearly neutral after white balance. From the unbalanced mosaic their colour
/// spread (largest minus smallest channel) averaged 6 % of the edge's contrast on Bayer layouts (0.2 % to 0.7 %
/// balanced), with yellow, green, cyan or magenta outlines up to 36 %.
#[test]
fn sharp_neutral_edges_stay_neutral() {
    let (w, h) = (96usize, 80usize);
    let (dark, bright) = (0.03f32, 0.85f32);
    let shape = |x: f32, y: f32| {
        let disc = (x - 70.0).powi(2) + (y - 24.0).powi(2) < 13.5f32.powi(2);
        let bar = (10.0..30.0).contains(&x) && (8.0..40.0).contains(&y);
        let slant = y > 46.0 && x * 0.6 + y > 70.0 && x < 60.0;
        let line = (80.0..81.0).contains(&x) && y > 46.0;
        disc || bar || slant || line
    };
    let light = photograph(w, h, |x, y| if shape(x, y) { bright } else { dark });
    // pixels within 1 px of an edge
    let inside = |x: usize, y: usize| shape(x as f32 + 0.5, y as f32 + 0.5);
    let edge = |x: usize, y: usize| (x - 1..=x + 1).any(|xx| (y - 1..=y + 1).any(|yy| inside(xx, yy) != inside(x, y)));
    for (name, cfa, method) in layouts() {
        let raw = raw_of(w, h, &cfa, DAYLIGHT, |x, y| [light[y * w + x]; 3]);
        let out = finish(raw.develop_balanced(method, DAYLIGHT).unwrap(), DAYLIGHT);
        let (mut n, mut worst, mut sum) = (0, 0f32, 0f32);
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                if !edge(x, y) {
                    continue;
                }
                let p = out.get(x, y);
                assert!(p.iter().all(|&v| v >= 0.0), "{name}: negative channel at ({x}, {y}): {p:?}");
                let spread = (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) / (bright - dark);
                worst = worst.max(spread);
                sum += spread;
                n += 1;
            }
        }
        let mean = sum / n as f32;
        assert!(n > 0, "{name}: {n} edge pixels");
        // X-Trans: our directional method's own green interpolation softens edges (mean 9 %, worst 43 %, and
        // negative channels, unbalanced)
        let (mean_max, worst_max) = if cfa.is_bayer() { (0.02, 0.15) } else { (0.085, 0.35) };
        assert!(mean < mean_max && worst < worst_max, "{name}: edge colour spread mean {mean:.3}, worst {worst:.3}");
    }
}

/// Large clipped areas on bright and dark neutral surfaces can leave magenta rims. The mask lets highlight
/// reconstruction rebuild the unknown interpolated channels and keep the surrounding pixels neutral.
#[test]
fn clipped_areas_have_no_coloured_rims() {
    let (w, h) = (96usize, 72usize);
    let lamp = |x: f32, y: f32| (x - 30.0).powi(2) + (y - 36.0).powi(2) < 14.0f32.powi(2) || ((62.0..80.0).contains(&x) && (20.0..52.0).contains(&y));
    let light = photograph(w, h, |x, y| {
        if lamp(x, y) {
            6.0
        } else if x < 48.0 {
            0.7
        } else {
            0.05
        }
    });
    // within 3 px of the clipped area, unclipped in the scene
    let rim = |x: usize, y: usize| {
        !lamp(x as f32 + 0.5, y as f32 + 0.5)
            && (x.saturating_sub(3)..=(x + 3).min(w - 1))
                .any(|xx| (y.saturating_sub(3)..=(y + 3).min(h - 1)).any(|yy| lamp(xx as f32 + 0.5, yy as f32 + 0.5)))
    };
    for (name, cfa, method) in layouts() {
        let raw = raw_of(w, h, &cfa, DAYLIGHT, |x, y| [light[y * w + x]; 3]);
        let (mut img, mask) = raw.develop_balanced_masked(method, DAYLIGHT, CLIP).unwrap();
        highlight::reconstruct_masked(&mut img, DAYLIGHT, CLIP, [1.0; 3], &mask.expect("clipped samples"));
        let img = finish_wb(img, DAYLIGHT);
        let (mut n, mut worst, mut sum) = (0, 0f32, 0f32);
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                if !rim(x, y) {
                    continue;
                }
                let p = img.get(x, y);
                let mean = (p[0] + p[1] + p[2]) / 3.0;
                let spread = (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) / mean.max(1e-3);
                worst = worst.max(spread);
                sum += spread;
                n += 1;
            }
        }
        let mean = sum / n as f32;
        assert!(n > 0, "{name}: {n} rim pixels");
        // (told by value: mean 7 to 17 %, worst 80 %; demosaiced unbalanced, channels below zero). X-Trans: a few
        // pixels at the lamp's edge where green interpolates just above the clip and red overshoots
        let worst_max = if cfa.is_bayer() { 0.6 } else { 0.8 };
        assert!(
            mean < 0.03 && worst < worst_max,
            "{name}: colour spread around the clipped area mean {mean:.3}, worst {worst:.3} of the pixel's mean"
        );
    }
}

#[test]
fn the_clip_mask_marks_the_colours_interpolated_from_clipped_samples() {
    let cfa = Cfa::bayer("RGGB").unwrap();
    let (w, h) = (16usize, 12usize);
    let mut raw = raw_of(w, h, &cfa, [1.0; 3], |_, _| [0.3; 3]);
    // Place clipped green and red samples at known CFA sites, then develop with an offset crop.
    if let RawData::F32(d) = &mut raw.data {
        d[4 * w + 5] = 1.0;
        d[8 * w + 10] = 1.0;
    }
    raw.crop = Rect::new(1, 1, 14, 10);
    let (img, mask) = raw.develop_balanced_masked(Method::Ahd, DAYLIGHT, CLIP).unwrap();
    assert_eq!(img, raw.develop_balanced(Method::Ahd, DAYLIGHT).unwrap(), "the same picture");
    let mask = mask.unwrap();
    assert_eq!(mask.len(), img.data.len());
    let at = |x: usize, y: usize| mask[(y - raw.crop.y) * img.width + x - raw.crop.x];
    assert_eq!(at(5, 4), highlight::CLIPPED_G, "the saturated green site");
    assert_eq!(at(10, 8), highlight::CLIPPED_R, "the saturated red site");
    assert_eq!(at(4, 4), highlight::FROM_CLIPPED_G | (1 << highlight::MEASURED_SHIFT), "red beside saturated green");
    assert_eq!(at(11, 8), highlight::FROM_CLIPPED_R | (2 << highlight::MEASURED_SHIFT), "green beside saturated red");
    assert_ne!(at(5, 6) & highlight::FROM_CLIPPED_G, 0, "clipped green can affect the other channels at this green site");
    assert_eq!(at(2, 8), 0, "a site away from both clipped samples");
    // An unclipped mosaic or an OpcodeList3 needs no mask.
    let clean = raw_of(w, h, &cfa, [1.0; 3], |_, _| [0.3; 3]);
    assert!(clean.develop_balanced_masked(Method::Ahd, DAYLIGHT, CLIP).unwrap().1.is_none());
    raw.opcodes.list3.push(Opcode::FixVignetteRadial { k: [0.0; 5], center: [0.5, 0.5] });
    assert!(raw.develop_balanced_masked(Method::Ahd, DAYLIGHT, CLIP).unwrap().1.is_none());
}

#[test]
fn a_caller_demosaicking_its_own_mosaic_gets_the_same_clip_mask() {
    let (w, h) = (40usize, 30usize);
    let light = photograph(w, h, |x, y| if (x - 18.0).powi(2) + (y - 14.0).powi(2) < 49.0 { 6.0 } else { 0.4 });
    let mut cases = layouts();
    cases.push(("RGGB bilinear".into(), Cfa::bayer("RGGB").unwrap(), Method::Bilinear));
    for (name, cfa, method) in cases {
        let mut raw = raw_of(w, h, &cfa, DAYLIGHT, |x, y| [light[y * w + x]; 3]);
        raw.crop = Rect::new(2, 1, 33, 27);
        let (_, expected) = raw.develop_balanced_masked(method, DAYLIGHT, CLIP).unwrap();
        let n = raw.normalized().unwrap();
        let crop = raw.develop_crop(n.width, n.height);
        assert_eq!(sensor::clip_mask(&n, crop, method, CLIP), expected, "{name}");
        assert!(expected.is_some(), "{name}: clipped samples");
        let outside = Rect::new(crop.x + 1, crop.y, n.width - crop.x, crop.height);
        assert!(sensor::clip_mask(&n, outside, method, CLIP).is_none(), "{name}: a crop past the mosaic");
        let mut four_colour = n.clone();
        four_colour.cfa.as_mut().unwrap().pattern[0] = 3;
        assert!(sensor::clip_mask(&four_colour, crop, method, CLIP).is_none(), "{name}: a colour index past blue");
        let huge = sensor::Normalized { width: usize::MAX, height: 2, data: Vec::new(), ..n.clone() };
        assert!(sensor::clip_mask(&huge, Rect::new(0, 0, 1, 1), method, CLIP).is_none(), "{name}: dimensions past usize");
    }
}

#[test]
fn ppg_clip_mask_marks_same_colour_dependencies() {
    let cfa = Cfa::bayer("RGGB").unwrap();
    let wb = [1.0; 3];
    let mut raw = raw_of(16, 12, &cfa, wb, |x, y| {
        let level = match (x, y) {
            (5, 4) | (6, 3) => 0.26,
            (7, 4) => 0.35,
            (8, 4) => 0.4,
            (6, 5) => 0.49,
            _ => 0.3,
        };
        [level; 3]
    });
    let clean = raw.develop(Method::Ppg).unwrap().get(6, 4);
    if let RawData::F32(samples) = &mut raw.data {
        samples[4 * raw.width + 4] = 1.0;
    }
    let (mut image, mask) = raw.develop_balanced_masked(Method::Ppg, wb, CLIP).unwrap();
    let changed = image.get(6, 4);
    assert!(changed[1] > clean[1] && changed.iter().all(|&v| v < CLIP));
    let mask = mask.unwrap();
    let entry = mask[4 * image.width + 6];
    assert_ne!(entry & highlight::FROM_CLIPPED_R, 0, "the clipped red sample changed interpolated green");
    assert_eq!(entry >> highlight::MEASURED_SHIFT, cfa.color_at(6, 4) + 1);
    assert_eq!(mask[4 * image.width + 10], 0, "no clipped sample is within reach");
    highlight::reconstruct_masked(&mut image, wb, CLIP, wb, &mask);
    assert_eq!(image.get(6, 4)[0], clean[0], "reconstruction keeps measured red");
    assert_ne!(image.get(6, 4)[1], changed[1], "reconstruction can use measured red to rebuild green");
}

#[test]
fn bilinear_clip_mask_marks_only_actual_taps() {
    let cfa = Cfa::bayer("RGGB").unwrap();
    for (width, method) in [(16, Method::Bilinear), (3, Method::Ahd), (3, Method::Ppg)] {
        let raw = raw_of(width, 10, &cfa, [1.0; 3], |x, y| if (x, y) == (0, 2) { [1.0; 3] } else { [0.21, 0.15, 0.11] });
        let (mut image, mask) = raw.develop_balanced_masked(method, [1.0; 3], CLIP).unwrap();
        let mask = mask.unwrap();
        let clean = image.get(2, 3);
        let blue = image.get(1, 2)[2];
        highlight::reconstruct_masked(&mut image, [1.0; 3], CLIP, [1.0; 3], &mask);
        assert_eq!(image.get(2, 3), clean, "the clipped red sample is outside these bilinear taps");
        assert_eq!(mask[3 * width + 2], 0);
        assert_eq!(mask[2 * width + 1] & highlight::FROM_CLIPPED_R, highlight::FROM_CLIPPED_R);
        assert_eq!(image.get(1, 2)[2], blue, "blue interpolation uses its own samples");
    }
}

#[test]
fn bilinear_clip_mask_keeps_the_wider_taps_for_sparse_cfas() {
    let cfa = Cfa { width: 6, height: 1, pattern: vec![0, 1, 1, 1, 2, 1], origin_x: 0, origin_y: 0 };
    let raw = raw_of(12, 10, &cfa, [1.0; 3], |x, y| if (x, y) == (0, 2) { [1.0; 3] } else { [0.21, 0.15, 0.11] });
    let (_, mask) = raw.develop_balanced_masked(Method::Bilinear, DAYLIGHT, CLIP).unwrap();
    let mask = mask.unwrap();
    assert_ne!(mask[2 * raw.width + 2] & highlight::FROM_CLIPPED_R, 0, "the closest red taps are two columns away");
}

/// Review of #1328: dark twigs and wires against a clipped sky (every channel clipped around them) keep their level.
/// A pixel's own CFA sample is measured: a clipped neighbour of the same colour says nothing about it, and the colours
/// interpolated from clipped samples are unknown, not lower bounds. Taken as lower bounds (with the pixel's own colour
/// marked as clipped by its neighbours), the line's pixels were rebuilt as the brightest neutral and the lines vanished.
#[test]
fn dark_lines_against_a_clipped_sky_keep_their_level() {
    let (w, h) = (96usize, 72usize);
    let sky = 3.0f32;
    // lines 3, 4 and 6 px wide (across, down, slanted) and a blob, at the level of dark bark
    let dark = |x: f32, y: f32| {
        (20.0..23.0).contains(&x)
            || (30.0..34.0).contains(&y)
            || ((x - y * 0.5 - 50.0).abs() < 3.0 && y < 60.0)
            || (x - 78.0).powi(2) + (y - 56.0).powi(2) < 25.0
    };
    let light = photograph(w, h, |x, y| if dark(x, y) { 0.05 } else { sky });
    for (name, cfa, method) in layouts() {
        let raw = raw_of(w, h, &cfa, DAYLIGHT, |x, y| [light[y * w + x]; 3]);
        let (mut img, mask) = raw.develop_balanced_masked(method, DAYLIGHT, CLIP).unwrap();
        highlight::reconstruct_masked(&mut img, DAYLIGHT, CLIP, [1.0; 3], &mask.expect("clipped samples"));
        let out = finish_wb(img, DAYLIGHT);
        let (mut n, mut lifted, mut worst) = (0, Vec::new(), 0f32);
        for y in 3..h - 3 {
            for x in 3..w - 3 {
                // what the lens let through is well below green's clip: the sensor measured it
                let level = light[y * w + x];
                if level > 0.5 {
                    continue;
                }
                n += 1;
                let p = out.get(x, y);
                let mean = (p[0] + p[1] + p[2]) / 3.0;
                worst = worst.max(mean / level);
                if mean > 2.0 * level + 0.05 {
                    lifted.push((x, y, level, p));
                }
            }
        }
        assert!(n > 0, "{name}: {n} line pixels");
        assert!(
            lifted.is_empty(),
            "{name}: {} of {n} dark pixels lifted (worst {worst:.1}x), e.g. {:?}",
            lifted.len(),
            &lifted[..lifted.len().min(3)]
        );
    }
}

/// White balance only (what [`finish`] does after the reconstruction it runs itself).
fn finish_wb(mut img: Rgb32f, wb: [f32; 3]) -> Rgb32f {
    img.data.iter_mut().for_each(|p| *p = [p[0] * wb[0], p[1] * wb[1], p[2] * wb[2]]);
    img
}

/// Review of #1328: twigs a pixel or two wide against a clipped sky stay continuous lines at about the level the lens
/// let through. Their pixels are partly clipped (green clips at the twig's centre, red and blue don't), so marking the
/// interpolated channels of the pixels whose own sample clipped as unknown broke them into dots: those pixels were
/// rebuilt as clipped white between dark ones (0.44 stop from one twig pixel to the next, unbalanced 0.04).
#[test]
fn thin_twigs_against_a_clipped_sky_stay_continuous() {
    let (w, h) = (96usize, 72usize);
    let dark = |x: f32, y: f32| {
        (20.0..21.0).contains(&x)
            || (40.0..41.5).contains(&x)
            || (60.0..62.0).contains(&x)
            || (30.0..31.5).contains(&y)
            || (x - y * 0.4 - 70.0).abs() < 0.8
    };
    let light = photograph(w, h, |x, y| if dark(x, y) { 0.05 } else { 3.0 });
    let level = |p: [f32; 3]| ((p[0] + p[1] + p[2]) / 3.0).min(3.0);
    for (name, cfa, method) in layouts() {
        let raw = raw_of(w, h, &cfa, DAYLIGHT, |x, y| [light[y * w + x]; 3]);
        let (mut img, mask) = raw.develop_balanced_masked(method, DAYLIGHT, CLIP).unwrap();
        highlight::reconstruct_masked(&mut img, DAYLIGHT, CLIP, [1.0; 3], &mask.expect("clipped samples"));
        let out = finish_wb(img, DAYLIGHT);
        // twig pixels (the lens let through less than 2 of the sky's 3): the error of their level, and the change from
        // one to the next along the twig
        let (mut n, mut err, mut jump, mut nj) = (0, 0f32, 0f32, 0);
        for y in 4..h - 4 {
            for x in 4..w - 4 {
                let l = light[y * w + x];
                if l >= 2.0 {
                    continue;
                }
                let m = level(out.get(x, y)).max(1e-3);
                err += (m / l).log2().abs();
                n += 1;
                for (xx, yy) in [(x + 1, y), (x, y + 1)] {
                    let l2 = light[yy * w + xx];
                    if l2 < 2.0 && (l2 / l - 1.0).abs() < 0.1 {
                        jump += (level(out.get(xx, yy)).max(1e-3) / m).log2().abs();
                        nj += 1;
                    }
                }
            }
        }
        let (err, jump) = (err / n as f32, jump / nj as f32);
        assert!(n > 0 && nj > 0, "{name}: {n} twig pixels, {nj} pairs");
        // (unbalanced: 0.19 / 0.04 stop on Bayer, 0.29 / 0.17 on X-Trans)
        let (err_max, jump_max) = if cfa.is_bayer() { (0.25, 0.12) } else { (0.35, 0.25) };
        assert!(err < err_max && jump < jump_max, "{name}: twig level off by {err:.3} stop, {jump:.3} stop from pixel to pixel");
    }
}

/// Round 2 review of #1328: strongly coloured highlights (a warm sun clipping red, or red and green, or every
/// channel at its core; red, yellow and green light tubes) keep their brightness just inside their bright edges.
/// Rebuilding a pixel's interpolated channels from its measured one alone (blue, at an orange sun's edge) with the
/// colour of the content around made a dark checkerboard line there: a CC0 sunset's pixel went from Y 0.87 to 0.21.
#[test]
fn coloured_highlights_keep_their_brightness_at_the_edges() {
    let (w, h) = (96usize, 80usize);
    // (tint, white-balanced level): an orange sun clipping red only, red and green, and every channel; light tubes
    let suns = [([1.0f32, 0.3, 0.06], 3.0f32), ([1.0, 0.45, 0.08], 4.0), ([1.0, 0.5, 0.12], 40.0)];
    let tubes = [([1.0f32, 0.15, 0.1], 3.5f32), ([1.0, 0.85, 0.15], 2.6), ([0.25, 1.0, 0.4], 2.0)];
    let scenes = suns.map(|(tint, level)| (tint, level, true)).into_iter().chain(tubes.map(|(tint, level)| (tint, level, false)));
    let lum = |p: [f32; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    // The gate develops every divisor-th (scene, layout) pair, a different layout per scene; nightly all of them.
    let divisor = craft_testkit::hostile::divisor();
    for (scene_index, (tint, level, sun)) in scenes.enumerate() {
        let scene_name = if sun { "sun" } else { "tube" };
        let scene = |x: f32, y: f32| {
            if sun {
                let radius = ((x - 48.0).powi(2) + (y - 40.0).powi(2)).sqrt();
                let k = if radius < 16.0 { 1.0 } else { (-(radius - 16.0) / 3.0).exp() };
                (k, 0.03)
            } else {
                let distance = (x - 30.0).abs().min((x - 0.5 * y - 45.0).abs() / 1.118);
                let k = if distance < 2.0 { 1.0 } else { 0.15 * (-(distance - 2.0) / 2.0).exp() };
                (k, 0.02)
            }
        };
        let lights: [Vec<f32>; 3] = std::array::from_fn(|c| {
            photograph(w, h, |x, y| {
                let (strength, background) = scene(x, y);
                tint[c] * level * strength + background
            })
        });
        let truth = |x: usize, y: usize| -> [f32; 3] { std::array::from_fn(|c| lights[c][y * w + x]) };
        for (layout_index, (name, cfa, method)) in layouts().into_iter().enumerate() {
            if !(scene_index + layout_index).is_multiple_of(divisor) {
                continue;
            }
            let raw = raw_of(w, h, &cfa, DAYLIGHT, truth);
            let (mut img, mask) = raw.develop_balanced_masked(method, DAYLIGHT, CLIP).unwrap();
            let mut by_value = img.clone();
            highlight::reconstruct(&mut by_value, DAYLIGHT, CLIP);
            highlight::reconstruct_masked(&mut img, DAYLIGHT, CLIP, [1.0; 3], &mask.expect("clipped samples"));
            let (out, by_value) = (finish_wb(img, DAYLIGHT), finish_wb(by_value, DAYLIGHT));
            // pixels the sensor measured in every channel (well below each clip), within 3 px of a clipped sample:
            // their brightness must stay near what the lens let through
            let clipped = |x: usize, y: usize| (0..3).any(|c| truth(x, y)[c] / DAYLIGHT[c] >= 1.0);
            let (mut n, mut dark, mut dark_by_value) = (0, Vec::new(), 0);
            for y in 3..h - 3 {
                for x in 3..w - 3 {
                    let t = truth(x, y);
                    if (0..3).any(|c| t[c] / DAYLIGHT[c] > 0.9) || !(x - 3..=x + 3).any(|xx| (y - 3..=y + 3).any(|yy| clipped(xx, yy))) {
                        continue;
                    }
                    n += 1;
                    let ratio = |p: [f32; 3]| lum(p) / lum(t).max(1e-4);
                    let masked_ratio = ratio(out.get(x, y));
                    let value_ratio = ratio(by_value.get(x, y));
                    assert!(
                        masked_ratio.is_finite() && value_ratio.is_finite(),
                        "{scene_name} {tint:?} x{level}, {name} at ({x}, {y}): masked ratio {masked_ratio}, by-value ratio {value_ratio}"
                    );
                    if masked_ratio < 0.6 {
                        dark.push((x, y, masked_ratio));
                    }
                    if value_ratio < 0.6 {
                        dark_by_value += 1;
                    }
                }
            }
            assert!(n > 0, "{scene_name} {tint:?} x{level}, {name}: {n} edge pixels");
            if !cfa.is_bayer() {
                // X-Trans darkens a few pixels inside thin red and yellow tubes told by value too (as main does); the
                // mask may add a few more, never many
                assert!(
                    dark.len() <= dark_by_value + n / 30,
                    "{scene_name} {tint:?} x{level}, {name}: {} of {n} edge pixels darkened ({dark_by_value} by value)",
                    dark.len()
                );
                continue;
            }
            assert!(
                dark.is_empty(),
                "{scene_name} {tint:?} x{level}, {name}: {} of {n} edge pixels darkened ({dark_by_value} by value), e.g. {:?}",
                dark.len(),
                &dark[..dark.len().min(3)]
            );
        }
    }
}

/// Round 2 review of #1328: finite but huge samples (a floating-point DNG holding 1e37) must not overflow once
/// balanced: the gains are applied only when every sample times the largest of them, and divided back by the
/// smallest, stays far inside `f32`; otherwise the mosaic is demosaiced unbalanced, as `develop` does (finite here).
/// Just inside that bound the gains apply, and every demosaic stays finite.
#[test]
fn huge_float_samples_develop_unbalanced_rather_than_overflow() {
    let mut raw = raw_of(24, 20, &Cfa::bayer("RGGB").unwrap(), [1.0; 3], |_, _| [0.1; 3]);
    raw.data = RawData::F32(vec![1e37; 24 * 20]);
    raw.bits = 32;
    let bytes = craft_raw::write_dng(&raw, &craft_raw::DngWriteOptions::default()).unwrap();
    let decoded = craft_raw::decode(&bytes).unwrap();
    for method in [Method::Ahd, Method::Ppg] {
        let plain = decoded.develop(method).unwrap();
        assert!(plain.data.iter().flatten().all(|v| v.is_finite()), "{method:?} unbalanced");
        for wb in [[64.0, 1.0, 1.0], [1.0, 1.0, 1.0 / 64.0], DAYLIGHT] {
            let out = decoded.develop_balanced(method, wb).unwrap();
            assert!(out.data.iter().flatten().all(|v| v.is_finite()), "{method:?} {wb:?}");
            assert_eq!(out, plain, "{method:?} {wb:?}: develops unbalanced");
        }
    }
    // just inside the bound the gains apply, and every demosaic stays finite on samples that large
    for (name, cfa, method) in layouts() {
        let (w, h) = (36usize, 36usize);
        let mut raw = raw_of(w, h, &cfa, [1.0; 3], |_, _| [0.1; 3]);
        raw.data = RawData::F32((0..w * h).map(|i| if (i % w + 2 * (i / w)) % 5 < 2 { 1.5e28 } else { 0.0 }).collect());
        let plain = raw.develop(method).unwrap();
        for wb in [[64.0, 1.0, 1.0], [1.0, 1.0, 1.0 / 64.0]] {
            let out = raw.develop_balanced(method, wb).unwrap();
            assert!(out.data.iter().flatten().all(|v| v.is_finite()), "{name} {wb:?}");
            assert_ne!(out, plain, "{name} {wb:?}: balanced");
        }
    }
}

/// Pixel-aspect correction changes the developed grid, so mosaic positions cannot mark it.
#[test]
fn non_square_pixels_keep_the_stretched_grid_without_a_mosaic_clip_mask() {
    let cfa = Cfa::bayer("RGGB").unwrap();
    let mut raw = raw_of(24, 18, &cfa, DAYLIGHT, |_, _| [3.0; 3]);
    let (square, mask) = raw.develop_balanced_masked(Method::Ahd, DAYLIGHT, CLIP).unwrap();
    assert_eq!(mask.unwrap().len(), square.data.len());
    for aspect in [2.0, 0.5] {
        raw.pixel_aspect = aspect;
        let (image, mask) = raw.develop_balanced_masked(Method::Ahd, DAYLIGHT, CLIP).unwrap();
        assert_eq!(image.width, square.width);
        assert_eq!(image.height > square.height, aspect > 1.0);
        assert_ne!(image.height, square.height);
        assert!(mask.is_none(), "the mosaic grid is not the stretched grid");
        assert_eq!(image, raw.develop_balanced(Method::Ahd, DAYLIGHT).unwrap());
    }
}

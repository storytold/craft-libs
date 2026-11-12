use super::tone::{ELASTICITY_BLACK, MAX_TONE_ELASTICITY, TONE_REPAIR_ELASTICITY, limit_elasticity};
use super::*;
use crate::sensor::prelude::*;

fn steep_toe_scene() -> (Rgb32f, Rgb32f) {
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        // A broad scene with a steep camera toe below the old JPEG floor of 0.015.
        // Permute brightness so the held-out pixels span the same range independently.
        let y = 0.002 + ((i * 7919) % 6144) as f32 / 6144.0 * 0.25;
        let rgb = [0.8 + (i % 11) as f32 * 0.04, 1.0, 0.8 + (i % 17) as f32 * 0.025];
        *src = rgb.map(|v| v * y / luminance_2020(rgb));
        let display = 3.0 * y * (y / (y + 0.02)).powi(2);
        *dst = src.map(|v| v * display / y);
    }
    (sensor, reference)
}

#[test]
fn camera_tone_follows_the_jpegs_deep_shadows() {
    let (sensor, reference) = steep_toe_scene();
    let look = fit_pairs_with(&sensor, &reference, Some((Mat3::IDENTITY, None))).unwrap();
    let dark: Vec<_> = sensor.data.iter().zip(&reference.data).filter(|(_, p)| luminance_2020(**p) < 0.015).collect();
    assert!(dark.len() > 256);
    let error = dark.iter().map(|(x, y)| (look.tone.apply(luminance_2020(**x)) - luminance_2020(**y)).abs()).sum::<f32>() / dark.len() as f32;
    assert!(error < 0.001, "deep-shadow luminance error {error}");
    assert_eq!(look.matrix, Mat3::IDENTITY);
}

/// Issue storytold/lightcraft#1904: two populations the camera renders at different brightness for the same scene luminance (saturated
/// blue at 1.7x, red at 2.64x) made quantile matching join them with a near-vertical step.
#[test]
fn tone_fit_does_not_step_between_two_pixel_populations() {
    let mut pairs = Vec::new();
    for i in 0..620 {
        let x = 0.040 + 0.012 * f64::from(i) / 620.0;
        pairs.push((x, 1.7 * x));
    }
    for i in 0..380 {
        let x = 0.02 * (10.0f64).powf(f64::from(i) / 380.0);
        pairs.push((x, 2.64 * x));
    }
    let elasticity = |k: &[[f32; 2]; 32]| {
        k.windows(2)
            .map(|w| {
                ((f64::from(w[1][1]) + ELASTICITY_BLACK) / (f64::from(w[0][1]) + ELASTICITY_BLACK)).ln()
                    / (f64::from(w[1][0]) / f64::from(w[0][0])).ln()
            })
            .fold(0.0, f64::max)
    };
    let mut xs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let mut ys: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let raw: [[f32; 2]; 32] = std::array::from_fn(|i| {
        let mid = (i * xs.len() / 32 + (i + 1) * xs.len() / 32) / 2;
        [xs[mid] as f32, ys[mid] as f32]
    });
    assert!(elasticity(&raw) > 10.0, "the synthetic scene must reproduce the step: {}", elasticity(&raw));
    let tone = fit_tone(pairs).unwrap();
    let knots = tone.knots();
    assert!(elasticity(knots) <= MAX_TONE_ELASTICITY + 1e-3, "{}", elasticity(knots));
    assert!(knots.iter().zip(&raw).all(|(a, b)| a[0] == b[0]), "only the outputs move");
    for w in knots.windows(2) {
        assert!(w[1][1] >= w[0][1]);
    }
    // A curve within the bound is left alone.
    let smooth: [[f32; 2]; 32] = std::array::from_fn(|i| {
        let x = 0.01 * 1.15f32.powi(i as i32);
        [x, 0.9 * (x / (x + 0.18))]
    });
    assert_eq!(limit_elasticity(smooth, &[]), Some(smooth));
}

/// A step at the top of the curve reaches its end: the pinned window and the free end are both under the bound, and
/// the pixel pairs choose between them.
#[test]
#[allow(clippy::needless_range_loop)] // the index addresses several arrays at once
fn a_step_at_the_curves_end_is_repaired_from_the_side_the_pixels_support() {
    let x = |i: usize| 0.01 * 1.15f64.powi(i as i32);
    let stepped: [[f32; 2]; 32] = std::array::from_fn(|i| [x(i) as f32, (0.3 * x(i) * if i >= 30 { 3.0 } else { 1.0 }) as f32]);
    let pairs = |gain: f64| -> Vec<(f64, f64)> {
        (0..4000).map(|j| x(0) * (x(31) / x(0)).powf(f64::from(j) / 3999.0)).map(|x| (x, 0.3 * x * if x >= 0.62 { gain } else { 1.0 })).collect()
    };
    // The pixels follow the curve below the step: the end is left free and the top knots drop to the repair slope.
    let free = limit_elasticity(stepped, &pairs(1.0)).unwrap();
    assert_eq!(free[..30], stepped[..30], "the curve up to the step is kept");
    let (u29, v29) = (x(29).ln(), (f64::from(stepped[29][1]) + ELASTICITY_BLACK).ln());
    for k in 30..32 {
        let limit = (v29 + TONE_REPAIR_ELASTICITY * (x(k).ln() - u29)).exp() - ELASTICITY_BLACK;
        assert!(f64::from(free[k][1]) <= limit + 1e-6, "knot {k}: {} over {limit}", free[k][1]);
    }
    // The pixels do sit above the step: the window stays pinned to the top knot.
    let pinned = limit_elasticity(stepped, &pairs(3.0)).unwrap();
    assert_eq!(pinned[31], stepped[31], "the end the pixels support is kept");
    assert!(pinned[29][1] > stepped[29][1] && pinned[30][1] < stepped[30][1], "the step is spread over the window");
}

#[test]
fn colour_holdouts_stay_out_of_the_wider_tone_training() {
    let (sensor, reference) = steep_toe_scene();
    let fit = |reference: &Rgb32f| {
        let (pairs, _) = collect_pairs(&sensor, reference, 0.005, None, false).unwrap();
        fit_tone_with_shadows(ToneFit::Quantile, &pairs, &sensor, reference, None, |x| x).unwrap()
    };
    let expected = fit(&reference);
    let mut changed = reference.clone();
    let mut index = 0;
    for (x, y) in sensor.data.iter().zip(&mut changed.data) {
        if colour_pair(*x, *y) {
            if index % 3 == 0 {
                *y = y.map(|v| v * 1.1);
                assert!(colour_pair(*x, *y));
            }
            index += 1;
        }
    }
    assert_eq!(fit(&changed), expected);
}

#[test]
fn camera_toe_is_robust_to_shadow_noise() {
    let (sensor, clean) = steep_toe_scene();
    let mut noisy = clean.clone();
    for (i, p) in noisy.data.iter_mut().enumerate() {
        if luminance_2020(*p) < 0.015 {
            // Quantized JPEG levels with bounded noise and occasional compression outliers.
            let noise = ((i * 17 % 11) as f32 - 5.0) * 0.00004;
            *p = p.map(|v| ((v + noise).max(0.0) / 0.0003).round() * 0.0003);
            if i % 23 == 0 {
                *p = [0.014; 3];
            }
        }
    }
    let look = fit_pairs_with(&sensor, &noisy, Some((Mat3::IDENTITY, None))).unwrap();
    let dark: Vec<_> = sensor.data.iter().zip(&clean.data).filter(|(_, p)| luminance_2020(**p) < 0.015).collect();
    let error = dark.iter().map(|(x, y)| (look.tone.apply(luminance_2020(**x)) - luminance_2020(**y)).abs()).sum::<f32>() / dark.len() as f32;
    assert!(error < 0.001, "noisy shadow luminance error {error}");
    assert_eq!(look.tone.apply(0.0), 0.0);
    assert_eq!(look.matrix, Mat3::IDENTITY);
    for i in 1..1000 {
        assert!(look.tone.apply(i as f32 / 1000.0) >= look.tone.apply((i - 1) as f32 / 1000.0));
    }
}

#[test]
fn camera_toe_requires_lower_independent_dark_error() {
    for holdout_matches_original in [true, false] {
        let mut sensor = Rgb32f::new(96, 64);
        let mut reference = sensor.clone();
        for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
            let y = if i < 600 { 0.0015 + (i % 200) as f32 * 0.00001 } else { 0.02 + ((i * 7919) % 6144) as f32 / 6144.0 * 0.23 };
            *src = [y; 3];
            // Midtones teach the original slope of 3. Dark training teaches 0.3,
            // but the independent dark holdout can agree with either slope.
            let slope = if i < 600 && !(holdout_matches_original && i % 3 == 0) { 0.3 } else { 3.0 };
            *dst = [y * slope; 3];
            assert_eq!(colour_pair(*src, *dst), i >= 600);
        }
        let (pairs, _) = collect_pairs(&sensor, &reference, f32::NEG_INFINITY, None, false).unwrap();
        let original = fit_tone(pairs.iter().enumerate().filter(|(i, _)| i % 3 != 0).map(|(_, (x, y))| (luma(*x), luma(*y))).collect()).unwrap();
        // Both sample sets start at a multiple of three, so this split matches
        // the helper's separate colour and extra-sample indices.
        let candidate = fit_tone(
            sensor
                .data
                .iter()
                .zip(&reference.data)
                .enumerate()
                .filter(|(i, _)| i % 3 != 0)
                .map(|(_, (x, y))| (luma(x.map(f64::from)), luma(y.map(f64::from))))
                .collect(),
        )
        .unwrap();
        let held: Pairs = sensor.data.iter().zip(&reference.data).take(600).step_by(3).map(|(x, y)| (x.map(f64::from), y.map(f64::from))).collect();
        assert_eq!(held.len(), 200);
        assert!(held.iter().all(|(_, y)| (0.0003..0.015).contains(&luma(*y))));
        assert!(rank_correlation_ties(&held) > 0.99);
        let error = |tone: &CameraTone| {
            held.iter().map(|(x, y)| (f64::from(tone.apply(luma(*x) as f32)).powf(1.0 / 2.2) - luma(*y).powf(1.0 / 2.2)).powi(2)).sum::<f64>()
        };
        assert_ne!(candidate, original);
        let selected = fit_tone_with_shadows(ToneFit::Quantile, &pairs, &sensor, &reference, None, |x| x).unwrap();
        if holdout_matches_original {
            assert!(error(&candidate) > error(&original) + 0.1);
            assert_eq!(selected, original, "a correlated but misleading training set must keep the original curve");
        } else {
            assert!(error(&candidate) < error(&original) * 0.99);
            assert_eq!(selected, candidate, "matching dark training and holdout must accept the candidate");
        }
        let mut previous = 0.0;
        assert_eq!(selected.apply(0.0), 0.0);
        for i in 0..10_000 {
            let y = selected.apply(10f32.powf(-8.0 + i as f32 / 1000.0));
            assert!(y.is_finite() && y >= previous && y <= 1.0);
            previous = y;
        }
        assert_eq!(selected.apply(f32::NAN), 0.0);
        assert_eq!(selected.apply(f32::INFINITY), 0.0);
    }
}

#[test]
fn unreliable_dark_pairs_keep_the_original_tone_and_colour() {
    let (sensor, reference) = steep_toe_scene();
    let dark: Vec<_> = reference.data.iter().enumerate().filter(|(_, p)| luminance_2020(**p) < 0.015).map(|(i, _)| i).collect();
    let mut excluded = sensor.clone();
    for &i in &dark {
        excluded.data[i] = [f32::NAN; 3];
    }
    let colour = Some((Mat3::IDENTITY, None));
    let original = fit_pairs_with(&excluded, &reference, colour.clone()).unwrap();
    for case in 0..6 {
        let (mut x, mut y) = (sensor.clone(), reference.clone());
        for (j, &i) in dark.iter().enumerate() {
            match case {
                0 => y.data[i] = [0.0; 3],                                            // JPEG black gives no measured shadow signal
                1 => x.data[i] = [0.00001; 3],                                        // below the sensor noise floor
                2 if j >= 95 => y.data[i] = [0.0; 3],                                 // too few dark pairs
                3 => x.data[i] = [0.003; 3],                                          // no scene luminance spread
                4 => y.data[i] = [0.003; 3],                                          // one quantized grey, not a toe
                5 => y.data[i] = [0.0004 + ((j * 7919) % 1000) as f32 * 0.000014; 3], // unrelated noise
                _ => {}
            }
        }
        let look = fit_pairs_with(&x, &y, colour.clone()).unwrap();
        assert_eq!(look.tone, original.tone, "case {case}");
        assert_eq!(look.matrix, original.matrix, "case {case}");
        assert!(look.hue_sat.is_none());
    }
}

#[test]
fn camera_jpeg_matches_the_sensor_proxy_size_exactly() {
    // An ILCE-7RM4 ARW (9504×6336 crop) embeds a 1616×1080 preview; for the camera-profile proxy
    // it is decoded at 384×257 (`proxies`: twice the proxy edge) next to a 192×128 sensor proxy.
    let sensor = fit(&Rgb32f::new(9504, 6336), PROFILE_PROXY, PROFILE_PROXY, Filter::Box);
    assert_eq!((sensor.width, sensor.height), (192, 128));
    let jpeg = Rgb32f::new(384, 257);
    // what fitting gave before: a pixel short, so every pair was rejected
    assert_eq!(fit(&jpeg, sensor.width, sensor.height, Filter::Box).width, 191);
    let r = reference_at(&jpeg, &sensor);
    assert_eq!((r.width, r.height), (192, 128));
    // and at the single-photo proxy size, where fitting happened to land on the same size
    let sensor = fit(&Rgb32f::new(9504, 6336), PROXY, PROXY, Filter::Box);
    let r = reference_at(&Rgb32f::new(192, 129), &sensor);
    assert_eq!((r.width, r.height), (sensor.width, sensor.height));
}

#[test]
fn linear_arw_gets_a_sensor_proxy_without_demosaicing() {
    use crate::sensor::{BlackLevel, ColorData, OpcodeLists, Orientation, RawData, RawFormat, Rect};
    let mut raw = RawImage {
        format: RawFormat::Arw,
        width: 32,
        height: 32,
        cpp: 3,
        data: RawData::F32([0.2, 0.3, 0.4].repeat(32 * 32)),
        cfa: None,
        bits: 16,
        black: BlackLevel::uniform(0.0),
        white: vec![1.0],
        active_area: Rect::new(0, 0, 32, 32),
        crop: Rect::new(0, 0, 32, 32),
        orientation: Orientation::from_exif(1),
        color: ColorData::default(),
        wb_multipliers: Some([1.0; 3]),
        linearized: true,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: Default::default(),
        cinema: Default::default(),
        level_sources: Default::default(),
        lens_metadata: Default::default(),
    };
    let proxy = sensor_proxy(&raw, 2, 384).unwrap();
    assert_eq!((proxy.width, proxy.height), (32, 32));
    assert_eq!(proxy.data[0], [0.2, 0.3, 0.4]);
    raw.data = RawData::F32(Vec::new());
    assert!(sensor_proxy(&raw, 2, 384).is_none());
}
#[test]
fn separates_nonlinear_tone_from_colour_and_keeps_sensor_headroom() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let mut sensor = Rgb32f::new(64, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let ev = 0.05 + (i % 31) as f32 * 0.017;
        *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
        let p = known.apply_f32(*src);
        let y = luminance_2020(p);
        *dst = p.map(|v| v * (1.0 - (-2.5 * y).exp()) / y);
    }
    let original = sensor.clone();
    let fit = fit_pairs(&sensor, &reference).unwrap();
    assert_eq!(sensor.data, original.data);
    let tone = ToneMap::camera(&fit.tone, 0.0, 0.0, 0.0);
    let error: f64 = sensor
        .data
        .iter()
        .zip(&reference.data)
        .map(|(x, y)| {
            let p = displayed(fit.matrix.apply(x.map(f64::from)), &tone);
            (0..3).map(|c| (p[c] - f64::from(y[c])).powi(2)).sum::<f64>() / 3.0
        })
        .sum::<f64>()
        / sensor.data.len() as f64;
    assert!(error.sqrt() < 0.025, "{error}");
    // The colour transform is homogeneous; tone mapping happens only after exposure.
    let p = fit.matrix.apply([2.0, 2.0, 2.0]);
    assert!(luma(p) > 1.0);
    assert!(tone.apply(0.2) < tone.apply(0.4));
}
#[test]
fn accepts_a_much_better_fit_despite_local_camera_processing() {
    // The camera JPEG departs from any global matrix + curve (local tone, vignetting): ±0.12
    // per-pixel deviations, ~0.07 RMS. The fit is still far closer than the fallback.
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let mut sensor = Rgb32f::new(64, 64);
    let mut reference = sensor.clone();
    let mut seed = 0x2545_f491_u32;
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let ev = 0.05 + (i % 31) as f32 * 0.017;
        *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
        let p = known.apply_f32(*src);
        let y = luminance_2020(p);
        *dst = p.map(|v| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed % 2001) as f32 / 1000.0 - 1.0;
            (v * (1.0 - (-2.5 * y).exp()) / y + 0.12 * noise).clamp(0.005, 0.97)
        });
    }
    assert!(fit_pairs(&sensor, &reference).is_some());
}

/// A camera that rotates saturated yellow-greens toward green (like Sony's "Standard" on a lime
/// shirt) can't be followed by a matrix alone: the fitted hue/saturation table must close most
/// of the gap, keep neutrals neutral and keep luminance.
#[test]
fn hue_table_follows_a_hue_dependent_camera_rendering() {
    let to = to_prophoto();
    let from = to.inverse().unwrap();
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    let mut lime = Vec::new();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let ev = 0.05 + (i % 29) as f32 * 0.02;
        // mostly greys and mild colours, plus a patch of saturated yellow-green
        *src = if i % 7 == 0 { [ev * 0.75, ev, ev * 0.2] } else { [ev * (0.85 + (i % 11) as f32 * 0.03), ev, ev * (0.85 + (i % 13) as f32 * 0.025)] };
        let (h, s) = hue_saturation(to.apply(src.map(f64::from))).unwrap();
        // the camera turns hues in 50°..110° (ProPhoto) by up to +15° and boosts their saturation
        let k = (1.0 - ((h - 80.0) / 30.0).powi(2)).max(0.0) * s.min(1.0);
        let mut p = to.apply(src.map(f64::from)).map(|v| v as f32);
        let table = HsvTable {
            hue_divisions: 1,
            sat_divisions: 2,
            val_divisions: 1,
            data: vec![[0.0, 1.0, 1.0], [15.0 * k as f32, 1.0 + 0.3 * k as f32, 1.0]],
            srgb_value: false,
        };
        p = table.apply(p);
        let p = from.apply(p.map(f64::from)).map(|v| v as f32);
        let scale = luminance_2020(*src) / luminance_2020(p);
        let p = p.map(|v| v * scale);
        let y = luminance_2020(p);
        *dst = p.map(|v| v * (1.0 - (-2.5 * y).exp()) / y);
        if i % 7 == 0 {
            lime.push(i);
        }
    }
    let fit = fit_pairs(&sensor, &reference).unwrap();
    let table = fit.hue_sat.as_ref().expect("a hue/saturation table is fitted");
    let hue_sat = HueSat::new(table).unwrap();
    let hue_error = |with_table: bool| {
        lime.iter()
            .map(|&i| {
                let p = fit.matrix.apply(sensor.data[i].map(f64::from)).map(|v| v as f32);
                let p = if with_table { hue_sat.apply(p) } else { p };
                let (h, _) = hue_saturation(to.apply(p.map(f64::from))).unwrap();
                let (t, _) = hue_saturation(to.apply(reference.data[i].map(f64::from))).unwrap();
                ((h - t + 180.0).rem_euclid(360.0) - 180.0).abs()
            })
            .sum::<f64>()
            / lime.len() as f64
    };
    let (before, after) = (hue_error(false), hue_error(true));
    assert!(after < before * 0.5, "lime hue error {before:.2}° -> {after:.2}°");
    // neutrals pass through and luminance is kept
    let grey = [0.3, 0.3, 0.3];
    assert!(hue_sat.apply(grey).iter().all(|v| (v - 0.3).abs() < 1e-4), "{:?}", hue_sat.apply(grey));
    let green = [0.2, 0.4, 0.05];
    assert!((luminance_2020(hue_sat.apply(green)) - luminance_2020(green)).abs() < 1e-5);
}

/// A camera that saturates shadows and bleaches highlights toward white (a per-channel curve),
/// which a luminance tone curve can't: the fitted chroma curve must follow it, so bright
/// warm-tinted rock renders white instead of cream.
#[test]
fn chroma_curve_follows_highlight_bleaching() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let camera_chroma = |o: f32| 1.3 - 1.1 * o;
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let ev = 0.02 + (i % 37) as f32 * 0.03;
        *src = [ev * (0.75 + (i % 11) as f32 * 0.05), ev, ev * (0.7 + (i % 13) as f32 * 0.05)];
        let p = known.apply_f32(*src);
        let y = luminance_2020(p);
        let o = 1.0 - (-2.5 * y).exp();
        let k = camera_chroma(o);
        *dst = p.map(|v| (o + (v * o / y - o) * k).clamp(0.0, 0.999));
    }
    let fit = fit_pairs(&sensor, &reference).unwrap();
    // relative to the matrix, which already carries the average colourfulness
    let chroma = fit.tone.chroma();
    assert!(chroma[6] < 0.5 * chroma[1], "highlights bleach relative to shadows: {chroma:?}");
    // and the rendered highlights land on the camera's, far closer than without the curve
    let with = ToneMap::camera(&fit.tone, 0.0, 0.0, 0.0);
    let without = ToneMap::camera(&fit.tone.with_chroma([1.0; crate::tone::CHROMA_N]).unwrap(), 0.0, 0.0, 0.0);
    let highlight_error = |map: &ToneMap| -> f64 {
        sensor
            .data
            .iter()
            .zip(&reference.data)
            .filter(|(_, y)| luminance_2020(**y) > 0.7)
            .map(|(x, y)| {
                let p = displayed(fit.matrix.apply(x.map(f64::from)), map);
                (0..3).map(|c| (p[c] - f64::from(y[c])).powi(2)).sum::<f64>()
            })
            .sum()
    };
    let (a, b) = (highlight_error(&without), highlight_error(&with));
    assert!(b < 0.5 * a, "highlight error {a:.4} -> {b:.4}");
}

/// Pooled pairs of several "photos" recover the camera's matrix, and a photo given that
/// colour model (a camera profile) keeps it, fitting only its own tone.
#[test]
fn profile_colour_is_pooled_and_then_used_as_given() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let photo = |seed: usize, strength: f32| {
        let mut sensor = Rgb32f::new(64, 48);
        let mut reference = sensor.clone();
        for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
            let i = i + seed * 7;
            let ev = 0.04 + (i % 23) as f32 * 0.02;
            *src = [ev * (0.7 + (i % 11) as f32 * 0.05), ev, ev * (0.7 + (i % 13) as f32 * 0.05)];
            let p = known.apply_f32(*src);
            let y = luminance_2020(p);
            // each photo has its own tone (DRO, picture style)
            *dst = p.map(|v| v * (1.0 - (-strength * y).exp()) / y);
        }
        (sensor, reference)
    };
    let mut pool = Vec::new();
    for seed in 0..4 {
        let (sensor, reference) = photo(seed, 2.0 + seed as f32 * 0.5);
        pool.extend(collect_pairs(&sensor, &reference, 0.05, None, false).unwrap().0);
    }
    let (matrix, _) = fit_profile(&pool).unwrap();
    // the camera's colours (luminance-normalised: the tone curve sets brightness)
    for x in [[0.8, 1.0, 0.75], [1.1, 1.0, 0.8], [0.75, 1.0, 1.2], [1.0; 3]] {
        let (a, b) = (matrix.apply(x), known.apply(x));
        let (la, lb) = (luma(a), luma(b));
        assert!((0..3).all(|c| (a[c] / la - b[c] / lb).abs() < 0.01), "{x:?}: {a:?} vs {b:?}");
    }
    let (sensor, reference) = photo(9, 3.5);
    let look = fit_pairs_with(&sensor, &reference, Some((matrix, None))).unwrap();
    assert_eq!(look.matrix, matrix);
    assert!(look.hue_sat.is_none());
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let error = sensor
        .data
        .iter()
        .zip(&reference.data)
        .map(|(x, y)| {
            let p = displayed(look.matrix.apply(x.map(f64::from)), &tone);
            (0..3).map(|c| (p[c] - f64::from(y[c])).powi(2)).sum::<f64>() / 3.0
        })
        .sum::<f64>()
        / sensor.data.len() as f64;
    assert!(error.sqrt() < 0.02, "this photo's own tone is followed: RMS {}", error.sqrt());
}

#[test]
fn profile_fits_tone_without_learning_colour_from_a_dull_scene() {
    let matrix = Mat3([[2.0, -0.02, 0.0], [0.0, 2.0, 0.0], [0.0, -0.03, 2.0]]);
    let mut sensor = Rgb32f::new(64, 48);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        *src = [0.02 + (i % 997) as f32 * 0.00045; 3];
        let p = matrix.apply_f32(*src);
        let y = luminance_2020(p);
        *dst = p.map(|v| v * (1.0 - (-2.5 * y).exp()) / y);
    }
    assert!(fit_pairs(&sensor, &reference).is_none(), "not enough colour to learn a matrix");
    let look = fit_pairs_with(&sensor, &reference, Some((matrix, None))).unwrap();
    assert_eq!(look.matrix, matrix);
    reference.map_in_place(|p| [luminance_2020(p); 3]);
    assert!(fit_pairs_with(&sensor, &reference, Some((matrix, None))).is_none(), "monochrome JPEGs remain rejected");
}

#[test]
fn rejected_profile_preserves_a_usable_per_photo_fit() {
    let matrix = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let wrong = Mat3([[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]);
    let mut sensor = Rgb32f::new(64, 64);
    let mut reference = sensor.clone();
    let mut seed = 0x2545_f491_u32;
    for (src, dst) in sensor.data.iter_mut().zip(&mut reference.data) {
        *src = std::array::from_fn(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            0.05 + (seed % 1000) as f32 * 0.0003
        });
        let p = matrix.apply_f32(*src);
        let y = luminance_2020(p);
        *dst = p.map(|v| (v * (1.0 - (-2.5 * y).exp()) / y).clamp(0.0, 0.999));
    }
    let per_photo = fit_pairs(&sensor, &reference).unwrap();
    let look = fit_pairs_with(&sensor, &reference, Some((wrong, None))).unwrap();
    assert_eq!(look.matrix, per_photo.matrix, "a rejected camera profile must not suppress the per-photo fit");
    assert_eq!(look.tone, per_photo.tone);
}

#[test]
fn hue_sat_passes_black_and_non_finite_pixels_through() {
    let table = HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[10.0, 1.5, 1.0]; 8], srgb_value: false };
    let hue_sat = HueSat::new(&table).unwrap();
    assert_eq!(hue_sat.apply([0.0; 3]), [0.0; 3]);
    let nan = hue_sat.apply([f32::NAN, 0.2, 0.1]);
    assert!(nan[0].is_nan() && nan[1] == 0.2 && nan[2] == 0.1);
    assert!(fit_hue_sat(&[], &Mat3::IDENTITY).is_none(), "too few samples");
}

#[test]
fn hue_sat_never_turns_near_black_noise_white() {
    // a saturating table, as fitted to high-ISO Sony files; near-black noise with mixed-sign channels
    let table = HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[10.0, 1.5, 1.0]; 8], srgb_value: false };
    let hue_sat = HueSat::new(&table).unwrap();
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0
    };
    let mut worst = 0.0f32;
    for _ in 0..200_000 {
        let rgb = [0.002 + 0.004 * next(), 0.002 + 0.004 * next(), 0.002 + 0.004 * next()];
        if luminance_2020(rgb) <= 0.0 {
            continue;
        }
        let out = hue_sat.apply(rgb);
        let scale = rgb.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        worst = worst.max(out.iter().fold(0.0f32, |m, v| m.max(v.abs())) / scale);
    }
    assert!(worst < 20.0, "a near-black pixel came out {worst}× its largest channel");
}

/// `apply_lanes` gives `apply`'s bits for any colour: random, near-black noise (the restore's blend), grey,
/// negative, tiny, above 1, huge, NaN and infinite components; on a fitted look's shape and a saturating table.
#[test]
fn hue_sat_lanes_match_one_by_one_bit_for_bit() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let unit = |r: u64, i: usize| ((r >> (16 * i)) & 0xffff) as f32 / 65535.0;
    let fitted = (0..72 * 5 * 5)
        .map(|_| {
            let r = next();
            [unit(r, 0) * 60.0 - 30.0, 0.5 + unit(r, 1), 1.0]
        })
        .collect();
    let fitted = HsvTable { hue_divisions: 72, sat_divisions: 5, val_divisions: 5, data: fitted, srgb_value: true };
    let saturating = HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[10.0, 1.5, 1.0]; 8], srgb_value: false };
    let special = [0.0, -0.0, 1e-30, -1e-6, 0.002, 0.5, 1.0, 3.0, 1e30, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    let mut colours = Vec::new();
    for a in special {
        for b in special {
            colours.extend(special.map(|c| [a, b, c]));
        }
    }
    for _ in 0..60_000 {
        let r = next();
        let (offset, scale) = [(0.15, 1.3), (0.15e-3, 1.3e-3), (0.6, 5.0), (-0.002, 0.008)][(r >> 62) as usize];
        colours.push(std::array::from_fn(|i| unit(r, i) * scale - offset));
    }
    for table in [fitted, saturating] {
        let hue_sat = HueSat::new(&table).unwrap();
        for chunk in colours.chunks(16) {
            let rgb: [[f32; 16]; 3] = std::array::from_fn(|c| std::array::from_fn(|l| chunk.get(l).map_or(0.0, |p| p[c])));
            let out = hue_sat.apply_lanes(rgb);
            for (l, p) in chunk.iter().enumerate() {
                let (got, want) = ([out[0][l], out[1][l], out[2][l]], hue_sat.apply(*p));
                assert_eq!(got.map(f32::to_bits), want.map(f32::to_bits), "{p:?}: {got:?} vs {want:?}");
            }
        }
    }
}

#[test]
fn hue_sat_restore_weight_blends_smoothly_between_the_thresholds() {
    assert_eq!(restore_weight(0.0), 0.0);
    assert_eq!(restore_weight(NO_LUMINANCE_KEPT), 0.0);
    assert_eq!(restore_weight(MIN_LUMINANCE_KEPT), 1.0);
    assert_eq!(restore_weight(1.0), 1.0);
    assert_eq!(restore_weight(f32::NAN), 0.0);
    // monotonic and without a step: neighbouring shares 0.001 apart differ by little
    let mut last = 0.0f32;
    for i in 0..=400 {
        let w = restore_weight(i as f32 * 0.001);
        assert!(w >= last && w - last < 0.02, "step at {}: {last} → {w}", i as f32 * 0.001);
        last = w;
    }
}

#[test]
fn hue_sat_keeps_colours_the_table_leaves_bright_exactly_as_before() {
    // the blend only touches colours whose luminance the table cut below MIN_LUMINANCE_KEPT
    let table = HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[10.0, 1.5, 1.0]; 8], srgb_value: false };
    let hue_sat = HueSat::new(&table).unwrap();
    let mul = |m: &[[f32; 3]; 3], v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) };
    for rgb in [[0.4, 0.2, 0.1], [0.05, 0.3, 0.2], [0.18, 0.18, 0.18], [0.6, 0.1, 0.5]] {
        let out = mul(&hue_sat.from, table.apply(mul(&hue_sat.to, rgb)));
        let (before, after) = (luminance_2020(rgb), luminance_2020(out));
        assert!(after >= MIN_LUMINANCE_KEPT * before);
        assert_eq!(hue_sat.apply(rgb), out.map(|v| v * before / after));
    }
}

fn raw_of(format: RawFormat, make: &str, model: &str, color: crate::sensor::ColorData) -> RawImage {
    let area = crate::sensor::Rect::new(0, 0, 2, 2);
    RawImage {
        format,
        width: 2,
        height: 2,
        cpp: 1,
        data: crate::sensor::RawData::U16(vec![0; 4]),
        cfa: None,
        bits: 14,
        black: Default::default(),
        white: vec![16383.0],
        active_area: area,
        crop: area,
        orientation: Default::default(),
        color,
        wb_multipliers: Some([2.0, 1.0, 1.5]),
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: Default::default(),
        camera: crate::sensor::Camera { make: Some(make.to_owned()), model: Some(model.to_owned()), ..Default::default() },
        cinema: Default::default(),
        level_sources: Default::default(),
        lens_metadata: Default::default(),
    }
}

/// The lens warp the colour fit follows is the first `WarpRectilinear`'s green plane; no warp, a DNG 1.6 chain
/// (phone DNGs, which carry colour matrices), a centre outside the area or a non-finite term give none.
#[test]
fn lens_geometry_reads_the_first_rectilinear_warp() {
    use crate::sensor::Opcode;
    let warp = |k0: f64, c: [f64; 2]| Opcode::WarpRectilinear {
        planes: vec![[9.0, 0.0, 0.0, 0.0, 0.0, 0.0], [k0, 0.1, -0.02, 0.003, 0.0, 0.0], [9.0, 0.0, 0.0, 0.0, 0.0, 0.0]],
        center: c,
    };
    assert_eq!(lens_geometry(&[warp(0.97, [0.5, 0.4])]), Some(LensGeometry { k: [0.97, 0.1, -0.02, 0.003], center: [0.5, 0.4] }));
    let single = Opcode::WarpRectilinear { planes: vec![[1.02, 0.0, 0.0, 0.0, 0.0, 0.0]], center: [0.5, 0.5] };
    assert_eq!(lens_geometry(std::slice::from_ref(&single)).map(|g| g.k[0]), Some(1.02));
    assert!(lens_geometry(&[]).is_none());
    assert!(lens_geometry(&[warp(1.0, [1.5, 0.5])]).is_none());
    assert!(lens_geometry(&[warp(f64::NAN, [0.5, 0.5])]).is_none());
    let mut p = [0.0; craft_raw::opcodes::WR2_LEN];
    p[0] = 1.0;
    p[18] = 1e6;
    let chain = Opcode::WarpRectilinear2 { planes: vec![p], center: [0.5, 0.5], reciprocal: false, optional: false };
    assert!(lens_geometry(&[chain, single]).is_none());
}

/// A binned sensor proxy moved into the camera JPEG's geometry: the identity warp leaves it as it is, a warp
/// whose source radius is 0.8 of the output's samples nearer the centre, output pixels whose source falls outside
/// the proxy are NaN (left out of the pairs), and a proxy of another size is left alone.
#[test]
fn the_sensor_proxy_follows_the_cameras_lens_warp() {
    let mut raw = raw_of(RawFormat::Nef, "NIKON CORPORATION", "NIKON Z 6_2", Default::default());
    raw.active_area = crate::sensor::Rect::new(0, 0, 64, 48);
    raw.crop = raw.active_area;
    // a 1/4 proxy whose pixels hold their own coordinates
    let mut img = Rgb32f::new(16, 12);
    for (i, p) in img.data.iter_mut().enumerate() {
        *p = [(i % 16) as f32, (i / 16) as f32, 1.0];
    }
    let geometry = |k0: f64| LensGeometry { k: [k0, 0.0, 0.0, 0.0], center: [0.5, 0.5] };
    let same = into_lens_geometry(&img, &raw, 4, &geometry(1.0)).unwrap();
    assert!(same.data.iter().zip(&img.data).all(|(a, b)| (0..3).all(|c| (a[c] - b[c]).abs() < 1e-4)));
    // the corner pixel's centre (1.5, 1.5) maps to (31.5 - 0.8 * 30, 23.5 - 0.8 * 22) = (7.5, 5.9) on the sensor
    let zoomed = into_lens_geometry(&img, &raw, 4, &geometry(0.8)).unwrap();
    assert!((zoomed.data[0][0] - 1.5).abs() < 1e-4 && (zoomed.data[0][1] - 1.1).abs() < 1e-4, "{:?}", zoomed.data[0]);
    let outside = into_lens_geometry(&img, &raw, 4, &geometry(1.3)).unwrap();
    assert!(outside.data[0][0].is_nan() && outside.data[6 * 16 + 8][0].is_finite());
    assert!(into_lens_geometry(&Rgb32f::new(15, 12), &raw, 4, &geometry(1.0)).is_none());
}

/// Step 1: a file's own colour matrices win over everything, including a spectral row for its model; no JPEG fit
/// is tried.
#[test]
fn a_files_own_matrices_come_first() {
    let own = Mat3([[0.6, -0.1, -0.05], [-0.4, 1.3, 0.1], [-0.1, 0.2, 0.7]]);
    let color = crate::sensor::ColorData { illuminant: [21, 0], color_matrix: [Some((own).0), None], ..Default::default() };
    let mut raw = raw_of(RawFormat::Cr2, "Canon", "Canon EOS 600D", color.clone());
    let (_, t, look) = starting_colour_with(
        &mut raw,
        |_, _| panic!("no JPEG fit for a raw with its own matrices"),
        |_, _| panic!("no black-and-white JPEG fit for a raw with its own matrices"),
        |_, _| panic!("no dark tone fit for a raw with its own matrices"),
    );
    assert_eq!(raw.color, color);
    assert!(!t.matrix_is_fallback && look.is_none());
}

/// Steps 2 and 3 before step 4: an accepted JPEG fit (with or without a camera profile) is kept for a model that
/// has spectral matrices, and the spectral matrices are not written. Photos that had a fitted look before the
/// spectral table render exactly as they did.
#[test]
fn an_accepted_jpeg_fit_comes_before_the_spectral_matrices() {
    for (make, model) in [("SONY", "ILCE-7M4"), ("Canon", "Canon EOS 600D")] {
        assert!(crate::sensor::spectral::find(Some(make), model).is_some(), "{model}");
        let mut raw = raw_of(RawFormat::Cr2, make, model, Default::default());
        let fitted = dro_test_look(&std::array::from_fn(|i| 0.006 * 1.13f32.powi(i as i32)), &|x| x.sqrt());
        let (xy, t, look) = starting_colour_with(
            &mut raw,
            |raw, t| {
                assert!(t.matrix_is_fallback && !crate::sensor::color::has_matrix(&raw.color), "the fit sees the raw without spectral matrices");
                (Some(fitted.clone()), false)
            },
            |_, _| panic!("no black-and-white JPEG fit after an accepted fit"),
            |_, _| panic!("no dark tone fit after an accepted fit"),
        );
        assert_eq!(look.map(|l| l.matrix), Some(fitted.matrix), "{model}");
        assert_eq!(raw.color, crate::sensor::ColorData::default(), "{model}: spectral matrices left out");
        assert!(t.matrix_is_fallback, "{model}");
        assert_eq!(xy, crate::sensor::color::as_shot_white_xy(&raw));
    }
}

/// Step 4: when the JPEG fit fails (here: no camera JPEG at all), a model with a spectral row takes its matrices,
/// whether or not it has a camera profile (the ILCE-7M4 has a bundled one).
#[test]
fn a_failed_jpeg_fit_falls_back_to_the_spectral_matrices() {
    assert!(crate::look::profiles::get("ILCE-7M4").is_some());
    let camera = crate::sensor::spectral::find(Some("SONY"), "ILCE-7M4").unwrap();
    let mut raw = raw_of(RawFormat::Arw, "SONY", "ILCE-7M4", Default::default());
    let (_, t, look) = starting_colour(&mut raw, &[], None, None);
    assert!(look.is_none());
    assert_eq!(raw.color.color_matrix, camera.color_matrix.map(|m| Some(m.0)));
    assert!(!t.matrix_is_fallback);

    let camera = crate::sensor::spectral::find(Some("Canon"), "Canon EOS 600D").unwrap();
    let mut raw = raw_of(RawFormat::Cr2, "Canon", "Canon EOS 600D", Default::default());
    let (xy, t, look) =
        starting_colour_with(&mut raw, |_, _| (None, false), |_, _| panic!("a colour JPEG never takes the black-and-white step"), |_, _| None);
    assert!(look.is_none());
    assert_eq!(raw.color.illuminant, [17, 21]);
    assert_eq!(raw.color.forward_matrix, camera.forward_matrix.map(|m| Some(m.0)));
    assert!(!t.matrix_is_fallback);
    assert_eq!(xy, crate::sensor::color::as_shot_white_xy(&raw));
    // white-balanced white stays white
    let w = t.matrix.apply([1.0; 3]);
    assert!(w.iter().all(|v| (v - 1.0).abs() < 1e-4), "{w:?}");
}

/// After step 4 or 5: a photo whose JPEG gave no fit and is black and white gets its tone curve on top of the
/// colour it has by then, the spectral matrices for a model in the table, the neutral fallback's otherwise
/// (issue storytold/lightcraft#577). The black-and-white fit sees that transform.
#[test]
fn a_black_and_white_jpeg_adds_its_tone_on_top_of_the_colour_it_gets_otherwise() {
    let toned = CameraLook {
        matrix: Mat3::IDENTITY,
        tone: dro_test_look(&std::array::from_fn(|i| 0.006 * 1.13f32.powi(i as i32)), &|x| x.sqrt()).tone,
        hue_sat: None,
    };
    let camera = crate::sensor::spectral::find(Some("Canon"), "Canon EOS 600D").unwrap();
    let mut raw = raw_of(RawFormat::Cr2, "Canon", "Canon EOS 600D", Default::default());
    let (xy, t, look) = starting_colour_with(
        &mut raw,
        |_, _| (None, true),
        |raw, t| {
            assert!(!t.matrix_is_fallback && raw.color.forward_matrix == camera.forward_matrix.map(|m| Some(m.0)), "the spectral matrices are in");
            Some(toned.clone())
        },
        |_, _| panic!("an accepted monochrome look keeps its tone"),
    );
    assert!(look.as_ref().is_some_and(|l| l.matrix == Mat3::IDENTITY && l.hue_sat.is_none()), "the tone curve on top");
    assert!(!t.matrix_is_fallback);
    assert_eq!(xy, crate::sensor::color::as_shot_white_xy(&raw));

    let mut raw = raw_of(RawFormat::Cr2, "Canon", "Canon EOS 7D", Default::default());
    let (_, t, look) = starting_colour_with(
        &mut raw,
        |_, _| (None, true),
        |_, t| {
            assert!(t.matrix_is_fallback, "outside the table: the neutral fallback's colour");
            Some(toned.clone())
        },
        |_, _| panic!("an accepted monochrome look keeps its tone"),
    );
    assert!(look.is_some() && t.matrix_is_fallback);
    assert_eq!(raw.color, crate::sensor::ColorData::default());
}

/// Step 5: a camera outside the table whose fit fails keeps the neutral fallback.
#[test]
fn a_camera_outside_the_table_keeps_the_fallback() {
    let mut raw = raw_of(RawFormat::Cr2, "Canon", "Canon EOS 7D", Default::default());
    let (_, t, look) = starting_colour(&mut raw, &[], None, None);
    assert!(look.is_none() && t.matrix_is_fallback);
    assert_eq!(raw.color, crate::sensor::ColorData::default());
    // without a model there is no table row
    let mut raw = raw_of(RawFormat::Cr2, "Canon", "", Default::default());
    raw.camera.model = None;
    assert!(starting_colour(&mut raw, &[], None, None).1.matrix_is_fallback);
}

#[test]
fn supported_raws_get_a_file_local_look() {
    // every decoded raw without a colour matrix of its own (issue storytold/lightcraft#310: CR2 rendered flat with the fallback)
    let local = [
        RawFormat::Arw,
        RawFormat::Nef,
        RawFormat::Nrw,
        RawFormat::Rw2,
        RawFormat::Raf,
        RawFormat::Cr3,
        RawFormat::Cr2,
        RawFormat::Pef,
        RawFormat::Srw,
        RawFormat::Orf,
        RawFormat::Mrw,
        RawFormat::Erf,
    ];
    assert!(local.into_iter().all(file_local_look));
    // DNG carries its own colour model
    assert!(!file_local_look(RawFormat::Dng));
}

#[test]
fn cr3_camera_framing_is_measured_independently_of_tone_and_rejects_unrelated_edges() {
    let (w, h) = (192, 128);
    let mut sensor = Rgb32f::new(w, h);
    for (i, pixel) in sensor.data.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        let value = 0.35 + 0.13 * (x * 0.15 + y * 0.22).sin() + 0.1 * (x * 0.33 - y * 0.14).sin() + 0.07 * (x * 0.09 - y * 0.31).cos();
        *pixel = [value; 3];
    }
    let expected = Cr3Framing { scale: 0.96, offset: [0.01, -0.006] };
    let mut reference = Rgb32f::new(w, h);
    for (i, pixel) in reference.data.iter_mut().enumerate() {
        let (x, y) = expected.source((i % w) as f32 + 0.5, (i / w) as f32 + 0.5, w, h);
        *pixel = sensor.sample_bilinear(x, y).map(|v| v.powf(0.7));
    }
    let measured = estimate_cr3_framing(&sensor, &reference).expect("coherent held-out edges locate camera framing despite nonlinear tone");
    assert!((measured.scale - expected.scale).abs() < 0.006, "{measured:?}");
    assert!((measured.offset[0] - expected.offset[0]).abs() < 0.003, "{measured:?}");
    assert!((measured.offset[1] - expected.offset[1]).abs() < 0.003, "{measured:?}");
    assert!(estimate_cr3_framing(&sensor, &sensor).is_none(), "already aligned pixels stay unchanged");
    reference.data.fill([0.2; 3]);
    assert!(estimate_cr3_framing(&sensor, &reference).is_none(), "flat previews cannot establish framing");
    for (i, pixel) in reference.data.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        *pixel = [0.35 + 0.13 * (x * 0.41 - y * 0.07).sin() + 0.1 * (x * 0.04 + y * 0.39).cos(); 3];
    }
    assert!(estimate_cr3_framing(&sensor, &reference).is_none(), "unrelated edges cannot change the framing");
}

/// A merged DNG of a raw's camera RGB (issue storytold/lightcraft#965) carries the colour the raw opens with: its tags give the same
/// white balance, colour and brightness as the fitted look at the as-shot white, the look's hue/saturation
/// table and its tone curve; without a look, those of the camera transform.
#[test]
fn dng_colour_renders_like_the_raws_starting_colour() {
    use crate::sensor::color::{as_shot_white_of, camera_transform_of};
    let t = camera_transform_of(&ColorData::default(), Xy::new(0.36, 0.37));
    assert!(t.wb.iter().any(|w| (w - 1.0).abs() > 0.1), "a white balance to keep: {:?}", t.wb);
    let look = dro_test_look(&std::array::from_fn(|i| 0.005 * 1.2f32.powi(i as i32)), &|x| 0.95 * x / (x + 0.1));
    let samples = [[0.2, 0.3, 0.1], [0.05, 0.02, 0.4], [0.6, 0.6, 0.6], [0.01, 0.5, 0.03]];
    let render = |m: &Mat3, wb: [f32; 3], ev: f64, c: [f64; 3]| m.apply(std::array::from_fn(|i| c[i] * wb[i] as f64)).map(|v| v * ev.exp2());
    for look in [Some(&look), None] {
        let dng = dng_colour(&ColorData::default(), &crate::load::DngWhite::At(t.white_xy), &t, look).unwrap();
        let back = camera_transform_of(&dng, as_shot_white_of(&dng, None));
        for i in 0..3 {
            assert!((back.wb[i] - t.wb[i]).abs() < 1e-4 * t.wb[i], "{:?} vs {:?}", back.wb, t.wb);
        }
        let m = look.map_or(t.matrix, |l| l.matrix.mul(&t.matrix));
        for c in samples {
            let (want, got) = (render(&m, t.wb, t.baseline_exposure, c), render(&back.matrix, back.wb, dng.baseline_exposure, c));
            assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 1e-5), "{c:?}: {want:?} vs {got:?}");
        }
        assert_eq!(dng.profile.hue_sat_for(1.0), look.and_then(|l| l.hue_sat.clone()));
        let tone = dng.profile.tone_curve.as_ref().and_then(crate::load::dng_tone_curve);
        match look {
            Some(look) => {
                let tone = tone.expect("the look's tone curve");
                let worst =
                    (0..=120).map(|i| 2f32.powf(-10.0 + i as f32 / 12.0)).map(|x| (tone.apply(x) - look.tone.apply(x)).abs()).fold(0.0, f32::max);
                // (the reader resamples a `ProfileToneCurve` at 32 knots: 0.0031 of white at worst here)
                assert!(worst < 4e-3, "{worst}");
            }
            None => assert!(tone.is_none()),
        }
    }
}

/// The merged DNG declares the white balance its source shows, also when the look's matrix doesn't keep white
/// neutral, as fitted looks often don't, and renders at As Shot as the source does. Real ARW merges declared their
/// colour matrix from the look: As Shot about 7000 K / +115 where the ARWs show 4800 K / +8.
#[test]
fn dng_colour_declares_the_white_balance_the_source_shows() {
    use crate::load::DngWhite;
    use crate::sensor::color::{as_shot_white_of, camera_transform_of, wb_multipliers};
    let tags = ColorData::default();
    let as_shot = [2.5039f32, 1.0, 1.5078];
    let t = camera_transform_of(&tags, as_shot_white_of(&tags, Some(as_shot)));
    let mut look = dro_test_look(&std::array::from_fn(|i| 0.005 * 1.2f32.powi(i as i32)), &|x| 0.95 * x / (x + 0.1));
    look.matrix = Mat3([[1.75, -0.4, -0.2], [-0.2, 1.35, -0.2], [0.05, -0.3, 1.3]]);
    let tint = look.matrix.apply([1.0; 3]);
    assert!((tint[0] - tint[1]).abs() > 0.1, "a look that tints white: {tint:?}");
    let renders_as_shot = |dng: &ColorData| {
        let r = camera_transform_of(dng, as_shot_white_of(dng, None));
        assert!(r.wb.iter().zip(t.wb).all(|(a, b)| (a - b).abs() < 1e-4 * b), "{:?} vs {:?}", r.wb, t.wb);
        let m = look.matrix.mul(&t.matrix);
        for c in [[0.2, 0.3, 0.1], [0.05, 0.02, 0.4], [0.6, 0.6, 0.6]] {
            let want = m.apply(std::array::from_fn(|i| c[i] * t.wb[i] as f64)).map(|v| v * t.baseline_exposure.exp2());
            let got = r.matrix.apply(std::array::from_fn(|i| c[i] * r.wb[i] as f64)).map(|v| v * dng.baseline_exposure.exp2());
            assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 1e-4), "{c:?}: {want:?} vs {got:?}");
        }
    };
    // a white the source's scale shows (Sony's presets, the relative scale's 6500 K / 0)
    for xy in [crate::color::cct::temp_tint_to_xy(5600.0, 3.0), crate::color::cct::temp_tint_to_xy(6500.0, 0.0)] {
        let dng = dng_colour(&tags, &DngWhite::At(xy), &t, Some(&look)).unwrap();
        let back = as_shot_white_of(&dng, None);
        assert!((back.x - xy.x).abs() < 1e-6 && (back.y - xy.y).abs() < 1e-6, "as shot {back:?}, shown {xy:?}");
        renders_as_shot(&dng);
    }
    // a camera with measured matrices (the ILCE-7M3): every white read through them, as for the source
    let camera = crate::sensor::spectral::find(Some("SONY"), "ILCE-7M3").expect("a covered camera");
    let mut model = tags.clone();
    assert!(camera.fill(&mut model));
    let shown = crate::sensor::spectral::as_shot_white_through(camera, &tags, Some(as_shot)).unwrap();
    let dng = dng_colour(&tags, &DngWhite::Matrices(Box::new(model.clone())), &t, Some(&look)).unwrap();
    let back = as_shot_white_of(&dng, None);
    assert!((back.x - shown.x).abs() < 1e-5 && (back.y - shown.y).abs() < 1e-5, "as shot {back:?}, shown {shown:?}");
    for w in [Xy::new(0.3127, 0.329), Xy::new(0.4476, 0.4074), Xy::new(0.3457, 0.3585)] {
        let (a, b) = (wb_multipliers(&model, w), wb_multipliers(&dng, w));
        assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6 * a), "{w:?}: {a:?} vs {b:?}");
    }
    renders_as_shot(&dng);
}

/// A look with tone curve `f` at knots `xs` and a colour model and chroma curve to keep.
fn dro_test_look(xs: &[f32; 32], f: &dyn Fn(f32) -> f32) -> CameraLook {
    CameraLook {
        matrix: Mat3([[1.6, -0.4, -0.2], [-0.2, 1.4, -0.2], [0.0, -0.3, 1.3]]),
        tone: CameraTone::new(xs.map(|x| [x, f(x)])).unwrap().with_chroma([1.3, 1.2, 1.1, 1.0, 1.0, 0.9, 0.6, 0.3]).unwrap(),
        hue_sat: Some(HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[5.0, 1.1, 1.0]; 8], srgb_value: false }),
    }
}

/// `fixed` (from `fitted`) is `min(fitted, Sony's DRO-off curve)` at every knot but the
/// second-last, which may stay higher (at most the fitted curve) to keep the shoulder past the
/// last knot below the fitted one; nowhere brighter than `fitted`, colour unchanged.
fn assert_lowered(fitted: &CameraLook, fixed: &CameraLook, what: &str) {
    let dro_off = sony_dro_off_tone().unwrap();
    assert_eq!((fixed.matrix, &fixed.hue_sat, fixed.tone.chroma()), (fitted.matrix, &fitted.hue_sat, fitted.tone.chroma()), "{what}");
    for (i, (k, own)) in fixed.tone.knots().iter().zip(fitted.tone.knots()).enumerate() {
        let lowered = own[1].min(dro_off.apply(own[0]));
        assert_eq!(k[0], own[0], "{what}: knot {i}");
        if i == 30 {
            assert!(k[1] >= lowered && k[1] <= own[1], "{what}: knot {i} {} not in {lowered}..={}", k[1], own[1]);
        } else {
            assert!((k[1] - lowered).abs() < 1e-6, "{what}: knot {i} {} vs {lowered}", k[1]);
        }
    }
    assert!(at_most(&fixed.tone, &fitted.tone), "{what}: brighter than the fitted curve somewhere");
    // the same check spelled out, from deep shadows to far past the last knot (the shoulder)
    for i in 0..=2400 {
        let x = 0.001 * 1.005f32.powi(i);
        assert!(fixed.tone.apply(x) <= fitted.tone.apply(x) + 1e-5, "{what} at {x}: {} > {}", fixed.tone.apply(x), fitted.tone.apply(x));
    }
    // V5's shoulder stays below too whenever the lowered last segment's slope over its squared gap to
    // white is at most the fitted one's (`dro_off` doesn't enforce it, so V1 to V4 keep their curve;
    // issue storytold/lightcraft#1632)
    if v5_bounded(&fixed.tone, &fitted.tone) {
        assert!(v5_below(&fixed.tone, &fitted.tone), "{what}: brighter than the fitted curve under V5");
    }
}

/// Whether the last segment of `a`, over its squared gap to white, is at most `b`'s: then `a`'s V5
/// shoulder ([`CameraTone::apply_rolloff`]) is nowhere above `b`'s (issue storytold/lightcraft#1632).
fn v5_bounded(a: &CameraTone, b: &CameraTone) -> bool {
    let rate = |t: &CameraTone| {
        let k = t.knots();
        ((k[31][1] - k[30][1]) / (k[31][0] - k[30][0])).max(0.0) / (1.0 - k[31][1]).powi(2)
    };
    rate(a) <= rate(b)
}

/// Whether `a`'s V5 curve is nowhere brighter than `b`'s, from deep shadows to far past the last knot.
fn v5_below(a: &CameraTone, b: &CameraTone) -> bool {
    (0..=2400).all(|i| {
        let x = 0.001 * 1.005f32.powi(i);
        a.apply_rolloff(x) <= b.apply_rolloff(x) + 1e-5
    })
}

/// Issue storytold/lightcraft#244: Sony's Dynamic Range Optimizer lifts the camera JPEG's shadows, not the raw's. A
/// look fitted to such a JPEG keeps its colour (matrix, hue/saturation table, chroma curve) and
/// its curve is lowered to Sony's without DRO wherever that is darker, never raised.
#[test]
fn a_dro_look_keeps_its_colour_and_is_lowered_to_the_dro_off_tone() {
    let dro_off = sony_dro_off_tone().expect("the built-in curve is valid");
    // the curve itself: black stays black, increasing, mid-grey where cameras put it (L* 50–75)
    assert_eq!(dro_off.apply(0.0), 0.0);
    let mut previous = 0.0;
    for i in 1..200 {
        let y = dro_off.apply(i as f32 * 0.005);
        assert!(y > previous && y < 1.0, "{i}: {y}");
        previous = y;
    }
    assert!((0.18..0.48).contains(&dro_off.apply(0.18)), "{}", dro_off.apply(0.18));
    let xs: [f32; 32] = std::array::from_fn(|i| 0.006 * 1.13f32.powi(i as i32));
    // a JPEG with lifted shadows: the curve comes down to Sony's, colour and chroma curve stay
    let lifted = dro_test_look(&xs, &|x| dro_off.apply(x).powf(0.8));
    let fixed = with_dro_off_tone(lifted.clone());
    assert_lowered(&lifted, &fixed, "lifted");
    assert!(fixed.tone.apply(0.02) < 0.85 * lifted.tone.apply(0.02));
    // a camera that maps raw values darker than Sony's curve keeps its own curve exactly
    let darker = dro_test_look(&xs, &|x| 0.7 * dro_off.apply(x));
    assert_eq!(with_dro_off_tone(darker.clone()).tone, darker.tone);
    // crossing curves: the lower of the two at the knots, still a valid increasing curve
    let crossing = dro_test_look(&xs, &|x| (dro_off.apply(x) * (0.6 + x)).min(0.95));
    assert_lowered(&crossing, &with_dro_off_tone(crossing.clone()), "crossing");
}

/// Past its last knot a camera curve continues as a shoulder whose rate comes from its last two
/// knots. Lowering the second-last knot more than the last steepened that shoulder: the curve
/// fitted to the CC0 NEX-5T sample (raw.pixls.us, DRO Auto) went from 0.740 to 0.843 at scene
/// luminance 0.63, brighter than before. The lowered curve must stay at or below the fitted one
/// through the shoulder, on that curve and on many others.
#[test]
fn the_lowered_curve_stays_below_the_fitted_one_past_its_last_knot() {
    let nex5t: [[f32; 2]; 32] = [
        [0.014948029, 0.017424058],
        [0.018395446, 0.023981506],
        [0.021539066, 0.030581286],
        [0.024555191, 0.0374646],
        [0.027602654, 0.044408206],
        [0.030193308, 0.049894042],
        [0.03261828, 0.055907387],
        [0.034544908, 0.06156683],
        [0.03669959, 0.06814718],
        [0.039161213, 0.075263545],
        [0.04173663, 0.0830011],
        [0.044166446, 0.088166036],
        [0.04682434, 0.095398724],
        [0.049544845, 0.1027153],
        [0.05235826, 0.11173598],
        [0.05497513, 0.119017586],
        [0.057503678, 0.12616614],
        [0.059836045, 0.13521175],
        [0.062677935, 0.14601946],
        [0.06599234, 0.15573128],
        [0.069809504, 0.16883339],
        [0.0755404, 0.1854325],
        [0.08250754, 0.20684941],
        [0.089204684, 0.23466235],
        [0.09650368, 0.25805998],
        [0.102161564, 0.27849936],
        [0.107812546, 0.29706743],
        [0.11410169, 0.31379074],
        [0.12000221, 0.33262977],
        [0.12776275, 0.35407156],
        [0.1387929, 0.38362882],
        [0.19492775, 0.43935108],
    ];
    let fitted = CameraLook { tone: CameraTone::new(nex5t).unwrap(), ..dro_test_look(&std::array::from_fn(|i| 0.01 + i as f32 * 0.01), &|x| x) };
    let fixed = with_dro_off_tone(fitted.clone());
    assert_lowered(&fitted, &fixed, "NEX-5T");
    assert!((fitted.tone.apply(0.63) - 0.7405).abs() < 1e-3 && fixed.tone.apply(0.63) <= fitted.tone.apply(0.63));
    assert_ne!(fixed.tone, fitted.tone, "the lifted mid-tones still come down");
    // curves that end at all kinds of scene luminances, slopes and heights relative to Sony's
    let dro_off = sony_dro_off_tone().unwrap();
    let mut seed = 0x9e37_79b9_u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f32::from((seed >> 16) as u16) / 65536.0
    };
    for case in 0..300 {
        let (start, span) = (0.002 + 0.03 * next(), 1.05 + 0.15 * next());
        let xs: [f32; 32] = std::array::from_fn(|i| start * span.powi(i as i32));
        let (lift, gain, top) = (0.5 + 0.6 * next(), 0.6 + 0.8 * next(), next());
        // brighter or darker than Sony's, with a last knot sometimes lowered less than the one before
        let f = |x: f32| (gain * dro_off.apply(x).powf(lift) + top * 0.2 * (x / xs[31]).powi(8)).min(0.995);
        let mut ys = xs.map(f);
        for i in 1..32 {
            ys[i] = ys[i].max(ys[i - 1]);
        }
        let Some(tone) = CameraTone::new(std::array::from_fn(|i| [xs[i], ys[i]])) else { continue };
        let fitted = CameraLook { tone, ..dro_test_look(&std::array::from_fn(|i| 0.01 + i as f32 * 0.01), &|x| x) };
        assert_lowered(&fitted, &with_dro_off_tone(fitted.clone()), &format!("case {case}"));
    }
}

/// Issue storytold/lightcraft#232: an ILCE-7RM2 camera JPEG is lens-corrected ("Distortion Comp.: Auto"), so on a
/// façade of window frames the raw and its JPEG are a pixel or two apart. The colours agree
/// everywhere away from the edges, and the look must still be found there.
#[test]
fn fits_a_slightly_misaligned_camera_jpeg_away_from_edges() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let (w, h, block, shift) = (96, 64, 8, 2);
    // Patches of 8×8 pixels, alternately dark and bright (frames and panes), varied in colour.
    let scene = |x: usize, y: usize| -> [f32; 3] {
        let (bx, by) = (x / block, y / block);
        let ev = if (bx + by) % 2 == 0 { 0.04 + (bx % 3) as f32 * 0.01 } else { 0.3 + (by % 4) as f32 * 0.08 };
        [ev * (0.7 + ((bx * 7 + by * 3) % 11) as f32 * 0.05), ev, ev * (0.7 + ((bx * 5 + by) % 13) as f32 * 0.04)]
    };
    let camera = |p: [f32; 3]| {
        let q = known.apply_f32(p);
        let y = luminance_2020(q);
        q.map(|v| v * (1.0 - (-2.5 * y).exp()) / y)
    };
    let mut sensor = Rgb32f::new(w, h);
    let mut reference = sensor.clone();
    for y in 0..h {
        for x in 0..w {
            sensor.data[y * w + x] = scene(x, y);
            // The JPEG shows the scene `shift` pixels to the right of where the raw has it.
            reference.data[y * w + x] = camera(scene(x.saturating_sub(shift), y));
        }
    }
    let fit = fit_pairs(&sensor, &reference).expect("look fitted away from the misaligned edges");
    // The look itself is right: on the aligned scene it reproduces the camera.
    let tone = ToneMap::camera(&fit.tone, 0.0, 0.0, 0.0);
    let (mut error, mut n) = (0.0, 0);
    for y in 0..h {
        for x in 0..w {
            let p = displayed(fit.matrix.apply(scene(x, y).map(f64::from)), &tone);
            let t = camera(scene(x, y));
            error += (0..3).map(|c| (p[c] - f64::from(t[c])).powi(2)).sum::<f64>();
            n += 3;
        }
    }
    assert!((error / n as f64).sqrt() < 0.03, "{}", (error / n as f64).sqrt());
    // Unrelated colours stay rejected away from edges too (covered below); a flat JPEG as well.
    reference.data.fill([0.2; 3]);
    assert!(fit_pairs(&sensor, &reference).is_none());
}

#[test]
fn rejects_monochrome_invalid_and_unrelated_previews() {
    let mut sensor = Rgb32f::new(32, 32);
    let mut reference = sensor.clone();
    for (i, p) in sensor.data.iter_mut().enumerate() {
        *p = [0.1 + (i % 13) as f32 * 0.02, 0.15, 0.1];
    }
    reference.data.fill([0.2; 3]);
    assert!(fit_pairs(&sensor, &reference).is_none());
    reference.data.fill([f32::NAN; 3]);
    assert!(fit_pairs(&sensor, &reference).is_none());
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        *src = [0.04 + (i % 11) as f32 * 0.02, 0.05 + (i % 17) as f32 * 0.01, 0.03 + (i % 23) as f32 * 0.01];
        *dst = [0.05 + (i % 7) as f32 * 0.07, 0.05 + (i % 19) as f32 * 0.02, 0.05 + (i % 29) as f32 * 0.01];
    }
    assert!(fit_pairs(&sensor, &reference).is_none());
    sensor.data.fill([0.1, 0.15, 0.12]);
    assert!(fit_pairs(&sensor, &reference).is_none());
    reference.data.truncate(8);
    assert!(fit_pairs(&sensor, &reference).is_none());
}

/// A busy scene whose JPEG is the camera curve of the scene two proxy pixels to the right (a
/// lens-corrected preview): the fitted curve follows the camera's and has no flat steps.
#[test]
fn tone_fit_has_no_steps_on_a_misregistered_jpeg() {
    let camera = |x: f64| 0.6 * x.powf(0.55) / (1.0 + 0.2 * x);
    let (w, h) = (96usize, 64usize);
    let mut seed = 0x2545_f491_u32;
    let mut scene = Vec::with_capacity(w * h);
    for _ in 0..w * h {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        scene.push(0.01 * (5.0 * f64::from(seed >> 8) / f64::from(1u32 << 24)).exp());
    }
    let pairs: Vec<(f64, f64)> =
        (0..h).flat_map(|y| (0..w - 2).map(move |x| (y * w + x, y * w + x + 2))).map(|(i, j)| (scene[i], camera(scene[j]))).collect();
    let curve = fit_tone(pairs).unwrap();
    let mut sorted = scene.clone();
    sorted.sort_by(f64::total_cmp);
    let (lo, hi) = (sorted[sorted.len() / 50], sorted[sorted.len() * 49 / 50]);
    let mut x = lo;
    while x < hi {
        let (a, b) = (curve.apply(x as f32), curve.apply((x * 1.02) as f32));
        assert!(b > a, "flat step at {x}: {a} -> {b}");
        assert!((f64::from(a) / camera(x) - 1.0).abs() < 0.05, "at {x}: {a} vs {}", camera(x));
        x *= 1.02;
    }
}

/// A synthetic photo whose JPEG is the camera look of the scene scaled per pixel by noise of
/// `spread` (a pseudo-random, registration-free stand-in for local tone mapping): the per-bin
/// median recovers the curve, while the JPEG's broadened distribution stretches the quantile fit.
fn noisy_look(spread: f32) -> (Rgb32f, Rgb32f) {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let (w, h) = (96usize, 64usize);
    let mut sensor = Rgb32f::new(w, h);
    let mut reference = sensor.clone();
    let mut seed = 0x9e37_79b9_u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f32::from((seed >> 16) as u16) / 32768.0 - 1.0
    };
    for (src, dst) in sensor.data.iter_mut().zip(&mut reference.data) {
        let ev = 0.05 + (next() + 1.0) * 0.2;
        *src = [ev * (0.8 + (next() + 1.0) * 0.1), ev, ev * (0.8 + (next() + 1.0) * 0.1)];
        let p = known.apply_f32(*src);
        let y = luminance_2020(p);
        let scale = (1.0 - (-2.5 * y).exp()) / y * (1.0 + spread * next());
        *dst = p.map(|v| v * scale);
    }
    (sensor, reference)
}

/// A camera rendering with constant colourfulness (no highlight bleaching) of a colourful scene
/// under a bright neutral sky, whose sensor pixels are clipped: one channel held at the clip
/// level turns the white sky magenta in the raw while the camera JPEG keeps it white. Returns
/// the sensor proxy, the JPEG proxy and the clip mask.
fn clipped_sky_scene() -> (Rgb32f, Rgb32f, Vec<bool>) {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let (w, h) = (96usize, 64usize);
    let mut sensor = Rgb32f::new(w, h);
    let mut reference = sensor.clone();
    let mut clipped = vec![false; w * h];
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f32::from((seed >> 16) as u16) / 65536.0
    };
    let camera = |p: [f32; 3]| {
        let y = luminance_2020(p);
        let o = 1.0 - (-2.5 * y).exp();
        p.map(|v| (v * o / y).clamp(0.0, 0.97))
    };
    // the sky: a quarter of the frame, white in the scene and in the JPEG
    let neutral = known.inverse().unwrap().apply_f32([0.32; 3]);
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let scene = if i < w * h / 4 {
            clipped[i] = true;
            let sky = neutral.map(|v| v * (0.8 + 0.4 * next()));
            *dst = camera(known.apply_f32(sky));
            // the green sample clips, red and blue (after white balance) go on rising
            *src = [sky[0] * 1.35, sky[1] * 0.85, sky[2] * 1.4];
            continue;
        } else {
            // colourful foliage, leaves, bark and bright flowers from dark to light
            let ev = 0.02 + next() * 0.4;
            [ev * (0.6 + next() * 0.8), ev, ev * (0.5 + next() * 0.8)]
        };
        *src = scene;
        *dst = camera(known.apply_f32(scene));
    }
    (sensor, reference, clipped)
}

/// Mean colourfulness (distance from neutral of luminance-normalised RGB) of the unclipped
/// pixels rendered with `look`, over that of the camera JPEG, per display-luminance band.
fn colourfulness_ratio(look: &CameraLook, sensor: &Rgb32f, reference: &Rgb32f, clipped: &[bool], band: std::ops::Range<f64>) -> f64 {
    let chroma = |p: [f64; 3]| {
        let y = luma(p);
        p.iter().map(|v| (v / y - 1.0).powi(2)).sum::<f64>().sqrt()
    };
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let (mut rendered, mut camera) = (0.0, 0.0);
    for ((x, y), c) in sensor.data.iter().zip(&reference.data).zip(clipped) {
        let y = y.map(f64::from);
        if *c || !band.contains(&luma(y)) {
            continue;
        }
        rendered += chroma(displayed(look.matrix.apply(x.map(f64::from)), &tone));
        camera += chroma(y);
    }
    rendered / camera
}

/// Clipped sensor pixels are left out of the look's training pairs: a clipped white sky no longer
/// dulls the colour matrix or makes the chroma curve remove colour at the sky's brightness.
#[test]
fn clipped_sky_does_not_dull_the_look() {
    let (sensor, reference, clipped) = clipped_sky_scene();
    let old = fit_pairs(&sensor, &reference).expect("the earlier fit accepts the look");
    let new = fit_look(&sensor, &reference, &clipped, None).expect("look without the clipped pixels");
    let (mid, bright) = (0.1..0.35, 0.35..0.75);
    let old_ratio = (
        colourfulness_ratio(&old, &sensor, &reference, &clipped, mid.clone()),
        colourfulness_ratio(&old, &sensor, &reference, &clipped, bright.clone()),
    );
    let new_ratio = (colourfulness_ratio(&new, &sensor, &reference, &clipped, mid), colourfulness_ratio(&new, &sensor, &reference, &clipped, bright));
    // with the clipped sky in the pairs, the bright colours lose much of their colour
    assert!(old_ratio.1 < 0.8, "earlier fit: colourfulness vs camera {old_ratio:?}, chroma {:?}", old.tone.chroma());
    // without it, the render keeps the camera's colourfulness at every brightness
    for r in [new_ratio.0, new_ratio.1] {
        assert!((0.9..1.1).contains(&r), "colourfulness vs camera {new_ratio:?}, chroma {:?}", new.tone.chroma());
    }
    assert!(new.tone.chroma().iter().all(|k| *k > 0.85), "no colour cut: {:?}", new.tone.chroma());
}

/// When the accepted attempt misses the gates without the clipped pixels (here: too few pixels
/// left), the look fitted on all pixels is kept unchanged.
#[test]
fn clipped_fit_falls_back_to_all_pixels() {
    let (sensor, reference, _) = clipped_sky_scene();
    let all = fit_pairs(&sensor, &reference).unwrap();
    // nearly everything clipped: too few pairs remain, the earlier look is used unchanged
    let mut clipped = vec![true; sensor.data.len()];
    clipped[..100].iter_mut().for_each(|c| *c = false);
    let fallback = fit_look(&sensor, &reference, &clipped, None).expect("the fallback keeps the look");
    assert_eq!((fallback.matrix.0, fallback.tone), (all.matrix.0, all.tone));
    // nothing clipped: the same look as before, exactly
    let none = fit_look(&sensor, &reference, &vec![false; sensor.data.len()], None).unwrap();
    assert_eq!((none.matrix.0, none.tone), (all.matrix.0, all.tone));
    // without clipped pixels the refit keeps the accepted attempt (here: all pixels, own colour, quantile tone)
    let check = HighlightCheck { pairs: highlights(&sensor, &reference), dro: false, grey: false };
    let (_, attempt) = search_ordered(&sensor, &reference, None, TONE_FITS, &check).unwrap();
    assert_eq!(attempt, Attempt { tone: ToneFit::Quantile, profile: false, away_from_edges: false, dark: false });
    assert!(without_clipped(&sensor, &[true; 3]).is_none(), "a mask of another size is ignored");
}

/// Quantile matching is tried first; the conditional-median fit is the fallback and is used
/// only where the quantile look misses the acceptance gates, so no photo loses its look.
#[test]
fn tone_fit_order_quantile_first_conditional_median_as_fallback() {
    // Clean data: both fits pass, the default order uses the quantile curve (increasing).
    let (sensor, reference) = noisy_look(0.0);
    let default = fit_pairs(&sensor, &reference).expect("clean look");
    let quantile = fit_pairs_ordered(&sensor, &reference, None, &[ToneFit::Quantile]).expect("quantile passes");
    assert_eq!(default.tone, quantile.tone);
    // Noisy data: the quantile fit misses the gates, the conditional median still passes.
    let (sensor, reference) = noisy_look(0.33);
    assert!(fit_pairs_ordered(&sensor, &reference, None, &[ToneFit::Quantile]).is_none(), "quantile alone misses the gates");
    let conditional = fit_pairs_ordered(&sensor, &reference, None, &[ToneFit::ConditionalMedian]).expect("conditional median passes");
    let default = fit_pairs(&sensor, &reference).expect("the fallback keeps the look");
    assert_eq!(default.tone, conditional.tone);
    // The order matters only through the first pass; with the fits swapped the conditional one wins even on clean data.
    let (sensor, reference) = noisy_look(0.0);
    let swapped = fit_pairs_ordered(&sensor, &reference, None, &[ToneFit::ConditionalMedian, ToneFit::Quantile]).unwrap();
    let conditional = fit_pairs_ordered(&sensor, &reference, None, &[ToneFit::ConditionalMedian]).unwrap();
    assert_eq!(swapped.tone, conditional.tone);
}

/// The camera look of `scene` (the known matrix of these tests, then a bright camera curve).
fn bright_camera(p: [f32; 3]) -> [f32; 3] {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let q = known.apply_f32(p);
    let y = luminance_2020(q);
    if y <= 0.0 { [0.0; 3] } else { q.map(|v| v * (1.0 - (-4.0 * y).exp()) / y) }
}

/// Mean squared error (all channels, display-linear) of `render` against `target` over `scene`.
fn mean_error(scene: &Rgb32f, target: impl Fn([f32; 3]) -> [f32; 3], render: impl Fn([f64; 3]) -> [f64; 3]) -> f64 {
    let error: f64 = scene
        .data
        .iter()
        .map(|x| {
            let (p, t) = (render(x.map(f64::from)), target(*x));
            (0..3).map(|c| (p[c] - f64::from(t[c])).powi(2)).sum::<f64>()
        })
        .sum();
    error / (3 * scene.data.len()) as f64
}

/// A colourful scene with fine detail everywhere (foliage) whose camera JPEG is lens-corrected:
/// barrel distortion moves the picture by up to about 4 proxy pixels toward the corners, so the
/// detail no longer pairs with itself and no global look passes the gates, on all pixels or
/// away from edges. The partial look still follows the camera's brightness, colourfulness and
/// hues: on the registered scene it is far closer to the camera than the neutral fallback,
/// which renders the scene dark and grey.
#[test]
fn partial_look_when_a_lens_corrected_jpeg_misses_the_gates() {
    let (w, h) = (96usize, 64usize);
    let scene = |u: f32, v: f32| -> [f32; 3] {
        let base = 0.1 * (1.0 + 0.6 * (0.05 * u + 0.04 * v).sin());
        let detail = 1.0 + 0.8 * (1.7 * u).sin() * (1.3 * v).sin();
        let y = base * detail;
        [y * (0.6 + 0.5 * (0.07 * u).sin().abs()), y, y * (0.5 + 0.6 * (0.09 * v + 0.03 * u).cos().abs())]
    };
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut sensor = Rgb32f::new(w, h);
    let mut reference = sensor.clone();
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let k = 1.0 + 0.08 * (dx * dx + dy * dy) / (cx * cx);
            sensor.data[y * w + x] = scene(x as f32, y as f32);
            reference.data[y * w + x] = bright_camera(scene(cx + dx * k, cy + dy * k));
        }
    }
    assert!(fit_pairs(&sensor, &reference).is_none(), "the full look misses the gates");
    let look = fit_look(&sensor, &reference, &vec![false; w * h], None).expect("a partial look instead of the neutral fallback");
    assert!(look.hue_sat.is_none());
    assert_ne!(look.matrix, Mat3::IDENTITY, "a colourful scene gets the damped matrix");
    // on the registered scene, against what the camera renders
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let neutral = ToneMap::new(0.0, 0.0, 0.0);
    let partial = mean_error(&sensor, bright_camera, |x| displayed(look.matrix.apply(x), &tone));
    let fallback = mean_error(&sensor, bright_camera, |x| displayed(x, &neutral));
    assert!(partial < fallback * 0.25, "partial {partial:.5}, neutral fallback {fallback:.5}");
    assert!(tone.apply(0.05) < tone.apply(0.2) && tone.apply(0.2) < tone.apply(0.8));
}

fn near_black_scene() -> (Rgb32f, Rgb32f) {
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (input, output)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let y = if i % 60 == 0 { 0.3 + (i % 7) as f32 * 0.01 } else { 0.006 + ((i * 7919) % 6144) as f32 / 6144.0 * 0.075 };
        let rgb = [1.15, 1.0, 0.75];
        *input = rgb.map(|v| v * y / luminance_2020(rgb));
        *output = input.map(|v| v * 0.08);
    }
    (sensor, reference)
}

#[test]
fn a_near_black_scene_gets_tone_without_learning_colour() {
    let (sensor, reference) = near_black_scene();
    assert!(collect_pairs(&sensor, &reference, 0.005, None, false).is_none(), "too few midtones even for the partial look");
    let mut raw = raw_of(RawFormat::Nef, "NIKON CORPORATION", "NIKON D610", Default::default());
    let (_, _, look) = starting_colour_with(
        &mut raw,
        |_, _| (fit_look(&sensor, &reference, &vec![false; sensor.data.len()], None), false),
        |_, _| None,
        |_, _| fit_dark_tone(&sensor, &reference, &vec![false; sensor.data.len()]),
    );
    let look = look.expect("tone from the dark pixels");
    assert_eq!(look.matrix, Mat3::IDENTITY);
    assert!(look.hue_sat.is_none());
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let error = |tone: &ToneMap| {
        sensor.data.iter().zip(&reference.data).map(|(x, y)| (luma(displayed(x.map(f64::from), tone)) - luma(y.map(f64::from))).powi(2)).sum::<f64>()
    };
    assert!(error(&tone) < error(&ToneMap::new(0.0, 0.0, 0.0)) * 0.001);
    assert_eq!(look.tone.apply(0.0), 0.0);
    let mut previous = 0.0;
    for i in 0..10_000 {
        let y = look.tone.apply(i as f32 / 10_000.0);
        assert!(y.is_finite() && y >= previous && y <= 1.0);
        previous = y;
    }
    // Models with spectral matrices keep them before the dark tone is fitted.
    let mut raw = raw_of(RawFormat::Cr2, "Canon", "Canon EOS 600D", Default::default());
    let (_, t, look) = starting_colour_with(
        &mut raw,
        |_, _| (None, false),
        |_, _| None,
        |raw, t| {
            assert!(!t.matrix_is_fallback && crate::sensor::color::has_matrix(&raw.color));
            fit_dark_tone(&sensor, &reference, &vec![false; sensor.data.len()])
        },
    );
    assert!(!t.matrix_is_fallback);
    assert_eq!(look.expect("tone on top of spectral colour").matrix, Mat3::IDENTITY);
}

#[test]
fn dark_tone_holdout_never_enters_training() {
    let (sensor, mut reference) = near_black_scene();
    let none = vec![false; sensor.data.len()];
    let original = fit_dark_tone(&sensor, &reference, &none).expect("dark tone");
    // Every pixel clears the wider signal floors. Changing only its held-out third must
    // change the measured error, but cannot change the fitted curve.
    for p in reference.data.iter_mut().step_by(3) {
        *p = p.map(|v| v * 1.03);
    }
    let changed = fit_dark_tone(&sensor, &reference, &none).expect("still clearly better than the fallback");
    assert_eq!(changed.tone, original.tone);
}

#[test]
fn dark_tone_requires_signal_correlation_and_unclipped_highlights() {
    let (sensor, reference) = near_black_scene();
    let none = vec![false; sensor.data.len()];
    let mut unrelated = reference.clone();
    for (i, p) in unrelated.data.iter_mut().enumerate() {
        *p = reference.data[(i * 3571) % reference.data.len()];
    }
    assert!(fit_dark_tone(&sensor, &unrelated, &none).is_none(), "unrelated dark JPEG");
    let mut flat = reference.clone();
    flat.data.fill([0.002; 3]);
    assert!(fit_dark_tone(&sensor, &flat, &none).is_none(), "a quantized flat JPEG cannot teach tone");
    let mut sparse = reference.clone();
    sparse.data.iter_mut().skip(80).for_each(|p| *p = [0.0; 3]);
    assert!(fit_dark_tone(&sensor, &sparse, &none).is_none(), "fewer than 256 usable pairs");
    let mut bright = reference.clone();
    bright.data[0] = [0.95; 3];
    assert!(fit_dark_tone(&sensor, &bright, &none).is_none(), "dark pairs cannot constrain a bright highlight");
    let mut clipped = none;
    clipped[0] = true;
    let original = fit_dark_tone(&sensor, &reference, &clipped).expect("tone without the clipped pixel");
    let mut changed = sensor.clone();
    changed.data[0] = [0.18; 3];
    let look = fit_dark_tone(&changed, &reference, &clipped).expect("the clipped pixel is still closer to its JPEG");
    assert_eq!(look.tone, original.tone, "clipped samples cannot change the fitted curve");
    let mut highlight = reference.clone();
    highlight.data[0] = [0.8; 3];
    assert!(fit_dark_tone(&changed, &highlight, &clipped).is_none(), "the curve cannot darken a clipped highlight away from the JPEG");
    assert!(fit_dark_tone(&sensor, &Rgb32f::new(64, 96), &clipped).is_none(), "mismatched dimensions");
    assert!(fit_dark_tone(&sensor, &reference, &[]).is_none(), "mismatched clip mask");
}

/// An overcast, nearly colourless scene (under 5% of the pixels coloured, so no colour matrix is
/// learnt and the full look is never tried) whose camera renders it two stops brighter than the
/// neutral fallback: the partial look is the tone curve (and chroma curve) alone.
#[test]
fn partial_look_brightens_a_dull_scene_without_learning_colour() {
    let (w, h) = (96usize, 64usize);
    let mut sensor = Rgb32f::new(w, h);
    for (i, p) in sensor.data.iter_mut().enumerate() {
        let ev = 0.01 + (i % 37) as f32 * 0.004;
        let tint = if i % 50 == 0 { 1.3 } else { 1.0 + (i % 5) as f32 * 0.004 };
        *p = [ev * tint, ev, ev * (2.0 - tint)];
    }
    let camera = |p: [f32; 3]| {
        let y = luminance_2020(p);
        p.map(|v| v * (1.0 - (-12.0 * y).exp()) / y)
    };
    let mut reference = sensor.clone();
    reference.map_in_place(camera);
    assert!(fit_pairs(&sensor, &reference).is_none(), "too little colour for the full look");
    let look = fit_look(&sensor, &reference, &vec![false; w * h], None).expect("a partial look");
    assert_eq!(look.matrix, Mat3::IDENTITY, "no colour matrix from a colourless scene");
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let neutral = ToneMap::new(0.0, 0.0, 0.0);
    let lightness = |tone: &ToneMap| sensor.data.iter().map(|p| f64::from(tone.apply(luminance_2020(*p)))).sum::<f64>() / sensor.data.len() as f64;
    let target = reference.data.iter().map(|p| f64::from(luminance_2020(*p))).sum::<f64>() / reference.data.len() as f64;
    assert!((lightness(&tone) / target - 1.0).abs() < 0.05, "{} vs {target}", lightness(&tone));
    assert!(lightness(&neutral) < target * 0.6, "the neutral fallback is far darker");
}

/// When the neutral fallback already matches the camera JPEG, or the JPEG is unrelated or
/// monochrome, the partial look isn't clearly better and the photo keeps the neutral fallback.
#[test]
fn partial_look_only_when_clearly_better_than_the_neutral_fallback() {
    let (w, h) = (96usize, 64usize);
    let mut sensor = Rgb32f::new(w, h);
    for (i, p) in sensor.data.iter_mut().enumerate() {
        let ev = 0.02 + (i % 41) as f32 * 0.012;
        *p = [ev * (0.7 + (i % 11) as f32 * 0.05), ev, ev * (0.7 + (i % 13) as f32 * 0.045)];
    }
    let none = vec![false; w * h];
    // the camera renders exactly as the neutral fallback: nothing to gain
    let neutral = ToneMap::new(0.0, 0.0, 0.0);
    let mut reference = sensor.clone();
    reference.map_in_place(|p| displayed(p.map(f64::from), &neutral).map(|v| v as f32));
    let check = HighlightCheck { pairs: highlights(&sensor, &reference), dro: false, grey: false };
    assert!(fit_partial(&sensor, &reference, &check).is_none(), "no clear gain over the fallback");
    // an unrelated picture
    for (i, p) in reference.data.iter_mut().enumerate() {
        *p = [0.05 + (i % 7) as f32 * 0.07, 0.05 + (i % 19) as f32 * 0.02, 0.05 + (i % 29) as f32 * 0.01];
    }
    assert!(fit_look(&sensor, &reference, &none, None).is_none(), "unrelated JPEG");
    // a black-and-white JPEG of the scene
    reference.data.iter_mut().zip(&sensor.data).for_each(|(r, s)| *r = [0.8 * luminance_2020(*s).sqrt(); 3]);
    assert!(fit_look(&sensor, &reference, &none, None).is_none(), "monochrome JPEG");
}

/// A coloured scene whose camera JPEG is black and white: the camera's tone curve applied to the scene's
/// luminance, grey in every pixel.
fn black_and_white_scene() -> (Rgb32f, Rgb32f) {
    let (w, h) = (96usize, 64usize);
    let mut sensor = Rgb32f::new(w, h);
    for (i, p) in sensor.data.iter_mut().enumerate() {
        let ev = 0.01 + (i % 53) as f32 * 0.009;
        *p = [ev * (0.6 + (i % 11) as f32 * 0.08), ev, ev * (0.6 + (i % 13) as f32 * 0.07)];
    }
    let mut reference = sensor.clone();
    reference.map_in_place(|p| [1.0 - (-3.0 * luminance_2020(p)).exp(); 3]);
    (sensor, reference)
}

#[test]
fn black_and_white_jpegs_are_recognised() {
    let (sensor, mut reference) = black_and_white_scene();
    assert!(monochrome(&reference));
    assert!(!monochrome(&sensor), "a colour JPEG");
    // a few coloured pixels (JPEG noise) are allowed, one in a hundred is colour
    let n = reference.data.len();
    reference.data.iter_mut().take(n / 1000).for_each(|p| p[0] += 0.05);
    assert!(monochrome(&reference));
    reference.data.iter_mut().take(n / 100).for_each(|p| p[0] += 0.05);
    assert!(!monochrome(&reference));
    // a sepia-toned monochrome
    let (_, mut toned) = black_and_white_scene();
    toned.map_in_place(|p| [p[0] * 1.08, p[1], p[2] * 0.85]);
    assert!(!monochrome(&toned));
    assert!(!monochrome(&Rgb32f::new(8, 8)), "too small to tell");
    // the check reads pixels, not the picture style: a colour JPEG whose pixels are all grey within 0.002 counts
    let (_, mut faint) = black_and_white_scene();
    faint.map_in_place(|p| [p[0] + 0.001, p[1], p[2] - 0.0005]);
    assert!(monochrome(&faint));
    faint.map_in_place(|p| [p[0] + 0.003, p[1], p[2]]);
    assert!(!monochrome(&faint));
    // pixels that aren't numbers can't vouch for grey
    let (_, mut broken) = black_and_white_scene();
    broken.data.iter_mut().take(n / 100).for_each(|p| *p = [f32::NAN; 3]);
    assert!(!monochrome(&broken));
}

/// Issue storytold/lightcraft#577: a black-and-white JPEG gives no look (no colour to fit), but its tone curve renders the scene's
/// brightness like the camera, with the colour left as it was and the chroma curve neutral.
#[test]
fn a_black_and_white_jpeg_gives_its_tone_curve() {
    let (sensor, reference) = black_and_white_scene();
    let none = vec![false; sensor.data.len()];
    assert!(fit_look(&sensor, &reference, &none, None).is_none(), "no colour to fit");
    let look = fit_tone_only(&sensor, &reference, None).expect("a tone curve");
    assert!(look.matrix == Mat3::IDENTITY && look.hue_sat.is_none(), "the colour is left as it was");
    assert!(look.tone.chroma().iter().all(|k| *k == 1.0), "the chroma curve stays neutral: {:?}", look.tone.chroma());
    let n = sensor.data.len() as f64;
    let mean = |tone: &ToneMap| sensor.data.iter().map(|p| luma(displayed(p.map(f64::from), tone))).sum::<f64>() / n;
    let target = reference.data.iter().map(|p| f64::from(luminance_2020(*p))).sum::<f64>() / n;
    let toned = mean(&ToneMap::camera(&look.tone, 0.0, 0.0, 0.0));
    assert!((toned / target - 1.0).abs() < 0.05, "{toned} vs {target}");
    assert!(mean(&ToneMap::new(0.0, 0.0, 0.0)) < target * 0.8, "the default curve is darker");
    // with a camera profile's colour, the look carries it
    let matrix = Mat3([[1.2, -0.15, -0.05], [-0.1, 1.15, -0.05], [-0.02, -0.1, 1.12]]);
    let look = fit_tone_only(&sensor, &reference, Some((matrix, None))).expect("a tone curve with the profile's colour");
    assert_eq!(look.matrix, matrix);
}

/// A camera profile's colour can fail the checks where the photo's own colour passes, so [`monochrome_look`] tries
/// again on the transform the photo has, as [`fit_look`] tries the photo's own colour after a profile's.
#[test]
fn a_rejected_profile_leaves_the_tone_curve_on_the_photos_own_colour() {
    // saturated colours, and a profile whose colour gets their luminance wrong (red and blue swapped)
    let hues = [[1.0, 0.15, 0.1], [0.1, 1.0, 0.15], [0.15, 0.1, 1.0], [1.0, 0.9, 0.1], [0.1, 0.9, 1.0], [1.0, 0.1, 0.9]];
    let mut sensor = Rgb32f::new(96, 64);
    for (i, p) in sensor.data.iter_mut().enumerate() {
        let ev = 0.02 + (i % 47) as f32 * 0.008;
        *p = hues[i % 6].map(|c| c * ev);
    }
    // a JPEG a little brighter than the default curve, so a curve has to fit closely to be clearly better
    let neutral = ToneMap::new(0.0, 0.0, 0.0);
    let mut reference = sensor.clone();
    reference.map_in_place(|p| [(1.15 * luma(displayed(p.map(f64::from), &neutral))) as f32; 3]);
    let swapped = Mat3([[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]);
    assert!(fit_tone_only(&sensor, &reference, Some((swapped, None))).is_none(), "the profile's colour fails the checks");
    let own = fit_tone_only(&sensor, &reference, None);
    assert!(own.is_some(), "the photo's own colour passes");
    // and the step runs for a model with a bundled profile (the ILCE-7M4 also has spectral matrices)
    assert!(crate::look::profiles::get("ILCE-7M4").is_some());
    let mut raw = raw_of(RawFormat::Arw, "SONY", "ILCE-7M4", Default::default());
    let (_, t, look) = starting_colour_with(
        &mut raw,
        |_, _| (None, true),
        |_, t| {
            assert!(!t.matrix_is_fallback, "on the spectral matrices");
            own.clone()
        },
        |_, _| None,
    );
    assert!(look.is_some() && !t.matrix_is_fallback);
}

/// [`alignment`] leaves out the pixels a proxy lacks (NaN outside a lens-warped one): read as black, they made a
/// false edge around the border, and the warped candidate lost against a worse unwarped one.
#[test]
fn alignment_leaves_out_missing_pixels() {
    let (w, h) = (48usize, 32usize);
    let mut a = Rgb32f::new(w, h);
    for (i, p) in a.data.iter_mut().enumerate() {
        *p = [0.1 + 0.05 * ((i * 7919) % 13) as f32; 3];
    }
    let mut b = a.clone();
    for (i, p) in b.data.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        if x < 4 || y < 4 || x >= w - 4 || y >= h - 4 {
            *p = [f32::NAN; 3];
        }
    }
    assert!(alignment(&a, &b) > 0.999, "{}", alignment(&a, &b));
}

/// The black-and-white fit has the partial look's checks: a JPEG of another picture, or one the default curve
/// already renders, gives no look.
#[test]
fn a_black_and_white_jpeg_gives_a_tone_curve_only_when_clearly_better() {
    let (sensor, mut reference) = black_and_white_scene();
    let neutral = ToneMap::new(0.0, 0.0, 0.0);
    reference.data.iter_mut().zip(&sensor.data).for_each(|(r, s)| *r = [luma(displayed(s.map(f64::from), &neutral)) as f32; 3]);
    assert!(monochrome(&reference));
    assert!(fit_tone_only(&sensor, &reference, None).is_none(), "no clear gain over the default curve");
    for (i, p) in reference.data.iter_mut().enumerate() {
        *p = [0.05 + (i % 29) as f32 * 0.02; 3];
    }
    assert!(fit_tone_only(&sensor, &reference, None).is_none(), "another picture");
    // a flat grey JPEG ranks nothing, even when the scene's pixels are stored in brightness order (with ties
    // ranked in pixel order it looked perfectly related, and the curve flattened the photo to one grey)
    let mut sorted = sensor.clone();
    sorted.data.sort_by(|a, b| luminance_2020(*a).total_cmp(&luminance_2020(*b)));
    reference.data.iter_mut().for_each(|p| *p = [0.4; 3]);
    assert!(fit_tone_only(&sorted, &reference, None).is_none(), "a flat JPEG");
}

/// A smooth synthetic scene (sensor proxy) and its camera rendering, `dx` pixels to the right.
fn scene_pair(w: usize, h: usize, dx: usize) -> (Rgb32f, Rgb32f) {
    let scene = |x: f32, y: f32| -> [f32; 3] {
        let l = 0.03 + 0.4 * (0.5 + 0.5 * (x * 0.21).sin() * (y * 0.17).cos()) + 0.2 * ((x - 30.0).hypot(y - 20.0) < 12.0) as u8 as f32;
        [l * (0.8 + 0.3 * (y / h as f32)), l, l * (1.1 - 0.4 * (x / w as f32))]
    };
    let camera = |p: [f32; 3]| {
        let y = luminance_2020(p);
        p.map(|v| v * (1.0 - (-2.5 * y).exp()) / y)
    };
    let mut sensor = Rgb32f::new(w, h);
    let mut reference = sensor.clone();
    for y in 0..h {
        for x in 0..w {
            sensor.data[y * w + x] = scene(x as f32, y as f32);
            reference.data[y * w + x] = camera(scene(x as f32 - dx as f32, y as f32));
        }
    }
    (sensor, reference)
}

/// The structural gate accepts the same picture whatever its tone, and refuses a shifted, a broken (stripes), a
/// flat and a differently sized preview.
#[test]
fn dng_structure_gate_refuses_shifted_broken_and_unrelated_previews() {
    let (sensor, reference) = scene_pair(96, 64, 0);
    assert!(structure_agrees(&sensor, &reference));
    assert!(structure_agrees(&sensor, &scene_pair(96, 64, 1).1), "one proxy pixel (lens correction) is tolerated");
    assert!(!structure_agrees(&sensor, &scene_pair(96, 64, 3).1), "differently framed");
    let mut stripes = reference.clone();
    for (i, p) in stripes.data.iter_mut().enumerate() {
        *p = if (i / 96) % 4 < 2 { [0.6, 0.1, 0.5] } else { [0.05, 0.4, 0.1] };
    }
    assert!(!structure_agrees(&sensor, &stripes), "a broken preview");
    let mut flat = reference.clone();
    flat.data.fill([0.2; 3]);
    assert!(!structure_agrees(&sensor, &flat));
    assert!(!structure_agrees(&sensor, &scene_pair(95, 64, 0).1));
}

/// With the file's colour given as the identity, only the tone is fitted: the colour stays and a camera that
/// renders mid-grey brighter than the default tone map is followed.
#[test]
fn dng_tone_fit_keeps_the_files_colour_and_follows_the_camera_tone() {
    let (mut sensor, mut reference) = scene_pair(96, 64, 0);
    for image in [&mut sensor, &mut reference] {
        for p in image.data.iter_mut().step_by(5) {
            *p = [p[0] * 1.6, p[1], p[2] * 0.5];
        }
    }
    let colour = Some((Mat3::IDENTITY, None));
    let check = HighlightCheck { pairs: highlights(&sensor, &reference), dro: false, grey: false };
    let look =
        fit_attempt(&sensor, &reference, &colour, Attempt { tone: ToneFit::Quantile, profile: true, away_from_edges: false, dark: true }, &check)
            .expect("tone fitted");
    assert_eq!(look.matrix, Mat3::IDENTITY);
    assert!(look.hue_sat.is_none());
    let (fitted, default) = (ToneMap::camera(&look.tone, 0.0, 0.0, 0.0), ToneMap::new(0.0, 0.0, 0.0));
    let target = 1.0 - (-2.5f32 * 0.18).exp();
    assert!((fitted.apply(0.18) - target).abs() < 0.03, "{} vs {target}", fitted.apply(0.18));
    assert!(fitted.apply(0.18) > default.apply(0.18) + 0.05, "{} vs default {}", fitted.apply(0.18), default.apply(0.18));
    assert!(!clips_more(&sensor, &reference, &fitted, &fitted), "a tone never clips more than itself");
}

/// The highlight gate compares the share of near-white pixels of the fitted tone with the plain render's and the
/// camera preview's, in one direction only: clipping what the camera clips is not refused.
#[test]
fn dng_highlight_gate_compares_near_white_shares() {
    let mut sensor = Rgb32f::new(32, 32);
    for (i, p) in sensor.data.iter_mut().enumerate() {
        *p = [0.02 + 1.5 * i as f32 / 1024.0; 3];
    }
    let (bright, dark) = (ToneMap::new(0.0, 1.0, 0.0), ToneMap::new(0.0, -1.0, 0.0));
    let mut white = Rgb32f::new(32, 32);
    white.data.iter_mut().for_each(|p| *p = [1.0; 3]);
    let mut black = Rgb32f::new(32, 32);
    black.data.iter_mut().for_each(|p| *p = [0.1; 3]);
    assert!(clips_more(&sensor, &black, &bright, &dark), "clips more than the plain render and a preview without white");
    assert!(!clips_more(&sensor, &black, &dark, &bright));
    assert!(!clips_more(&sensor, &black, &bright, &bright));
    assert!(!clips_more(&sensor, &white, &bright, &dark), "the camera preview clips as much");
}

/// A dark scene's shadows below the usual training floor still reach the tone-only fit.
#[test]
fn dng_tone_fit_trains_on_dark_pixels() {
    let mut sensor = Rgb32f::new(64, 64);
    let mut reference = Rgb32f::new(64, 64);
    for (i, (s, r)) in sensor.data.iter_mut().zip(reference.data.iter_mut()).enumerate() {
        let v = 0.0005 + 0.004 * (i % 64) as f32 / 64.0 + if i % 7 == 0 { 0.2 } else { 0.0 };
        *s = [v, v * 1.1, v * 0.9];
        *r = [(v * 3.0).min(0.9), (v * 3.3).min(0.9), (v * 2.7).min(0.9)];
    }
    let usual = collect_pairs(&sensor, &reference, 0.0, None, false).map_or(0, |p| p.0.len());
    let dark = collect_pairs(&sensor, &reference, 0.0, None, true).map_or(0, |p| p.0.len());
    assert!(dark > usual, "{dark} vs {usual}");
}

/// The embedded vignetting correction reaches the sensor proxy as the pipeline applies it to the render.
#[test]
fn dng_sensor_proxy_gets_the_embedded_vignetting_correction() {
    use craft_develop_model::{EmbeddedLens, EmbeddedVignette};
    let lens = EmbeddedLens {
        vignette: Some(EmbeddedVignette { k: [1.0, 0.0, 0.0, 0.0, 0.0], center: craft_geom::Point::new(0.5, 0.5), radius: 0.6 }),
        ..Default::default()
    };
    let mut proxy = Rgb32f::new(96, 64);
    proxy.data.fill([0.2; 3]);
    correct_vignetting(&mut proxy, Some(&lens), craft_geom::Orientation::Normal);
    let (centre, corner) = (proxy.data[32 * 96 + 48][1] / 0.2, proxy.data[0][1] / 0.2);
    assert!((centre - 1.0).abs() < 0.01 && (1.9..2.0).contains(&corner), "centre {centre}, corner {corner}");
    let mut plain = Rgb32f::new(96, 64);
    plain.data.fill([0.2; 3]);
    correct_vignetting(&mut plain, None, craft_geom::Orientation::Normal);
    assert!(plain.data.iter().all(|p| *p == [0.2; 3]));
}

/// Which DNGs get a tone fitted to their camera JPEG: a colour matrix, a tone curve that is absent or the
/// identity, with or without a look table; and no other raw format.
#[test]
fn dng_tone_gates_follow_the_files_own_tags() {
    use crate::sensor::profile::{HsvTable, ToneCurve};
    let own = Mat3([[0.6, -0.1, -0.05], [-0.4, 1.3, 0.1], [-0.1, 0.2, 0.7]]);
    let color = crate::sensor::ColorData { illuminant: [21, 0], color_matrix: [Some((own).0), None], ..Default::default() };
    let eligible = |format: RawFormat, color: crate::sensor::ColorData| {
        let raw = raw_of(format, "Make", "Model", color);
        let t = crate::sensor::color::camera_transform(&raw, crate::sensor::color::as_shot_white_xy(&raw));
        dng_tone_eligible(&raw, &t)
    };
    assert!(eligible(RawFormat::Dng, color.clone()));
    assert!(!eligible(RawFormat::Cr2, color.clone()), "other formats keep their own path");
    assert!(!eligible(RawFormat::Dng, Default::default()), "no matrix, no colour to keep");
    let mut identity = color.clone();
    identity.profile.tone_curve = ToneCurve::from_tag(&[0.0, 0.0, 0.5, 0.5, 1.0, 1.0]);
    assert!(identity.profile.tone_curve.is_some() && eligible(RawFormat::Dng, identity), "an identity ProfileToneCurve is no tone");
    let mut curved = color.clone();
    curved.profile.tone_curve = ToneCurve::from_tag(&[0.0, 0.0, 0.18, 0.4, 1.0, 1.0]);
    assert!(curved.profile.tone_curve.is_some() && !eligible(RawFormat::Dng, curved), "the file's own tone curve is kept");
    let mut looked = color;
    looked.profile.look_table = HsvTable::from_tags(&[1, 2, 1], &[0.0, 1.0, 1.0, 0.0, 1.0, 1.0], 0);
    assert!(looked.profile.look_table.is_some() && eligible(RawFormat::Dng, looked), "a look table defines colour, not tone");
}

/// The 1024 px preview Lightroom Classic writes into a 7008 px DNG is fitted; a 256 px thumbnail is not.
#[test]
fn dng_tone_fit_takes_lightroom_sized_previews() {
    let floor = |raw_long: f64| (MIN_PREVIEW_FRACTION * raw_long).max(384.0);
    assert!(1024.0 >= floor(7008.0));
    assert!(256.0 < floor(7008.0));
    assert!(1024.0 >= floor(9504.0), "a 61 MP raw too");
}

/// Pairs whose scene luminance spans `ratio` from the darkest to the brightest pixel, mapped
/// to the JPEG through a steep camera curve (a hazy scene the camera stretches).
fn low_contrast_pairs(ratio: f64) -> Vec<(f64, f64)> {
    (0..4096).map(|i| 0.08 * ratio.powf(i as f64 / 4095.0)).map(|x| (x, (0.2 + (x - 0.08) * 3.0).min(0.95))).collect()
}

/// The ratio the tone fits test: scene luminance at the middle of the brightest 1/32 over that
/// of the darkest 1/32 (their last and first knots).
fn knot_range(mut scene: Vec<f64>) -> f64 {
    scene.sort_by(f64::total_cmp);
    let n = scene.len();
    scene[31 * n / 32 + n / 64] / scene[n / 64]
}

/// Issue storytold/lightcraft#631: a low-contrast scene (its darker and brighter parts under 1.5× apart) still gets
/// a tone curve; only a nearly uniform one doesn't. Both tone fits use the same limit.
#[test]
fn tone_fits_accept_low_contrast_scenes() {
    for fit in [fit_tone, fit_tone_conditional] {
        let pairs = low_contrast_pairs(1.47);
        let range = knot_range(pairs.iter().map(|p| p.0).collect());
        assert!((1.4..1.5).contains(&range), "the scene sits just below the old limit of 1.5: {range}");
        let curve = fit(pairs).expect("a low-contrast scene, as in the burst of issue #631");
        assert!((curve.apply(0.1) - 0.26).abs() < 0.01, "follows the camera curve: {}", curve.apply(0.1));
        assert!(fit(low_contrast_pairs(1.1)).is_none(), "a nearly uniform frame");
    }
}

/// The same scene end to end: colour varies, luminance spans under 1.5×, and the camera
/// stretches it; the look is accepted and lands on the JPEG instead of the neutral fallback.
#[test]
fn a_low_contrast_scene_gets_a_camera_look() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let ev = 0.06 + (i % 31) as f32 * 0.0008;
        *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
        let p = known.apply_f32(*src);
        let y = luminance_2020(p);
        *dst = p.map(|v| v * (0.2 + (y - 0.07) * 3.0) / y);
    }
    let look = fit_pairs(&sensor, &reference).expect("a camera look, not the neutral fallback");
    let range = knot_range(sensor.data.iter().map(|x| luma(look.matrix.apply(x.map(f64::from)))).collect());
    assert!(range < 1.5, "the scene sits below the old limit of 1.5: {range}");
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let error: f64 = sensor
        .data
        .iter()
        .zip(&reference.data)
        .map(|(x, y)| {
            let p = displayed(look.matrix.apply(x.map(f64::from)), &tone);
            (0..3).map(|c| (p[c] - f64::from(y[c])).powi(2)).sum::<f64>() / 3.0
        })
        .sum::<f64>()
        / sensor.data.len() as f64;
    assert!(error.sqrt() < 0.02, "{}", error.sqrt());
}

/// Codex review of storytold/lightcraft#631: a low-contrast scene's curve is fitted to its midtones only, and the
/// camera JPEG's bright areas (luminance 0.85 and up) are in neither the fit nor the held-out
/// gates. With 1.2 alone, this frame's curve passed the gates and rendered its bright sky
/// (camera JPEG 0.93) at about 0.28. The second review found the same through the retries'
/// filters: a clipped sky (left out by the refit without clipped pixels), scattered bright
/// pixels (left out away from edges) and sensor values over 1.5 (out of the pairs' range).
#[test]
fn a_narrow_curve_that_darkens_the_bright_areas_is_refused() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    for (case, sky_value, is_sky) in [
        ("bright sky", 0.9, &(|i: usize| i < 5600) as &dyn Fn(usize) -> bool),
        ("clipped sky", 1.0, &|i: usize| i < 5600),
        ("scattered bright pixels", 0.9, &|i: usize| i.is_multiple_of(8)),
        ("sky over the pairs' range", 1.6, &|i: usize| i < 5600),
    ] {
        let mut sensor = Rgb32f::new(96, 64);
        let mut reference = sensor.clone();
        let mut clipped = vec![false; sensor.data.len()];
        for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
            if is_sky(i) {
                // bright, nearly white in the camera JPEG
                *src = [sky_value; 3];
                *dst = [0.93; 3];
                clipped[i] = case == "clipped sky";
                continue;
            }
            let ev = 0.06 + (i % 31) as f32 * 0.0008;
            *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
            let p = known.apply_f32(*src);
            let y = luminance_2020(p);
            // nearly flat midtones: the fitted curve's extrapolation can't reach the sky
            *dst = p.map(|v| v * (0.2 + (y - 0.07) * 0.05) / y);
        }
        let Some(look) = fit_look(&sensor, &reference, &clipped, None) else { continue };
        let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
        let correction = look.hue_sat.as_ref().and_then(HueSat::new);
        let p = look.matrix.apply([f64::from(sky_value); 3]);
        let p = correction.as_ref().map_or(p, |c| c.apply(p.map(|v| v as f32)).map(f64::from));
        let sky = luma(displayed(p, &tone));
        assert!(sky > 0.8, "{case}: an accepted look must not darken what the camera renders at 0.93: {sky}");
    }
}

/// Third Codex review of storytold/lightcraft#631: for a Sony photo shot with DRO, the fitted curve is lowered to
/// Sony's without DRO after the fit, so it must be checked as lowered. On this frame the
/// fitted curve keeps the sky (0.81 against the fallback's 0.79), and lowered it rendered the
/// sky at 0.61 where the camera JPEG has 0.93.
#[test]
fn a_narrow_curve_is_checked_as_lowered_for_dro() {
    let known = Mat3([[1.8, -0.4, -0.1], [-0.2, 1.5, -0.1], [-0.05, -0.3, 1.7]]);
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        if i < 5600 {
            *src = [0.9; 3];
            *dst = [0.93; 3];
            continue;
        }
        let ev = 0.06 + (i % 31) as f32 * 0.0008;
        *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
        let p = known.apply_f32(*src);
        let y = luminance_2020(p);
        *dst = p.map(|v| v * (0.6 + (y - 0.07) * 0.25) / y);
    }
    let neutral = luma(displayed([0.9; 3], &ToneMap::new(0.0, 0.0, 0.0)));
    if let Some(look) = fit_look_with(&sensor, &reference, &vec![false; sensor.data.len()], None, true) {
        let look = with_dro_off_tone(look);
        let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
        let correction = look.hue_sat.as_ref().and_then(HueSat::new);
        let p = look.matrix.apply([0.9; 3]);
        let p = correction.as_ref().map_or(p, |c| c.apply(p.map(|v| v as f32)).map(f64::from));
        let sky = luma(displayed(p, &tone));
        assert!(sky > neutral - 0.02, "the DRO-lowered curve must not darken the sky past the fallback ({neutral}): {sky}");
    }
}

/// A black-and-white JPEG of a low-contrast scene (issues storytold/lightcraft#577 and storytold/lightcraft#631): grey pixels, scene luminance spanning
/// under 1.5×, which the old range limit refused before any gate. Its tone curve is fitted.
#[test]
fn a_low_contrast_black_and_white_jpeg_gives_its_tone_curve() {
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        let ev = 0.06 + (i % 31) as f32 * 0.0008;
        *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
        *dst = [0.2 + (luminance_2020(*src) - 0.06) * 3.0; 3];
    }
    let range = knot_range(sensor.data.iter().map(|x| luma(x.map(f64::from))).collect());
    assert!(range < 1.5, "the scene sits below the old limit of 1.5: {range}");
    let look = fit_tone_only(&sensor, &reference, None).expect("the JPEG's tone curve");
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let error = sensor.data.iter().zip(&reference.data).map(|(x, y)| (luma(displayed(x.map(f64::from), &tone)) - f64::from(y[0])).powi(2));
    let rms = (error.sum::<f64>() / sensor.data.len() as f64).sqrt();
    assert!(rms < 0.02, "{rms}");
}

/// [`a_narrow_curve_that_darkens_the_bright_areas_is_refused`] for a black-and-white JPEG: its tone curve is
/// fitted to nearly flat midtones under 1.5×, and the bright sky (camera JPEG 0.93) is left out of the fit.
#[test]
fn a_narrow_black_and_white_curve_that_darkens_the_bright_areas_is_refused() {
    let mut sensor = Rgb32f::new(96, 64);
    let mut reference = sensor.clone();
    for (i, (src, dst)) in sensor.data.iter_mut().zip(&mut reference.data).enumerate() {
        if i < 5600 {
            *src = [0.9; 3];
            *dst = [0.93; 3];
            continue;
        }
        let ev = 0.06 + (i % 31) as f32 * 0.0008;
        *src = [ev * (0.8 + (i % 11) as f32 * 0.025), ev, ev * (0.8 + (i % 17) as f32 * 0.014)];
        *dst = [0.2 + (luminance_2020(*src) - 0.06) * 0.05; 3];
    }
    if let Some(look) = fit_tone_only(&sensor, &reference, None) {
        let sky = luma(displayed(look.matrix.apply([0.9; 3]), &ToneMap::camera(&look.tone, 0.0, 0.0, 0.0)));
        assert!(sky > 0.8, "an accepted curve must not darken what the camera renders at 0.93: {sky}");
    }
}

/// A synthetic ILCE-7M4 raw (it has a bundled camera profile; demosaiced grey `sensor` levels) and its file bytes:
/// a minimal TIFF holding the camera JPEG, a bright camera curve on the `scene` luminance as an 8-bit sRGB JPEG in
/// `tint` (grey for `[1.0; 3]`).
fn grey_raw_with_jpeg(
    (w, h): (usize, usize),
    sensor: impl Fn(usize, usize) -> f32,
    scene: impl Fn(usize, usize) -> f32,
    tint: [f32; 3],
) -> (RawImage, Vec<u8>) {
    use crate::sensor::{BlackLevel, ColorData, OpcodeLists, Orientation, RawData, Rect};
    let raw = RawImage {
        format: RawFormat::Arw,
        width: w,
        height: h,
        cpp: 3,
        data: RawData::F32((0..w * h).flat_map(|i| [sensor(i % w, i / w); 3]).collect()),
        cfa: None,
        bits: 16,
        black: BlackLevel::uniform(0.0),
        white: vec![1.0],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::from_exif(1),
        color: ColorData::default(),
        wb_multipliers: Some([1.0; 3]),
        linearized: true,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: crate::sensor::Camera { make: Some("SONY".into()), model: Some("ILCE-7M4".into()), ..Default::default() },
        cinema: Default::default(),
        level_sources: Default::default(),
        lens_metadata: Default::default(),
    };
    let srgb = |v: f32| if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    let pixels: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let v = 1.0 - (-3.0 * scene(i % w, i / w)).exp();
            tint.map(|t| (srgb(v * t).clamp(0.0, 1.0) * 255.0).round() as u8)
        })
        .collect();
    let mut jpeg = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut jpeg, 95);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
    encoder.encode(&pixels, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    // II*, IFD0 at 8 with JPEGInterchangeFormat (0x201) and its length (0x202), the JPEG after it
    let mut tiff = b"II*\0\x08\0\0\0\x02\0".to_vec();
    for (tag, value) in [(0x0201u16, 38u32), (0x0202, jpeg.len() as u32)] {
        tiff.extend(tag.to_le_bytes());
        tiff.extend(4u16.to_le_bytes());
        tiff.extend(1u32.to_le_bytes());
        tiff.extend(value.to_le_bytes());
    }
    tiff.extend(0u32.to_le_bytes());
    assert_eq!(tiff.len(), 38);
    tiff.extend(jpeg);
    (raw, tiff)
}

/// [`monochrome_look`] on a synthetic raw file: an ILCE-7M4 whose camera JPEG is black and white gets the tone curve,
/// on the colour of the transform it is given; the same file with a tinted JPEG gets nothing from this step.
#[test]
fn monochrome_look_reads_the_files_black_and_white_jpeg() {
    let (w, h) = (192usize, 128usize);
    let level = |x: usize, y: usize| 0.01 + 0.5 * (x as f32 / w as f32) * (0.3 + 0.7 * y as f32 / h as f32);
    let (raw, grey) = grey_raw_with_jpeg((w, h), level, level, [1.0; 3]);
    assert!(crate::look::profiles::get("ILCE-7M4").is_some());
    let t = crate::sensor::color::camera_transform(&raw, crate::sensor::color::as_shot_white_xy(&raw));
    let look = monochrome_look(&raw, &grey, &t, None, None).expect("a tone curve from the black-and-white JPEG");
    assert!(look.matrix == Mat3::IDENTITY && look.hue_sat.is_none(), "on the transform's colour");
    let curve = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let mid = curve.apply(0.25);
    assert!((mid - (1.0 - (-0.75f32).exp())).abs() < 0.05, "the camera's curve at 0.25: {mid}");
    let (_, tinted) = grey_raw_with_jpeg((w, h), level, level, [1.0, 0.9, 0.8]);
    assert!(monochrome_look(&raw, &tinted, &t, None, None).is_none(), "a tinted JPEG is not black and white");
}

/// The look is fitted on the sensor as the render shows it, with the file's embedded vignetting correction: the
/// camera corrected its JPEG, so a curve fitted to the uncorrected sensor learns that brightening, and the render
/// then applies it twice (X-S10 RAFs up to 0.3 EV brighter than their camera JPEGs).
#[test]
fn the_look_is_fitted_on_the_vignetting_corrected_sensor() {
    use craft_develop_model::{EmbeddedLens, EmbeddedVignette};
    let (w, h) = (192usize, 128usize);
    let vignette = EmbeddedVignette { k: [1.0, 0.0, 0.0, 0.0, 0.0], center: craft_geom::Point::new(0.5, 0.5), radius: 0.6 };
    let gain = |x: usize, y: usize| {
        let point = craft_geom::Point::new(x as f64 + 0.5, y as f64 + 0.5);
        crate::optics::embedded_vignette_gain(&vignette, point, w as f64, h as f64) as f32
    };
    let scene = |x: usize, y: usize| 0.01 + 0.5 * (x as f32 / w as f32) * (0.3 + 0.7 * y as f32 / h as f32);
    let (raw, bytes) = grey_raw_with_jpeg((w, h), |x, y| scene(x, y) / gain(x, y), scene, [1.0; 3]);
    let t = crate::sensor::color::camera_transform(&raw, crate::sensor::color::as_shot_white_xy(&raw));
    let lens = EmbeddedLens { vignette: Some(vignette), ..Default::default() };
    let camera = 1.0 - (-0.75f32).exp();
    let at_quarter = |embedded: Option<&EmbeddedLens>| {
        let look = monochrome_look(&raw, &bytes, &t, None, embedded).expect("a tone curve");
        ToneMap::camera(&look.tone, 0.0, 0.0, 0.0).apply(0.25)
    };
    let (corrected, uncorrected) = (at_quarter(Some(&lens)), at_quarter(None));
    assert!((corrected - camera).abs() < 0.03, "the camera's curve at 0.25: {corrected} vs {camera}");
    assert!(uncorrected > camera + 0.03, "without the correction the curve brightens: {uncorrected} vs {camera}");
}

/// The colour fit gets the embedded vignetting correction too ([`starting_colour`] forwards it to every fit): the
/// tinted camera JPEG of the vignetted sensor takes the colour path, not the black-and-white one.
#[test]
fn the_colour_look_is_fitted_on_the_vignetting_corrected_sensor() {
    use craft_develop_model::{EmbeddedLens, EmbeddedVignette};
    let (w, h) = (192usize, 128usize);
    let vignette = EmbeddedVignette { k: [1.0, 0.0, 0.0, 0.0, 0.0], center: craft_geom::Point::new(0.5, 0.5), radius: 0.6 };
    let gain = |x: usize, y: usize| {
        let point = craft_geom::Point::new(x as f64 + 0.5, y as f64 + 0.5);
        crate::optics::embedded_vignette_gain(&vignette, point, w as f64, h as f64) as f32
    };
    let scene = |x: usize, y: usize| 0.01 + 0.5 * (x as f32 / w as f32) * (0.3 + 0.7 * y as f32 / h as f32);
    let (raw, bytes) = grey_raw_with_jpeg((w, h), |x, y| scene(x, y) / gain(x, y), scene, [1.0, 0.9, 0.8]);
    let lens = EmbeddedLens { vignette: Some(vignette), ..Default::default() };
    let at_quarter = |embedded: Option<&EmbeddedLens>| {
        let (_, _, look) = starting_colour(&mut raw.clone(), &bytes, None, embedded);
        let look = look.expect("a colour look");
        assert!(look.matrix != Mat3::IDENTITY || look.hue_sat.is_some(), "the colour fit, not the black-and-white one");
        ToneMap::camera(&look.tone, 0.0, 0.0, 0.0).apply(0.25)
    };
    let (corrected, uncorrected) = (at_quarter(Some(&lens)), at_quarter(None));
    assert!(uncorrected > corrected + 0.03, "without the correction the curve brightens: {uncorrected} vs {corrected}");
}

/// A lens-warped sensor proxy gets the vignetting gain of the position it samples, as the render applies it: the
/// vignetted sensor, corrected, gives the proxy of the same sensor without vignetting.
#[test]
fn the_vignetting_correction_is_applied_before_the_lens_warp() {
    use craft_develop_model::{EmbeddedLens, EmbeddedVignette};
    // twice the 384 px proxy edge: the demosaiced proxy of this linear raw is then the size a warp needs (bins of 2)
    let (w, h) = (768usize, 512usize);
    let vignette = EmbeddedVignette { k: [1.0, 0.0, 0.0, 0.0, 0.0], center: craft_geom::Point::new(0.5, 0.5), radius: 0.6 };
    let gain = |x: usize, y: usize| {
        let point = craft_geom::Point::new(x as f64 + 0.5, y as f64 + 0.5);
        crate::optics::embedded_vignette_gain(&vignette, point, w as f64, h as f64) as f32
    };
    // a texture for the warped candidate to line up with: the JPEG shows the sensor's centre zoomed by 1 / 0.8
    let texture = |x: f64, y: f64| 0.2 * (1.0 + 0.3 * ((x / 48.0).floor() + (y / 48.0).floor()).rem_euclid(2.0)) as f32;
    let warp = LensGeometry { k: [0.8, 0.0, 0.0, 0.0], center: [0.5, 0.5] };
    let (cx, cy) = ((w - 1) as f64 / 2.0, (h - 1) as f64 / 2.0);
    let jpeg = |x: usize, y: usize| texture(cx + (x as f64 - cx) * 0.8, cy + (y as f64 - cy) * 0.8);
    let sensor_proxy = |vignetted: bool, embedded: Option<&EmbeddedLens>| {
        let sensor = |x: usize, y: usize| texture(x as f64, y as f64) / if vignetted { gain(x, y) } else { 1.0 };
        let (raw, bytes) = grey_raw_with_jpeg((w, h), sensor, jpeg, [1.0; 3]);
        let t = crate::sensor::color::camera_transform(&raw, crate::sensor::color::as_shot_white_xy(&raw));
        let (plain, _, _) = proxies(&raw, &bytes, &t, PROXY, 0.0, None, embedded).unwrap();
        let (sensor, _, _) = proxies(&raw, &bytes, &t, PROXY, 0.0, Some(&warp), embedded).unwrap();
        assert!(sensor.data.iter().zip(&plain.data).any(|(a, b)| a != b), "the warped candidate is chosen");
        sensor
    };
    let lens = EmbeddedLens { vignette: Some(vignette), ..Default::default() };
    let (corrected, expected) = (sensor_proxy(true, Some(&lens)), sensor_proxy(false, None));
    let worst = corrected.data.iter().zip(&expected.data).map(|(a, b)| (a[1] / b[1] - 1.0).abs()).filter(|d| d.is_finite()).fold(0.0, f32::max);
    assert!(worst < 0.02, "largest deviation {worst}");
}

use crate::raster::Rgb32f;
use craft_develop_model::{DevelopSettings, controls};

use crate::{RenderRequest, SourceInfo, render};

fn mean_luma(img: &crate::raster::Rgba8) -> f32 {
    img.data.iter().map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).sum::<f32>() / img.len() as f32
}

fn scene() -> Rgb32f {
    crate::scenes::demo_library()[0].render(240, 160)
}

#[test]
fn default_render_is_sane() {
    let r = render(&scene(), &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(240, 240));
    assert_eq!((r.image.width, r.image.height), (240, 160));
    let m = mean_luma(&r.image);
    assert!((40.0..220.0).contains(&m), "{m}");
    assert!(r.histogram.total > 0);
}

#[test]
fn grey_ramp_stays_neutral() {
    let src = Rgb32f::from_fn(64, 8, |x, _| [0.002 * 1.12f32.powi(x as i32); 3]);
    let r = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(64, 8));
    for p in &r.image.data {
        assert!((p[0] as i32 - p[1] as i32).abs() <= 1 && (p[1] as i32 - p[2] as i32).abs() <= 1, "{p:?}");
    }
}

#[test]
fn every_slider_changes_or_keeps_output_without_panicking() {
    let src = scene();
    let base = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(96, 96)).image;
    for c in controls::CONTROLS {
        for v in [c.min, c.max] {
            let mut s = DevelopSettings::default();
            controls::set(&mut s, c.id, v);
            let r = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(96, 96));
            assert!(r.image.width > 0, "{}", c.id);
            let _ = &base;
        }
    }
}

#[test]
fn exposure_is_monotone() {
    let src = scene();
    let mut prev = -1.0;
    for ev in [-3.0, -1.5, 0.0, 1.0, 2.5] {
        let mut s = DevelopSettings::default();
        s.light.exposure = ev;
        let m = mean_luma(&render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(96, 96)).image);
        assert!(m > prev, "{ev}: {m} <= {prev}");
        prev = m;
    }
}

#[test]
fn directional_sliders() {
    let src = scene();
    let at = |id: &str, v: f64| {
        let mut s = DevelopSettings::default();
        controls::set(&mut s, id, v);
        mean_luma(&render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(96, 96)).image)
    };
    let base = at("light.exposure", 0.0);
    assert!(at("light.shadows", 100.0) > base);
    assert!(at("light.highlights", -100.0) < base);
    assert!(at("light.whites", 100.0) > base);
    assert!(at("light.blacks", -100.0) < base);
    assert!(at("vignette.amount", -100.0) < base);
    assert!(at("vignette.amount", 100.0) > base);
    assert!(at("effects.dehaze", -100.0) != base);
}

/// Dehaze crushed dark regions next to bright ones to black: the dark channel is a blur, not a
/// patch minimum, so near the boundary it credited the dark pixels with the bright side's veil and
/// the subtraction went past zero (storytold/lightcraft#2504). The veil credited to a pixel is now bounded by its own
/// minimum channel, so a uniform dark region renders uniformly under dehaze.
#[test]
fn dehaze_does_not_crush_dark_regions_next_to_bright_ones() {
    // a dark field against a bright hazy sky; the blur bleeds the sky's dark channel ~10 px into
    // the field, which is where the blotches formed
    let src = Rgb32f::from_fn(240, 160, |x, _| if x < 120 { [0.02, 0.03, 0.012] } else { [0.45, 0.5, 0.55] });
    let mut s = DevelopSettings::default();
    s.effects.dehaze = 80.0;
    let r = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(240, 160)).image;
    for ch in 0..3 {
        let mut v: Vec<u8> = Vec::with_capacity(160 * 100);
        for y in 0..160 {
            for x in 0..100 {
                v.push(r.get(x, y)[ch]);
            }
        }
        v.sort_unstable();
        let mid = v[v.len() / 2] as i32;
        // a field rendered all black would pass the comparison below vacuously
        assert!(mid > 2, "the dark field itself is crushed in ch {ch}: level {mid}");
        for y in 0..160 {
            for x in 0..120 {
                let p = r.get(x, y)[ch] as i32;
                assert!(p >= mid - 2, "crushed at {x},{y} ch {ch}: {p} vs field level {mid}");
            }
        }
    }
}

#[test]
fn resolution_independence_of_local_contrast() {
    // Clarity at two preview sizes should give similar results after downscaling.
    let src = crate::scenes::demo_library()[3].render(480, 320);
    let mut s = DevelopSettings::default();
    s.effects.clarity = 80.0;
    s.light.shadows = 60.0;
    let small = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(120, 120)).image;
    let big = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(480, 480)).image;
    let bl = big.to_linear();
    let down = crate::raster::resample::resize(&bl, small.width, small.height, crate::raster::resample::Filter::Box).to_srgb8();
    let err: f32 = small.data.iter().zip(&down.data).map(|(a, b)| (a[1] as f32 - b[1] as f32).abs()).sum::<f32>() / small.len() as f32;
    assert!(err < 9.0, "mean abs error {err}");
}

#[test]
fn crop_and_straighten_output_size() {
    let mut s = DevelopSettings::default();
    s.crop.geometry = craft_geom::crop_fit_angle(240.0, 160.0, 8.0, Some(1.0));
    let r = render(&scene(), &SourceInfo::default(), &s, &RenderRequest::fit(200, 200));
    assert_eq!((r.image.width, r.image.height), (200, 200));
}

#[test]
fn mask_brightens_only_inside() {
    use craft_develop_model::{Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let src = Rgb32f::filled(100, 100, [0.1, 0.1, 0.1]);
    let mut s = DevelopSettings::default();
    s.masks.push(Mask {
        components: vec![MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.2, ry: 0.2, angle: 0.0, feather: 10.0, invert: false },
        }],
        adjust: craft_develop_model::LocalAdjustments { exposure: 2.0, ..Default::default() },
        ..Default::default()
    });
    let r = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(100, 100)).image;
    assert!(r.get(50, 50)[0] > r.get(5, 5)[0] + 40);
}

#[test]
#[ignore]
fn perf_report() {
    let src = crate::scenes::demo_library()[0].render(3000, 2000);
    let mut s = DevelopSettings::default();
    s.light.shadows = 40.0;
    s.effects.clarity = 20.0;
    s.effects.dehaze = 10.0;
    s.color.vibrance = 20.0;
    let t = std::time::Instant::now();
    let n = 5;
    for _ in 0..n {
        let _ = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(2560, 1440));
    }
    eprintln!("render 2160x1440 from 6 MP: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
}

#[test]
fn unedited_rendered_source_is_passthrough() {
    // An sRGB ramp decoded to linear Rec.2020 must render back to the same 8-bit values, white
    // included (issue storytold/lightcraft#1945: neutral 255, 254 and 253 came back one level darker).
    let src8 = crate::raster::Rgba8::from_fn(256, 4, |x, y| [x as u8, (x as u8).wrapping_add(y as u8 * 40), 255 - x as u8, 255]);
    let m = crate::color::SRGB.to_space(&crate::color::REC2020);
    let src = src8.to_linear().map(|c| m.apply_f32(c));
    let r = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(256, 4)).image;
    let mut worst = 0i32;
    for (a, b) in src8.data.iter().zip(&r.data) {
        for c in 0..3 {
            worst = worst.max((a[c] as i32 - b[c] as i32).abs());
        }
    }
    assert_eq!(worst, 0, "max error {worst}");
}

#[test]
fn unedited_neutral_near_whites_are_passthrough() {
    // Issue storytold/lightcraft#1945: neutral 255, 254 and 253 rendered one level darker (the display tone map's
    // shoulder ran at neutral settings), so a white background never exported as 255.
    let vals = [255u8, 254, 253, 252, 250, 240, 200, 128, 0];
    let src8 = crate::raster::Rgba8::from_fn(vals.len(), 1, |x, _| {
        let v = vals[x];
        [v, v, v, 255]
    });
    let m = crate::color::SRGB.to_space(&crate::color::REC2020);
    let src = src8.to_linear().map(|c| m.apply_f32(c));
    let r = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(vals.len(), 1)).image;
    let got: Vec<[u8; 3]> = r.data.iter().map(|p| [p[0], p[1], p[2]]).collect();
    let want: Vec<[u8; 3]> = vals.iter().map(|&v| [v, v, v]).collect();
    assert_eq!(got, want);
}

fn typical_edits() -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.light.exposure = 0.4;
    s.light.highlights = -40.0;
    s.light.shadows = 30.0;
    s.effects.clarity = 15.0;
    s.effects.texture = 10.0;
    s.effects.dehaze = 12.0;
    s.detail.nr_luminance = 30.0;
    s.detail.nr_color = 25.0;
    s.detail.sharpen_amount = 40.0;
    s
}

fn max_diff(a: &crate::raster::Rgba8, b: &crate::raster::Rgba8) -> i32 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.data.iter().zip(&b.data).map(|(p, q)| (0..3).map(|c| (p[c] as i32 - q[c] as i32).abs()).max().unwrap_or(0)).max().unwrap_or(0)
}

#[test]
#[allow(clippy::type_complexity)] // the type is spelled out where it is used
fn stage_cache_matches_uncached_render_through_a_slider_session() {
    use crate::{Quality, StageCache, render_cached};
    use std::sync::Arc;
    let src = Arc::new(scene());
    let info = SourceInfo { raw: true, ..Default::default() };
    let cache = StageCache::default();
    let full = RenderRequest::fit(200, 200);
    let draft = RenderRequest { quality: Quality::Draft, ..RenderRequest::fit(120, 120) };
    let mut s = typical_edits();
    // every kind of edit: tone, exposure, spatial amounts, NR, WB, crop, colour — at two sizes
    let steps: Vec<Box<dyn Fn(&mut DevelopSettings)>> = vec![
        Box::new(|s| s.light.contrast = 20.0),
        Box::new(|s| s.light.exposure = -0.7),
        Box::new(|s| s.light.highlights = -80.0),
        Box::new(|s| s.effects.clarity = -30.0),
        Box::new(|s| s.effects.dehaze = 0.0),
        Box::new(|s| s.effects.dehaze = 25.0),
        Box::new(|s| s.detail.nr_luminance = 60.0),
        Box::new(|s| {
            s.wb.mode = craft_develop_model::WbMode::Custom;
            s.wb.temp = 4000.0;
        }),
        Box::new(|s| s.crop.geometry.rect = craft_geom::Rect::new(0.1, 0.1, 0.9, 0.8)),
        Box::new(|s| s.color.saturation = 30.0),
        // optics and geometry (merged after the stage cache): defringe lives in the cached linear stage
        Box::new(|s| s.optics.defringe_purple_amount = 10.0),
        Box::new(|s| s.optics.distortion = 30.0),
        Box::new(|s| s.optics.remove_ca = true),
        Box::new(|s| s.geometry.vertical = 20.0),
    ];
    for (i, step) in steps.iter().enumerate() {
        step(&mut s);
        for req in [&full, &draft] {
            let a = render_cached(&src, &info, &s, req, &cache).image;
            let b = render(&src, &info, &s, req).image;
            if req.quality == Quality::Draft && i == 6 {
                // a draft while noise reduction changes keeps the wide planes of the image before it
                let d = max_diff(&a, &b);
                assert!(d <= 3, "step {i} draft: {d}");
            } else {
                assert_eq!(a, b, "step {i} {:?}", req.quality);
            }
        }
    }
    assert_eq!(cache.len(), 2);
    // both sizes' sampled + linear images at least (12 bytes per pixel each), nothing after clear()
    let b = cache.bytes();
    assert!(b >= 2 * 12 * (200 * 133 + 120 * 80), "{b}");
    cache.clear();
    assert_eq!(cache.bytes(), 0);
    // a different source buffer never hits another source's entries
    let other = Arc::new(Rgb32f::filled(src.width, src.height, [0.3, 0.3, 0.3]));
    assert_eq!(render_cached(&other, &info, &s, &full, &cache).image, render(&other, &info, &s, &full).image);
}

#[test]
fn drafts_keep_wide_planes_across_a_noise_reduction_change_and_full_renders_do_not() {
    use crate::{Quality, StageCache, render_cached};
    use std::sync::Arc;
    let src = Arc::new(scene());
    let info = SourceInfo { raw: true, ..Default::default() };
    let cache = StageCache::default();
    let draft = RenderRequest { quality: Quality::Draft, ..RenderRequest::fit(160, 160) };
    let full = RenderRequest { quality: Quality::Full, ..draft };
    let mut s = typical_edits();
    s.effects.clarity = 60.0;
    s.detail.sharpen_amount = 80.0;
    render_cached(&src, &info, &s, &draft, &cache);
    let clarity_plane = |c: &StageCache| c.entries.lock().unwrap().first().and_then(|e| e.planes.clarity.clone()).unwrap().1;
    let sharpen_plane = |c: &StageCache| c.entries.lock().unwrap().first().and_then(|e| e.planes.sharpen.clone()).unwrap().1;
    let first = clarity_plane(&cache);
    let first_sharpen = sharpen_plane(&cache);
    s.detail.nr_luminance = 50.0;
    render_cached(&src, &info, &s, &draft, &cache);
    assert!(Arc::ptr_eq(&first, &clarity_plane(&cache)), "a draft redid the clarity plane for a noise-reduction change");
    assert!(!Arc::ptr_eq(&first_sharpen, &sharpen_plane(&cache)), "a draft reused the old sharpening plane after noise reduction changed");
    for v in [10.0, 45.0, 80.0] {
        s.detail.nr_luminance = v;
        s.detail.nr_color = v / 2.0;
        let a = render_cached(&src, &info, &s, &draft, &cache).image;
        let d = max_diff(&a, &render(&src, &info, &s, &draft).image);
        // close to the exact draft (a few levels at edges, where clarity is strong)
        assert!(d <= 8, "nr {v}: {d}");
        // a full-quality render after the drafts is exact (it never takes the stand-in planes)
        assert_eq!(render_cached(&src, &info, &s, &full, &cache).image, render(&src, &info, &s, &full).image, "nr {v}");
    }
}

#[test]
fn exposure_after_spatial_filters_equals_exposing_the_source() {
    // The pipeline filters the image before exposure and applies exposure per pixel; filters on
    // log luminance are shift-equivariant, so this must equal scaling the source.
    let src = scene();
    let info = SourceInfo { raw: true, ..Default::default() };
    let mut s = typical_edits();
    s.light.exposure = 1.3;
    let a = render(&src, &info, &s, &RenderRequest::fit(200, 200)).image;
    let g = 2f32.powf(1.3);
    let scaled = src.map(|c| c.map(|v| v * g));
    s.light.exposure = 0.0;
    let b = render(&scaled, &info, &s, &RenderRequest::fit(200, 200)).image;
    let d = max_diff(&a, &b);
    assert!(d <= 2, "max difference {d}");
}

#[test]
fn color_range_sampling_respects_disabled_sections() {
    use crate::{MaskView, Overlay, color_range_sample};
    let src = Rgb32f::from_fn(32, 24, |_, _| [0.15, 0.25, 0.08]);
    let info = SourceInfo::default();
    let req = RenderRequest::fit(32, 24);
    let point = craft_geom::Point::new(0.5, 0.5);
    let plain = color_range_sample(&src, &info, &DevelopSettings::default(), &req, point).unwrap();
    for section in ["light", "color"] {
        let mut s = DevelopSettings::default();
        if section == "light" {
            s.light.exposure = 3.0;
        } else {
            s.wb.mode = craft_develop_model::WbMode::Custom;
            s.wb.temp = 12000.0;
            s.wb.tint = 100.0;
        }
        assert_ne!(color_range_sample(&src, &info, &s, &req, point).unwrap(), plain, "enabled {section}");
        s.set_section_enabled(section, false);
        let sample = color_range_sample(&src, &info, &s, &req, point).unwrap();
        assert_eq!(sample, plain, "disabled {section} must not change the picked colour");
        s.masks.push(craft_develop_model::Mask {
            id: 1,
            components: vec![craft_develop_model::MaskComponent {
                name: None,
                op: craft_develop_model::MaskOp::Add,
                invert: false,
                shape: craft_develop_model::MaskShape::ColorRange { samples: vec![sample], refine: 0.0 },
            }],
            ..Default::default()
        });
        let overlay = RenderRequest { overlay: Overlay::Mask { id: 1, view: MaskView::WhiteOnBlack, color: [255, 0, 0], opacity: 100 }, ..req };
        let img = render(&src, &info, &s, &overlay).image;
        assert_eq!(img.data[12 * 32 + 16], [255; 4], "the sampled colour is selected with {section} off");
    }
}

#[test]
fn color_range_sampling_uses_the_requested_hdr_rendition() {
    use crate::{OutputDepth, color_range_sample};
    let src = Rgb32f::from_fn(32, 24, |_, _| [0.15, 0.25, 0.08]);
    let info = SourceInfo::default();
    let point = craft_geom::Point::new(0.5, 0.5);
    let mut s = DevelopSettings::default();
    s.hdr.enabled = true;
    s.hdr.sdr_brightness = 100.0;
    let req = RenderRequest::fit(32, 24);
    let expected = color_range_sample(&src, &info, &s.sdr_rendition(), &req, point).unwrap();
    assert_eq!(color_range_sample(&src, &info, &s, &req, point).unwrap(), expected, "SDR preview brightness affects the picked colour");
    let hdr = RenderRequest { depth: OutputDepth::F32Hdr, ..req };
    let plain = color_range_sample(&src, &info, &DevelopSettings::default(), &hdr, point).unwrap();
    assert_eq!(color_range_sample(&src, &info, &s, &hdr, point).unwrap(), plain, "HDR does not use the SDR offset");
    assert_ne!(plain, expected);
}

/// A section whose eye is off renders as if it were at its defaults (issue storytold/lightcraft#316): Light, Color
/// and Detail, which the pipeline doesn't switch off where it applies them.
#[test]
fn sections_switched_off_render_as_defaults() {
    let src = Rgb32f::from_fn(32, 24, |x, y| [0.05 + x as f32 / 40.0, 0.1 + y as f32 / 30.0, 0.3]);
    let info = SourceInfo::default();
    let req = RenderRequest::fit(32, 24);
    let plain = render(&src, &info, &DevelopSettings::default(), &req).image;
    let mut s = DevelopSettings::default();
    s.light.exposure = 1.5;
    s.light.contrast = 40.0;
    s.color.saturation = -60.0;
    s.color.vibrance = 30.0;
    s.detail.sharpen_amount = 120.0;
    let edited = render(&src, &info, &s, &req).image;
    assert_ne!(edited.data, plain.data);
    for section in ["light", "color", "detail"] {
        s.set_section_enabled(section, false);
    }
    assert_eq!(render(&src, &info, &s, &req).image.data, plain.data, "every edited section off = the unedited photo");
    // one section back on brings back only its own adjustments
    s.set_section_enabled("light", true);
    let mut light_only = DevelopSettings::default();
    light_only.light.exposure = 1.5;
    light_only.light.contrast = 40.0;
    assert_eq!(render(&src, &info, &s, &req).image.data, render(&src, &info, &light_only, &req).image.data);
    assert!(matches!(DevelopSettings::default().effective(), std::borrow::Cow::Borrowed(_)), "nothing is copied when every section is on");
}

#[test]
fn calibration_shifts_colours_but_keeps_greys() {
    let info = SourceInfo { raw: true, ..Default::default() };
    let req = RenderRequest::fit(64, 8);
    // a grey ramp is untouched by the primaries
    let grey = Rgb32f::from_fn(64, 8, |x, _| [0.002 * 1.12f32.powi(x as i32); 3]);
    let mut s = DevelopSettings::default();
    s.calibration.red_hue = 80.0;
    s.calibration.green_sat = -60.0;
    let a = render(&grey, &info, &DevelopSettings::default(), &req).image;
    let b = render(&grey, &info, &s, &req).image;
    assert!(max_diff(&a, &b) <= 1);
    // a red patch rotates towards yellow (green rises relative to blue)
    let red = Rgb32f::filled(16, 16, [0.3, 0.04, 0.03]);
    let r0 = render(&red, &info, &DevelopSettings::default(), &RenderRequest::fit(16, 16)).image.data[0];
    let r1 = render(&red, &info, &s, &RenderRequest::fit(16, 16)).image.data[0];
    assert!(r1[1] as i32 - r1[2] as i32 > r0[1] as i32 - r0[2] as i32 + 5, "{r0:?} -> {r1:?}");
    // shadows tint: magenta (+) lowers green in a dark grey
    let dark = Rgb32f::filled(16, 16, [0.01, 0.01, 0.01]);
    s = DevelopSettings::default();
    s.calibration.shadows_tint = 100.0;
    let d = render(&dark, &info, &s, &RenderRequest::fit(16, 16)).image.data[0];
    assert!(d[1] + 3 < d[0], "{d:?}");
    // the section toggle disables it
    s.set_section_enabled("calibration", false);
    let off = render(&dark, &info, &s, &RenderRequest::fit(16, 16)).image.data[0];
    assert!(off[1].abs_diff(off[0]) <= 1, "{off:?}");
}

#[test]
fn refine_saturation_tames_a_contrast_curve() {
    let info = SourceInfo { raw: true, ..Default::default() };
    let src = Rgb32f::filled(16, 16, [0.12, 0.05, 0.03]);
    let req = RenderRequest::fit(16, 16);
    let sat = |p: [u8; 4]| p[0] as i32 - p[2] as i32;
    let mut s = DevelopSettings::default();
    let flat = render(&src, &info, &s, &req).image.data[0];
    s.curve.master = vec![
        craft_geom::Point::new(0.0, 0.0),
        craft_geom::Point::new(0.3, 0.15),
        craft_geom::Point::new(0.7, 0.85),
        craft_geom::Point::new(1.0, 1.0),
    ];
    let full = render(&src, &info, &s, &req).image.data[0];
    s.curve.refine_saturation = 0.0;
    let refined = render(&src, &info, &s, &req).image.data[0];
    assert!(sat(full) > sat(refined), "{flat:?} {full:?} {refined:?}");
    // the curve's tone change stays
    let y = |p: [u8; 4]| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
    assert!((y(full) - y(refined)).abs() < 2.0);
}

#[test]
fn soft_proof_maps_into_the_proof_gamut_and_flags_what_does_not_fit() {
    use crate::{OutputSpace, Proof};
    let src = scene();
    let mut s = DevelopSettings::default();
    s.color.saturation = 100.0;
    s.color.vibrance = 100.0;
    let info = SourceInfo { raw: true, ..Default::default() };
    let req = |space, proof| RenderRequest { space, proof, ..RenderRequest::fit(120, 120) };
    let red = |img: &crate::raster::Rgba8| img.data.iter().filter(|p| p[..3] == [255, 0, 0]).count();
    let blue = |img: &crate::raster::Rgba8| img.data.iter().filter(|p| p[2] == 255 && p[0] == 0 && p[1] < 80).count();

    // proofing sRGB on an sRGB render changes nothing but the warning
    let plain = render(&src, &info, &s, &req(OutputSpace::Srgb, None)).image;
    let same = render(&src, &info, &s, &req(OutputSpace::Srgb, Some(Proof { space: OutputSpace::Srgb, ..Default::default() }))).image;
    assert_eq!(plain, same);
    let warned = render(
        &src,
        &info,
        &s,
        &req(OutputSpace::Srgb, Some(Proof { space: OutputSpace::Srgb, dest_warning: true, display_warning: false, profile: 0 })),
    )
    .image;
    let n = red(&warned);
    assert!(n > 0 && n < warned.len(), "{n}");
    // …and nothing is flagged on an unsaturated edit of a neutral ramp
    let grey = Rgb32f::from_fn(64, 8, |x, _| [0.002 * 1.12f32.powi(x as i32); 3]);
    let g = render(
        &grey,
        &info,
        &DevelopSettings::default(),
        &RenderRequest {
            proof: Some(Proof { space: OutputSpace::Srgb, dest_warning: true, display_warning: true, profile: 0 }),
            ..RenderRequest::fit(64, 8)
        },
    );
    assert_eq!(red(&g.image) + blue(&g.image), 0);

    // a wide-gamut render proofed for sRGB loses the colours sRGB can't hold
    let wide = render(&src, &info, &s, &req(OutputSpace::DisplayP3, None)).image;
    let proofed = render(&src, &info, &s, &req(OutputSpace::DisplayP3, Some(Proof { space: OutputSpace::Srgb, ..Default::default() }))).image;
    assert_ne!(wide, proofed);
    // proofing a wider space than the display: what the display can't show is flagged blue
    let pro = render(
        &src,
        &info,
        &s,
        &req(OutputSpace::Srgb, Some(Proof { space: OutputSpace::ProPhoto, dest_warning: true, display_warning: true, profile: 0 })),
    )
    .image;
    assert!(blue(&pro) > 0);
    assert!(red(&pro) < n, "ProPhoto holds more than sRGB");
}

/// A preview for a monitor with a display profile renders into the display's primaries: for a
/// display that *is* a standard space, exactly what an 8-bit render into that space gives.
#[test]
fn display_space_renders_like_the_matching_output_space() {
    use crate::{DisplaySpace, OutputSpace, Proof};
    let src = scene();
    let mut s = DevelopSettings::default();
    s.color.saturation = 100.0;
    s.color.vibrance = 100.0;
    let info = SourceInfo { raw: true, ..Default::default() };
    let req = |space, display, proof| RenderRequest { space, display, proof, ..RenderRequest::fit(120, 120) };
    for space in [OutputSpace::Srgb, OutputSpace::DisplayP3] {
        let d = DisplaySpace::of(space, 7).unwrap();
        let want = render(&src, &info, &s, &req(space, None, None)).image;
        let got = render(&src, &info, &s, &req(OutputSpace::Srgb, Some(d), None)).image;
        assert!(max_diff(&got, &want) <= 1, "{space:?}: {}", max_diff(&got, &want));
    }
    // a wide display shows the saturated colours sRGB clips
    let srgb = render(&src, &info, &s, &req(OutputSpace::Srgb, None, None)).image;
    let p3 = render(&src, &info, &s, &req(OutputSpace::Srgb, Some(DisplaySpace::of(OutputSpace::DisplayP3, 1).unwrap()), None)).image;
    assert!(max_diff(&srgb, &p3) > 4);

    // soft proofing on a display: the display gamut warning is about *this* display
    let blue = |img: &crate::raster::Rgba8| img.data.iter().filter(|p| p[2] == 255 && p[0] == 0 && p[1] < 80).count();
    let pro = Some(Proof { space: OutputSpace::ProPhoto, dest_warning: false, display_warning: true, profile: 0 });
    let on_srgb = blue(&render(&src, &info, &s, &req(OutputSpace::Srgb, Some(DisplaySpace::of(OutputSpace::Srgb, 2).unwrap()), pro)).image);
    let on_wide = blue(&render(&src, &info, &s, &req(OutputSpace::Srgb, Some(DisplaySpace::of(OutputSpace::Rec2020, 3).unwrap()), pro)).image);
    assert!(on_srgb > 0 && on_wide < on_srgb, "{on_srgb} {on_wide}");
    let plain = blue(&render(&src, &info, &s, &req(OutputSpace::Srgb, None, pro)).image);
    assert_eq!(plain, on_srgb, "an sRGB display warns like no display profile");
}

/// Proofing against an output-device table: the table's colours are what the render shows, and its
/// gamut distances drive the destination warning.
#[test]
fn soft_proof_through_a_device_table() {
    use crate::{OutputSpace, Proof, ProofLut, register_proof_profile};
    let n = 5;
    // a "paper" that dims everything to 80 % and calls every non-grey colour out of gamut
    let data: Vec<[f32; 4]> = (0..n * n * n)
        .map(|i| {
            let c = [i / (n * n), (i / n) % n, i % n].map(|k| crate::color::transfer::srgb_to_linear(k as f32 / (n - 1) as f32));
            let grey = c[0] == c[1] && c[1] == c[2];
            [c[0] * 0.8, c[1] * 0.8, c[2] * 0.8, if grey { 0.0 } else { 1.0 }]
        })
        .collect();
    assert!(ProofLut::new(n, data[1..].to_vec()).is_none());
    assert!(ProofLut::new(n, vec![[f32::NAN; 4]; n * n * n]).is_none());
    let id = register_proof_profile(0xfeed, ProofLut::new(n, data).unwrap());
    let info = SourceInfo { raw: true, ..Default::default() };
    let white = Rgb32f::from_fn(16, 4, |_, _| [100.0; 3]);
    let req = |proof| RenderRequest { space: OutputSpace::Srgb, proof, ..RenderRequest::fit(16, 4) };
    let plain = render(&white, &info, &DevelopSettings::default(), &req(None)).image;
    let pr = Proof { profile: id, dest_warning: true, ..Default::default() };
    let paper = render(&white, &info, &DevelopSettings::default(), &req(Some(pr))).image;
    assert!(plain.data[0][0] > 250 && paper.data[0][0] < 240 && paper.data[0][0] > 200, "{:?} {:?}", plain.data[0], paper.data[0]);
    // an unknown id falls back to the space proof (sRGB on sRGB: no change)
    let none = render(&white, &info, &DevelopSettings::default(), &req(Some(Proof { profile: 12345, ..Default::default() }))).image;
    assert_eq!(none, plain);
    // saturated colours are flagged
    let src = scene();
    let mut s = DevelopSettings::default();
    s.color.saturation = 100.0;
    let warned = render(&src, &info, &s, &req(Some(pr))).image;
    assert!(warned.data.iter().any(|p| p[..3] == [255, 0, 0]));
}

#[test]
fn display_space_rejects_degenerate_matrices() {
    use crate::DisplaySpace;
    assert!(DisplaySpace::new(crate::color::Mat3([[1.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]]), 1).is_none());
    assert!(DisplaySpace::new(crate::color::Mat3([[f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]), 1).is_none());
    assert!(DisplaySpace::new(crate::color::Mat3::IDENTITY, 1).is_some());
}

#[test]
fn tint_negative_is_green_and_positive_is_magenta() {
    use craft_develop_model::WbMode;
    let src = Rgb32f::filled(16, 16, [0.18; 3]);
    // Rendered images, uncalibrated RAW and calibrated RAW with a nonzero As Shot tint.
    for info in [
        SourceInfo::default(),
        SourceInfo { raw: true, relative_wb: true, ..Default::default() },
        SourceInfo { raw: true, as_shot_temp: 4200.0, as_shot_tint: 15.0, ..Default::default() },
        // camera-space white balance: the same sign through the camera's own colour model
        SourceInfo { raw: true, as_shot_temp: 4200.0, as_shot_tint: 15.0, camera_color: Some(camera_color(4200.0, 15.0)), ..Default::default() },
    ] {
        let mut s = DevelopSettings::default();
        let neutral = render(&src, &info, &s, &RenderRequest::fit(16, 16)).image.get(8, 8);
        assert!((neutral[0] as i16 - neutral[1] as i16).abs() <= 1);
        assert!((neutral[2] as i16 - neutral[1] as i16).abs() <= 1);
        s.wb.mode = WbMode::Custom;
        s.wb.temp = info.as_shot_temp;
        for delta in [-50.0, 50.0] {
            s.wb.tint = info.as_shot_tint + delta;
            let p = render(&src, &info, &s, &RenderRequest::fit(16, 16)).image.get(8, 8);
            let magenta = (p[0] as f64 + p[2] as f64) / 2.0 - p[1] as f64;
            assert!(magenta * delta > 100.0, "delta {delta} must follow the green/magenta track, got {p:?}");
        }
    }
}

/// Settings written on the relative scale (6500 K / 0 = as shot, no `WbScale`) for a raw whose camera matrices now
/// give it a Kelvin scale render exactly as they did: the same white-balance matrix, and the same pixels, as the
/// relative render; the Kelvin / tint they convert to renders the same again (issue storytold/lightcraft#730, the class of bug of storytold/lightcraft#510).
#[test]
fn legacy_relative_white_balance_renders_as_before_on_a_kelvin_raw() {
    use crate::local::{effective_wb, legacy_to_kelvin, wb_matrix_for};
    use craft_develop_model::{WbMode, WbScale};
    let old_info = SourceInfo { raw: true, relative_wb: true, as_shot_temp: 6500.0, as_shot_tint: 0.0, ..Default::default() };
    let src = Rgb32f::filled(16, 16, [0.3, 0.18, 0.09]);
    // (as-shot whites the ILCE-7M3 table of issue storytold/lightcraft#730 reads, and the relative slider's whole reach: −100..+100 is
    // 2000..13550 K around 6500 K; a white bluer than 50000 K after the composition has no Kelvin and clamps, so
    // the blue end is paired with as-shot whites up to 7000 K only)
    for (as_t, as_tint) in [(4986.0, -2.4), (2707.0, -4.0), (9338.0, 8.9), (6500.0, 0.0), (3157.0, 40.0), (7000.0, -20.0)] {
        let new_info =
            SourceInfo { raw: true, relative_wb: false, legacy_relative_wb: true, as_shot_temp: as_t, as_shot_tint: as_tint, ..Default::default() };
        let blue_end = as_t <= 7000.0;
        for (temp, tint) in [(6500.0, 0.0), (5800.0, 3.0), (7682.0, 10.0), (4000.0, -30.0), (2000.0, 150.0), (12000.0, 55.0), (13550.0, -150.0)] {
            if temp > 10000.0 && !blue_end {
                continue;
            }
            let mut s = DevelopSettings::default();
            (s.wb.mode, s.wb.temp, s.wb.tint) = (WbMode::Custom, temp, tint);
            assert_eq!(s.wb.scale, WbScale::Legacy);
            let old = wb_matrix_for(&old_info, &s);
            let new = wb_matrix_for(&new_info, &s);
            let near = |a: Option<[[f32; 3]; 3]>, b: Option<[[f32; 3]; 3]>, tol: f32| match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() <= tol * (1.0 + x.abs())),
                _ => false,
            };
            // the same matrix to f32 precision: the white is composed, not read back through Kelvin / tint
            assert!(near(old, new, 2e-6), "as shot {as_t}/{as_tint}, {temp}/{tint}: {old:?} vs {new:?}");
            // the converted Kelvin values, once written, render the same again to the precision of the Kelvin /
            // tint parametrisation (whose seam at 4000 K costs up to 0.01 %)
            let (kt, ktint) = legacy_to_kelvin(temp, tint, (as_t, as_tint));
            assert_eq!(effective_wb(&new_info, &s), (kt, ktint));
            let mut k = s.clone();
            (k.wb.temp, k.wb.tint, k.wb.scale) = (kt, ktint, WbScale::Kelvin);
            assert!(near(old, wb_matrix_for(&new_info, &k), 3e-4), "as shot {as_t}/{as_tint}, {temp}/{tint} → {kt}/{ktint}");
            // and the rendered pixels agree
            let a = render(&src, &old_info, &s, &RenderRequest::fit(16, 16)).image;
            let b = render(&src, &new_info, &s, &RenderRequest::fit(16, 16)).image;
            let c = render(&src, &new_info, &k, &RenderRequest::fit(16, 16)).image;
            for (x, (y, z)) in a.data.iter().zip(b.data.iter().zip(&c.data)) {
                assert_eq!(x, y, "as shot {as_t}/{as_tint}, {temp}/{tint}");
                assert!(x.iter().zip(z).all(|(p, q)| p.abs_diff(*q) <= 1), "{x:?} {z:?}");
            }
        }
        // as shot on the relative scale is as shot on the Kelvin scale, exactly
        assert_eq!(legacy_to_kelvin(6500.0, 0.0, (as_t, as_tint)), (as_t, as_tint));
    }
    // a preset stored on the relative scale keeps its old look, not the preset's Kelvin value
    let new_info = SourceInfo { raw: true, legacy_relative_wb: true, as_shot_temp: 4986.0, as_shot_tint: -2.4, ..Default::default() };
    let mut s = DevelopSettings::default();
    (s.wb.mode, s.wb.temp, s.wb.tint) = (WbMode::Cloudy, 6500.0 * 6500.0 / 5500.0, 10.0);
    assert_eq!(effective_wb(&new_info, &s), legacy_to_kelvin(s.wb.temp, s.wb.tint, (4986.0, -2.4)));
    s.wb.scale = WbScale::Kelvin;
    assert_eq!(effective_wb(&new_info, &s), WbMode::Cloudy.preset().unwrap());
    // a raw whose file carries the matrices (DNG) never had a relative scale: its old values are Kelvin
    let dng = SourceInfo { raw: true, as_shot_temp: 5100.0, as_shot_tint: 4.0, ..Default::default() };
    s.wb.scale = WbScale::Legacy;
    (s.wb.mode, s.wb.temp, s.wb.tint) = (WbMode::Custom, 3200.0, 10.0);
    assert_eq!(effective_wb(&dng, &s), (3200.0, 10.0));
    // hostile numbers come back unchanged
    assert!(legacy_to_kelvin(f64::NAN, 0.0, (5000.0, 0.0)).0.is_nan());
    assert_eq!(legacy_to_kelvin(5000.0, f64::INFINITY, (5000.0, 0.0)).1, f64::INFINITY);
    assert_eq!(legacy_to_kelvin(5000.0, 0.0, (f64::NAN, 0.0)), (5000.0, 0.0));
    assert_eq!(legacy_to_kelvin(6500.0, 0.0, (f64::NAN, 0.0)), (6500.0, 0.0));
    assert_eq!(crate::local::legacy_white(f64::NAN, 0.0, (5000.0, 0.0)), None);
    let hostile = SourceInfo { raw: true, legacy_relative_wb: true, as_shot_temp: f64::NAN, as_shot_tint: 0.0, ..Default::default() };
    let _ = render(&Rgb32f::filled(4, 4, [0.2; 3]), &hostile, &s, &RenderRequest::fit(4, 4));
}

/// Before an edit, the settings' white balance is brought onto the source's scale: Kelvin raws mark it (converting
/// relative values once, into a Custom value with the same look), everything else clears it.
#[test]
fn normalize_wb_marks_kelvin_raws_and_converts_relative_values_once() {
    use crate::local::{legacy_to_kelvin, normalize_wb};
    use craft_develop_model::{WbMode, WbScale};
    let kelvin = SourceInfo { raw: true, legacy_relative_wb: true, as_shot_temp: 4986.0, as_shot_tint: -2.4, ..Default::default() };
    // as shot: marked, values untouched
    let mut s = DevelopSettings::default();
    normalize_wb(&mut s, &kelvin);
    assert_eq!((s.wb.mode, s.wb.temp, s.wb.tint, s.wb.scale), (WbMode::AsShot, 6500.0, 0.0, WbScale::Kelvin));
    // a relative custom value: converted, Custom, marked; a second pass changes nothing
    let mut s = DevelopSettings::default();
    (s.wb.mode, s.wb.temp, s.wb.tint) = (WbMode::Cloudy, 7682.0, 10.0);
    normalize_wb(&mut s, &kelvin);
    let (t, tint) = legacy_to_kelvin(7682.0, 10.0, (4986.0, -2.4));
    assert_eq!((s.wb.mode, s.wb.temp, s.wb.tint, s.wb.scale), (WbMode::Custom, t, tint, WbScale::Kelvin));
    let again = s.clone();
    normalize_wb(&mut s, &kelvin);
    assert_eq!(s, again);
    // a DNG's old values are Kelvin already: marked, untouched
    let dng = SourceInfo { raw: true, as_shot_temp: 5100.0, as_shot_tint: 4.0, ..Default::default() };
    let mut s = DevelopSettings::default();
    (s.wb.mode, s.wb.temp, s.wb.tint) = (WbMode::Custom, 3200.0, 10.0);
    normalize_wb(&mut s, &dng);
    assert_eq!((s.wb.mode, s.wb.temp, s.wb.tint, s.wb.scale), (WbMode::Custom, 3200.0, 10.0, WbScale::Kelvin));
    // a rendered photo and a raw without matrices: the scale is cleared
    for info in [SourceInfo::default(), SourceInfo { raw: true, relative_wb: true, ..Default::default() }] {
        let mut s = DevelopSettings::default();
        (s.wb.mode, s.wb.temp, s.wb.scale) = (WbMode::Custom, 5800.0, WbScale::Kelvin);
        normalize_wb(&mut s, &info);
        assert_eq!((s.wb.mode, s.wb.temp, s.wb.scale), (WbMode::Custom, 5800.0, WbScale::Legacy));
    }
}

/// A dual-illuminant (A / D65) camera colour model, its pixels developed for `temp` / `tint`.
fn camera_color(temp: f64, tint: f64) -> std::sync::Arc<crate::CameraColor> {
    use crate::color::Mat3;
    let tags = crate::sensor::ColorData {
        illuminant: [17, 21],
        color_matrix: [
            Some((Mat3([[0.9, 0.2, -0.15], [-0.3, 1.25, 0.08], [0.02, -0.12, 0.85]])).0),
            Some((Mat3([[0.7, 0.3, -0.1], [-0.35, 1.3, 0.1], [0.05, -0.2, 1.0]])).0),
        ],
        ..Default::default()
    };
    std::sync::Arc::new(crate::CameraColor { tags, developed_for: crate::color::cct::temp_tint_to_xy(temp, tint) })
}

/// Camera-space white balance: a raw source with its colour model is re-developed for the chosen
/// white, so a grey card lit by that white renders neutral. Without the model, or with a relative
/// white balance, the developed pixels are adapted (Bradford, neutral luminance kept).
#[test]
fn custom_white_balance_redevelops_in_camera_space() {
    use crate::color::cct::temp_tint_to_xy;
    use crate::local::wb_matrix_for;
    use craft_develop_model::WbMode;
    let cc = camera_color(5500.0, 0.0);
    let info = SourceInfo { raw: true, as_shot_temp: 5500.0, as_shot_tint: 0.0, camera_color: Some(cc.clone()), ..Default::default() };
    let mut s = DevelopSettings::default();
    (s.wb.mode, s.wb.temp, s.wb.tint) = (WbMode::Custom, 3200.0, 10.0);
    // a grey card lit by 3200 K / +10, as the camera records it, developed for the as-shot white
    let a = crate::sensor::color::camera_transform_of(&cc.tags, cc.developed_for);
    let n = crate::sensor::color::camera_neutral(&cc.tags, temp_tint_to_xy(3200.0, 10.0));
    let card = a.matrix.apply(std::array::from_fn(|i| n[i] * a.wb[i] as f64)).map(|v| v as f32);
    let apply = |m: [[f32; 3]; 3], c: [f32; 3]| -> [f32; 3] { std::array::from_fn(|r| m[r][0] * c[0] + m[r][1] * c[1] + m[r][2] * c[2]) };
    let m = wb_matrix_for(&info, &s).unwrap();
    let out = apply(m, card);
    assert!(out.iter().all(|v| (v / out[1] - 1.0).abs() < 1e-4), "{card:?} -> {out:?}");
    // the adaptation path for sources without a colour model, and for relative white balance
    let luma = |c: [f32; 3]| crate::color::luminance_2020(c);
    for other in [SourceInfo { camera_color: None, ..info.clone() }, SourceInfo { relative_wb: true, ..info.clone() }] {
        let b = wb_matrix_for(&other, &s).unwrap();
        assert_ne!(b, m);
        assert!((luma(apply(b, [1.0; 3])) - 1.0).abs() < 1e-4);
    }
    // the white-balance picker / auto white balance invert the same model: the card names its white
    let picked = crate::auto::auto_wb(&Rgb32f::filled(4, 4, card), &info);
    assert_eq!(picked, (3200.0, 10.0));
    // the as-shot white needs nothing
    s.wb.mode = WbMode::AsShot;
    assert!(wb_matrix_for(&info, &s).is_none());
}

/// Every stored process number renders with a process this build knows: settings saved before
/// process versions existed exactly as V1, and a number from a newer LightCraft exactly as the
/// latest process here (`docs/process-versions.md`). Raw (base tone curve), camera-tone and
/// rendered sources, plain and edited, 8-bit and 16-bit.
#[test]
fn stored_process_numbers_render_with_a_known_process() {
    use craft_develop_model::{Process, ProcessVersion};
    let src = scene();
    let curve = crate::tone::CameraTone::new(std::array::from_fn(|i| {
        let x = 0.004 * 1.18f32.powi(i as i32);
        [x, 1.0 - (-2.0 * x).exp()]
    }))
    .unwrap();
    let infos = [
        SourceInfo::default(),
        SourceInfo { raw: true, as_shot_temp: 5200.0, as_shot_tint: 4.0, ..Default::default() },
        SourceInfo { raw: true, relative_wb: true, camera_tone: Some(curve), ..Default::default() },
    ];
    let mut edited = DevelopSettings::for_raw(5200.0, 4.0);
    for (id, v) in [("light.exposure", 0.4), ("light.contrast", 30.0), ("light.highlights", -60.0), ("light.whites", 15.0), ("light.blacks", -20.0)] {
        controls::set(&mut edited, id, v);
    }
    let legacy = |s: &DevelopSettings| {
        let mut v = s.to_json();
        v.as_object_mut().unwrap().remove("process");
        DevelopSettings::from_json(&v).unwrap()
    };
    let with = |s: &DevelopSettings, p: ProcessVersion| DevelopSettings { process: p, ..s.clone() };
    let deep = RenderRequest { depth: crate::OutputDepth::U16, ..RenderRequest::fit(96, 96) };
    for info in &infos {
        for s in [DevelopSettings::default(), edited.clone()] {
            for req in [RenderRequest::fit(96, 96), deep] {
                let out = |s: &DevelopSettings| {
                    let r = render(&src, info, s, &req);
                    (r.image.data, r.deep)
                };
                let v1 = out(&with(&s, ProcessVersion::V1));
                assert_eq!(legacy(&s).process, ProcessVersion::V1);
                assert!(out(&legacy(&s)) == v1, "saved before process versions: rendered as V1");
                let latest = out(&with(&s, Process::LATEST.version()));
                assert!(out(&with(&s, ProcessVersion(Process::LATEST.version().0 + 7))) == latest, "newer than this build: as the latest");
            }
        }
    }
}

/// A crop a hair wide (or of no size, as a command or a sidecar can set) shown to fit made the
/// tone, clarity and dehaze stages blur with sigmas of millions of pixels (infinite for no size):
/// the box width's square overflowed (a panic in debug builds, a radius of billions of pixels and
/// a hung render in release ones). The blur is held to twice the picture's size: the render is
/// quick and its pixels are numbers.
#[test]
fn sliver_crops_render_quickly_with_wide_stages_on() {
    let src = scene();
    for (rect, angle) in [([0.0, 0.0, 1e-9, 1.0], 45.0), ([0.5, 0.5, 0.5, 0.5], 0.0), ([0.0, 0.0, 1e-5, 1.0], -30.0)] {
        let mut s = DevelopSettings::default();
        s.crop.geometry.rect = craft_geom::Rect::new(rect[0], rect[1], rect[2], rect[3]);
        s.crop.geometry.angle = angle;
        s.effects.clarity = 80.0;
        s.effects.dehaze = 50.0;
        s.effects.texture = 40.0;
        s.light.highlights = -60.0;
        s.light.shadows = 60.0;
        let t = std::time::Instant::now();
        let r = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(280, 180));
        assert!(t.elapsed().as_secs_f64() < 20.0, "{rect:?}: {:?}", t.elapsed());
        assert!(r.image.width >= 1 && r.image.height >= 1, "{rect:?}");
    }
}

// Given a cached render with one process, a change to a process with another colour filter (Update
// to Current Process, undo) renders like a fresh render, not the cached denoised image
#[test]
fn a_process_change_does_not_reuse_the_other_colour_filter() {
    use crate::{StageCache, render_cached};
    use craft_develop_model::Process;
    use std::sync::Arc;
    let src = Arc::new(colour_noise_scene(128, 96, 128));
    let info = SourceInfo::default();
    let req = RenderRequest::fit(128, 96);
    let cache = StageCache::default();
    let mut old = DevelopSettings::default();
    old.detail.nr_color = 25.0;
    old.detail.nr_color_smoothness = 0.0;
    old.process = Process::V2.version();
    let current = DevelopSettings { process: Process::LATEST.version(), ..old.clone() };
    let first = render_cached(&src, &info, &old, &req, &cache).image;
    let fresh = render(&src, &info, &current, &req).image;
    assert!(max_diff(&first, &fresh) > 0, "the two colour filters differ on this scene");
    assert_eq!(render_cached(&src, &info, &current, &req, &cache).image, fresh, "process update");
    assert_eq!(render_cached(&src, &info, &old, &req, &cache).image, first, "undo");
}

/// Flat grey with chroma noise: `quiet` columns on the left with little noise, the rest with much.
pub(crate) fn colour_noise_scene(w: usize, h: usize, quiet: usize) -> Rgb32f {
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        ((seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5) * 2.0
    };
    let mut img = Rgb32f::new(w, h);
    for (i, p) in img.data.iter_mut().enumerate() {
        let sigma = if i % w < quiet { 0.001 } else { 0.015 };
        let y = 0.04f32;
        let (r, b) = (y + noise() * sigma, y + noise() * sigma);
        *p = [r, (y - 0.2627 * r - 0.0593 * b) / 0.6780, b];
    }
    img
}

/// A crop of no width used to give grain NaN coordinates (and NaN pixels); the render stays finite.
#[test]
fn zero_width_crop_with_grain_renders_finite_pixels() {
    let mut s = DevelopSettings::default();
    s.grain.amount = 60.0;
    s.crop.geometry.rect = craft_geom::Rect::new(0.5, 0.1, 0.5, 0.9);
    let mut req = RenderRequest::fit(240, 240);
    req.depth = crate::OutputDepth::F32Linear;
    let r = render(&scene(), &SourceInfo::default(), &s, &req);
    if let Some(d) = r.deep
        && let crate::DeepSamples::F32(v) = d.samples
    {
        assert!(v.iter().all(|x| x.is_finite()), "non-finite sample in a zero-width crop with grain");
    }
}

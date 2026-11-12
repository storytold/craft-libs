//! Geometry / optics tests on synthetic images with known distortions.

use crate::raster::resample::{Filter, resize};
use crate::raster::{Rgb32f, Rgba8};
use craft_develop_model::{DevelopSettings, EmbeddedLens, EmbeddedVignette, EmbeddedWarp};
use craft_geom::Point;

use crate::{RenderRequest, SourceInfo, render};

/// A light grid: dark lines every `step` px (soft, ~3 px wide) on a light background.
pub(crate) fn grid_value(p: Point, step: f64) -> f32 {
    let d = |v: f64| {
        let m = v.rem_euclid(step);
        m.min(step - m)
    };
    let dist = d(p.x).min(d(p.y));
    (0.05 + 0.75 * ((dist - 1.0) / 1.5).clamp(0.0, 1.0)) as f32
}

/// Render a source of `w × h` where each pixel shows `pattern` at `undistort(pixel centre)`.
pub(crate) fn synth(w: usize, h: usize, undistort: impl Fn(Point) -> Point + Sync, pattern: impl Fn(Point) -> [f32; 3] + Sync) -> Rgb32f {
    Rgb32f::from_fn(w, h, |x, y| pattern(undistort(Point::new(x as f64 + 0.5, y as f64 + 0.5))))
}

/// Sub-pixel x of the dark vertical line nearest `x0` on row `y` (green channel darkness centroid).
pub(crate) fn line_x(img: &Rgba8, y: usize, x0: f64, win: f64) -> f64 {
    let (a, b) = ((x0 - win).max(0.0) as usize, ((x0 + win) as usize).min(img.width - 1));
    let row: Vec<f64> = (a..=b).map(|x| img.get(x, y)[1] as f64).collect();
    let max = row.iter().cloned().fold(0.0, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    if max - min <= 1.0 {
        return (a as f64 + b as f64) / 2.0 + 0.5;
    }
    let (mut s, mut sw) = (0.0, 0.0);
    for (i, v) in row.iter().enumerate() {
        let wgt = (max - v).max(0.0).powi(2);
        s += wgt * (a + i) as f64;
        sw += wgt;
    }
    s / sw.max(1e-9) + 0.5
}

/// Sub-pixel y of the dark horizontal line nearest `y0` in column `x`.
pub(crate) fn line_y(img: &Rgba8, x: usize, y0: f64, win: f64) -> f64 {
    let (a, b) = ((y0 - win).max(0.0) as usize, ((y0 + win) as usize).min(img.height - 1));
    let col: Vec<f64> = (a..=b).map(|y| img.get(x, y)[1] as f64).collect();
    let max = col.iter().cloned().fold(0.0, f64::max);
    let min = col.iter().cloned().fold(f64::MAX, f64::min);
    if max - min <= 1.0 {
        return (a as f64 + b as f64) / 2.0 + 0.5;
    }
    let (mut s, mut sw) = (0.0, 0.0);
    for (i, v) in col.iter().enumerate() {
        let wgt = (max - v).max(0.0).powi(2);
        s += wgt * (a + i) as f64;
        sw += wgt;
    }
    s / sw.max(1e-9) + 0.5
}

fn spread(v: &[f64]) -> f64 {
    v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
}

/// Asserts that a vertical edge with contrast > 1 LSB exists in row `y` around `x0 ± win`.
fn assert_vertical_edge(img: &Rgba8, y: usize, x0: f64, win: f64) {
    let (a, b) = ((x0 - win).max(0.0) as usize, ((x0 + win) as usize).min(img.width - 1));
    let values: Vec<f64> = (a..=b).map(|x| img.get(x, y)[1] as f64).collect();
    assert!(spread(&values) > 1.0, "no vertical edge near x={x0} on row {y}");
}

/// Asserts that a horizontal edge with contrast > 1 LSB exists in column `x` around `y0 ± win`.
fn assert_horizontal_edge(img: &Rgba8, x: usize, y0: f64, win: f64) {
    let (a, b) = ((y0 - win).max(0.0) as usize, ((y0 + win) as usize).min(img.height - 1));
    let values: Vec<f64> = (a..=b).map(|y| img.get(x, y)[1] as f64).collect();
    assert!(spread(&values) > 1.0, "no horizontal edge near y={y0} on column {x}");
}

/// Barrel distortion with coefficient `k` (< 0): distorted radius = r·(1 + k·r²), r in half-diagonal units.
fn barrel_undistort(w: f64, h: f64, k: f64) -> impl Fn(Point) -> Point + Sync {
    let (cx, cy, hd) = (w / 2.0, h / 2.0, w.hypot(h) / 2.0);
    move |p: Point| {
        let (dx, dy) = ((p.x - cx) / hd, (p.y - cy) / hd);
        let rd = dx.hypot(dy);
        if rd < 1e-12 {
            return p;
        }
        // solve r·(1 + k r²) = rd (Newton)
        let mut r = rd;
        for _ in 0..20 {
            let f = r * (1.0 + k * r * r) - rd;
            r -= f / (1.0 + 3.0 * k * r * r);
        }
        let s = r / rd;
        Point::new(cx + dx * s * hd, cy + dy * s * hd)
    }
}

#[test]
fn manual_distortion_straightens_barrel_grid() {
    let (w, h) = (480usize, 320usize);
    // the Distortion slider at +60 corrects exactly this barrel
    let k = -0.2 * 0.6;
    let src = synth(w, h, barrel_undistort(w as f64, h as f64, k), |p| [grid_value(p, 40.0); 3]);
    let rows = [12usize, 150, 308];
    let measure = |s: &DevelopSettings| {
        let img = render(&src, &SourceInfo::default(), s, &RenderRequest::fit(w, h)).image;
        let xs: Vec<f64> = rows.iter().map(|&y| line_x(&img, y, 40.0, 15.0)).collect();
        let ys: Vec<f64> = [12usize, 220, 468].iter().map(|&x| line_y(&img, x, 40.0, 15.0)).collect();
        (spread(&xs), spread(&ys))
    };
    let (bx, by) = measure(&DevelopSettings::default());
    assert!(bx > 4.0 && by > 3.0, "the synthetic barrel should bend lines: {bx} {by}");
    let mut s = DevelopSettings::default();
    s.optics.distortion = 60.0;
    let (cx, cy) = measure(&s);
    assert!(cx < 1.0 && cy < 1.0, "corrected lines should be straight: {cx} {cy}");
}

/// CA: red plane magnified by `ar`, blue by `ab`, on a neutral disc grid.
fn ca_source(w: usize, h: usize, ar: f64, ab: f64) -> Rgb32f {
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let disc = move |p: Point| -> f32 {
        let cell = 40.0;
        let (fx, fy) = ((p.x / cell).rem_euclid(1.0) - 0.5, (p.y / cell).rem_euclid(1.0) - 0.5);
        let d = fx.hypot(fy) * cell - 11.0;
        (0.04 + 0.8 * (d / 1.2 + 0.5).clamp(0.0, 1.0)) as f32
    };
    Rgb32f::from_fn(w, h, |x, y| {
        let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
        let at = |a: f64| disc(Point::new(cx + (p.x - cx) / (1.0 + a), cy + (p.y - cy) / (1.0 + a)));
        [at(ar), at(0.0), at(ab)]
    })
}

fn fringe(img: &Rgba8) -> f64 {
    img.data.iter().map(|p| (p[0] as f64 - p[1] as f64).abs() + (p[2] as f64 - p[1] as f64).abs()).sum::<f64>() / img.len() as f64
}

#[test]
fn remove_ca_aligns_colour_planes() {
    let (w, h) = (640usize, 420usize);
    let src = ca_source(w, h, 0.005, -0.004);
    let before = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(w, h)).image;
    let mut s = DevelopSettings::default();
    s.optics.remove_ca = true;
    let after = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(w, h)).image;
    let (fb, fa) = (fringe(&before), fringe(&after));
    assert!(fa < fb * 0.35, "fringing {fb:.2} → {fa:.2}");
    // manual sliders can do the same: +ca_red shrinks... the red plane is sampled further out
    let mut m = DevelopSettings::default();
    m.optics.ca_red = 0.005 / 0.003 * 100.0;
    m.optics.ca_blue = -0.004 / 0.003 * 100.0;
    let manual = render(&src, &SourceInfo::default(), &m, &RenderRequest::fit(w, h)).image;
    assert!(fringe(&manual) < fb * 0.35, "manual {:.2}", fringe(&manual));
}

#[test]
fn profile_corrections_use_embedded_lens_only_when_enabled() {
    let (w, h) = (300usize, 200usize);
    let src = synth(w, h, |p| p, |p| [grid_value(p, 30.0); 3]);
    let lens = EmbeddedLens {
        warp: Some(EmbeddedWarp { planes: [[1.0, -0.08, 0.0, 0.0, 0.0, 0.0]; 3], center: Point::new(0.5, 0.5), radius: 0.6 }),
        vignette: Some(EmbeddedVignette { k: [0.5, 0.0, 0.0, 0.0, 0.0], center: Point::new(0.5, 0.5), radius: 0.6 }),
        ..Default::default()
    };
    let info = SourceInfo { lens: Some(lens), ..SourceInfo::default() };
    let mut s = DevelopSettings::default();
    let off = render(&src, &info, &s, &RenderRequest::fit(w, h)).image;
    let plain = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(w, h)).image;
    assert_eq!(off.data, plain.data, "profile corrections off: embedded data ignored");
    s.optics.lens_profile = true;
    let on = render(&src, &info, &s, &RenderRequest::fit(w, h)).image;
    assert_ne!(on.data, off.data);
    // corners brightened by the embedded vignette correction
    assert!(on.get(2, 2)[1] as i32 >= off.get(2, 2)[1] as i32);
    s.optics.profile_distortion = 0.0;
    s.optics.profile_vignetting = 0.0;
    let zero = render(&src, &info, &s, &RenderRequest::fit(w, h)).image;
    assert_eq!(zero.data, off.data, "0 % = no correction");
}

fn mean_abs_diff_downscaled(small: &Rgba8, big: &Rgba8) -> f32 {
    let down = resize(&big.to_linear(), small.width, small.height, Filter::Box).to_srgb8();
    small.data.iter().zip(&down.data).map(|(a, b)| (0..3).map(|i| (a[i] as f32 - b[i] as f32).abs()).sum::<f32>() / 3.0).sum::<f32>()
        / small.len() as f32
}

#[test]
fn optics_preview_matches_export() {
    let src = crate::scenes::demo_library()[2].render(960, 640);
    let mut s = DevelopSettings::default();
    s.optics.distortion = 40.0;
    s.optics.vignetting = 60.0;
    s.optics.vignetting_midpoint = 30.0;
    s.optics.ca_red = 30.0;
    s.optics.defringe_purple_amount = 8.0;
    let small = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(240, 240)).image;
    let big = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(960, 960)).image;
    let err = mean_abs_diff_downscaled(&small, &big);
    assert!(err < 4.0, "mean abs error {err}");
}

// ------------------------------------------------------------------------------------------ M4.3 transforms

/// Source whose content is a straight grid after the forward transform `h` (centred coordinates of a `w × h`
/// image): each source pixel shows the grid at its transformed position.
fn keystoned_grid(w: usize, h: usize, fwd: craft_geom::Homography, step: f64) -> Rgb32f {
    let l = w.max(h) as f64 / 2.0;
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    synth(
        w,
        h,
        move |p| {
            let q = fwd.apply(Point::new((p.x - cx) / l, (p.y - cy) / l));
            Point::new(cx + q.x * l, cy + q.y * l)
        },
        move |p| [grid_value(p, step); 3],
    )
}

#[test]
fn manual_vertical_keystone_widens_the_top() {
    let g = craft_develop_model::Geometry { vertical: -50.0, ..Default::default() };
    let hm = crate::transform::manual(&g);
    let (tl, tr) = (hm.apply(Point::new(-1.0, -0.66)), hm.apply(Point::new(1.0, -0.66)));
    let (bl, br) = (hm.apply(Point::new(-1.0, 0.66)), hm.apply(Point::new(1.0, 0.66)));
    assert!(tr.x - tl.x > (br.x - bl.x) * 1.15, "top {} bottom {}", tr.x - tl.x, br.x - bl.x);
    let c = hm.apply(Point::new(0.0, 0.0));
    assert!(c.dist(Point::new(0.0, 0.0)) < 1e-9, "centre stays put");
    let g = craft_develop_model::Geometry { horizontal: -50.0, ..Default::default() };
    let hm = crate::transform::manual(&g);
    let (lt, lb) = (hm.apply(Point::new(-0.9, -0.6)), hm.apply(Point::new(-0.9, 0.6)));
    let (rt, rb) = (hm.apply(Point::new(0.9, -0.6)), hm.apply(Point::new(0.9, 0.6)));
    assert!(lb.y - lt.y > rb.y - rt.y, "left side enlarged");
}

/// Verifies manual geometry corrections straighten a keystoned grid test fixture.
#[test]
fn manual_transforms_straighten_known_keystone() {
    let (w, h) = (480usize, 320usize);
    for g in [
        craft_develop_model::Geometry { vertical: -40.0, ..Default::default() },
        craft_develop_model::Geometry { horizontal: 35.0, rotate: 3.0, ..Default::default() },
        craft_develop_model::Geometry { vertical: 25.0, aspect: 30.0, scale: 110.0, offset_x: 6.0, offset_y: -4.0, ..Default::default() },
    ] {
        // the source was "shot" with the inverse transform; correcting with `g` must give the straight grid back
        let fwd = crate::transform::manual(&g);
        let src = keystoned_grid(w, h, fwd, 40.0);
        let s = DevelopSettings { geometry: g.clone(), ..Default::default() };
        let img = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(w, h)).image;
        for &y in &[60usize, 150, 260] {
            assert_vertical_edge(&img, y, 200.0, 15.0);
        }
        for &x in &[60usize, 220, 420] {
            assert_horizontal_edge(&img, x, 200.0, 15.0);
        }
        let xs: Vec<f64> = [60usize, 150, 260].iter().map(|&y| line_x(&img, y, 200.0, 15.0)).collect();
        let ys: Vec<f64> = [60usize, 220, 420].iter().map(|&x| line_y(&img, x, 200.0, 15.0)).collect();
        assert!(spread(&xs) < 1.0 && (xs[0] - 200.0).abs() < 1.0, "{g:?}: vertical line at {xs:?}");
        assert!(spread(&ys) < 1.0 && (ys[0] - 200.0).abs() < 1.0, "{g:?}: horizontal line at {ys:?}");
        // and without the correction the lines are not straight/where they belong
        let raw = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(w, h)).image;
        let rx: Vec<f64> = [60usize, 150, 260].iter().map(|&y| line_x(&raw, y, 200.0, 15.0)).collect();
        let ry: Vec<f64> = [60usize, 220, 420].iter().map(|&x| line_y(&raw, x, 200.0, 15.0)).collect();
        assert!(spread(&rx) > 1.5 || spread(&ry) > 1.5 || (rx[0] - 200.0).abs() > 1.5, "{g:?}: source should be distorted");
    }
}

fn blank_fraction(img: &Rgba8) -> f64 {
    let blank =
        crate::render(&Rgb32f::filled(4, 4, crate::geometry::BLANK), &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(4, 4))
            .image
            .get(1, 1);
    img.data.iter().filter(|p| (0..3).all(|i| (p[i] as i32 - blank[i] as i32).abs() <= 1)).count() as f64 / img.len() as f64
}

#[test]
fn constrain_crop_removes_blank_areas() {
    let src = Rgb32f::from_fn(300, 200, |x, y| [0.05 + 0.002 * x as f32, 0.1 + 0.002 * y as f32, 0.3]);
    let mut s = DevelopSettings::default();
    s.geometry.vertical = -60.0;
    s.geometry.rotate = 4.0;
    let free = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(300, 300)).image;
    assert!(blank_fraction(&free) > 0.03, "keystone leaves blank corners: {}", blank_fraction(&free));
    s.geometry.constrain_crop = true;
    let c = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(300, 300)).image;
    assert_eq!((c.width, c.height), (free.width, free.height), "constrain keeps the aspect");
    assert!(blank_fraction(&c) < 0.002, "constrained: {}", blank_fraction(&c));
}

/// Issue storytold/lightcraft#2361: lens corrections never show the blank past the photo's edge, with or without
/// Constrain Crop, in every colour plane; perspective alone still needs Constrain Crop.
#[test]
fn lens_corrections_leave_no_blank_corners() {
    let src = Rgb32f::from_fn(300, 200, |x, y| [0.05 + 0.002 * x as f32, 0.1 + 0.002 * y as f32, 0.3]);
    for (distortion, ca) in [(-80.0, 0.0), (80.0, 0.0), (-40.0, 100.0), (0.0, -100.0)] {
        let mut s = DevelopSettings::default();
        s.optics.distortion = distortion;
        s.optics.ca_red = ca;
        s.optics.ca_blue = -ca;
        let r = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(300, 300)).image;
        assert!(blank_fraction(&r) < 0.002, "distortion {distortion}, CA {ca}: {}", blank_fraction(&r));
    }
    let mut s = DevelopSettings::default();
    s.optics.distortion = -60.0;
    s.geometry.vertical = -60.0;
    let r = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(300, 300)).image;
    assert!(blank_fraction(&r) > 0.03, "the keystone keeps its corners: {}", blank_fraction(&r));
}

#[test]
fn transforms_preview_matches_export() {
    let src = crate::scenes::demo_library()[4].render(960, 640);
    let mut s = DevelopSettings::default();
    s.geometry.vertical = -30.0;
    s.geometry.horizontal = 15.0;
    s.geometry.rotate = 2.5;
    s.geometry.scale = 105.0;
    s.geometry.constrain_crop = true;
    let small = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(240, 240)).image;
    let big = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(960, 960)).image;
    assert_eq!(small.width * 4, big.width);
    let err = mean_abs_diff_downscaled(&small, &big);
    assert!(err < 4.0, "mean abs error {err}");
}

// ------------------------------------------------------------------------------------------ M4.4 Upright

use crate::upright::{Segment, detect_segments, solve_guided};
use craft_develop_model::Upright;

#[test]
fn segment_detector_finds_known_lines() {
    // three dark lines at known angles on a light background
    let lines = [
        (Point::new(60.0, 40.0), Point::new(80.0, 280.0)),
        (Point::new(120.0, 300.0), Point::new(420.0, 260.0)),
        (Point::new(300.0, 30.0), Point::new(440.0, 200.0)),
    ];
    let dist = |p: Point, a: Point, b: Point| {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0);
        p.dist(a + ab * t)
    };
    let lum = crate::raster::Plane::from_fn(480, 320, |x, y| {
        let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
        let d = lines.iter().map(|(a, b)| dist(p, *a, *b)).fold(f64::MAX, f64::min);
        (0.15 + 0.7 * ((d - 1.0) / 1.2).clamp(0.0, 1.0)) as f32
    });
    let segs = detect_segments(&lum);
    let l = 240.0;
    for (a, b) in lines {
        let want = (b.y - a.y).atan2(b.x - a.x);
        let len = a.dist(b) / l;
        let hit = segs.iter().any(|s| {
            let got = (s.b.y - s.a.y).atan2(s.b.x - s.a.x);
            let mut d = (got - want).abs() % std::f64::consts::PI;
            if d > std::f64::consts::FRAC_PI_2 {
                d = std::f64::consts::PI - d;
            }
            d.to_degrees() < 0.7 && s.len() > 0.6 * len
        });
        assert!(hit, "line {a:?}–{b:?} not found in {segs:?}");
    }
}

/// x of the same dark vertical line at the top, bottom and middle of the output (tracked from the middle).
fn vertical_line_track(img: &Rgba8, x0: f64) -> Vec<f64> {
    let mid = line_x(img, img.height / 2 + 7, x0, 14.0);
    [img.height / 6 + 7, img.height * 5 / 6 - 7].iter().map(|&y| line_x(img, y, mid, 10.0)).chain([mid]).collect()
}

fn horizontal_line_track(img: &Rgba8, y0: f64) -> Vec<f64> {
    let mid = line_y(img, img.width / 2 + 7, y0, 14.0);
    [img.width / 6 + 7, img.width * 5 / 6 - 7].iter().map(|&x| line_y(img, x, mid, 10.0)).chain([mid]).collect()
}

fn upright_render(src: &Rgb32f, mode: Upright) -> Rgba8 {
    let mut s = DevelopSettings::default();
    s.geometry.upright = mode;
    s.geometry.upright_transform = Some(crate::upright::auto_transform(src, &SourceInfo::default(), &s, mode));
    render(src, &SourceInfo::default(), &s, &RenderRequest::fit(src.width, src.height)).image
}

#[test]
fn upright_vertical_fixes_converging_verticals() {
    let (w, h) = (720usize, 480usize);
    // "shot from below" and slightly rolled: the inverse of what the correction should do
    let shot = craft_develop_model::Geometry { vertical: 35.0, rotate: -2.0, ..Default::default() };
    let src = keystoned_grid(w, h, crate::transform::manual(&shot), 48.0);
    let raw = render(&src, &SourceInfo::default(), &DevelopSettings::default(), &RenderRequest::fit(w, h)).image;
    let before = vertical_line_track(&raw, 130.0);
    assert!(spread(&before) > 6.0, "source verticals converge: {before:?}");
    for mode in [Upright::Vertical, Upright::Auto, Upright::Full] {
        let img = upright_render(&src, mode);
        for x0 in [130.0, 360.0, 590.0] {
            let t = vertical_line_track(&img, x0);
            assert!(spread(&t) < 1.2, "{mode:?}: line near {x0} at {t:?}");
        }
        let t = horizontal_line_track(&img, 240.0);
        assert!(spread(&t) < 1.5, "{mode:?}: horizontals stay level: {t:?}");
    }
}

#[test]
fn upright_level_straightens_a_tilted_horizon() {
    let (w, h) = (720usize, 480usize);
    let shot = craft_develop_model::Geometry { rotate: 3.5, ..Default::default() };
    let src = keystoned_grid(w, h, crate::transform::manual(&shot), 48.0);
    let img = upright_render(&src, Upright::Level);
    for y0 in [120.0, 240.0, 360.0] {
        let t = horizontal_line_track(&img, y0);
        assert!(spread(&t) < 1.2, "line near {y0} at {t:?}");
    }
}

#[test]
fn upright_full_fixes_both_families() {
    let (w, h) = (720usize, 480usize);
    let shot = craft_develop_model::Geometry { vertical: 25.0, horizontal: -25.0, ..Default::default() };
    let src = keystoned_grid(w, h, crate::transform::manual(&shot), 48.0);
    let img = upright_render(&src, Upright::Full);
    for x0 in [200.0, 360.0, 520.0] {
        let t = vertical_line_track(&img, x0);
        assert!(spread(&t) < 1.5, "vertical near {x0}: {t:?}");
    }
    for y0 in [150.0, 330.0] {
        let t = horizontal_line_track(&img, y0);
        assert!(spread(&t) < 1.5, "horizontal near {y0}: {t:?}");
    }
}

#[test]
fn guided_upright_from_two_vertical_guides() {
    let (w, h) = (720usize, 480usize);
    let shot = craft_develop_model::Geometry { vertical: 30.0, ..Default::default() };
    let fwd = crate::transform::manual(&shot);
    let src = keystoned_grid(w, h, fwd, 48.0);
    // guides drawn along two building edges: straight lines of the "true" image, seen through fwd⁻¹
    let inv = fwd.inverse().unwrap();
    let l = 360.0;
    let norm = |p: Point| {
        let q = inv.apply(p);
        Point::new((q.x * l + 360.0) / 720.0, (q.y * l + 240.0) / 480.0)
    };
    let mut s = DevelopSettings::default();
    s.geometry.upright = Upright::Guided;
    s.geometry.guides =
        vec![(norm(Point::new(-0.5, -0.5)), norm(Point::new(-0.5, 0.5))), (norm(Point::new(0.55, -0.4)), norm(Point::new(0.55, 0.5)))];
    let img = render(&src, &SourceInfo::default(), &s, &RenderRequest::fit(w, h)).image;
    for x0 in [180.0, 360.0, 540.0] {
        let t = vertical_line_track(&img, x0);
        assert!(spread(&t) < 1.2, "guided: line near {x0} at {t:?}");
    }
    // a single guide only levels (rotation): it becomes vertical
    let one = solve_guided(&[Segment { a: Point::new(0.1, -0.5), b: Point::new(0.12, 0.5) }]);
    let p = one.apply(Point::new(0.1, -0.5));
    let q = one.apply(Point::new(0.12, 0.5));
    assert!((p.x - q.x).abs() < 1e-9, "single vertical guide becomes vertical: {p:?} {q:?}");
}

#[test]
fn upright_analysis_is_resolution_independent() {
    let shot = craft_develop_model::Geometry { vertical: 30.0, rotate: 1.5, ..Default::default() };
    let fwd = crate::transform::manual(&shot);
    let big = keystoned_grid(1200, 800, fwd, 80.0);
    let small = keystoned_grid(600, 400, fwd, 40.0);
    let s = DevelopSettings::default();
    let a = crate::upright::auto_transform(&big, &SourceInfo::default(), &s, Upright::Vertical);
    let b = crate::upright::auto_transform(&small, &SourceInfo::default(), &s, Upright::Vertical);
    for p in [Point::new(-1.0, -0.66), Point::new(1.0, -0.66), Point::new(1.0, 0.66), Point::new(-1.0, 0.66)] {
        assert!(a.apply(p).dist(b.apply(p)) < 0.006, "{:?} vs {:?}", a.apply(p), b.apply(p));
    }
    // a setting that names the mode without a stored transform renders like the stored one
    let mut s1 = DevelopSettings::default();
    s1.geometry.upright = Upright::Vertical;
    let lazy = render(&small, &SourceInfo::default(), &s1, &RenderRequest::fit(300, 300)).image;
    s1.geometry.upright_transform = Some(b);
    let stored = render(&small, &SourceInfo::default(), &s1, &RenderRequest::fit(300, 300)).image;
    assert_eq!(lazy.data, stored.data);
}

/// Regression: Upright Auto must not invent perspective in natural scenes (conical trees, ridgelines, a level
/// water line) — the demo library has no buildings, so Auto should stay close to identity everywhere.
#[test]
fn upright_auto_leaves_natural_scenes_nearly_alone() {
    use craft_develop_model::Upright;
    for scene in crate::scenes::demo_library() {
        let src = scene.render_fit(768);
        let h = crate::upright::auto_transform(&src, &SourceInfo::default(), &DevelopSettings::default(), Upright::Auto);
        // how far the corners of the centred unit frame move
        let worst = [(-1.0, -0.66), (1.0, -0.66), (-1.0, 0.66), (1.0, 0.66)]
            .iter()
            .map(|&(x, y)| {
                let p = h.apply(craft_geom::Point::new(x, y));
                (p.x - x).hypot(p.y - y)
            })
            .fold(0.0f64, f64::max);
        assert!(worst < 0.06, "{}: Auto moved a corner by {worst:.3} (centred units)", scene.name);
    }
}

/// Diagnostics: `SCENE="Lake" cargo test -p craft-develop debug_upright_scene -- --ignored --nocapture`.
#[test]
#[ignore]
fn debug_upright_scene() {
    let scene = crate::scenes::demo_library().into_iter().find(|s| s.name.starts_with(std::env::var("SCENE").as_deref().unwrap_or("Lake"))).unwrap();
    let src = scene.render_fit(768);
    let a = crate::upright::analyze_source(&src, &SourceInfo::default(), &DevelopSettings::default());
    eprintln!("vert {} horiz {} vvp {:?} hvp {:?}", a.vertical.len(), a.horizontal.len(), a.vertical_vp, a.horizontal_vp);
    for s in a.vertical.iter().take(20) {
        eprintln!("  v {:.3},{:.3} -> {:.3},{:.3} len {:.3}", s.a.x, s.a.y, s.b.x, s.b.y, s.len());
    }
    for m in [Upright::Level, Upright::Vertical, Upright::Full, Upright::Auto] {
        eprintln!("{m:?}: {:?}", crate::upright::solve(m, &a).0);
    }
}

/// Sign convention for Auto straighten: a horizon descending 4° to the right needs crop angle −4°,
/// which `level_degrees` must report (the crop panel uses it as the crop angle directly).
#[test]
fn level_degrees_sign_matches_crop_angle() {
    let (w, h) = (900usize, 600usize);
    let t = 4f32.to_radians().tan();
    let src = Rgb32f::from_fn(w, h, |x, y| {
        let yh = h as f32 / 2.0 + (x as f32 - w as f32 / 2.0) * t;
        if (y as f32) < yh {
            [0.6, 0.7, 0.9]
        } else if (((y as f32 - yh) / 30.0) as i32) % 2 == 0 {
            [0.05, 0.1, 0.2]
        } else {
            [0.1, 0.2, 0.3]
        }
    });
    let deg = crate::upright::level_degrees(&src, &SourceInfo::default(), &DevelopSettings::default()).expect("lines found");
    assert!((deg + 4.0).abs() < 0.5, "{deg}");
}

/// A "horizon" of `deg` degrees: sky above, below it bands of `band` px (0: one edge only).
fn tilted_scene(deg: f32, band: f32) -> Rgb32f {
    let (w, h) = (900usize, 600usize);
    let t = deg.to_radians().tan();
    Rgb32f::from_fn(w, h, |x, y| {
        let yh = h as f32 / 2.0 + (x as f32 - w as f32 / 2.0) * t;
        if (y as f32) < yh {
            [0.6, 0.7, 0.9]
        } else if band > 0.0 && (((y as f32 - yh) / band) as i32) % 2 == 1 {
            [0.1, 0.2, 0.3]
        } else {
            [0.05, 0.1, 0.2]
        }
    })
}

fn auto_level(src: &Rgb32f) -> Option<f64> {
    crate::upright::level_degrees(src, &SourceInfo::default(), &DevelopSettings::default())
}

/// Thick dark lines on a light ground, each from `a` to `b` in centred coordinates (half the long
/// edge = 1).
#[allow(clippy::type_complexity)] // the type is spelled out where it is used
fn line_drawing(lines: &[((f32, f32), (f32, f32))]) -> Rgb32f {
    let (w, h) = (900usize, 600usize);
    let half = w as f32 / 2.0;
    Rgb32f::from_fn(w, h, |x, y| {
        let p = ((x as f32 - w as f32 / 2.0) / half, (y as f32 - h as f32 / 2.0) / half);
        let near = lines.iter().any(|&((ax, ay), (bx, by))| {
            let (dx, dy) = (bx - ax, by - ay);
            let t = (((p.0 - ax) * dx + (p.1 - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            let (qx, qy) = (ax + t * dx - p.0, ay + t * dy - p.1);
            qx.hypot(qy) * half < 3.0
        });
        if near { [0.05, 0.05, 0.05] } else { [0.8, 0.8, 0.8] }
    })
}

/// A single short line (the frame of a window, a branch) levels nothing: it turned straight
/// photos in a measurement on real ones.
#[test]
fn auto_straighten_ignores_a_single_short_line() {
    let t = 4f32.to_radians().tan();
    assert_eq!(auto_level(&line_drawing(&[((-0.3, -0.3 * t), (0.3, 0.3 * t))])), None, "one short line");
    // a horizon is one straight edge too, but across the whole width
    let deg = auto_level(&tilted_scene(4.0, 0.0)).expect("a horizon");
    assert!((deg + 4.0).abs() < 0.3, "{deg}");
}

/// Auto straighten turns by at most `straighten::MAX_DEGREES` (5°): none of 113 photos
/// straightened by hand needed more than 6.1°, and larger "levels" came from texture.
#[test]
fn auto_straighten_leaves_steep_lines_alone() {
    assert_eq!(auto_level(&tilted_scene(20.0, 30.0)), None, "20° is not a horizon to level");
    assert_eq!(auto_level(&tilted_scene(8.0, 30.0)), None, "nor is 8°");
    let deg = auto_level(&tilted_scene(4.0, 30.0)).expect("4° is");
    assert!((deg + 4.0).abs() < 0.3, "{deg}");
}

/// A sea horizon among ripples: the short, randomly tilted edges of the water outnumber the
/// horizon, which the line consensus took for no answer (or the ripples' angle), but the horizon
/// runs across the whole width and levels the photo.
#[test]
fn auto_straighten_finds_a_sea_horizon_among_ripples() {
    let (w, h) = (900usize, 600usize);
    let t = 2f32.to_radians().tan();
    let mut seed = 7u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    // ripples: short bright dashes tilted up to ±12°
    let ripples: Vec<(f32, f32, f32, f32)> =
        (0..400).map(|_| (rnd() * w as f32, rnd() * h as f32, (rnd() - 0.5) * 0.4, 8.0 + rnd() * 25.0)).collect();
    let src = Rgb32f::from_fn(w, h, |x, y| {
        let (x, y) = (x as f32, y as f32);
        let yh = h as f32 * 0.4 + (x - w as f32 / 2.0) * t;
        if y < yh {
            return [0.75, 0.8, 0.9];
        }
        let lit = ripples.iter().any(|&(rx, ry, slope, len)| {
            let dx = x - rx;
            dx.abs() < len && (y - ry - slope * dx).abs() < 1.2 && ry > yh + 4.0
        });
        if lit { [0.5, 0.55, 0.6] } else { [0.1, 0.15, 0.25] }
    });
    let deg = auto_level(&src).expect("the horizon");
    assert!((deg + 2.0).abs() < 0.3, "{deg}");
}

/// A building shot upward and rolled 3°: its verticals converge on a vanishing point above the
/// frame. Their mean tilt depends on where in the frame they stand (here all right of centre);
/// the vanishing point gives the roll.
#[test]
fn auto_straighten_takes_the_roll_from_converging_verticals() {
    let roll = 3f32.to_radians();
    let vp = (6.0 * roll.sin(), -6.0 * roll.cos());
    let lines: Vec<_> = (0..12)
        .map(|i| {
            let x = 0.05 + i as f32 * 0.07;
            let bottom = (x, 0.6);
            // two thirds of the way from the bottom towards the vanishing point, ending in the frame
            let top_t = (0.6 + 0.6) / (0.6 - vp.1);
            (bottom, (x + (vp.0 - x) * top_t, 0.6 + (vp.1 - 0.6) * top_t))
        })
        .collect();
    let deg = auto_level(&line_drawing(&lines)).expect("the vanishing point");
    assert!((deg + 3.0).abs() < 0.3, "{deg}");
}

/// The Upright analysis cache tells apart what changes the analysis image: another lens (`info.lens`)
/// or the optics section switched off (review of storytold/lightcraft#2809).
#[test]
fn the_analysis_cache_tells_lenses_and_optics_apart() {
    let src = tilted_scene(2.0, 30.0);
    let s = DevelopSettings::default();
    let plain = SourceInfo::default();
    let vignette = craft_develop_model::EmbeddedVignette::default();
    let lens =
        SourceInfo { lens: Some(craft_develop_model::EmbeddedLens { vignette: Some(vignette), ..Default::default() }), ..SourceInfo::default() };
    let key = crate::upright::analysis_key;
    assert_eq!(key(&src, &plain, &s), key(&src, &plain.clone(), &s), "the same source, settings and lens");
    assert_ne!(key(&src, &plain, &s), key(&src, &lens, &s), "another lens");
    let mut off = s.clone();
    off.disabled_sections.push("optics".into());
    assert!(!off.section_enabled("optics"));
    assert_ne!(key(&src, &plain, &s), key(&src, &plain, &off), "the optics section off");
}

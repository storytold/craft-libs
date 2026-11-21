//! End-to-end tests: synthetic logos made in code, with known geometry, traced and measured.

use crate::{Method, Mode, Raster, TraceError, TraceParams, TraceResult, trace_within};
use craft_geom::Point;
use kurbo::{ParamCurve, Shape};

/// A soft, anti-aliased render of `shade` (`Some(colour)` where the shape is), supersampled `ss × ss`
/// and blurred by `blur` pixels, on a transparent background.
fn render(w: u32, h: u32, blur: f64, shade: impl Fn(f64, f64) -> Option<[u8; 3]>) -> Raster {
    let ss = 8usize;
    let (wu, hu) = (w as usize, h as usize);
    let mut px = vec![[0.0f64; 4]; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            let mut acc = [0.0f64; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let (fx, fy) = (x as f64 + (sx as f64 + 0.5) / ss as f64, y as f64 + (sy as f64 + 0.5) / ss as f64);
                    if let Some(c) = shade(fx, fy) {
                        acc[0] += f64::from(c[0]) / 255.0;
                        acc[1] += f64::from(c[1]) / 255.0;
                        acc[2] += f64::from(c[2]) / 255.0;
                        acc[3] += 1.0;
                    }
                }
            }
            let n = (ss * ss) as f64;
            px[y * wu + x] = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
        }
    }
    if blur > 0.0 {
        let r = (3.0 * blur).ceil() as i64;
        let k: Vec<f64> = (-r..=r).map(|i| (-0.5 * (i as f64 / blur).powi(2)).exp()).collect();
        let sum: f64 = k.iter().sum();
        let k: Vec<f64> = k.iter().map(|v| v / sum).collect();
        for horizontal in [true, false] {
            let src = px.clone();
            for y in 0..hu as i64 {
                for x in 0..wu as i64 {
                    let mut acc = [0.0f64; 4];
                    for (j, kv) in k.iter().enumerate() {
                        let d = j as i64 - r;
                        let (xx, yy) = if horizontal { ((x + d).clamp(0, wu as i64 - 1), y) } else { (x, (y + d).clamp(0, hu as i64 - 1)) };
                        let p = src[(yy as usize) * wu + xx as usize];
                        for c in 0..4 {
                            acc[c] += kv * p[c];
                        }
                    }
                    px[(y as usize) * wu + x as usize] = acc;
                }
            }
        }
    }
    let mut rgba = Vec::with_capacity(wu * hu * 4);
    for p in px {
        let a = p[3];
        let un = |v: f64| if a > 1e-9 { ((v / a) * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    Raster::new(w, h, rgba).unwrap()
}

const RED: [u8; 3] = [210, 40, 50];
const BLUE: [u8; 3] = [40, 80, 200];
const LIGHT: [u8; 3] = [240, 240, 220];
const SKY: [u8; 3] = [40, 120, 240];
const WHITE: [u8; 3] = [255, 255, 255];

fn logo_params() -> TraceParams {
    TraceParams {
        mode: Mode::Logo,
        colors: 6,
        paths: 60.0,
        corners: 50.0,
        noise: 6,
        snap_curves_to_lines: true,
        method: Method::Abutting,
        ..TraceParams::default()
    }
}

fn trace(img: &Raster, p: &TraceParams) -> TraceResult {
    trace_within(img, p, 100_000).expect("trace")
}

fn samples(res: &TraceResult) -> Vec<Point> {
    let mut out = Vec::new();
    for p in &res.paths {
        for seg in p.path.to_bezpath().segments() {
            out.extend((0..=40).map(|i| seg.eval(f64::from(i) / 40.0)));
        }
    }
    out
}

fn covered(res: &TraceResult, p: Point) -> bool {
    res.paths.iter().any(|t| t.path.to_bezpath().winding(p) != 0)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= tol)
}

#[test]
fn a_blurred_disc_becomes_a_true_circle_within_a_tenth_of_a_pixel() {
    let (cx, cy, r) = (31.7, 32.4, 19.3);
    let img = render(64, 64, 0.7, |x, y| ((x - cx).hypot(y - cy) <= r).then_some(RED));
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1, "{} paths", res.paths.len());
    assert!(near(res.paths[0].color, RED, 3), "colour {:?}", res.paths[0].color);
    let errs: Vec<f64> = samples(&res).iter().map(|p| (p.distance(Point::new(cx, cy)) - r).abs()).collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    let max = errs.iter().copied().fold(0.0, f64::max);
    assert!(mean < 0.06 && max < 0.15, "mean {mean:.3} max {max:.3}");
    assert!(res.anchor_count() <= 6, "{} anchors for a circle", res.anchor_count());
}

#[test]
fn a_blurred_rectangle_keeps_square_corners_and_four_anchors() {
    let (x0, x1, y0, y1) = (10.4, 77.9, 8.7, 38.2);
    let img = render(90, 48, 0.8, |x, y| ((x0..=x1).contains(&x) && (y0..=y1).contains(&y)).then_some(BLUE));
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.anchor_count(), 4, "a rectangle is four corners");
    let b = res.paths[0].path.bounds().expect("bounds");
    for (got, want, what) in [(b.x0, x0, "left"), (b.x1, x1, "right"), (b.y0, y0, "top"), (b.y1, y1, "bottom")] {
        // Sub-pixel: within 0.15 px of the true edge, from a blurred 90 px image.
        assert!((got - want).abs() < 0.15, "{what} edge {got:.3} vs {want}");
    }
}

#[test]
fn touching_colours_leave_no_gap_between_them() {
    let edge = 40.35;
    let img = render(80, 50, 0.7, |x, y| {
        if !(10.2..=40.6).contains(&y) {
            None
        } else if (10.3..=edge).contains(&x) {
            Some(RED)
        } else if (edge..=70.6).contains(&x) {
            Some(BLUE)
        } else {
            None
        }
    });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 2);
    assert!(!near(res.paths[0].color, res.paths[1].color, 20));
    for i in 0..200 {
        let y = 12.0 + 27.0 * f64::from(i) / 199.0;
        for dx in [-0.03, 0.0, 0.03] {
            assert!(covered(&res, Point::new(edge + dx, y)), "gap at ({}, {y})", edge + dx);
        }
    }
}

#[test]
fn letters_knocked_out_of_a_badge_become_white_or_holes() {
    let ring = |x: f64, y: f64| {
        let d = (x - 32.0).hypot(y - 32.0);
        (12.0..=25.0).contains(&d).then_some(BLUE)
    };
    let img = render(64, 64, 0.7, ring);
    // Default: the knockout is painted white, on top of the badge.
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 2, "badge + white knockout");
    assert!(res.paths.iter().any(|p| p.color == [255, 255, 255]));
    assert!(covered(&res, Point::new(32.0, 32.0)));
    // Ignore white: the knockout is a real hole in the badge.
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].path.subpaths.len(), 2, "outer outline and a hole");
    assert!(!covered(&res, Point::new(32.0, 32.0)), "the hole must be empty");
    assert!(covered(&res, Point::new(32.0, 32.0 + 18.0)), "the ring itself is filled");
}

#[test]
fn a_gradient_is_refused_not_posterised() {
    let img = Raster::from_fn(64, 32, |x, y| [(x * 4) as u8, 255 - (x * 4) as u8, (y * 6) as u8, 255]);
    match trace_within(&img, &logo_params(), 100_000) {
        Err(TraceError::NotFlat { fraction }) => assert!(fraction < 0.93),
        other => panic!("expected NotFlat, got {other:?}"),
    }
    let msg = TraceError::NotFlat { fraction: 0.69 }.to_string();
    assert!(msg.contains("69%") && msg.contains("Color mode"), "{msg}");
}

#[test]
fn a_white_background_logo_drops_the_white_when_asked() {
    let img = Raster::from_fn(80, 60, |x, y| {
        let d = (x as f64 + 0.5 - 40.0).hypot(y as f64 + 0.5 - 30.0);
        if (10.0..=24.0).contains(&d) { [20, 20, 20, 255] } else { [255, 255, 255, 255] }
    });
    let kept = trace(&img, &logo_params());
    assert!(kept.paths.iter().any(|p| p.color == [255, 255, 255]), "white background kept by default");
    let dropped = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert!(dropped.paths.iter().all(|p| p.color != [255, 255, 255]));
    assert!(!dropped.paths.is_empty());
}

#[test]
fn the_named_palette_is_used_exactly_and_a_bad_one_is_an_error() {
    let img = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(RED));
    let named = TraceParams { logo_colors: vec!["#d22832".into()], ..logo_params() };
    let res = trace(&img, &named);
    assert_eq!(res.paths[0].color, [0xd2, 0x28, 0x32]);
    let bad = TraceParams { logo_colors: vec!["red".into()], ..logo_params() };
    assert!(matches!(trace_within(&img, &bad, 1000), Err(TraceError::BadPalette(s)) if s == "red"));
}

/// The color of the topmost path at `p` (paths are listed bottom first).
fn top_color(res: &TraceResult, p: Point) -> Option<[u8; 3]> {
    res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color)
}

#[test]
fn logo_white_off_traces_only_the_named_colors() {
    // A light disc in a black ring, a blue square on it, and two white spots.
    let img = render(120, 120, 0.7, |x, y| {
        let d = (x - 60.0).hypot(y - 60.0);
        if (x - 32.0).hypot(y - 40.0) <= 6.0 || (x - 78.0).hypot(y - 86.0) <= 6.0 {
            Some(WHITE)
        } else if (45.0..=75.0).contains(&x) && (45.0..=75.0).contains(&y) {
            Some(SKY)
        } else if d <= 46.0 {
            Some(LIGHT)
        } else if d <= 50.0 {
            Some([0, 0, 0])
        } else {
            None
        }
    });
    let named = TraceParams { logo_colors: vec!["#000000".into(), "#F0F0DC".into(), "#2878F0".into()], ..logo_params() };
    // By default white is a fourth color.
    let res = trace(&img, &named);
    assert_eq!(res.palette.len(), 4, "{:?}", res.palette);
    assert_eq!(top_color(&res, Point::new(32.0, 40.0)), Some(WHITE));
    // Without it, the three named colors only: the white spots take the light color.
    let res = trace(&img, &TraceParams { logo_white: false, ..named.clone() });
    assert_eq!(res.palette.len(), 3, "{:?}", res.palette);
    assert!(!res.palette.contains(&WHITE));
    for p in [Point::new(32.0, 40.0), Point::new(78.0, 86.0), Point::new(20.0, 60.0)] {
        assert_eq!(top_color(&res, p), Some(LIGHT), "at {p:?}");
    }
    assert_eq!(top_color(&res, Point::new(60.0, 60.0)), Some(SKY));
    assert_eq!(top_color(&res, Point::new(60.0, 12.0)), Some([0, 0, 0]));
    assert!(!covered(&res, Point::new(5.0, 5.0)));
    // Ignore White still leaves the white spots out.
    let res = trace(&img, &TraceParams { logo_white: false, ignore_white: true, ..named.clone() });
    for p in [Point::new(32.0, 40.0), Point::new(78.0, 86.0)] {
        assert!(!covered(&res, p), "at {p:?}");
    }
    assert_eq!(top_color(&res, Point::new(20.0, 60.0)), Some(LIGHT));
}

#[test]
fn logo_white_off_keeps_a_named_white_and_knockouts_as_holes() {
    let disc = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(WHITE));
    let only_white = TraceParams { logo_colors: vec!["#ffffff".into()], logo_white: false, ..logo_params() };
    let res = trace(&disc, &only_white);
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].color, WHITE);
    // An enclosed transparent area stays a hole: there is no white to paint it with.
    let ring = render(64, 64, 0.7, |x, y| (12.0..=25.0).contains(&(x - 32.0).hypot(y - 32.0)).then_some(BLUE));
    let res = trace(&ring, &TraceParams { logo_colors: vec!["#2850c8".into()], logo_white: false, ..logo_params() });
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].path.subpaths.len(), 2, "outer outline and a hole");
    assert!(!covered(&res, Point::new(32.0, 32.0)), "the hole must be empty");
    assert!(covered(&res, Point::new(32.0, 32.0 + 18.0)), "the ring itself is filled");
}

#[test]
fn logo_white_off_gives_each_white_the_nearest_named_color() {
    // Red and blue are equally far from white: the white disc takes the first one named.
    let disc = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(WHITE));
    let params = TraceParams { logo_colors: vec!["#ff0000".into(), "#0000ff".into()], logo_white: false, ..logo_params() };
    let res = trace(&disc, &params);
    assert_eq!(top_color(&res, Point::new(32.0, 32.0)), Some([255, 0, 0]));
    assert!(!covered(&res, Point::new(5.0, 5.0)));
    // Each white takes the named color nearest it, so two named whites both stay.
    const OFF: [u8; 3] = [245, 245, 245];
    let two = render(96, 64, 0.7, |x, y| {
        if (x - 24.0).hypot(y - 32.0) <= 16.0 {
            Some(OFF)
        } else if (x - 72.0).hypot(y - 32.0) <= 16.0 {
            Some(WHITE)
        } else {
            None
        }
    });
    let params = TraceParams { logo_colors: vec!["#f5f5f5".into(), "#ffffff".into()], logo_white: false, ..logo_params() };
    let res = trace(&two, &params);
    assert_eq!(top_color(&res, Point::new(24.0, 32.0)), Some(OFF));
    assert_eq!(top_color(&res, Point::new(72.0, 32.0)), Some(WHITE));
    // A white square on blue keeps its edges where they are when it turns red.
    let square = render(100, 100, 0.7, |x, y| Some(if (30.0..70.0).contains(&x) && (30.0..70.0).contains(&y) { WHITE } else { [0, 0, 255] }));
    let res = trace(&square, &TraceParams { logo_colors: vec!["#ff0000".into(), "#0000ff".into()], logo_white: false, ..logo_params() });
    for (inside, outside) in [(30.3, 29.7), (69.7, 70.3)] {
        assert_eq!(top_color(&res, Point::new(inside, 50.0)), Some([255, 0, 0]), "at x {inside}");
        assert_eq!(top_color(&res, Point::new(outside, 50.0)), Some([0, 0, 255]), "at x {outside}");
    }
}

#[test]
fn a_large_image_is_traced_at_reduced_size_and_scaled_back() {
    let (cx, cy, r) = (1200.0, 600.0, 400.0);
    let img = Raster::from_fn(2400, 1200, |x, y| {
        if (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) <= r { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] }
    });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    let b = res.paths[0].path.bounds().expect("bounds");
    for (got, want) in [(b.x0, cx - r), (b.x1, cx + r), (b.y0, cy - r), (b.y1, cy + r)] {
        assert!((got - want).abs() < 2.0, "{got} vs {want}");
    }
}

/// A light square inset 50 px with slanted 2 px black lines across it, on a transparent background.
fn lined_square(n: u32) -> Raster {
    Raster::from_fn(n, n, |x, y| {
        if !(50..n - 50).contains(&x) || !(50..n - 50).contains(&y) {
            [0, 0, 0, 0]
        } else if (i64::from(x) - i64::from(y) / 8).rem_euclid(40) < 2 {
            [0, 0, 0, 255]
        } else {
            [240, 240, 220, 255]
        }
    })
}

#[test]
fn a_trace_averaged_down_says_the_size_it_worked_at() {
    let named = TraceParams { logo_colors: vec!["#000000".into(), "#F0F0DC".into()], ..logo_params() };
    // 1025 px is over the working size: averaged down by 2, to 513 px.
    let res = trace(&lined_square(1025), &named);
    assert!(!res.paths.is_empty());
    assert_eq!(res.worked_at, Some((513, 513)));
    let res = trace(&lined_square(1024), &named);
    assert!(!res.paths.is_empty());
    assert_eq!(res.worked_at, None);
}

#[test]
fn named_colors_a_few_levels_off_trace_an_image_up_to_1024_px_at_full_size() {
    // A light disc in a dark ring with a blue square on it, hard edges, 800 px square, and a dark
    // 2 px dot on the disc at odd pixels: averaged down by 2, the dot would be a quarter of each of
    // four pixels and leave no shape.
    let img = Raster::from_fn(800, 800, |x, y| {
        let (fx, fy) = (f64::from(x) / 5.0, f64::from(y) / 5.0);
        let d = (fx - 80.0).hypot(fy - 80.0);
        if (401..=402).contains(&x) && (201..=202).contains(&y) {
            [4, 4, 3, 255]
        } else if (70.0..=110.0).contains(&fx) && (70.0..=110.0).contains(&fy) {
            [34, 124, 250, 255]
        } else if d <= 60.0 {
            [252, 248, 227, 255]
        } else if d <= 68.0 {
            [4, 4, 3, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    for (off, names) in [(8, ["#0C0C0B", "#F4F0DB", "#1A74F2"]), (12, ["#10100F", "#F0ECD7", "#1670EE"])] {
        let p = TraceParams { logo_colors: names.iter().map(|s| s.to_string()).collect(), ..logo_params() };
        let res = trace(&img, &p);
        let color_at = |p: Point| res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color);
        assert_eq!(res.paths.len(), 4, "names {off} levels off");
        let (dark, cream, blue) = ([4 + off, 4 + off, 3 + off], [252 - off, 248 - off, 227 - off], [34 - off, 124 - off, 250 - off]);
        assert_eq!(color_at(Point::new(402.0, 202.0)), Some(dark), "names {off} levels off");
        assert_eq!(color_at(Point::new(450.0, 450.0)), Some(blue), "names {off} levels off");
        assert_eq!(color_at(Point::new(200.0, 400.0)), Some(cream), "names {off} levels off");
    }
}

/// A dark ring around a tan disc with a blue square on it, hard edges, on a transparent background.
fn hard_badge() -> Raster {
    Raster::from_fn(160, 160, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let d = (fx - 80.0).hypot(fy - 80.0);
        if (60.0..=100.0).contains(&fx) && (60.0..=100.0).contains(&fy) {
            [34, 124, 250, 255]
        } else if d <= 60.0 {
            [200, 170, 120, 255]
        } else if d <= 68.0 {
            [4, 4, 3, 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

#[test]
fn flat_art_is_traced_with_named_colors_far_from_its_own() {
    // Every named color is 20 levels off the image's in each channel: the image is still flat art,
    // and each area takes the named color nearest it.
    let names = ["#181817", "#B49664", "#0E68E6"];
    let res = trace(&hard_badge(), &TraceParams { logo_colors: names.iter().map(|s| s.to_string()).collect(), ..logo_params() });
    assert_eq!(res.paths.len(), 3, "{:?}", res.paths.iter().map(|p| p.color).collect::<Vec<_>>());
    assert_eq!(top_color(&res, Point::new(80.0, 80.0)), Some([14, 104, 230]));
    assert_eq!(top_color(&res, Point::new(80.0, 30.0)), Some([180, 150, 100]));
    assert_eq!(top_color(&res, Point::new(80.0, 15.0)), Some([24, 24, 23]));
}

#[test]
fn art_that_is_not_flat_is_refused_by_its_own_colors_whatever_is_named() {
    let img = Raster::from_fn(64, 32, |x, y| [(x * 4) as u8, 255 - (x * 4) as u8, (y * 6) as u8, 255]);
    let refused = |p: &TraceParams| match trace_within(&img, p, 100_000) {
        Err(TraceError::NotFlat { fraction }) => fraction,
        other => panic!("expected NotFlat, got {other:?}"),
    };
    let own = refused(&logo_params());
    // Names close to some of the gradient's colors, and names far from all of them.
    for names in [["#08F710", "#80807F", "#F00C60"], ["#FF00FF", "#00FFFF", "#FFFF00"]] {
        let named = TraceParams { logo_colors: names.iter().map(|s| s.to_string()).collect(), ..logo_params() };
        assert_eq!(refused(&named), own, "names {names:?}");
        assert_eq!(refused(&TraceParams { logo_white: false, ..named }), own, "names {names:?} without white");
    }
}

#[test]
fn a_hatch_gives_the_same_result_with_close_and_far_named_colors() {
    // Black lines one pixel wide across, blue lines down, on a cream ground.
    let img = render(120, 120, 0.5, |x, y| {
        Some(if y.rem_euclid(8.0) < 1.0 {
            [12, 12, 12]
        } else if x.rem_euclid(8.0) < 1.0 {
            [30, 70, 200]
        } else {
            [236, 226, 196]
        })
    });
    let outcome = |names: [&str; 3]| match trace_within(
        &img,
        &TraceParams { logo_colors: names.iter().map(|s| s.to_string()).collect(), ..logo_params() },
        100_000,
    ) {
        Ok(res) => Ok(res.paths.len()),
        Err(TraceError::NotFlat { fraction }) => Err(fraction),
        Err(e) => panic!("{e:?}"),
    };
    let close = outcome(["#0C0C0C", "#1E46C8", "#ECE2C4"]);
    // Every channel 20 levels off.
    assert_eq!(outcome(["#202020", "#325ADC", "#D8CEB0"]), close);
}

#[test]
fn more_named_colors_than_the_colors_setting_do_not_make_the_refusal_depend_on_the_names() {
    let img = Raster::from_fn(64, 32, |x, y| [(x * 4) as u8, 255 - (x * 4) as u8, (y * 6) as u8, 255]);
    let refused = |p: &TraceParams| match trace_within(&img, p, 100_000) {
        Err(TraceError::NotFlat { fraction }) => fraction,
        other => panic!("expected NotFlat, got {other:?}"),
    };
    // Six names with Colors at 2: seven of the image's own colors are looked for (six and white), as
    // with Colors at 7.
    let own = refused(&TraceParams { colors: 7, ..logo_params() });
    for names in
        [["#00FF00", "#40BF3C", "#807F78", "#C03FB4", "#FC03BA", "#649B1E"], ["#FF00FF", "#00FFFF", "#FFFF00", "#000000", "#FF0000", "#0000FF"]]
    {
        let named = TraceParams { logo_colors: names.iter().map(|s| s.to_string()).collect(), colors: 2, ..logo_params() };
        assert_eq!(refused(&named), own, "names {names:?}");
    }
}

#[test]
fn areas_of_colors_that_are_not_named_are_traced_in_the_named_colors() {
    // Red, green, blue and yellow bands; only red and blue are named.
    const BANDS: [[u8; 3]; 4] = [[200, 30, 30], [30, 160, 60], [60, 70, 180], [230, 200, 40]];
    let img = Raster::from_fn(160, 40, |x, _| {
        let c = BANDS[(x / 40) as usize];
        [c[0], c[1], c[2], 255]
    });
    let res = trace(&img, &TraceParams { logo_colors: vec!["#C81E1E".into(), "#3C46B4".into()], ..logo_params() });
    for p in &res.paths {
        assert!([BANDS[0], BANDS[2], [255, 255, 255]].contains(&p.color), "{:?}", p.color);
    }
    assert_eq!(top_color(&res, Point::new(20.0, 20.0)), Some(BANDS[0]));
    assert_eq!(top_color(&res, Point::new(100.0, 20.0)), Some(BANDS[2]));
}

#[test]
fn flat_art_on_white_with_more_named_colors_than_the_colors_setting_is_flat() {
    // Red, green and blue bands and a white one, the three colors named, with Colors at 2: white
    // does not take the place of a named color while the image's colors are found.
    const BANDS: [[u8; 3]; 4] = [[220, 30, 30], [30, 180, 60], [40, 60, 210], [255, 255, 255]];
    let img = Raster::from_fn(160, 40, |x, _| {
        let c = BANDS[(x / 40) as usize];
        [c[0], c[1], c[2], 255]
    });
    let names = vec!["#DC1E1E".to_string(), "#1EB43C".to_string(), "#283CD2".to_string()];
    let res = trace(&img, &TraceParams { logo_colors: names, colors: 2, ..logo_params() });
    for (i, c) in BANDS.iter().enumerate() {
        assert_eq!(top_color(&res, Point::new(20.0 + 40.0 * i as f64, 20.0)), Some(*c), "band {i}");
    }
}

#[test]
fn flat_art_with_more_named_colors_than_the_colors_setting_is_flat() {
    // Eight flat bands, each color named, with Colors at 2: as many of the image's own colors are
    // looked for as are named, with white, so the art is still flat.
    const BANDS: [[u8; 3]; 8] =
        [[200, 30, 30], [30, 160, 60], [60, 70, 180], [230, 200, 40], [120, 40, 160], [20, 20, 20], [240, 130, 30], [40, 190, 200]];
    let img = Raster::from_fn(160, 40, |x, _| {
        let c = BANDS[(x / 20) as usize];
        [c[0], c[1], c[2], 255]
    });
    let names: Vec<String> = BANDS.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect();
    let res = trace(&img, &TraceParams { logo_colors: names, colors: 2, ..logo_params() });
    for (i, c) in BANDS.iter().enumerate() {
        assert_eq!(top_color(&res, Point::new(10.0 + 20.0 * i as f64, 20.0)), Some(*c), "band {i}");
    }
}

/// Every anchor of every path is finite and inside (a small margin around) the image.
fn assert_sane(res: &TraceResult, w: u32, h: u32) {
    for t in &res.paths {
        for sp in &t.path.subpaths {
            for a in &sp.anchors {
                assert!(a.p.x.is_finite() && a.p.y.is_finite() && a.h_in.x.is_finite() && a.h_out.y.is_finite(), "non-finite anchor");
                assert!(a.p.x > -2.0 && a.p.y > -2.0 && a.p.x < f64::from(w) + 2.0 && a.p.y < f64::from(h) + 2.0, "anchor {:?} outside {w}x{h}", a.p);
            }
        }
    }
}

#[test]
fn degenerate_images_are_fine() {
    let p = logo_params();
    for (w, h, img) in [
        (0, 0, Raster::new(0, 0, vec![]).unwrap()),
        (1, 1, Raster::new(1, 1, vec![200, 30, 30, 255]).unwrap()),
        (3, 3, Raster::new(3, 3, vec![0; 36]).unwrap()),
        (2, 2, Raster::new(2, 2, vec![255; 16]).unwrap()),
        (5, 1, Raster::new(5, 1, vec![9; 20]).unwrap()),
        (1, 7, Raster::new(1, 7, vec![130; 28]).unwrap()),
    ] {
        match trace_within(&img, &p, 1000) {
            Ok(res) => assert_sane(&res, w, h),
            Err(TraceError::NotFlat { .. }) => {}
            Err(e) => panic!("unexpected error for {w}x{h}: {e}"),
        }
    }
}

#[test]
fn extreme_parameters_never_panic() {
    let img = render(48, 48, 0.7, |x, y| ((x - 24.0).hypot(y - 24.0) <= 15.0).then_some(RED));
    let weird = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -50.0, 0.0, 100.0, 1e9];
    for &paths in &weird {
        for &corners in &weird {
            let p = TraceParams { paths, corners, ..logo_params() };
            let _ = trace_within(&img, &p, 10_000);
        }
    }
    for colors in [0, 1, 2, 300, u32::MAX] {
        for noise in [0, 1, 1_000_000, u32::MAX] {
            let p = TraceParams { colors, noise, ..logo_params() };
            let _ = trace_within(&img, &p, 10_000);
        }
    }
    // A tiny anchor budget is reported as too complex, not exceeded.
    let r = trace_within(&img, &logo_params(), 0);
    assert!(r.is_err() || r.is_ok_and(|t| t.anchor_count() == 0));
}

#[test]
fn noise_specks_are_dropped() {
    let img = render(64, 64, 0.5, |x, y| if (x - 32.0).hypot(y - 32.0) <= 18.0 || (x - 6.0).hypot(y - 6.0) <= 0.6 { Some(RED) } else { None });
    let res = trace(&img, &TraceParams { noise: 20, ..logo_params() });
    assert_eq!(res.paths.len(), 1, "the speck should be gone");
}

#[test]
fn nested_rings_keep_their_own_holes_under_ignore_white() {
    // Red ring, white band, red ring, white centre.
    let img = render(80, 80, 0.7, |x, y| {
        let d = (x - 40.0).hypot(y - 40.0);
        ((22.0..=28.0).contains(&d) || (8.0..=16.0).contains(&d)).then_some(RED)
    });
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert_eq!(res.paths.len(), 2, "two red rings");
    for p in &res.paths {
        assert_eq!(p.path.subpaths.len(), 2, "each ring is an outline and its own hole, no more");
    }
    assert!(!covered(&res, Point::new(40.0, 40.0)), "centre is empty");
    assert!(covered(&res, Point::new(52.0, 40.0)), "inner ring is filled");
    assert!(!covered(&res, Point::new(59.0, 40.0)), "the band between the rings is empty");
    assert!(covered(&res, Point::new(65.0, 40.0)), "outer ring is filled");
}

#[test]
fn too_many_outlines_are_refused_not_silently_truncated() {
    let specks =
        |x: f64, y: f64| (x.rem_euclid(5.0) >= 1.0 && x.rem_euclid(5.0) < 4.0 && y.rem_euclid(5.0) >= 1.0 && y.rem_euclid(5.0) < 4.0).then_some(RED);
    let img = render(200, 200, 0.0, specks);
    assert!(matches!(trace_within(&img, &logo_params(), 1_000_000), Err(TraceError::TooComplex { .. })));
    // Specks under the noise threshold do not count towards the limit.
    let res = trace_within(&img, &TraceParams { noise: 200, ..logo_params() }, 1_000_000).expect("trace");
    assert!(res.paths.is_empty());
}

#[test]
fn holes_that_are_not_cut_do_not_count_towards_the_outline_limit() {
    // A blue field with 625 holes, each filled with red, white or green: more holes than the limit, fewer of each fill.
    const GREEN: [u8; 3] = [40, 170, 60];
    let img = render(125, 125, 0.0, |x, y| {
        let (cx, cy) = (x.rem_euclid(5.0), y.rem_euclid(5.0));
        if (1.0..4.0).contains(&cx) && (1.0..4.0).contains(&cy) {
            Some([RED, [255, 255, 255], GREEN][((x / 5.0) as usize + (y / 5.0) as usize) % 3])
        } else {
            Some(BLUE)
        }
    });
    let res = trace_within(&img, &logo_params(), 1_000_000).expect("trace");
    assert!(res.paths.iter().any(|p| near(p.color, BLUE, 3)), "the blue field is traced");
    // Under Ignore White the holes are cut, so they count.
    assert!(matches!(trace_within(&img, &TraceParams { ignore_white: true, ..logo_params() }, 1_000_000), Err(TraceError::TooComplex { .. })));
}

#[test]
fn a_large_odd_sized_image_never_overshoots_its_edges() {
    // 1025 wide: the last working pixel is narrower than the others.
    let img = Raster::from_fn(1025, 20, |x, y| if x >= 600 && (2..18).contains(&y) { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    let b = res.paths[0].path.bounds().expect("bounds");
    assert!(b.x1 <= 1025.15 && b.x1 >= 1024.7, "right edge {}", b.x1);
    assert!((b.x0 - 600.0).abs() < 0.6 && (b.y0 - 2.0).abs() < 0.6 && (b.y1 - 18.0).abs() < 0.6, "{b:?}");
    assert_sane(&res, 1025, 20);
}

#[test]
fn a_hostile_palette_is_bounded_and_near_duplicates_merge() {
    let img = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(RED));
    let many: Vec<String> = std::iter::once("#d22832".to_string()).chain((1..200_000u32).map(|i| format!("#{:06x}", i * 37 % 0xfffff0))).collect();
    let started = std::time::Instant::now();
    let res = trace_within(&img, &TraceParams { logo_colors: many, ..logo_params() }, 100_000);
    assert!(started.elapsed().as_secs() < 20, "palette handling took {:?}", started.elapsed());
    if let Ok(r) = res {
        assert!(r.palette.len() <= 11);
    }
    // Ten shades within a couple of units of one red are one colour, so one path, not ten layers.
    let shades: Vec<String> = (0..10).map(|i| format!("#{:02x}{:02x}{:02x}", 210 + i % 3, 40 + i % 2, 50 + i % 3)).collect();
    let res = trace(&img, &TraceParams { logo_colors: shades, ..logo_params() });
    assert_eq!(res.paths.len(), 1, "{} paths", res.paths.len());
    // White cannot be named: naming only white leaves nothing to trace.
    assert!(trace(&img, &TraceParams { logo_colors: vec!["#ffffff".into()], ..logo_params() }).paths.is_empty());
}

#[test]
fn a_small_accent_colour_is_not_lost() {
    let img = render(100, 100, 0.6, |x, y| {
        if (x - 50.0).hypot(y - 50.0) <= 40.0 {
            Some(BLUE)
        } else if (x - 90.0).hypot(y - 90.0) <= 3.5 {
            Some(RED)
        } else {
            None
        }
    });
    let res = trace(&img, &logo_params());
    assert!(res.paths.iter().any(|p| near(p.color, RED, 12)), "palette {:?}", res.palette);
    assert!(res.paths.iter().any(|p| near(p.color, BLUE, 12)));
}

#[test]
fn a_hard_edged_source_still_gets_smooth_circles() {
    // No anti-aliasing at all: every pixel is exactly one colour.
    let img = Raster::from_fn(80, 80, |x, y| {
        let d = (f64::from(x) + 0.5 - 40.0).hypot(f64::from(y) + 0.5 - 40.0);
        if (10.0..=28.0).contains(&d) { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] }
    });
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert!(res.anchor_count() <= 40, "{} anchors for two circles from a hard-edged source", res.anchor_count());
    let errs: Vec<f64> = samples(&res)
        .iter()
        .map(|p| {
            let d = p.distance(Point::new(40.0, 40.0));
            (d - 28.0).abs().min((d - 10.0).abs())
        })
        .collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    assert!(mean < 0.3 && errs.iter().copied().fold(0.0, f64::max) < 0.8, "mean {mean:.3}");
}

#[test]
fn random_small_images_and_settings_never_panic() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let weird = [f64::NAN, f64::INFINITY, -3.0, 0.0, 35.0, 60.0, 100.0, 1e12];
    let colours = [[210u8, 40, 50], [40, 80, 200], [255, 255, 255], [20, 160, 70], [0, 0, 0]];
    for _ in 0..250 {
        let (w, h) = ((next() % 24 + 1) as u32, (next() % 24 + 1) as u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let r = next();
                let c = colours[((u64::from(x) / 3 + u64::from(y) / 4 + r % 2) % 5) as usize];
                rgba.extend_from_slice(&match r % 11 {
                    0 => [0, 0, 0, 0],
                    1 => [(r >> 8) as u8, (r >> 16) as u8, (r >> 24) as u8, (r >> 32) as u8],
                    _ => [c[0], c[1], c[2], 255],
                });
            }
        }
        let img = Raster::new(w, h, rgba).unwrap();
        let p = TraceParams {
            paths: weird[(next() % 8) as usize],
            corners: weird[(next() % 8) as usize],
            noise: [0, 1, 6, 1000, u32::MAX][(next() % 5) as usize],
            colors: [0, 2, 6, 40][(next() % 4) as usize],
            ignore_white: next() % 2 == 0,
            snap_curves_to_lines: next() % 2 == 0,
            ..logo_params()
        };
        if let Ok(res) = trace_within(&img, &p, 20_000) {
            assert_sane(&res, w, h);
        }
    }
}

/// A scan's paper is darker in one corner than in another: it is still one colour, not a band of
/// paths for each tone of it.
#[test]
fn uneven_paper_is_one_colour() {
    let img = Raster::from_fn(160, 120, |x, y| {
        let shade = 1.0 - 0.1 * f64::from(x) / 160.0 - 0.04 * (f64::from(y) / 40.0).sin().abs();
        let ink = (40..120).contains(&x) && (30..90).contains(&y);
        let c = if ink { [40.0, 60.0, 150.0] } else { [236.0, 230.0, 214.0] };
        [(c[0] * shade) as u8, (c[1] * shade) as u8, (c[2] * shade) as u8, 255]
    });
    let res = trace(&img, &crate::preset("Flat Logo").unwrap());
    assert_eq!(res.paths.len(), 2, "{:?}", res.paths.iter().map(|p| (p.color, p.pixels)).collect::<Vec<_>>());
}

/// Line art: thin dark strokes and small red marks (2.4 px wide) on white paper, anti-aliased. No stroke fills a
/// uniform 3 × 3 neighbourhood, so the solid pixels are all paper; the strokes' colours must still
/// be found (from the strokes' cores) and traced, not cut as noise into a blank result.
#[test]
fn thin_line_art_on_paper_is_traced_not_blank() {
    const INK: [u8; 3] = [46, 36, 38];
    const MARK: [u8; 3] = [196, 38, 66];
    // Distance from the point to the segment a–b.
    let seg = |x: f64, y: f64, a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
    };
    let strokes = [((10.0, 12.0), (150.0, 30.0)), ((20.0, 110.0), (140.0, 60.0)), ((80.0, 8.0), (86.0, 112.0)), ((30.0, 40.0), (60.0, 100.0))];
    let marks = [((110.0, 95.0), (128.0, 101.0)), ((40.0, 20.0), (52.0, 18.0))];
    let img = render(160, 120, 0.0, |x, y| {
        Some(if marks.iter().any(|&(a, b)| seg(x, y, a, b) <= 1.2) {
            MARK
        } else if strokes.iter().any(|&(a, b)| seg(x, y, a, b) <= 1.2) {
            INK
        } else {
            [255, 255, 255]
        })
    });
    let res = trace_within(&img, &crate::preset("Flat Logo").unwrap(), 100_000).expect("line art is traced, not refused");
    assert!(res.paths.iter().any(|p| near(p.color, INK, 30)), "no ink path; palette {:?}", res.palette);
    assert!(res.paths.iter().any(|p| near(p.color, MARK, 30)), "no red mark; palette {:?}", res.palette);
    // The middle of each stroke and mark is painted with its colour (the topmost path there).
    let colour_at = |p: Point| res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color);
    for (&(a, b), colour) in strokes.iter().map(|s| (s, INK)).chain(marks.iter().map(|m| (m, MARK))) {
        for k in 1..8 {
            let t = f64::from(k) / 8.0;
            let p = Point::new(a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            let at = colour_at(p);
            assert!(at.is_some_and(|c| near(c, colour, 30)), "{p:?} is {at:?}, not {colour:?}");
        }
    }
}

/// Blank white paper with a light grain (a scan, a JPEG: each channel 243 to 255): the grain's
/// darkest specks are no ink, and the paper traces to nothing (a blank page stays blank).
#[test]
fn grainy_blank_paper_has_no_ink() {
    let grain = |x: u32, y: u32, k: u32| {
        let mut v = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ k.wrapping_mul(83_492_791)).wrapping_mul(2_654_435_761);
        v ^= v >> 15;
        (255 - (v % 13)) as u8
    };
    let img = Raster::from_fn(160, 120, |x, y| [grain(x, y, 1), grain(x, y, 2), grain(x, y, 3), 255]);
    let res = trace(&img, &crate::preset("Flat Logo").unwrap());
    assert!(res.palette.is_empty() && res.paths.is_empty(), "palette {:?}", res.palette);
}

/// White paper with 24 short grey marks (12 × 2 px) of one shade: light (239, 223) or mid grey
/// (191). They cover under 1% of the page, so it is paper with a few specks on it, not line art:
/// no colour is found and the page traces to nothing, as plain white paper does.
#[test]
fn sparse_marks_on_paper_are_not_promoted_to_colours() {
    let marks: Vec<(u32, u32)> = (0..24u32).map(|k| (10 + (k * 37) % 220, 10 + (k * 71) % 220)).collect();
    for shade in [239u8, 223, 191] {
        let img = Raster::from_fn(256, 256, |x, y| {
            let on = marks.iter().any(|&(mx, my)| (mx..mx + 12).contains(&x) && (my..my + 2).contains(&y));
            let v = if on { shade } else { 255 };
            [v, v, v, 255]
        });
        let res = trace(&img, &crate::preset("Flat Logo").unwrap());
        assert!(res.palette.is_empty(), "shade {shade}: palette {:?}", res.palette);
        assert!(res.paths.is_empty(), "shade {shade}: {} paths", res.paths.len());
    }
}

/// Bars two pixels wide (whole pixels of one color) on white beside a black disc, with the colors
/// found: no pixel of a bar has a 3 × 3 neighborhood of its own color, and a gray bar is a blend of
/// black and white. Light grays, a red and a pale yellow are found and traced in their own color. A
/// gray bar darker than about `#9B9B9B` stays black: a black stroke 1 to 1.6 px wide on the pixel
/// grid gives the same pixels. A single bar two pixels wide and three pixels long, down or across
/// the image, is found in its color too.
#[test]
fn a_bar_two_pixels_wide_beside_black_art_is_found_in_its_color() {
    const INK: [u8; 3] = [4, 4, 3];
    let layouts = [
        (
            vec![(50..52, 8..50), (60..110, 30..32)],
            vec![Point::new(51.0, 20.0), Point::new(51.0, 40.0), Point::new(70.0, 31.0), Point::new(100.0, 31.0)],
        ),
        (vec![(50..52, 20..23)], vec![Point::new(51.0, 21.5)]),
        (vec![(70..73, 30..32)], vec![Point::new(71.5, 31.0)]),
    ];
    for (bars, points) in &layouts {
        for (bar, want) in [
            ([170u8, 170, 170], None),
            ([200, 200, 200], None),
            ([230, 230, 230], None),
            ([210, 40, 50], None),
            ([250, 230, 150], None),
            ([128, 128, 128], Some(INK)),
            ([51, 51, 51], Some(INK)),
        ] {
            if bars.len() == 1 && bar == [128, 128, 128] {
                // Half ink and half paper, a single bar three pixels long is traced as paper.
                continue;
            }
            let img = Raster::from_fn(120, 60, |x, y| {
                let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let c = if (fx - 25.0).hypot(fy - 28.0) <= 18.0 {
                    INK
                } else if bars.iter().any(|(xs, ys)| xs.contains(&x) && ys.contains(&y)) {
                    bar
                } else {
                    [255, 255, 255]
                };
                [c[0], c[1], c[2], 255]
            });
            let res = trace(&img, &crate::preset("Flat Logo").unwrap());
            let colour_at = |p: Point| res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color);
            let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
            let want = want.unwrap_or(bar);
            for &at in points {
                assert_eq!(colour_at(at), Some(want), "bars {bars:?}, bar {bar:?}: {at:?}; {paths:?}");
            }
            assert_eq!(colour_at(Point::new(25.0, 28.0)), Some(INK), "bars {bars:?}, bar {bar:?}: the disc; {paths:?}");
        }
    }
}

/// Three rows of thin anti-aliased "lettering" (rings, slants and bars 1.6 px wide) in red, blue
/// and green on white. The lighter shades along the strokes' anti-aliased edges are blends of an
/// ink with the paper, not colours of their own: the palette is the three inks and white.
#[test]
fn anti_aliased_coloured_lettering_has_only_its_inks() {
    const INKS: [[u8; 3]; 3] = [[210, 40, 50], [30, 90, 200], [20, 130, 70]];
    let seg = |x: f64, y: f64, a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
    };
    // One "letter" in the cell at (cx, cy): a ring, a slanted stroke or a bar with a foot.
    let glyph = |x: f64, y: f64, k: usize, cx: f64, cy: f64| match k % 3 {
        0 => ((x - cx).hypot(y - cy) - 6.0).abs(),
        1 => seg(x, y, (cx - 5.0, cy + 8.0), (cx + 4.0, cy - 8.0)).min(seg(x, y, (cx - 1.0, cy), (cx + 6.0, cy + 7.5))),
        _ => seg(x, y, (cx - 0.3, cy - 8.0), (cx + 0.4, cy + 8.0)).min(seg(x, y, (cx - 5.0, cy + 8.2), (cx + 5.0, cy + 7.9))),
    };
    let img = render(256, 256, 0.0, |x, y| {
        for (row, ink) in INKS.iter().enumerate() {
            let cy = 40.0 + 80.0 * row as f64 + 0.37 * row as f64;
            for k in 0..14 {
                let cx = 14.0 + 17.3 * k as f64;
                if glyph(x, y, k + row, cx, cy) <= 0.8 {
                    return Some(*ink);
                }
            }
        }
        Some([255, 255, 255])
    });
    let res = trace(&img, &crate::preset("Flat Logo").unwrap());
    let colours: Vec<[u8; 3]> = res.palette.iter().copied().filter(|c| !c.iter().all(|&v| v >= 245)).collect();
    assert_eq!(colours.len(), 3, "palette {:?}", res.palette);
    for ink in INKS {
        assert!(colours.iter().any(|&c| near(c, ink, 12)), "{ink:?} missing from {:?}", res.palette);
    }
}

/// An outline whose fit has no area is left out. A pixel-sized black and cream noise, with both
/// colors named exactly, fits some specks to two anchors whose segments run along one line and
/// others to one anchor, which has no segment.
#[test]
fn an_outline_fitted_without_area_is_left_out() {
    use kurbo::ParamCurveArea;
    let noise = render(96, 96, 0.0, |x, y| {
        let n = ((x.floor() * 12.9898 + y.floor() * 78.233).sin() * 43758.5453).fract();
        Some(if n > 0.5 { [4, 4, 3] } else { [252, 248, 227] })
    });
    let res = trace(&noise, &TraceParams { logo_colors: vec!["#040403".into(), "#FCF8E3".into()], ..logo_params() });
    assert!(res.paths.len() > 1, "{} paths", res.paths.len());
    for t in &res.paths {
        for sp in &t.path.subpaths {
            let origin = sp.anchors[0].p.to_vec2();
            let area: f64 = (0..sp.segment_count()).map(|i| (kurbo::Affine::translate(-origin) * sp.segment(i)).signed_area()).sum();
            assert!(area.abs() >= 1e-6, "a {:?} subpath of {} anchors with area {area:e}", t.color, sp.anchors.len());
        }
    }
}

/// A black disc and a gray square on white, the gray lying on the line from black to white: the
/// disc's edge pixels are blends of black and white, so the gray is one shape (the square) and no
/// gray lines the disc. Colors found and named exactly, the darker gray also named 52 levels off;
/// sharp and soft edges.
#[test]
fn a_gray_between_black_and_white_is_one_shape() {
    const INK: [u8; 3] = [4, 4, 3];
    for gray in [[230u8, 230, 230], [200, 200, 200]] {
        for blur in [0.0, 0.6] {
            let img = render(120, 60, blur, |x, y| {
                Some(if (x - 30.0).hypot(y - 30.0) <= 20.0 {
                    INK
                } else if (60.0..=110.0).contains(&x) && (10.0..=50.0).contains(&y) {
                    gray
                } else {
                    WHITE
                })
            });
            // (A gray of 230 named 52 levels darker lies nearer white than its name, and is traced as white.)
            let offs: &[Option<u8>] = if gray[0] == 200 { &[None, Some(0), Some(52)] } else { &[None, Some(0)] };
            for &by in offs {
                let (p, gray_name) = match by {
                    None => (logo_params(), gray),
                    Some(by) => {
                        // Each name moved `by` levels toward mid-gray: the black lighter, the gray darker.
                        let names = [INK.map(|v| v.saturating_add(by)), gray.map(|v| v.saturating_sub(by))];
                        let logo_colors = names.iter().map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])).collect();
                        (TraceParams { logo_colors, ..logo_params() }, names[1])
                    }
                };
                let what = format!("gray {gray:?}, blur {blur}, names moved {by:?}");
                let res = trace(&img, &p);
                let grays = res.paths.iter().filter(|t| t.color == gray_name).count();
                let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
                assert_eq!(grays, 1, "{what}: {grays} gray paths; {paths:?}");
                assert_eq!(top_color(&res, Point::new(85.0, 30.0)), Some(gray_name), "{what}: the square");
                for k in 0..16 {
                    let a = f64::from(k) * std::f64::consts::TAU / 16.0;
                    let at = Point::new(30.0 + 20.0 * a.cos(), 30.0 + 20.0 * a.sin());
                    assert_ne!(top_color(&res, at), Some(gray_name), "{what}: the disc's edge at {at:?}");
                }
            }
        }
    }
}

/// Flat Logo with the black of the tests below, `gray` and white named exactly.
fn black_gray_white(gray: [u8; 3]) -> TraceParams {
    let logo_colors = vec!["#040403".into(), format!("#{:02X}{:02X}{:02X}", gray[0], gray[1], gray[2]), "#FFFFFF".into()];
    TraceParams { logo_colors, ..logo_params() }
}

/// A black disc and a gray square on white, blurred by 1 to 3 pixels: the edge pixels around the
/// disc as light as the gray read as blends of black and white, so the gray is one shape (the
/// square) and no gray lines the disc, with the colors found or named. A light gray (230) blurred by
/// 1.5 pixels leaves at most 4 slivers of up to 3 pixels, touching the disc's edge at no more than 4
/// of 192 samples. At a blur of 3 pixels, with the colors named, a gray of 128 is still one shape, a
/// gray of 200 leaves 4 slivers of no area, and a light gray leaves at most 8 slivers, touching the
/// disc's edge at no more than 4 of 192 samples. (Without named colors, an image blurred by 3 pixels
/// traces without its gray.)
#[test]
fn blurred_black_edges_beside_gray_leave_no_gray_slivers() {
    const INK: [u8; 3] = [4, 4, 3];
    for gray in [[230u8, 230, 230], [200, 200, 200], [128, 128, 128]] {
        for blur in [1.0, 1.5, 2.0, 3.0] {
            let img = render(120, 60, blur, |x, y| {
                Some(if (x - 30.0).hypot(y - 30.0) <= 20.0 {
                    INK
                } else if (60.0..=110.0).contains(&x) && (10.0..=50.0).contains(&y) {
                    gray
                } else {
                    WHITE
                })
            });
            for by_name in [false, true] {
                if !by_name && blur > 2.0 {
                    continue;
                }
                let res = trace(&img, &if by_name { black_gray_white(gray) } else { logo_params() });
                let what = format!("gray {}, blur {blur}, named {by_name}", gray[0]);
                let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
                assert_eq!(top_color(&res, Point::new(85.0, 30.0)), Some(gray), "{what}: the square; {paths:?}");
                let mut rim = 0;
                for r in [19.0, 20.0, 21.0] {
                    for k in 0..64 {
                        let a = f64::from(k) * std::f64::consts::TAU / 64.0;
                        if top_color(&res, Point::new(30.0 + r * a.cos(), 30.0 + r * a.sin())) == Some(gray) {
                            rim += 1;
                        }
                    }
                }
                let mut grays: Vec<_> = res.paths.iter().filter(|t| t.color == gray).map(|t| t.pixels).collect();
                grays.sort_unstable();
                grays.pop();
                match (gray[0], blur == 3.0) {
                    (230, false) if blur == 1.5 => {
                        assert!(grays.len() <= 4 && grays.iter().all(|&n| n <= 3), "{what}: slivers {grays:?}");
                        assert!(rim <= 4, "{what}: {rim} of 192 samples along the disc's edge traced gray");
                    }
                    (230, true) => {
                        assert!(grays.len() <= 8, "{what}: {} slivers; {paths:?}", grays.len());
                        assert!(rim <= 4, "{what}: {rim} of 192 samples along the disc's edge traced gray");
                    }
                    (200, true) => {
                        assert!(grays.len() <= 4 && grays.iter().all(|&n| n == 0), "{what}: slivers {grays:?}");
                        assert_eq!(rim, 0, "{what}: samples along the disc's edge traced gray");
                    }
                    _ => {
                        assert!(grays.is_empty(), "{what}: slivers {grays:?}; {paths:?}");
                        assert_eq!(rim, 0, "{what}: samples along the disc's edge traced gray");
                    }
                }
            }
        }
    }
}

/// A soft-edged gray band lying along a black disc traces as black and white, with at most one of
/// every 16 samples traced gray, when it is 2 to 4 pixels wide and blurred by 1.5 or 2 pixels, 2 or
/// 3 pixels wide and blurred by 3 pixels, or 2 pixels wide and blurred by 1 pixel: its pixels are
/// reached from the disc and from the white through pixels that keep getting darker and lighter, as
/// across a blurred black-to-white edge. A band 8 pixels wide keeps at least half of its gray at
/// blurs of 1 to 3 pixels. The bands 3 and 4 pixels wide of the three grays, blurred by 1 pixel,
/// keep about 60 of their 336 samples gray, and those 4 pixels wide, blurred by 3 pixels, about 39
/// of 192. With sharp edges every band stays gray, and with edges blurred by 0.6 pixels a band 3 to
/// 8 pixels wide keeps at least 4 of every 5 samples gray, and the bands 2 to 4 pixels wide of the
/// three grays keep about 306 of their 432 samples gray: beside a sharp edge, a path that comes to a
/// pixel of the gray stops there.
#[test]
fn a_soft_gray_band_along_black_traces_as_black_and_white() {
    const INK: [u8; 3] = [4, 4, 3];
    // The samples traced gray over the three grays: of the bands 2 to 4 pixels wide blurred by 0.6
    // pixels, of the bands 3 and 4 pixels wide blurred by 1 pixel, and of the bands 4 pixels wide
    // blurred by 3 pixels.
    let (mut nearly_sharp, mut blur_one, mut blur_three) = (0, 0, 0);
    for gray in [[230u8, 230, 230], [200, 200, 200], [128, 128, 128]] {
        for blur in [0.0, 0.6, 1.0, 1.5, 2.0, 3.0] {
            for wide in [2u8, 3, 4, 8] {
                let img = render(120, 60, blur, |x, y| {
                    Some(if (x - 25.0).hypot(y - 28.0) <= 18.0 {
                        INK
                    } else if (40.0..43.0 + f64::from(wide)).contains(&x) && (20.0..36.0).contains(&y) {
                        gray
                    } else {
                        WHITE
                    })
                });
                let res = trace(&img, &black_gray_white(gray));
                // The band's samples: 16 rows of its `wide` columns outside the disc.
                let kept = (0..wide)
                    .flat_map(|c| (20..36).map(move |y| Point::new(43.5 + f64::from(c), f64::from(y) + 0.5)))
                    .filter(|&at| top_color(&res, at) == Some(gray))
                    .count();
                let all = 16 * usize::from(wide);
                let what = format!("gray {}, blur {blur}, {wide} px band: {kept} of {all} samples traced gray", gray[0]);
                let lost = if blur == 1.0 {
                    wide <= 2
                } else if blur == 3.0 {
                    wide <= 3
                } else {
                    wide <= 4
                };
                if blur == 0.0 {
                    assert_eq!(kept, all, "{what}");
                } else if blur == 0.6 {
                    if wide >= 3 {
                        assert!(5 * kept >= 4 * all, "{what}");
                    }
                    if wide <= 4 {
                        nearly_sharp += kept;
                    }
                } else if lost {
                    assert!(16 * kept <= all, "{what}");
                } else if wide == 8 {
                    assert!(2 * kept >= all, "{what}");
                } else if blur == 1.0 {
                    blur_one += kept;
                } else if blur == 3.0 {
                    blur_three += kept;
                }
            }
        }
    }
    assert!((300..=312).contains(&nearly_sharp), "bands 2 to 4 px wide blurred by 0.6 px: {nearly_sharp} of 432 samples traced gray");
    assert!((56..=64).contains(&blur_one), "bands 3 and 4 px wide blurred by 1 px: {blur_one} of 336 samples traced gray");
    assert!((36..=42).contains(&blur_three), "bands 4 px wide blurred by 3 px: {blur_three} of 192 samples traced gray");
}

/// A soft-edged gray spoke two or three pixels wide running straight out of a black disc, blurred
/// by 1 or 1.5 pixels, loses 3 or 4 of its 16 samples nearest the disc and keeps the rest. Blurred
/// by 2 pixels, a spoke three pixels wide loses 2 or 3 of them, and one two pixels wide traces as
/// black and white along its whole length.
#[test]
fn a_soft_gray_spoke_loses_a_few_pixels_where_it_meets_black() {
    const INK: [u8; 3] = [4, 4, 3];
    for gray in [[230u8, 230, 230], [128, 128, 128]] {
        for blur in [1.0, 1.5, 2.0] {
            for wide in [2.0, 3.0] {
                let img = render(120, 60, blur, |x, y| {
                    Some(if (x - 25.0).hypot(y - 28.0) <= 18.0 {
                        INK
                    } else if (38.0..90.0).contains(&x) && (27.0..27.0 + wide).contains(&y) {
                        gray
                    } else {
                        WHITE
                    })
                });
                let res = trace(&img, &black_gray_white(gray));
                let at = |x: i32| Point::new(f64::from(x) + 0.5, 27.0 + wide / 2.0);
                let near = (44..60).filter(|&x| top_color(&res, at(x)) == Some(gray)).count();
                let far = (60..86).filter(|&x| top_color(&res, at(x)) == Some(gray)).count();
                let what = format!("gray {}, blur {blur}, {wide} px spoke", gray[0]);
                if blur == 2.0 && wide == 2.0 {
                    assert_eq!(near + far, 0, "{what}: {near} and {far} samples traced gray");
                    continue;
                }
                let kept = if blur == 2.0 { 13..=14 } else { 12..=13 };
                assert!(kept.contains(&near), "{what}: {near} of the 16 samples nearest the disc traced gray");
                assert_eq!(far, 26, "{what}: the rest of the spoke");
            }
        }
    }
}

/// A soft-edged light gray stroke 2 pixels wide beside a black disc, blurred by 1.5 pixels: 2
/// pixels from the disc it keeps about 10 of its 34 samples gray, 4 pixels away about 25, and 6 or
/// 8 pixels away all of them.
#[test]
fn a_soft_gray_stroke_near_black_loses_gray_by_its_distance() {
    const INK: [u8; 3] = [4, 4, 3];
    const GRAY: [u8; 3] = [230, 230, 230];
    for gap in [2, 4, 6, 8] {
        let x0 = 43.0 + f64::from(gap);
        let img = render(120, 60, 1.5, |x, y| {
            Some(if (x - 25.0).hypot(y - 28.0) <= 18.0 {
                INK
            } else if (x0..x0 + 2.0).contains(&x) && (8.0..50.0).contains(&y) {
                GRAY
            } else {
                WHITE
            })
        });
        let res = trace(&img, &black_gray_white(GRAY));
        let kept = (12..46).filter(|&y| top_color(&res, Point::new(x0 + 1.0, f64::from(y) + 0.5)) == Some(GRAY)).count();
        let what = format!("{gap} px from the disc: {kept} of 34 samples traced gray");
        match gap {
            2 => assert!((7..=13).contains(&kept), "{what}"),
            4 => assert!((22..=28).contains(&kept), "{what}"),
            _ => assert_eq!(kept, 34, "{what}"),
        }
    }
}

/// A one-pixel gray line between flat black and flat white, with the three colors named, stays a
/// gray line along its whole length, beside a black disc whose edge pixels are as light as the gray.
#[test]
fn a_one_pixel_gray_line_between_black_and_white_stays_gray() {
    const INK: [u8; 3] = [4, 4, 3];
    for gray in [[230u8, 230, 230], [200, 200, 200], [128, 128, 128]] {
        let img = render(120, 80, 0.0, |x, y| {
            Some(if x < 40.0 || (x - 90.0).hypot(y - 40.0) <= 15.0 {
                INK
            } else if x < 41.0 {
                gray
            } else {
                WHITE
            })
        });
        let logo_colors = vec!["#040403".into(), format!("#{:02X}{:02X}{:02X}", gray[0], gray[1], gray[2]), "#FFFFFF".into()];
        let res = trace(&img, &TraceParams { logo_colors, ..logo_params() });
        let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
        for y in 2..78 {
            let at = Point::new(40.5, f64::from(y) + 0.5);
            assert_eq!(top_color(&res, at), Some(gray), "gray {gray:?}: the line at {at:?}; {paths:?}");
        }
        for k in 0..16 {
            let a = f64::from(k) * std::f64::consts::TAU / 16.0;
            let at = Point::new(90.0 + 15.0 * a.cos(), 40.0 + 15.0 * a.sin());
            assert_ne!(top_color(&res, at), Some(gray), "gray {gray:?}: the disc's edge at {at:?}");
        }
    }
}

/// A gray stroke 12 pixels long touching a black disc on white, with the three colors named: the
/// stroke's pixels join the disc's edge pixels that are as light as the gray, but no gray lines the
/// rest of the disc's edge, and a sharp stroke stays gray. Strokes two and three pixels wide outside
/// the disc, on and off the pixel grid, sharp and soft. A soft stroke about two pixels wide that
/// touches black is traced as black and white, and one about three pixels wide in part, so the
/// stroke's own color is checked on sharp strokes.
#[test]
fn a_gray_stroke_touching_a_black_disc_leaves_its_edge_black_and_white() {
    const INK: [u8; 3] = [4, 4, 3];
    for gray in [[230u8, 230, 230], [200, 200, 200], [128, 128, 128]] {
        for blur in [0.0, 0.6] {
            // The disc reaches x = 42 to 43 along the stroke's rows; each stroke starts inside it.
            for (sx, wide) in [(41.0, 4.0), (41.5, 4.0), (41.0, 5.0)] {
                let img = render(120, 60, blur, |x, y| {
                    Some(if (x - 25.0).hypot(y - 28.0) <= 18.0 {
                        INK
                    } else if (sx..sx + wide).contains(&x) && (22.0..34.0).contains(&y) {
                        gray
                    } else {
                        WHITE
                    })
                });
                let logo_colors = vec!["#040403".into(), format!("#{:02X}{:02X}{:02X}", gray[0], gray[1], gray[2]), "#FFFFFF".into()];
                let res = trace(&img, &TraceParams { logo_colors, ..logo_params() });
                let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
                let what = format!("gray {gray:?}, blur {blur}, stroke at {sx} {wide} px wide");
                if blur == 0.0 {
                    for y in 24..32 {
                        let at = Point::new(sx + wide - 1.0, f64::from(y) + 0.5);
                        assert_eq!(top_color(&res, at), Some(gray), "{what}: the stroke at {at:?}; {paths:?}");
                    }
                }
                // Points on the disc's edge more than 30 degrees from the stroke (which spans about 20).
                for k in 6..=58 {
                    let a = f64::from(k) * std::f64::consts::TAU / 64.0;
                    let at = Point::new(25.0 + 18.0 * a.cos(), 28.0 + 18.0 * a.sin());
                    assert_ne!(top_color(&res, at), Some(gray), "{what}: the disc's edge at {at:?}; {paths:?}");
                }
            }
        }
    }
}

/// Stripes on white (black, a dark gray with a black disc on it, a light gray), with the colors
/// named 52 levels off: each gray then lies on the line between two other names, and the dark
/// stripe's rows beside white keep the stripe's color above and below the disc. Sharp and soft edges,
/// with and without white named.
#[test]
fn a_gray_stripe_beside_white_keeps_its_color_beside_a_black_disc() {
    const INK: [u8; 3] = [5, 5, 3];
    for blur in [0.0, 0.6] {
        let img = render(300, 160, blur, |x, y| {
            let inside = |x0: f64, y0: f64, x1: f64, y1: f64| (x0..x1).contains(&x) && (y0..y1).contains(&y);
            Some(if (x - 150.0).hypot(y - 80.0) <= 12.0 || inside(20.0, 20.0, 280.0, 50.0) {
                INK
            } else if inside(20.0, 65.0, 280.0, 95.0) {
                [100, 100, 100]
            } else if inside(20.0, 110.0, 280.0, 140.0) {
                [190, 190, 190]
            } else {
                WHITE
            })
        });
        for white in [false, true] {
            let mut logo_colors: Vec<String> = ["#393937", "#989898", "#8A8A8A"].iter().map(|s| (*s).to_string()).collect();
            if white {
                logo_colors.push("#FFFFFF".into());
            }
            let res = trace(&img, &TraceParams { logo_colors, ..logo_params() });
            let what = format!("blur {blur}, white named {white}");
            let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
            let stripe = top_color(&res, Point::new(60.0, 80.0));
            assert_ne!(stripe, Some([0x39, 0x39, 0x37]), "{what}: the stripe; {paths:?}");
            for x in [140.0, 145.0, 150.0, 155.0, 160.0] {
                for y in [65.5, 94.5] {
                    assert_eq!(top_color(&res, Point::new(x, y)), stripe, "{what}: ({x}, {y}); {paths:?}");
                }
            }
            if blur == 0.0 {
                let inks = res.paths.iter().filter(|t| t.color == [0x39, 0x39, 0x37]).count();
                assert_eq!(inks, 2, "{what}: {inks} paths in the black's name; {paths:?}");
            }
        }
    }
}

/// Gray strokes two and three pixels wide on white (whole pixels of the gray), 3 and 4 px from a
/// black disc, and a two-pixel stroke six pixels long, fewer pixels than a line needs to keep its
/// class, 2 to 4.5 px from the disc: the strokes' pixels are blends of black and white too, but nothing
/// darker than the gray lies beside them, and they stay gray.
#[test]
fn thin_gray_strokes_beside_black_stay_gray() {
    const INK: [u8; 3] = [4, 4, 3];
    for gray in [[230u8, 230, 230], [200, 200, 200]] {
        for wide in [2.0, 3.0] {
            let img = render(120, 60, 0.0, |x, y| {
                Some(if (x - 25.0).hypot(y - 28.0) <= 18.0 {
                    INK
                } else if ((47.0..47.0 + wide).contains(&x) && (8.0..50.0).contains(&y))
                    || ((8.0..110.0).contains(&x) && (49.0..49.0 + wide).contains(&y))
                    || ((44.0..46.0).contains(&x) && (16.0..22.0).contains(&y))
                {
                    gray
                } else {
                    WHITE
                })
            });
            let logo_colors = vec!["#040403".into(), format!("#{:02X}{:02X}{:02X}", gray[0], gray[1], gray[2])];
            let res = trace(&img, &TraceParams { logo_colors, ..logo_params() });
            let what = format!("gray {gray:?}, {wide} px strokes");
            let short = (17..21).map(|y| Point::new(45.0, f64::from(y) + 0.5));
            let long = [Point::new(47.0 + wide / 2.0, 20.0), Point::new(47.0 + wide / 2.0, 30.0), Point::new(25.0, 49.0 + wide / 2.0)];
            for at in long.into_iter().chain(short) {
                let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
                assert_eq!(top_color(&res, at), Some(gray), "{what}: {at:?}; {paths:?}");
            }
        }
    }
}

/// A sharp black disc and a mid-gray square on white, with up to 2 levels of noise added to each
/// pixel: the edge pixels as light as the gray, read as blends of black and white, still count as
/// unblended when judging whether the source is aliased, so the trace is softened as without them
/// and no gray lines the disc.
#[test]
fn a_noisy_hard_edged_gray_between_black_and_white_is_one_shape() {
    const GRAY: [u8; 3] = [128, 128, 128];
    let mut img = render(120, 60, 0.0, |x, y| {
        Some(if (x - 30.0).hypot(y - 30.0) <= 20.0 {
            [0, 0, 0]
        } else if (60.0..=110.0).contains(&x) && (10.0..=50.0).contains(&y) {
            GRAY
        } else {
            WHITE
        })
    });
    for (i, px) in img.rgba.chunks_mut(4).enumerate() {
        let mut h = (i as u64 ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0x2545_F491_4F6C_DD1D);
        h ^= h >> 29;
        let d = (h % 5) as i32 - 2;
        for c in px.iter_mut().take(3) {
            *c = (i32::from(*c) + d).clamp(0, 255) as u8;
        }
    }
    for named in [false, true] {
        let p = if named {
            TraceParams { logo_colors: vec!["#000000".into(), "#808080".into(), "#FFFFFF".into()], ..logo_params() }
        } else {
            logo_params()
        };
        let res = trace(&img, &p);
        let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
        let grays = res.paths.iter().filter(|t| t.color == GRAY).count();
        assert_eq!(grays, 1, "named {named}: {grays} gray paths; {paths:?}");
        assert_eq!(top_color(&res, Point::new(85.0, 30.0)), Some(GRAY), "named {named}: the square");
        for k in 0..16 {
            let a = f64::from(k) * std::f64::consts::TAU / 16.0;
            let at = Point::new(30.0 + 20.0 * a.cos(), 30.0 + 20.0 * a.sin());
            assert_ne!(top_color(&res, at), Some(GRAY), "named {named}: the disc's edge at {at:?}; {paths:?}");
        }
    }
}

#[test]
fn a_disc_on_a_gray_panel_on_a_paler_ground_has_no_slivers_of_the_ground() {
    // The panel's gray lies on the line from the ground's gray to the disc's charcoal, so the disc's
    // edge pixels are also a mix of the ground and the disc.
    let (ground, panel, disc) = ([240, 240, 240], [200, 200, 200], [60, 60, 60]);
    for blur in [0.0, 0.7] {
        let grays = render(150, 150, blur, move |x, y| {
            let (cx, cy) = (x.clamp(35.0, 115.0), y.clamp(35.0, 115.0));
            Some(if (x - 75.0).hypot(y - 75.0) < 30.0 {
                disc
            } else if (20.0..130.0).contains(&x) && (20.0..130.0).contains(&y) && (x - cx).hypot(y - cy) < 15.0 {
                panel
            } else {
                ground
            })
        });
        for named in [false, true] {
            let logo_colors = if named { vec!["#F0F0F0".into(), "#C8C8C8".into(), "#3C3C3C".into()] } else { Vec::new() };
            let res = trace(&grays, &TraceParams { logo_colors, ..logo_params() });
            let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
            assert_eq!(res.paths.len(), 3, "blur {blur}, named {named}: one path per color; {paths:?}");
            for k in 0..16 {
                let a = f64::from(k) * std::f64::consts::TAU / 16.0;
                let at = Point::new(75.0 + 31.0 * a.cos(), 75.0 + 31.0 * a.sin());
                assert_eq!(top_color(&res, at), Some(panel), "blur {blur}, named {named}: beside the disc at {at:?}");
            }
            assert_eq!(top_color(&res, Point::new(8.0, 75.0)), Some(ground), "blur {blur}, named {named}: the ground");
            assert_eq!(top_color(&res, Point::new(75.0, 75.0)), Some(disc), "blur {blur}, named {named}: the disc");
        }
    }
}

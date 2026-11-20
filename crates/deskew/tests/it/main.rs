//! Synthetic pages: rectangle "glyphs" in lines, drawn at a known tilt.

use craft_deskew::{Error, Image, Layout, Method, Options, Outcome, Paper, Reason, estimate_skew, rotate, straighten};

const W: u32 = 1000;
const H: u32 = 1300;

/// Whether `(x, y)` is ink on a page of text lines tilted `degrees` counter-clockwise as
/// displayed: lines every 30 px, glyphs 8 × 12 px with word gaps, in a 100 px margin.
fn text_ink(x: u32, y: u32, degrees: f64) -> bool {
    text_band(x, y, degrees, 100.0, f64::from(H) - 100.0)
}

/// [`text_ink`] with the lines only between `top` and `bottom` on the level page.
fn text_band(x: u32, y: u32, degrees: f64, top: f64, bottom: f64) -> bool {
    text_on((W, H), x, y, degrees, top, bottom)
}

/// [`text_band`] on a page of `size`.
fn text_on((w, h): (u32, u32), x: u32, y: u32, degrees: f64, top: f64, bottom: f64) -> bool {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let (u, v) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
    // Back to the level page.
    let px = u * cos - v * sin + cx;
    let py = u * sin + v * cos + cy;
    if !(100.0..f64::from(w) - 100.0).contains(&px) || !(top..bottom).contains(&py) {
        return false;
    }
    let (col, row) = (px as u32, py as u32);
    let in_line = row % 30 < 12;
    let glyph = (col / 12) % 7 != 6 && col % 12 < 8;
    in_line && glyph
}

/// Pack an ink mask into `layout`, ink as `ink` and paper as `paper` (one value per component).
fn pack(layout: Layout, ink: impl Fn(u32, u32) -> bool, ink_px: &[u32], paper_px: &[u32]) -> Vec<u8> {
    let bpc = usize::from(layout.bits_per_component);
    let comps = usize::from(layout.components);
    let row_bytes = (layout.width as usize * comps * bpc).div_ceil(8);
    let mut out = vec![0u8; row_bytes * layout.height as usize];
    for y in 0..layout.height {
        for x in 0..layout.width {
            let px = if ink(x, y) { ink_px } else { paper_px };
            for (c, &value) in px.iter().enumerate() {
                let bit = (x as usize * comps + c) * bpc;
                let at = y as usize * row_bytes + bit / 8;
                match bpc {
                    16 => {
                        out[at] = (value >> 8) as u8;
                        out[at + 1] = value as u8;
                    }
                    8 => out[at] = value as u8,
                    _ => out[at] |= (value as u8) << (8 - bpc - bit % 8),
                }
            }
        }
    }
    out
}

fn grey_page(degrees: f64) -> Vec<u8> {
    pack(Layout::grey8(W, H), |x, y| text_ink(x, y, degrees), &[0], &[255])
}

fn measure(layout: Layout, samples: &[u8]) -> Option<f64> {
    let image = Image::new(layout, samples).unwrap();
    estimate_skew(&image, &Options::default()).map(|s| s.angle_degrees)
}

#[test]
fn recovers_text_skew_across_the_range() {
    for degrees in [-12.0, -5.0, -2.0, -0.5, -0.1, 0.0, 0.1, 0.3, 2.0, 5.0, 12.0] {
        let got = measure(Layout::grey8(W, H), &grey_page(degrees)).unwrap();
        assert!((got - degrees).abs() < 0.1, "{degrees}° measured as {got}°");
    }
}

/// Every supported layout straightens a 3° page, and its result measures level.
#[test]
fn straightens_every_layout_at_its_own_depth() {
    let layout = |components, bits_per_component, paper| Layout::new(W, H, components, bits_per_component, paper);
    let cases: [(Layout, &[u32], &[u32]); 6] = [
        (layout(1, 1, Paper::Max), &[0], &[1]),
        (layout(1, 4, Paper::Max).as_indexed(), &[3], &[9]),
        (layout(1, 8, Paper::Max), &[0], &[255]),
        (layout(3, 8, Paper::Max), &[20, 20, 20], &[255, 255, 255]),
        (layout(1, 16, Paper::Max), &[0], &[65535]),
        (layout(4, 8, Paper::Zero), &[0, 0, 0, 255], &[0, 0, 0, 0]),
    ];
    for (layout, ink, blank) in cases {
        let (bits, components) = (layout.bits_per_component, layout.components);
        let samples = pack(layout, |x, y| text_ink(x, y, 3.0), ink, blank);
        let image = Image::new(layout, &samples).unwrap();
        let Outcome::Straightened { skew, residual_degrees, samples: level } = straighten(&image, &Options::default()) else {
            panic!("{bits}-bit × {components} was not straightened");
        };
        assert!((skew.angle_degrees - 3.0).abs() < 0.1, "{bits}-bit × {components}: {}", skew.angle_degrees);
        assert!(residual_degrees.abs() < 0.1);
        assert_eq!(level.len(), samples.len());
        assert!(measure(layout, &level).unwrap().abs() < 0.1);
    }
}

#[test]
fn bilevel_and_indexed_rotation_invents_no_values() {
    let layout = Layout::new(W, H, 1, 4, Paper::Max).as_indexed();
    let samples = pack(layout, |x, y| text_ink(x, y, 0.0), &[3], &[9]);
    let turned = rotate(&Image::new(layout, &samples).unwrap(), 2.0);
    let values: std::collections::BTreeSet<u8> = turned.iter().flat_map(|b| [b >> 4, b & 15]).collect();
    assert!(values.is_subset(&[3, 9].into()), "palette indices blended: {values:?}");
}

#[test]
fn uncovered_corners_take_the_paper_tint() {
    let tint = [240, 230, 200];
    let layout = Layout::new(W, H, 3, 8, Paper::Max);
    let samples = pack(layout, |x, y| text_ink(x, y, 0.0), &[0, 0, 0], &tint);
    let turned = rotate(&Image::new(layout, &samples).unwrap(), 5.0);
    assert_eq!(&turned[..3], &[240u8, 230, 200]);
}

/// A dark scanner border is square to the image, not to the page. Left in, it pulls a sparse
/// page's angle towards zero.
#[test]
fn a_dark_scanner_border_does_not_pull_the_angle_to_zero() {
    let frame = |x: u32, y: u32| x < 70 || y < 70 || x >= W - 70 || y >= H - 70;
    let samples = pack(Layout::grey8(W, H), |x, y| frame(x, y) || text_band(x, y, 2.0, 500.0, 600.0), &[0], &[255]);
    let got = measure(Layout::grey8(W, H), &samples).unwrap();
    assert!((got - 2.0).abs() < 0.1, "measured {got}°");
}

/// A 300 dpi A4 page is measured on a subsampled grid; its thin strokes must survive it.
#[test]
fn a_full_size_bilevel_scan_is_straightened() {
    let size = (2480, 3508);
    let layout = Layout::new(size.0, size.1, 1, 1, Paper::Max);
    let bottom = f64::from(size.1) - 100.0;
    let samples = pack(layout, |x, y| text_on(size, x, y, -1.5, 100.0, bottom), &[0], &[1]);
    let outcome = straighten(&Image::new(layout, &samples).unwrap(), &Options::default());
    let Outcome::Straightened { skew, residual_degrees, .. } = outcome else {
        panic!("not straightened: {outcome:?}");
    };
    assert!((skew.angle_degrees + 1.5).abs() < 0.1, "measured {}°", skew.angle_degrees);
    assert!(residual_degrees.abs() < skew.angle_degrees.abs());
}

/// Black printed as K alone is ink. Straight rules have no text corners, and level ones no
/// stair-steps, to rescue them through the edge vote.
#[test]
fn black_rules_printed_in_k_alone_are_measured() {
    for degrees in [0.0, 3.0] {
        let (sin, cos) = f64::to_radians(degrees).sin_cos();
        let rules = |x: u32, y: u32| {
            let (u, v) = (f64::from(x) - 500.0, f64::from(y) - 650.0);
            let (along, across) = (u * cos - v * sin, u * sin + v * cos);
            along.abs() < 400.0 && (across + 500.0).rem_euclid(200.0) < 2.0
        };
        let layout = Layout::new(W, H, 4, 8, Paper::Zero);
        let samples = pack(layout, rules, &[0, 0, 0, 255], &[0, 0, 0, 0]);
        let got = measure(layout, &samples);
        assert!(got.is_some_and(|got| (got - degrees).abs() < 0.1), "{degrees}° measured as {got:?}");
    }
}

/// One long rule is too little ink to project, but its edges still vote.
#[test]
fn a_sparse_page_is_measured_by_its_edges() {
    let rule = |x: u32, y: u32| {
        let (u, v) = (f64::from(x) - 500.0, f64::from(y) - 650.0);
        let along = u * 4f64.to_radians().cos() - v * 4f64.to_radians().sin();
        let across = u * 4f64.to_radians().sin() + v * 4f64.to_radians().cos();
        along.abs() < 400.0 && across.abs() < 1.0
    };
    let samples = pack(Layout::grey8(W, H), rule, &[0], &[255]);
    let image = Image::new(Layout::grey8(W, H), &samples).unwrap();
    let skew = estimate_skew(&image, &Options::default()).unwrap();
    assert_eq!(skew.method, Method::Edges);
    assert!((skew.angle_degrees - 4.0).abs() < 0.2, "measured {}°", skew.angle_degrees);
    let projection_only = Options::default().edge_fallback(false);
    assert_eq!(estimate_skew(&image, &projection_only), None);
}

#[test]
fn blank_and_solid_pages_are_unmeasured_not_straight() {
    for value in [255u8, 0] {
        let samples = vec![value; (W * H) as usize];
        let outcome = straighten(&Image::new(Layout::grey8(W, H), &samples).unwrap(), &Options::default());
        assert_eq!(outcome, Outcome::Declined(Reason::NothingToMeasure), "value {value}");
    }
}

/// Text turned 90° has vertical lines: it must not be "straightened" by some large angle.
#[test]
fn a_page_turned_on_its_side_is_not_straightened() {
    let samples = pack(Layout::grey8(W, H), |x, y| text_ink(y * W / H, x * H / W, 0.0), &[0], &[255]);
    let outcome = straighten(&Image::new(Layout::grey8(W, H), &samples).unwrap(), &Options::default());
    if let Outcome::Straightened { skew, .. } = outcome {
        panic!("turned page straightened by {}°", skew.angle_degrees);
    }
}

#[test]
fn hostile_layouts_are_errors_and_odd_shapes_do_not_panic() {
    let grey = |w, h| Layout::grey8(w, h);
    assert!(matches!(Image::new(grey(0, 5), &[]), Err(Error::Layout(_))));
    assert!(matches!(Image::new(grey(5, 5), &[0; 24]), Err(Error::ShortBuffer { .. })));
    assert!(matches!(Image::new(grey(u32::MAX, u32::MAX), &[]), Err(Error::TooLarge { .. })));
    let odd = |bits, components| Layout::new(4, 4, components, bits, Paper::Max);
    for layout in [odd(3, 1), odd(8, 0), odd(8, 33), odd(8, 3).as_indexed()] {
        assert!(matches!(Image::new(layout, &[0; 4096]), Err(Error::Layout(_))), "{layout:?}");
    }
    for (w, h) in [(1, 1), (1, 100_000), (63, 63), (100_000, 2)] {
        let samples = vec![0u8; (w * h) as usize];
        let image = Image::new(grey(w, h), &samples).unwrap();
        let _ = straighten(&image, &Options::default());
        assert_eq!(rotate(&image, 7.0).len(), samples.len());
    }
}

/// Scattered dots, sparse or dense, have no lines: whatever they measure, it is not acted on.
#[test]
fn noise_is_never_straightened() {
    let mut state = 0x2545_f491_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for per_mille in [1, 500] {
        let samples: Vec<u8> = (0..W * H).map(|_| if next() % 1000 < per_mille { 0 } else { 255 }).collect();
        let outcome = straighten(&Image::new(Layout::grey8(W, H), &samples).unwrap(), &Options::default());
        assert!(!matches!(outcome, Outcome::Straightened { .. }), "{per_mille}‰ noise: {outcome:?}");
    }
}

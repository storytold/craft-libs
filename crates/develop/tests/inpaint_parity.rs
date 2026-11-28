//! Parity with LightCraft (craft-libs#587): the hole fills of one scene are what `lightcraft-inpaint` at
//! `a75497d6` gave for it. The fixtures in `tests/fixtures/` are that crate's `complete` (default parameters) and
//! `membrane_fill` on [`scene`], three little-endian `f32` per pixel, row by row.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use craft_develop::inpaint::{CompleteParams, complete, membrane_fill};
use craft_render_parity::{Image, Tolerance, compare};

const W: usize = 96;
const H: usize = 72;

/// A ramp with stripes and a weave, and two holes in it: a rectangle and a disc.
fn scene() -> (Vec<f32>, Vec<bool>) {
    let mut img = Vec::with_capacity(W * H * 3);
    let mut hole = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let (fx, fy) = (x as f32 / W as f32, y as f32 / H as f32);
            let stripe = if (x / 6 + y / 9) % 2 == 0 { 0.12 } else { 0.0 };
            let weave = if (x * 3 + y * 5) % 7 < 2 { 0.05 } else { 0.0 };
            img.extend([0.15 + 0.6 * fx + stripe, 0.2 + 0.5 * fy + weave, 0.7 - 0.4 * fx * fy + stripe * 0.5]);
            let (dx, dy) = (x as i32 - 60, y as i32 - 44);
            hole.push((20..38).contains(&x) && (14..30).contains(&y) || dx * dx + dy * dy <= 81);
        }
    }
    (img, hole)
}

fn fixture(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

fn assert_parity(expected: &[u8], actual: &[f32]) {
    let expected = fixture(expected);
    let report = compare(&Image::new(W as u32, H as u32, &expected).unwrap(), &Image::new(W as u32, H as u32, actual).unwrap()).unwrap();
    assert!(report.within(&Tolerance::BASELINE), "{report}: {:?}", report.exceeded(&Tolerance::BASELINE));
}

#[test]
fn completion_matches_lightcraft() {
    let (img, hole) = scene();
    let filled = complete(W, H, 3, &img, &hole, &CompleteParams::default()).expect("a completion");
    assert_ne!(filled, img, "the holes were filled");
    assert_parity(include_bytes!("fixtures/inpaint-complete.f32le"), &filled);
}

#[test]
fn membrane_fill_matches_lightcraft() {
    let (img, hole) = scene();
    assert_parity(include_bytes!("fixtures/inpaint-membrane.f32le"), &membrane_fill(W, H, 3, &img, &hole));
}

/// The comparison can fail: the unfilled scene is not within tolerance of the filled one.
#[test]
fn the_unfilled_scene_is_not_at_parity() {
    let (img, _) = scene();
    let expected = fixture(include_bytes!("fixtures/inpaint-complete.f32le"));
    let report = compare(&Image::new(W as u32, H as u32, &expected).unwrap(), &Image::new(W as u32, H as u32, &img).unwrap()).unwrap();
    assert!(!report.within(&Tolerance::BASELINE));
}

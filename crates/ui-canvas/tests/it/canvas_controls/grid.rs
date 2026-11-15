//! The canvas grid and the pixel grid (20.6): line counts, the per-axis cap, subdivisions, and the pixel grid's maths and
//! region cache.

use craft_ui::canvas::PixelRect;
use craft_ui::canvas_controls::{GridSegment, PixelGrid};
use craft_ui::testing::trial;
use egui::Color32;
use std::collections::BTreeMap;

/// The colour of the pixel at (`x`, `y`) of a document the tests draw: some see-through, the rest in four greys.
fn pixel(x: i32, y: i32, shift: i32) -> [f32; 4] {
    let grey = ((x * 3 + y * 2 + shift).rem_euclid(4)) as f32 / 3.0;
    [grey, grey, grey, if (x + y).rem_euclid(5) == 0 { 0.0 } else { 1.0 }]
}

fn pixels(rect: PixelRect, shift: impl Fn(i32, i32) -> i32) -> Vec<[f32; 4]> {
    (rect.y0..rect.y1).flat_map(|y| (rect.x0..rect.x1).map(move |x| (x, y))).map(|(x, y)| pixel(x, y, shift(x, y))).collect()
}

/// Every unit edge the segments cover, with its colour, so runs split differently still compare equal.
fn edges(segments: &[GridSegment]) -> BTreeMap<(bool, i32, i32), Color32> {
    let mut edges = BTreeMap::new();
    for segment in segments {
        if segment.from[0] == segment.to[0] {
            (segment.from[1]..segment.to[1]).for_each(|y| {
                edges.insert((true, segment.from[0], y), segment.color);
            });
        } else {
            (segment.from[0]..segment.to[0]).for_each(|x| {
                edges.insert((false, x, segment.from[1]), segment.color);
            });
        }
    }
    edges
}

#[trial]
fn redrawing_a_damaged_part_of_the_pixel_grid_matches_drawing_the_region_again() {
    let region = PixelRect::new(0, 0, 12, 10);
    let dirty = PixelRect::new(3, 2, 7, 6);
    let old = PixelGrid::segments(&pixels(region, |_, _| 0), region);
    let inside = |x: i32, y: i32| if (dirty.x0..dirty.x1).contains(&x) && (dirty.y0..dirty.y1).contains(&y) { 1 } else { 0 };
    let patched = PixelGrid::patched(&old, &pixels(dirty, |_, _| 1), dirty);
    let again = PixelGrid::segments(&pixels(region, inside), region);
    assert_eq!(edges(&patched), edges(&again));
}

#[trial]
fn the_grid_colour_follows_the_pixel_below() {
    let grey = |v: f32, alpha: f32| [v, v, v, alpha];
    let row = [grey(0.0, 1.0), grey(1.0, 1.0), grey(0.0, 0.0), grey(0.54, 1.0), grey(0.56, 1.0), grey(0.0, 0.5)];
    let edges = edges(&PixelGrid::segments(&row, PixelRect::new(0, 0, 6, 1)));
    // Each pixel owns its left edge: the vertical edge at its x, as unmultiplied grey and alpha.
    let line = |x: i32| edges.get(&(true, x, 0)).map(|color| color.to_srgba_unmultiplied()).map(|[grey, _, _, alpha]| (grey, alpha));
    let (dark, light) = (line(0).expect("a black pixel has a line"), line(1).expect("a white pixel has a line"));
    assert_eq!(dark, (255, 64), "a dark pixel gets a quarter white");
    assert_eq!(light.0, 0, "a light pixel gets a black line: {light:?}");
    assert!(light.1 < dark.1, "the black line is weaker than the white one: {light:?} against {dark:?}");
    assert_eq!(line(2), None, "a transparent pixel gets no line");
    let (below, above) = (line(3).expect("a mid-tone has a line"), line(4).expect("a mid-tone has a line"));
    for mid in [below, above] {
        assert!((32..=223).contains(&mid.0), "a mid-tone blends white and black: {mid:?}");
    }
    assert!(below.0.abs_diff(above.0) <= 40, "neighbouring mid-tones do not flip: {below:?} against {above:?}");
    let faint = line(5).expect("a half see-through dark pixel has a line");
    assert!(faint.1 < dark.1, "the line follows the pixel's alpha: {faint:?} against {dark:?}");
}

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

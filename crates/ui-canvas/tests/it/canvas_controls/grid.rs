//! The canvas grid and the pixel grid (20.6): line counts, the per-axis cap, subdivisions, and the pixel grid's maths and
//! region cache.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::{Canvas, CanvasSpace, PixelRect, RasterDisplay};
use craft_ui::canvas_controls::{CanvasGrid, GridCrowding, GridSegment, PixelGrid};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::canvas_story::StoryPicture;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, Pos2, Rect, Shape, TextureId, Vec2};
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

/// The major lines `grid` draws on the x axis in a 400 by 300 point view at `zoom`.
fn majors(grid: &CanvasGrid, zoom: f64) -> Vec<f64> {
    let view = CanvasView::new(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0)), Point::new(200.0, 150.0), zoom);
    grid.lines(&CanvasSpace::new(view, 1.0)).x.major
}

#[trial]
fn a_thinned_grid_doubles_its_step_until_it_fits_and_a_hiding_one_draws_nothing() {
    const STEP: f64 = 100.0;
    let grid = |crowding| CanvasGrid::new("Grid").spacing((STEP, STEP).into()).crowding(crowding);
    // At 5 % the step is five points, under the six-point minimum.
    assert!(majors(&grid(GridCrowding::Hide), 0.05).is_empty(), "a hiding grid draws nothing where its lines crowd");
    let thin = majors(&grid(GridCrowding::Thin), 0.05);
    assert!(thin.len() > 10, "a thinned grid keeps lines: {}", thin.len());
    assert_eq!(thin[1] - thin[0], 2.0 * STEP, "one doubling is enough at 5 %");
    assert!(thin.iter().all(|at| at.rem_euclid(STEP) == 0.0), "the thinned lines lie on lines of the undoubled grid");
    // Where the lines already fit, `Thin` changes nothing.
    assert_eq!(majors(&grid(GridCrowding::Thin), 1.0), majors(&grid(GridCrowding::Hide), 1.0));
    // 32 doublings reach the minimum at 2e-11 and 31 would not; at 1e-12 even 32 do not, so the axis is empty.
    let far = majors(&grid(GridCrowding::Thin), 2e-11);
    assert!(far.len() >= 2, "a grid that fits after 32 doublings draws");
    assert_eq!(far[1] - far[0], STEP * 2f64.powi(32), "the loop stops at 32 doublings");
    assert!(majors(&grid(GridCrowding::Thin), 1e-12).is_empty(), "a grid that needs more than 32 doublings draws nothing");
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

/// How many shapes a canvas paints before and after its picture, with the grid shown before the picture carrying the
/// `picture` flag when `under` is set. The empty shapes a paint slot leaves where it took shapes from paint nothing, so
/// they are not counted.
fn shapes_around_the_picture(under: bool) -> (usize, usize) {
    let room = Vec2::new(800.0, 600.0);
    let source = StoryPicture::new(40, 30, |_| Color32::WHITE);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, room), Point::new(20.0, 15.0), 4.0);
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let grid = CanvasGrid::new("Grid").spacing((10.0, 10.0).into());
            let _ = if under { grid.picture() } else { grid }.show_at(ui, body.rect);
            let _ = RasterDisplay::new("Picture", &source).picture().show_at(ui, body.rect);
        });
    });
    fixture.room(room);
    fixture.settle();
    let shapes: Vec<Shape> = fixture.frame_shapes().into_iter().filter(|shape| !matches!(shape, Shape::Noop)).collect();
    let at =
        shapes.iter().position(|shape| matches!(shape, Shape::Mesh(mesh) if mesh.texture_id != TextureId::default())).expect("the picture's mesh");
    (at, shapes.len() - at - 1)
}

#[trial]
fn a_grid_shown_before_a_picture_lies_under_it_and_one_without_the_flag_lies_over_it() {
    let (under_before, under_after) = shapes_around_the_picture(true);
    let (over_before, over_after) = shapes_around_the_picture(false);
    assert!(under_before > over_before, "the flagged grid's lines come before the picture: {under_before} against {over_before}");
    assert!(under_after < over_after, "the grid without the flag comes after it: {under_after} against {over_after}");
}

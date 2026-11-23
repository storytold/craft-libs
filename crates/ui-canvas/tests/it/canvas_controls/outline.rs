//! `path_runs`: the screen polylines a path outline is stroked from.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{BezPath, Point};
use craft_ui::canvas_controls::path_runs;
use craft_ui::testing::trial;
use egui::{Pos2, Rect, vec2};

fn view() -> CanvasView {
    CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), Point::new(200.0, 150.0), 1.0)
}

#[trial]
fn a_curve_far_larger_than_the_screen_flattens_to_a_bounded_run() {
    let far = Point::new(1e40, 0.0);
    let mut path = BezPath::new();
    path.move_to(far);
    path.curve_to(far, Point::new(20.0, 20.0), Point::new(20.0, 20.0));
    path.curve_to((60.0, 20.0), (60.0, 120.0), (40.0, 120.0));
    let runs = path_runs(&view(), &path);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].0.len() < 10_000);
}

#[trial]
fn a_non_finite_segment_breaks_the_run_and_the_rest_still_draws() {
    let mut path = BezPath::new();
    path.move_to((10.0, 10.0));
    path.line_to((50.0, 10.0));
    path.line_to((f64::NAN, 10.0));
    path.line_to((50.0, 80.0));
    path.line_to((90.0, 80.0));
    let runs = path_runs(&view(), &path);
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().flat_map(|(points, _)| points).all(|p| p.x.is_finite() && p.y.is_finite()));
}

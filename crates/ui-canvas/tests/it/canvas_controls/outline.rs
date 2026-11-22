//! `path_runs`: the screen polylines a path outline is stroked from.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{BezPath, Point, Rect as DocRect, Shape as _, Size};
use craft_ui::canvas::Canvas;
use craft_ui::canvas_controls::{SelectionOutline, path_runs};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::style::DataColor;
use craft_ui::testing::{Fixture, trial};
use egui::epaint::ColorMode;
use egui::{Color32, Pos2, Rect, Shape, vec2};
use std::cell::RefCell;
use std::rc::Rc;

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

/// What the outline trial's canvas shows: the host's side of the `revision` contract.
struct Shown {
    paths: Vec<BezPath>,
    revision: u64,
    centre: Point,
}

const RED: Color32 = Color32::RED;

/// A closed rectangle path of `width` by `height` document units at `at`.
fn rect_path(at: Point, width: f64, height: f64) -> BezPath {
    DocRect::new(at.x, at.y, at.x + width, at.y + height).to_path(0.1)
}

/// The vertices of every line the outline strokes in `RED` in one frame.
fn outline_lines(fixture: &mut Fixture) -> Vec<Vec<Pos2>> {
    fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Path(path) if path.stroke.color == ColorMode::Solid(RED) => Some(path.points),
            _ => None,
        })
        .collect()
}

/// `lines` moved by `by` points.
fn moved(lines: &[Vec<Pos2>], by: egui::Vec2) -> Vec<Vec<Pos2>> {
    lines.iter().map(|line| line.iter().map(|p| *p + by).collect()).collect()
}

fn assert_same(got: &[Vec<Pos2>], want: &[Vec<Pos2>], what: &str) {
    let close =
        got.len() == want.len() && got.iter().zip(want).all(|(a, b)| a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (*p - *q).length() < 1e-2));
    assert!(close, "{what}: {got:?} against {want:?}");
}

/// The outline trial's zoom: no whole number of device pixels per document unit, so the canvas shows the view as it is
/// set (`pixel_aligned`) and a pan by a fraction of a pixel reaches the outline.
const ZOOM: f64 = 1.5;

#[trial]
fn a_selection_outline_keeps_its_shapes_while_its_revision_holds_and_follows_a_whole_pixel_pan() {
    let first = vec![rect_path(Point::new(1980.0, 1980.0), 40.0, 30.0)];
    let second = vec![rect_path(Point::new(1950.0, 1950.0), 10.0, 20.0)];
    let state = Rc::new(RefCell::new(Shown { paths: first.clone(), revision: 1, centre: Point::new(2000.0, 2000.0) }));
    let shown = Rc::clone(&state);
    let mut fixture = Fixture::new(move |ui| {
        let state = shown.borrow();
        let view = CanvasView::new(ui.available_rect_before_wrap(), state.centre, ZOOM);
        let _ = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).view(view).show(ui, |ui, body| {
            let _ = SelectionOutline::solid("Selected", &state.paths).color(DataColor::new(RED)).revision(state.revision).show_at(ui, body.rect);
        });
    });
    fixture.settle();
    let built = outline_lines(&mut fixture);
    assert!(!built.is_empty(), "the outline strokes its path");

    state.borrow_mut().paths = second;
    assert_same(&outline_lines(&mut fixture), &built, "the revision holds, so the first paths' shapes stay");

    state.borrow_mut().revision = 2;
    let rebuilt = outline_lines(&mut fixture);
    assert_ne!(rebuilt, built, "a new revision builds the new paths");

    // The first paths come back but the revision stays: only a translation of the kept shapes gives the second
    // paths' lines moved left by the pan.
    {
        let mut state = state.borrow_mut();
        state.paths = first;
        state.centre.x += 8.0;
    }
    assert_same(&outline_lines(&mut fixture), &moved(&rebuilt, vec2(-8.0 * ZOOM as f32, 0.0)), "a pan by whole pixels moves the kept shapes");

    // A pan that is not whole device pixels builds again: the first paths, at the panned view.
    state.borrow_mut().centre.x += 0.3;
    assert_same(&outline_lines(&mut fixture), &moved(&built, vec2(-8.3 * ZOOM as f32, 0.0)), "a pan by a fraction rebuilds");
}

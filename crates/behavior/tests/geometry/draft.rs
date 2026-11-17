//! The outline draft: closing by clicking its start.

use craft_behavior::geometry::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
use craft_keymap::Mods;
use kurbo::Point;

const REACH: DraftReach = DraftReach::new(8.0, 3.0);

fn at(x: f64, y: f64, zoom: f64) -> DraftPoint {
    DraftPoint::new(Point::new(x, y), Mods::NONE, zoom)
}

#[test]
fn a_polygon_closes_by_clicking_near_its_start_at_any_zoom() {
    for zoom in [1.0, 4.0] {
        let unit = 1.0 / zoom;
        let build = || {
            let mut draft = OutlineDraft::new(DraftKind::Polygonal, REACH);
            for (x, y) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)] {
                draft.press(at(x, y, zoom), true);
                draft.release(at(x, y, zoom));
            }
            draft
        };
        assert!(matches!(build().press(at(7.0 * unit, 0.0, zoom), true), Some(DraftEvent::Closed { .. })));
        assert_eq!(build().press(at(9.0 * unit, 0.0, zoom), true), Some(DraftEvent::Changed));
    }
}

#[test]
fn a_small_polygon_does_not_close_at_its_last_corner() {
    // A square 4 points wide on screen at either zoom: its corners are nearer the start than the 8-point reach.
    for zoom in [1.0, 2.0] {
        let unit = 1.0 / zoom;
        let mut draft = OutlineDraft::new(DraftKind::Polygonal, REACH);
        for (x, y) in [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)] {
            draft.press(at(x * unit, y * unit, zoom), true);
            draft.release(at(x * unit, y * unit, zoom));
        }
        assert_eq!(draft.press(at(0.0, 4.0 * unit, zoom), true), Some(DraftEvent::Changed), "the last corner is a vertex");
        draft.release(at(0.0, 4.0 * unit, zoom));
        assert_eq!(draft.points().len(), 4);
        // A quarter of the narrow side is 1 point.
        assert_eq!(draft.clone().press(at(1.5 * unit, 0.0, zoom), true), Some(DraftEvent::Changed));
        assert!(matches!(draft.press(at(0.5 * unit, 0.0, zoom), true), Some(DraftEvent::Closed { points, .. }) if points.len() == 4));
    }
}

#[test]
fn shift_steps_keep_the_shorter_diagonal_offset() {
    let shift = |x: f64, y: f64| DraftPoint::new(Point::new(x, y), Mods::SHIFT, 1.0);
    let mut draft = OutlineDraft::new(DraftKind::Polygonal, REACH);
    draft.press(at(0.0, 0.0, 1.0), true);
    draft.release(at(0.0, 0.0, 1.0));
    let mut click = |point: DraftPoint| {
        assert_eq!(draft.press(point, true), Some(DraftEvent::Changed));
        draft.release(point);
        *draft.points().last().expect("a vertex")
    };
    // Near the diagonal: both offsets become the shorter one (7), not the 45° turn of the whole length.
    assert_eq!(click(shift(10.0, 7.0)), Point::new(7.0, 7.0));
    // Near an axis: the segment keeps its length.
    let along = click(shift(27.0, 9.0));
    assert!((along.x - (7.0 + 404f64.sqrt())).abs() < 1e-9 && along.y == 7.0, "{along:?}");
    // The signs of the offset are kept.
    let back = click(shift(along.x - 9.0, 1.0));
    assert!((back.x - (along.x - 6.0)).abs() < 1e-9 && back.y == 1.0, "{back:?}");
}

// A ⇧ freehand drag locks the nearest of eight directions once it has travelled a few screen points and stays on that
// line while the pointer wanders; the first point and the drag's earlier points are untouched.
#[test]
fn shift_holds_a_freehand_drag_to_one_line() {
    let shift = |x: f64, y: f64| DraftPoint::new(Point::new(x, y), Mods::SHIFT, 1.0);
    let mut draft = OutlineDraft::new(DraftKind::Mixed, REACH).locking_shift_drags();
    draft.press(at(10.0, 60.0, 1.0), true);
    for (x, y) in [(30.0, 63.0), (50.0, 55.0), (80.0, 68.0), (100.0, 52.0)] {
        draft.drag(shift(x, y));
    }
    let points = draft.points();
    assert_eq!(points[0], Point::new(10.0, 60.0));
    assert!(points.len() >= 3, "{points:?}");
    assert!(points.iter().all(|p| p.y == 60.0), "the drag stays on the horizontal line through its start: {points:?}");
    assert_eq!(points.last().map(|p| p.x), Some(100.0), "and follows the pointer along it");

    // A selection Lasso's ⇧ at the press adds to the selection: its drag keeps the pointer's path.
    let mut lasso = OutlineDraft::new(DraftKind::Mixed, REACH);
    lasso.press(at(10.0, 60.0, 1.0), true);
    for (x, y) in [(30.0, 63.0), (50.0, 55.0)] {
        lasso.drag(shift(x, y));
    }
    assert_eq!(lasso.points().last(), Some(&Point::new(50.0, 55.0)));
}

#[test]
fn moving_a_freehand_outline_shifts_every_point_by_the_pointers_step() {
    let mut draft = OutlineDraft::new(DraftKind::Freehand, REACH);
    draft.press(at(50.0, 50.0, 1.0), true);
    draft.drag(at(120.0, 50.0, 1.0));
    draft.drag(at(120.0, 100.0, 1.0));
    assert_eq!(draft.move_to(at(170.0, 130.0, 1.0)), Some(DraftEvent::Changed));
    assert_eq!(draft.points(), [Point::new(100.0, 80.0), Point::new(170.0, 80.0), Point::new(170.0, 130.0)]);
    // Drawing goes on from the moved end.
    draft.drag(at(180.0, 130.0, 1.0));
    assert_eq!(draft.points().len(), 4);
    // With the button up nothing moves.
    draft.release(at(180.0, 130.0, 1.0));
    assert_eq!(draft.move_to(at(0.0, 0.0, 1.0)), None);
}

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

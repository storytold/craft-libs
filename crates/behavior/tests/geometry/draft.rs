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

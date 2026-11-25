//! The bounds of nodes a scrolling body shows only in part, or not at all.

use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The scrolling body's region.
const VIEW: Rect = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(210.0, 110.0));
/// The region under it, which a control fills.
const BELOW: Rect = Rect::from_min_max(Pos2::new(10.0, 110.0), Pos2::new(210.0, 210.0));

/// A control past the end of a scrolling body's view is not painted, and the bounds the platform gets for its node stay
/// inside the view: a hit test on the control shown under the view finds that control, not the hidden one laid out
/// beneath it. A control the view shows whole keeps its rect as its bounds, and the trials' tree keeps every node's
/// laid-out rect.
#[trial]
fn a_control_scrolled_out_of_view_has_no_bounds_over_the_control_beside_the_view() {
    let seen: Rc<[RefCell<Seen>; 3]> = Rc::new(Default::default());
    let shown = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let [first, hidden, below] = &*shown;
        in_region(ui, VIEW, |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                Probe::new(first).label("First").id_salt("first").show(ui);
                Block::new(Vec2::new(VIEW.width(), VIEW.height())).show(ui);
                Probe::new(hidden).label("Hidden").id_salt("hidden").show(ui);
            });
        });
        in_region(ui, BELOW, |ui| {
            Probe::new(below).label("Below").size(BELOW.size()).id_salt("below").show(ui);
        });
    });
    fixture.settle();
    let [first, hidden, below] = &*seen;
    let key = |seen: &RefCell<Seen>| {
        let id = seen.borrow().id.unwrap_or_else(|| panic!("the probe ran"));
        fixture.node_of(id).unwrap_or_else(|| panic!("the probe has a node")).node
    };
    let platform =
        |seen: &RefCell<Seen>| fixture.platform_tree().bounds_of(key(seen)).unwrap_or_else(|| panic!("the platform gets the probe's bounds"));
    let (first_rect, hidden_rect, below_rect) = (first.borrow().rect, hidden.borrow().rect, below.borrow().rect);
    assert!(
        hidden_rect.intersects(below_rect) && hidden_rect.min.y >= VIEW.max.y,
        "the trial lays the hidden control out past the view, over the control below: hidden {hidden_rect:?}, below {below_rect:?}"
    );
    assert_eq!(platform(first), first_rect, "a control shown whole keeps its rect as its bounds");
    let bounds = platform(hidden);
    let overlap = bounds.intersect(below_rect);
    assert!(
        overlap.width() <= 0.0 || overlap.height() <= 0.0,
        "the hidden control's bounds {bounds:?} do not cover the control below {below_rect:?}"
    );
    assert!(VIEW.contains_rect(bounds), "the hidden control's bounds {bounds:?} stay inside the view {VIEW:?}");
    assert_eq!(platform(below), below_rect, "the control below keeps its rect as its bounds");
    let laid_out = fixture.tree().bounds_of(key(hidden));
    assert_eq!(laid_out, Some(hidden_rect), "the trials' tree keeps the hidden control's laid-out rect");
}

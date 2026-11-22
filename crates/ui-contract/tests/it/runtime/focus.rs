//! Focus after the focused stop goes away.

use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(200.0, 100.0);

/// The focused stop goes away and hands the focus to the next stop in its scope, far below a scrolling body's view:
/// the stop takes the focus and the body stays where it is. A neighbour that inherits the focus was not shown by
/// anyone, so nothing reveals it (#15555: the rulers' corner panned the canvas when a closed text editor handed it the
/// focus).
#[trial]
fn a_stop_that_inherits_the_focus_from_a_removed_one_is_not_revealed() {
    let (first, next) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let shown = Cell::new(true);
    let top = Cell::new(0.0f32);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                top.set(ui.next_widget_position().y);
                if shown.get() {
                    Probe::new(&first).label("First").id_salt("first").show(ui);
                }
                Block::new(Vec2::new(VIEW.x, VIEW.y * 4.0)).show(ui);
                Probe::new(&next).label("Next").id_salt("next").show(ui);
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let first_id = first.borrow().id.expect("the first stop ran");
    request(&frames.ctx, FocusTarget::Id(first_id));
    frames.idle(3, &mut show);
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(first_id), "the first stop took the focus");
    let start = top.get();
    shown.set(false);
    frames.idle(6, &mut show);
    let next_id = next.borrow().id.expect("the next stop ran");
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(next_id), "the next stop inherits the focus");
    assert!((top.get() - start).abs() < 0.5, "the body keeps its offset: content top {} from {start}", top.get());
}

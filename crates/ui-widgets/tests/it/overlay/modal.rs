//! Modals: blocked hits and commands below, and focus return.

use super::kit::{Go, Leaf, Opened, Opener, Seen};
use craft_ui::contract::{Overlay, Props as _, Show as _, Trigger};
use craft_ui::runtime::commands::{FireSource, fire};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::runtime::overlay::{OpenFilter, any_open};
use craft_ui::testing::focus::is_focused;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Id, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const OPENER: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(180.0, 120.0) };
const BELOW: Rect = Rect { min: Pos2::new(600.0, 500.0), max: Pos2::new(660.0, 530.0) };

#[trial]
fn a_modal_blocks_hits_and_commands_below_and_returns_focus_on_close() {
    let mut frames = Frames::new();
    let below = RefCell::new(Seen::default());
    let content = RefCell::new(Seen::default());
    let opened = Opened::default();
    let (open, close) = (Cell::new(false), Cell::new(false));
    let opener_id = Cell::new(Id::NULL);
    let mut show = |ui: &mut Ui| {
        opened.shown.set(false);
        let opener = Opener::new(&opened, &content, Trigger::Manual, Overlay::modal).open(open.get()).close(close.get());
        let response = opener.id_salt("opener").show_at(ui, OPENER);
        opener_id.set(response.id);
        Leaf::new(&below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
    };
    frames.idle(3, &mut show);
    assert!(fire(&frames.ctx, Go::Go, FireSource::Automation), "a command fires while no modal is open");
    // Focus the opener, then open the modal.
    request(&frames.ctx, FocusTarget::Id(opener_id.get()));
    frames.idle(2, &mut show);
    assert!(is_focused(&frames.ctx, opener_id.get()), "the opener has focus");
    open.set(true);
    frames.frame(vec![], &mut show);
    open.set(false);
    frames.idle(30, &mut show);
    assert!(opened.shown.get(), "the modal shows");
    assert!(any_open(&frames.ctx, OpenFilter::Modal), "the layer stack lists the modal");
    assert!(!fire(&frames.ctx, Go::Go, FireSource::Automation), "a modal blocks commands below it");
    let content_id = content.borrow().id;
    assert!(content_id.is_some_and(|id| is_focused(&frames.ctx, id)), "focus moved into the modal");
    // A press below the modal reaches nothing.
    let at = BELOW.center();
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    frames.frame(vec![release(at)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(below.borrow().presses, 0, "the modal blocks hits below it");
    assert!(opened.shown.get(), "a press outside does not close a dialog");
    // Closing returns focus to the opener and unblocks commands.
    close.set(true);
    frames.frame(vec![], &mut show);
    close.set(false);
    frames.idle(30, &mut show);
    assert!(!opened.shown.get(), "the modal closed");
    assert!(!any_open(&frames.ctx, OpenFilter::Modal));
    assert!(is_focused(&frames.ctx, opener_id.get()), "focus returned to the opener");
    assert!(fire(&frames.ctx, Go::Go, FireSource::Automation), "commands fire again after the modal closed");
}

//! Popups: the dismissal rules.

use crate::overlay::kit::{Leaf, Opened, Opener, Seen};
use craft_ui::contract::{Overlay, Props as _, Show as _, Trigger};
use craft_ui::testing::pointer::{click, moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const OPENER: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(180.0, 120.0) };
const BELOW: Rect = Rect { min: Pos2::new(300.0, 300.0), max: Pos2::new(360.0, 330.0) };

/// An opener of the preset `make` and a leaf below it at a fixed place; returns whether the popup body ran.
fn scene(frames: &mut Frames, events: Vec<egui::Event>, trigger: Trigger, make: fn() -> Overlay, open: bool, below: &RefCell<Seen>) -> bool {
    let opened = Opened::default();
    let content = RefCell::new(Seen::default());
    frames.frame(events, |ui: &mut Ui| {
        Opener::new(&opened, &content, trigger, make).open(open).id_salt("opener").show_at(ui, OPENER);
        Leaf::new(below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
    });
    opened.shown.get()
}

/// Opens the popup, then presses outside it on the leaf below; returns whether the popup still shows after the press
/// and what the leaf below saw.
fn press_outside(make: fn() -> Overlay) -> (bool, u32) {
    let mut frames = Frames::new();
    let below = RefCell::new(Seen::default());
    let frame = |frames: &mut Frames, events, open| scene(frames, events, Trigger::Manual, make, open, &below);
    for _ in 0..3 {
        frame(&mut frames, vec![], false);
    }
    frame(&mut frames, vec![], true);
    for _ in 0..30 {
        frame(&mut frames, vec![], false);
    }
    assert!(frame(&mut frames, vec![], false), "the popup is open before the press");
    let at = BELOW.center();
    frame(&mut frames, vec![moved(at)], false);
    frame(&mut frames, vec![press(at)], false);
    frame(&mut frames, vec![release(at)], false);
    let still = [frame(&mut frames, vec![], false), frame(&mut frames, vec![], false)];
    let presses = below.borrow().presses;
    (still[1], presses)
}

#[trial]
fn an_outside_press_closes_a_dropdown_and_reaches_the_widget_below() {
    let (open, presses) = press_outside(Overlay::dropdown);
    assert!(!open, "an outside press closes the popup");
    assert!(presses > 0, "the press reaches the widget below");
}

#[trial]
fn a_click_on_the_opener_closes_its_open_menu() {
    let mut frames = Frames::new();
    let below = RefCell::new(Seen::default());
    let opened = Opened::default();
    let content = RefCell::new(Seen::default());
    let mut show = |ui: &mut Ui| {
        opened.shown.set(false);
        Opener::new(&opened, &content, Trigger::Click, Overlay::menu).id_salt("opener").show_at(ui, OPENER);
        Leaf::new(&below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
    };
    frames.idle(3, &mut show);
    click(&mut frames, OPENER.center(), &mut show);
    frames.idle(30, &mut show);
    assert!(opened.shown.get(), "a click opens the menu");
    click(&mut frames, OPENER.center(), &mut show);
    frames.idle(30, &mut show);
    assert!(!opened.shown.get(), "the press on the opener reaches it, and its toggle closes the menu");
}

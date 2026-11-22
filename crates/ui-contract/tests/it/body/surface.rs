//! The click on a container's own surface: it reports in the pass of its primary release.

use craft_behavior::Sense;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const PROBE: Vec2 = Vec2::new(40.0, 40.0);
const BLOCK: Vec2 = Vec2::new(100.0, 60.0);

/// What the shown container leaves for the trial to read.
struct Shown {
    rect: Cell<Rect>,
    clicked: Cell<bool>,
    seen: RefCell<Seen>,
}

impl Default for Shown {
    fn default() -> Self {
        Shown { rect: Cell::new(Rect::ZERO), clicked: Cell::default(), seen: RefCell::default() }
    }
}

impl Shown {
    /// A panel that senses its surface, with a child that takes clicks above a block that senses nothing.
    fn show(&self, ui: &mut Ui) {
        let out = Panel::new().sense(Sense::SURFACE).show(ui, |ui, cx| {
            self.clicked.set(self.clicked.get() | cx.surface_clicked());
            Probe::new(&self.seen).size(PROBE).sense(Sense::CLICK).show(ui);
            Block::new(BLOCK).show(ui);
        });
        self.rect.set(out.output.rect);
    }
}

struct Rig {
    frames: Frames,
    shown: Shown,
}

impl Rig {
    fn new() -> Self {
        let mut rig = Rig { frames: Frames::new(), shown: Shown::default() };
        rig.frames.idle(3, |ui| rig.shown.show(ui));
        rig
    }

    /// Runs one frame with `events`; whether the container reported a click in it.
    fn frame(&mut self, events: Vec<egui::Event>) -> bool {
        self.shown.clicked.set(false);
        self.frames.frame(events, |ui| self.shown.show(ui));
        self.shown.clicked.get()
    }

    /// A point on the block, which senses nothing: the container's own surface.
    fn surface(&self) -> Pos2 {
        self.shown.rect.get().min + Vec2::new(20.0, PROBE.y + 30.0)
    }

    /// Moves to `at` and presses there; whether the press pass reported a click.
    fn press(&mut self, at: Pos2) -> bool {
        self.frame(vec![pointer::moved(at)]);
        self.frame(vec![pointer::press(at)])
    }
}

#[trial]
fn a_surface_click_reports_on_release_and_a_drag_a_cancel_or_a_release_elsewhere_report_none() {
    // A click reports in the release pass, not the press pass, and a click again reports again.
    let mut rig = Rig::new();
    let at = rig.surface();
    for _ in 0..2 {
        assert!(!rig.press(at), "the press pass reports no click");
        assert!(rig.frame(vec![pointer::release(at)]), "the release pass reports the click");
        assert!(!rig.frame(Vec::new()), "the next pass reports none");
        rig.frames.advance(1.0);
    }

    // A drag past the slop reports none.
    let mut rig = Rig::new();
    let at = rig.surface();
    rig.press(at);
    let far = at + Vec2::new(40.0, 0.0);
    rig.frame(vec![pointer::moved(far)]);
    assert!(!rig.frame(vec![pointer::release(far)]), "a drag is no click");

    // Escape cancels the press.
    let mut rig = Rig::new();
    let at = rig.surface();
    rig.press(at);
    rig.frame(vec![key_press(Key::Escape, Modifiers::NONE)]);
    assert!(!rig.frame(vec![pointer::release(at)]), "a cancelled press is no click");

    // With Cmd or Ctrl held too, as a tool borrowed with Cmd holds it through its drag.
    let mut rig = Rig::new();
    let at = rig.surface();
    rig.press(at);
    rig.frame(vec![key_press(Key::Escape, Modifiers::COMMAND)]);
    assert!(!rig.frame(vec![pointer::release(at)]), "a press cancelled by Cmd+Escape is no click");

    // A blur cancels it too.
    let mut rig = Rig::new();
    let at = rig.surface();
    rig.press(at);
    rig.frame(vec![pointer::window_blurred()]);
    assert!(!rig.frame(vec![pointer::release(at)]), "a blurred press is no click");

    // A release outside the rect, within the slop of the press, is no click.
    let mut rig = Rig::new();
    let corner = rig.shown.rect.get().max - Vec2::splat(0.5);
    rig.press(corner);
    let outside = corner + Vec2::splat(1.5);
    assert!(!rig.frame(vec![pointer::release(outside)]), "a release outside is no click");

    // A press a child takes is no surface click.
    let mut rig = Rig::new();
    let on_child = rig.shown.rect.get().min + Vec2::splat(20.0);
    rig.press(on_child);
    assert!(!rig.frame(vec![pointer::release(on_child)]), "a press the child took is no surface click");
}

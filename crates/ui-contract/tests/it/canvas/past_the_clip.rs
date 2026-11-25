//! A press past its root's clip, which only automation makes (a document point an agent names off the window): a part
//! that shows inside the clip takes it, as a tool takes an agent's press anywhere on the document; a part the clip hides
//! wholly (a handle past the window's edge) does not.

use craft_behavior::{HitShape, PartKey, PressPhase, Sense};
use craft_ui::contract::{Common, Component, Cx, PartsCx, Pointer as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response, Vec2};
use std::cell::Cell;

const HANDLE: PartKey = PartKey::new("handle");

/// A leaf with one part over `part` of its rect.
#[derive(Component)]
struct Handle<'a> {
    #[common]
    common: Common,
    part: Rect,
    pressed: &'a Cell<bool>,
}

fn area() -> Rect {
    Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(200.0, 120.0))
}

/// The clip the leaf is shown in: its left half.
fn clip() -> Rect {
    Rect::from_min_size(area().min, Vec2::new(100.0, 120.0))
}

impl Component for Handle<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: HandleStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(200.0, 120.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(HANDLE, HitShape::Rect(self.part)).sense(Sense::DRAG | Sense::CLICK);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if matches!(cx.part(HANDLE).press.phase, PressPhase::Down { .. }) {
            self.pressed.set(true);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Whether a press at `at` is the press of a part over `part`.
fn pressed_at(part: Rect, at: Pos2) -> bool {
    let pressed = Cell::new(false);
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        ui.set_clip_rect(clip());
        Handle { common: Common::new(), part, pressed: &pressed }.show_at(ui, area());
    };
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    pressed.get()
}

#[trial]
fn past_the_clip_only_a_part_that_shows_takes_a_press() {
    let past = Pos2::new(clip().right() + 40.0, clip().center().y);
    assert!(pressed_at(area(), clip().center()), "where the part shows the press is its");
    assert!(pressed_at(area(), past), "a part that shows inside the clip takes a press past it");
    let hidden = Rect::from_center_size(past, Vec2::splat(12.0));
    assert!(!pressed_at(hidden, past), "a handle the clip hides wholly is not pressed");
}

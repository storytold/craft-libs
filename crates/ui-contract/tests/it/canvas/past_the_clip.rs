//! A part reaching past its root's clip: a press on the part where the clip hides it is no press on it (a handle past
//! the window's edge); where it shows, the press is the part's.

use craft_behavior::{HitShape, PartKey, PressPhase, Sense};
use craft_ui::contract::{Common, Component, Cx, PartsCx, Pointer as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response, Vec2};
use std::cell::Cell;

const HANDLE: PartKey = PartKey::new("handle");

/// A leaf whose one part covers its whole rect.
#[derive(Component)]
struct Handle<'a> {
    #[common]
    common: Common,
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

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(HANDLE, HitShape::Rect(rect)).sense(Sense::DRAG | Sense::CLICK);
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

fn pressed_at(at: Pos2) -> bool {
    let pressed = Cell::new(false);
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        ui.set_clip_rect(clip());
        Handle { common: Common::new(), pressed: &pressed }.show_at(ui, area());
    };
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    pressed.get()
}

#[trial]
fn a_press_where_the_clip_hides_a_part_is_not_the_parts() {
    assert!(pressed_at(clip().center()), "where the part shows the press is its");
    assert!(!pressed_at(Pos2::new(clip().right() + 40.0, clip().center().y)), "past the clip the part is not pressed");
}

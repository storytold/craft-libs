//! A part shaped as the area outside a turned box: a press just outside a corner, inside the box's bounding
//! rectangle, is the part's; a press inside the box is not, so it reaches whatever lies beneath.

use craft_behavior::{HitShape, PartKey, PressPhase, Sense};
use craft_ui::contract::{Common, Component, Cx, PartsCx, Pointer as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response, Vec2};
use std::cell::Cell;

const AROUND: PartKey = PartKey::new("around");

/// The half diagonal of the turned box.
const REACH: f32 = 30.0;

/// A leaf whose only part is everything around a box turned a quarter of a right angle.
#[derive(Component)]
struct Around<'a> {
    #[common]
    common: Common,
    pressed: &'a Cell<bool>,
}

fn area() -> Rect {
    Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(200.0, 120.0))
}

impl Component for Around<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: AroundStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(200.0, 120.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let c = rect.center();
        let diamond = vec![c + Vec2::new(0.0, -REACH), c + Vec2::new(REACH, 0.0), c + Vec2::new(0.0, REACH), c + Vec2::new(-REACH, 0.0)];
        cx.part(AROUND, HitShape::outside_polygon(rect, diamond)).sense(Sense::DRAG | Sense::CLICK);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if matches!(cx.part(AROUND).press.phase, PressPhase::Down { .. }) {
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
        Around { common: Common::new(), pressed: &pressed }.show_at(ui, area());
    };
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    pressed.get()
}

#[trial]
fn a_press_inside_a_turned_box_is_not_the_surrounds() {
    assert!(!pressed_at(area().center() + Vec2::splat(5.0)), "inside the box the press goes to what lies beneath");
}

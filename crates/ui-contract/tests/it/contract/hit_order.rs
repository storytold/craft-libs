//! The stacking order of a component's `over` parts and the children it shows from `paint_over` (8.3): one rule, a
//! later child ranks above an earlier `over` part.

use craft_behavior::{PartKey, PressPhase, Sense};
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, PaintUi, PartsCx, Pointer as _, Props as _, Role, ShowContainer as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;

const OVER_PART: PartKey = PartKey::new("grip");

/// An arena container with an `over` part across its rect and a child it shows from `paint_over` at the same rect.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Overlaid<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    grip: &'a RefCell<Seen>,
    child: &'a RefCell<Seen>,
}

impl Component for Overlaid<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: OverlaidStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(OVER_PART, rect).sense(Sense::CLICK).over();
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let phase = cx.part(OVER_PART).press.phase;
        if phase.is_press() {
            self.grip.borrow_mut().press.push(phase);
        }
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.child("kid", cx.rect, Probe::new(self.child).size(cx.rect.size()).sense(Sense::CLICK));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Overlaid<'_> {}

#[trial]
fn a_child_shown_from_paint_over_ranks_above_the_owners_over_part() {
    let (grip, child) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let area = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(100.0, 60.0));
    let mut show = |ui: &mut Ui| {
        Overlaid { common: Common::new(), props: ContainerProps::new().arena(true), grip: &grip, child: &child }.id_salt("overlaid").show_at(
            ui,
            area,
            |_, _| {},
        );
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let at = area.center();
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    frames.frame(vec![release(at)], &mut show);
    frames.idle(2, &mut show);
    let released = |seen: &RefCell<Seen>| seen.borrow().press.iter().any(|phase| matches!(phase, PressPhase::Release { .. }));
    assert!(released(&child), "the press went to the child: {:?}", child.borrow().press);
    assert!(!released(&grip), "the owner's over part took no press under the child: {:?}", grip.borrow().press);
}

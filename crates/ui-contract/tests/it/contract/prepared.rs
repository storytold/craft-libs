//! Adjacent measurement and show reuse only unchanged environment and style inputs.

use craft_behavior::{Proposal, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _, SideLabel};
use craft_ui::style::{ComponentStyle as _, StyleCx};
use craft_ui::testing::{Fixture, last_pass_prepared_leaves, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Response, Vec2};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Component)]
struct Child {
    #[common]
    common: Common,
    #[style_key]
    changed: bool,
    mutate: bool,
    seen: Rc<Cell<f32>>,
}

impl Component for Child {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, key: ChildStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(if key.changed { 40.0 } else { 20.0 }, 20.0))
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if self.mutate && cx.proposal == Proposal::Ideal {
            self.changed = true;
        }
        Vec2::new(10.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.seen.set(cx.style.metrics().min_size.x);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[derive(Component)]
struct Owner {
    #[common]
    common: Common,
    mutate: bool,
    seen: Rc<Cell<f32>>,
}

impl Component for Owner {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: OwnerStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(200.0, 50.0))
    }
    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let child = Child { common: Common::new(), changed: false, mutate: self.mutate, seen: self.seen.clone() };
        ui.labelled("child", cx.rect, "Caption".into(), SideLabel::new(), child);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

fn show(mutate: bool) -> (u32, f32) {
    let seen = Rc::new(Cell::new(0.0));
    let output = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        Owner { common: Common::new(), mutate, seen: output.clone() }.id_salt("owner").show(ui);
    });
    fixture.frame(Vec::new());
    (last_pass_prepared_leaves(fixture.ctx()), seen.get())
}

#[trial]
fn adjacent_leaf_reuses_steps_only_while_its_style_key_is_unchanged() {
    let (reused, unchanged) = show(false);
    assert!(reused > 0, "unchanged child reuses its adjacent measure steps");
    assert_eq!(unchanged, 20.0);
    let (reused, changed) = show(true);
    assert_eq!(reused, 0, "a changed style key uses the ordinary path");
    assert_eq!(changed, 40.0, "the shown child receives its changed style");
}

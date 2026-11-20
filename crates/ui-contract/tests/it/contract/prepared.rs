//! Adjacent measurement and show reuse only unchanged environment and style inputs.

use craft_behavior::{Proposal, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _, SideLabel};
use craft_ui::primitives::TextRun;
use craft_ui::style::{ComponentStyle as _, StyleCx};
use craft_ui::testing::{Fixture, last_pass_prepared_leaves, last_pass_text_layouts, trial};
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

#[derive(Component)]
struct EnablesOnMinimum {
    #[common]
    common: Common,
    seen: Rc<Cell<Option<bool>>>,
}

impl Component for EnablesOnMinimum {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: EnablesOnMinimumStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if cx.proposal == Proposal::Min {
            self.common = Common::new();
        }
        Vec2::new(10.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.seen.set(Some(cx.core.resolved.enabled));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[derive(Component)]
struct EnabledOwner {
    #[common]
    common: Common,
    seen: Rc<Cell<Option<bool>>>,
}

impl Component for EnabledOwner {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: EnabledOwnerStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(200.0, 50.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let child = EnablesOnMinimum { common: Common::new(), seen: self.seen.clone() }.enabled(false);
        ui.labelled("child", cx.rect, "Caption".into(), SideLabel::new(), child);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn labelled_child_enabling_itself_in_minimum_measure_uses_the_current_enabled_state() {
    let seen = Rc::new(Cell::new(None));
    let output = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        EnabledOwner { common: Common::new(), seen: output.clone() }.id_salt("owner").show(ui);
    });
    fixture.frame(Vec::new());
    assert_eq!(seen.get(), Some(true), "the shown child uses its enabled state after the minimum measure");
    assert_eq!(last_pass_prepared_leaves(fixture.ctx()), 0, "different inherited enabled states use the ordinary path");
}

#[derive(Component)]
struct LeastTwice {
    #[common]
    common: Common,
    measured: Rc<Cell<u32>>,
}

impl Component for LeastTwice {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: LeastTwiceStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let run = TextRun::new("the complete least layout is retained").elide();
        let first = cx.least(&run);
        let count = cx.ui.ctx().fonts(|fonts| fonts.num_galleys_in_cache());
        let second = cx.least(&run);
        assert_eq!(first.width, second.width);
        assert!(std::sync::Arc::ptr_eq(&first.whole.galley, &second.whole.galley));
        assert_eq!(cx.ui.ctx().fonts(|fonts| fonts.num_galleys_in_cache()), count, "a repeated least request performs no new layouts");
        self.measured.set(self.measured.get() + 1);
        second.size()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn repeated_measure_least_reuses_both_results() {
    let measured = Rc::new(Cell::new(0));
    let output = measured.clone();
    let mut fixture = Fixture::new(move |ui| {
        LeastTwice { common: Common::new(), measured: output.clone() }.id_salt("least-twice").show(ui);
    });
    fixture.frame(Vec::new());
    let first_count = measured.get();
    assert!(first_count > 0);
    fixture.frame(Vec::new());
    assert!(measured.get() > first_count, "the second pass must actually request the least layout");
    assert_eq!(last_pass_text_layouts(fixture.ctx()), 0, "repeated least requests reuse the whole layout and least width");
}

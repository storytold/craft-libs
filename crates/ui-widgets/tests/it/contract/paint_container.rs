//! A container shown from `paint` through `PaintUi::container` lays its body out as the same container shown in an
//! egui `Ui`, so one body function serves a composite that paints and a host with a `Ui`.

use craft_behavior::{Metrics, Sense};
use craft_ui::contract::{Bind, BodyCx, Common, Component, Cx, PaintUi, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Rect, Response, Ui, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;

/// Where both forms show the group.
const AT: Rect = Rect { min: pos2(20.0, 20.0), max: pos2(220.0, 140.0) };
const AXES: [&str; 2] = ["X", "Y"];

#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The stacked fields both forms show: one labelled number field per axis, as a node face stacks a vector.
fn stacked(ui: &mut Ui, _: &mut BodyCx<'_>) {
    for axis in AXES {
        Field::new(axis).show(ui, |ui, _| NumberField::new(axis, Bind::given(Some(1.0))).show(ui));
    }
}

/// A leaf that paints the group over its whole rect through `PaintUi`, as a graph node paints its inline controls.
#[derive(craft_ui::contract::Component)]
struct Painted {
    #[common]
    common: Common,
    /// The group's body ran in the last pass.
    ran: Rc<Cell<bool>>,
}

impl Component for Painted {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PaintedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let shown = ui.container("group", cx.rect, FieldGroup::new("Value"), stacked);
        self.ran.set(shown.inner.is_some());
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The rects of the axis fields `fixture` shows.
fn fields(fixture: &Fixture) -> Vec<Rect> {
    AXES.iter().map(|axis| fixture.by_label(axis).unwrap_or_else(|| panic!("the group shows field {axis}")).rect).collect()
}

#[trial]
fn a_container_shown_from_paint_lays_its_body_out_as_in_an_egui_ui() {
    let ran = Rc::new(Cell::new(false));
    let held = Rc::clone(&ran);
    let mut painted = Fixture::new(move |ui| {
        let _ = Painted { common: Common::new(), ran: Rc::clone(&held) }.id_salt("painted").show_at(ui, AT);
    });
    painted.room(vec2(400.0, 300.0)).settle();
    let mut direct = Fixture::new(|ui| {
        let _ = FieldGroup::new("Value").show_at(ui, AT, stacked);
    });
    direct.room(vec2(400.0, 300.0)).settle();

    assert!(ran.get(), "the body runs inside the painting composite");
    let (from_paint, from_ui) = (fields(&painted), fields(&direct));
    assert!(from_paint.iter().all(|field| AT.contains_rect(*field)), "the fields lie inside the composite's rect: {from_paint:?}");
    assert!(from_paint[1].top() >= from_paint[0].bottom(), "the fields stack top to bottom: {from_paint:?}");
    for (paint, ui) in from_paint.iter().zip(&from_ui) {
        assert!((paint.min - ui.min).length() < 0.5 && (paint.max - ui.max).length() < 0.5, "{paint:?} lies where the egui form puts it, {ui:?}");
    }
}

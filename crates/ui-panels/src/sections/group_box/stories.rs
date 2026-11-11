//! Stories of `GroupBox`.

use super::GroupBox;
use crate::contract::{Bind, Role, Show as _, ShowContainer as _, Validity};
use crate::runtime::env::Message;
use crate::sections::sample::body;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use crate::text::NumberField;
use craft_behavior::Mixed;
use egui::Ui;

fn toggle_gate(fixture: &mut Fixture) {
    let gate = fixture.by_role(Role::CHECK_BOX).into_iter().find(|check| fixture.tree().name_of(check.node) == Some("Enable"));
    if let Some(gate) = gate {
        fixture.click(&gate);
    }
}

#[story(GroupBox<'static>, covers = [Default], drive = Drive::Custom(toggle_gate))]
fn group_box(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    GroupBox::new("Grid", Some(Bind::bound(data.value("gate", true)))).show(ui, |ui, _| body(ui)).into()
}

#[story(GroupBox<'static>, covers = [], drive = Drive::Never)]
fn group_box_without_gate(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    GroupBox::new("Grid", None).show(ui, |ui, _| body(ui)).into()
}

#[story(GroupBox<'static>, covers = [], drive = Drive::Never)]
fn group_box_gate_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    GroupBox::new("Grid", Some(Bind::bound(data.value("gate", false)))).show(ui, |ui, _| body(ui)).into()
}

#[story(GroupBox<'static>, covers = [Mixed], drive = Drive::Never)]
fn group_box_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    GroupBox::new("Grid", Some(Bind::bound_mixed(data.value("mixed", Mixed::Mixed)))).show(ui, |ui, _| body(ui)).into()
}

#[story(GroupBox<'static>, covers = [Invalid], drive = Drive::Never)]
fn group_box_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("Not available")) } else { Validity::Ok };
    GroupBox::new("Grid", Some(Bind::bound(data.value("gate", true)).check(refuse_on))).show(ui, |ui, _| body(ui)).into()
}

fn toggle_apply(fixture: &mut Fixture) {
    if let Some(check) = fixture.by_role(Role::CHECK_BOX).into_iter().next() {
        fixture.click(&check);
    }
}

#[story(GroupBox<'static>, covers = [], drive = Drive::Custom(toggle_apply))]
fn group_box_apply(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = *data.value("opacity", Mixed::<Option<f64>>::Mixed);
    GroupBox::apply("Opacity", Bind::bound(data.value("apply", false)))
        .show(ui, |ui, _| {
            NumberField::new("Opacity", Bind::given(value)).show(ui);
        })
        .into()
}

#[story(GroupBox<'static>, covers = [], drive = Drive::Never)]
fn group_box_apply_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    GroupBox::apply("Opacity", Bind::bound(data.value("apply", true)))
        .show(ui, |ui, _| {
            NumberField::new("Opacity", Bind::given(Some(80.0))).show(ui);
        })
        .into()
}

#[story(GroupBox<'static>, covers = [], drive = Drive::Never)]
fn group_box_apply_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    GroupBox::apply("Opacity", Bind::bound_mixed(data.value("mixed", Mixed::<bool>::Mixed))).show(ui, |_, _| {}).into()
}

#[story(GroupBox<'static>, covers = [], drive = Drive::Never)]
fn group_box_apply_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("Not available")) } else { Validity::Ok };
    GroupBox::apply("Opacity", Bind::bound(data.value("apply", true)).check(refuse_on)).show(ui, |_, _| {}).into()
}

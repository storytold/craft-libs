//! Stories of `Section`.

use super::Section;
use crate::contract::{Bind, Role, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::sections::sample::body;
use crate::sections::{HeaderAction, HeaderStrength};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

fn toggle_disclosure(fixture: &mut Fixture) {
    let lead = fixture.by_role(Role::BUTTON).into_iter().find(|button| fixture.tree().name_of(button.node) == Some("Layers"));
    if let Some(lead) = lead {
        fixture.click(&lead);
    }
}

#[story(Section<'static>, covers = [Default], drive = Drive::Custom(toggle_disclosure))]
fn section(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Section::new("Layers").open(Bind::bound(data.value("open", true))).show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_closed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Section::new("Layers").open(Bind::bound(data.value("open", false))).show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_fixed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Section::fixed("Layers").show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_caption(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Section::fixed("Preset details").strength(HeaderStrength::Caption).show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_with_menu_and_reorder(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown =
        Section::new("Layers").open(Bind::bound(data.value("open", true))).actions([HeaderAction::Menu]).reorder(true).show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_with_controls(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Section::new("Recent")
        .open(Bind::bound(data.value("open", true)))
        .controls(|ui, _| {
            Text::new("12 items").show(ui);
        })
        .show(ui, |ui, _| body(ui));
    shown.output.into()
}

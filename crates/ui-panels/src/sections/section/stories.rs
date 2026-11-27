//! Stories of `Section`.

use super::Section;
use crate::contract::{Props as _, Role, SectionRhythm, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout_components::Stack;
use crate::runtime::env::with_env;
use crate::sections::sample::body;
use crate::sections::{HeaderAction, HeaderStrength, QuietTitleRow};
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
    let shown = Section::new("Layers").open(data.bind("open", true)).show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_closed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Section::new("Layers").open(data.bind("open", false)).show(ui, |ui, _| body(ui));
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
fn section_subheading(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Section::fixed("Shape modes").strength(HeaderStrength::Subheading).show(ui, |ui, _| body(ui));
    shown.output.into()
}

/// A sub-heading of an app that sets `QuietTitleRow::Text` at its root: a text row high over its controls, not a
/// control high.
#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_subheading_text_row(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    with_env(
        ui,
        |overrides| {
            overrides.values.set(QuietTitleRow::Text);
        },
        |ui| Section::fixed("Shape modes").strength(HeaderStrength::Subheading).show(ui, |ui, _| body(ui)).output.into(),
    )
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_with_menu_and_reorder(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Section::new("Layers").open(data.bind("open", true)).actions([HeaderAction::Menu]).reorder(true).show(ui, |ui, _| body(ui));
    shown.output.into()
}

#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_with_controls(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Section::new("Recent")
        .open(data.bind("open", true))
        .controls(|ui, _| {
            Text::new("12 items").show(ui);
        })
        .show(ui, |ui, _| body(ui));
    shown.output.into()
}

/// A row and two sections under each `SectionRhythm`: `Tight` (the default) gives each rule the stack's gap and its
/// header follows at once; `Spaced` stands each rule `space.gap.section` off what precedes it and its header
/// `space.gap.row` below it.
#[story(Section<'static>, covers = [], drive = Drive::Never)]
fn section_rhythm(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut column = |ui: &mut Ui, key: &'static str| {
        Stack::new().id_salt(key).show(ui, |ui, _| {
            Text::new("Fill").show(ui);
            Section::new("Stroke").open(data.bind(key, true)).show(ui, |ui, _| body(ui));
            Section::fixed("Effects").show(ui, |ui, _| body(ui));
        });
    };
    column(ui, "tight");
    with_env(
        ui,
        |overrides| {
            overrides.values.set(SectionRhythm::Spaced);
        },
        |ui| column(ui, "spaced"),
    );
    StoryOutput::default()
}

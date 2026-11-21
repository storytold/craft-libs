//! Stories of `ScreenModeButton`.

use super::ScreenModeButton;
use crate::contract::Role;
use crate::runtime::env::with_env;
use crate::shell::{ScreenMode, ScreenModeSet};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

/// More than the 0.5 s a press must be held to open the list.
const HELD: f64 = 0.7;

#[story(ScreenModeButton, covers = [Default], drive = Drive::Never)]
fn screen_mode_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ScreenModeButton::new(ScreenMode::Standard).show(ui).into()
}

fn hold_button(fixture: &mut Fixture) {
    if let Some(target) = fixture.by_role(Role::BUTTON).into_iter().next() {
        fixture.press(&target);
        fixture.advance(HELD);
        fixture.idle(3);
    }
}

/// The list of the three modes, open after a hold.
#[story(ScreenModeButton, covers = [Open], drive = Drive::Custom(hold_button))]
fn screen_mode_button_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ScreenModeButton::new(ScreenMode::Standard).show(ui).into()
}

/// The button in a document window's shell: its list holds Standard, Full Screen With Menu Bar, Focus and Immersive,
/// open after a hold, with Focus marked.
#[story(ScreenModeButton, covers = [], drive = Drive::Custom(hold_button))]
fn screen_mode_button_document(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    with_env(
        ui,
        |overrides| {
            overrides.values.set(ScreenModeSet::Document);
        },
        |ui| ScreenModeButton::new(ScreenMode::Focus).show(ui),
    )
    .into()
}

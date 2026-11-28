//! Stories of the footer actions.

use super::{DialogButtonLook, DialogRole, DropAction, FooterAction, FooterBar, FooterButton};
use crate::buttons::IconButton;
use crate::contract::{PopupKind, Props as _, Show as _, ShowContainer as _};
use crate::runtime::env::with_env;
use crate::story_kit::sample::words;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_theme::variant::Tone;
use egui::Ui;

#[story(FooterBar, covers = [Default])]
fn footer_bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FooterBar::body().show(ui, |ui, _| words(ui, 0..1)).into()
}

#[story(FooterBar, covers = [], drive = Drive::Never)]
fn footer_actions(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let actions = [
        FooterAction::new(IconId::new("trash"), "Delete layer").accepts_drops::<u8>(|_| true).takes_drops_disabled(true).enabled(false),
        FooterAction::new(IconId::new("plus"), "New layer").popup(PopupKind::Menu).expanded(false),
        FooterAction::new(IconId::new("settings"), "Layer settings").pressed(true).tone(Tone::Accent),
    ];
    FooterBar::actions(actions).show_footer(ui).into()
}

/// The dialog buttons of an app that sets `DialogButtonLook::Filled` at its root: Cancel, the Alternate answer and
/// another outcome are filled neutral tiles with no edge beside the accented OK, at rest, under the pointer and pressed.
#[story(FooterBar, covers = [], drive = Drive::Never, title = "Filled dialog buttons")]
fn footer_filled_buttons(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let buttons = [
        FooterButton::new("Reset", DialogRole::Other),
        FooterButton::new("Don't Save", DialogRole::Alternate),
        FooterButton::new("Cancel", DialogRole::Cancel),
        FooterButton::new("OK", DialogRole::Accept),
    ];
    with_env(
        ui,
        |overrides| {
            overrides.values.set(DialogButtonLook::Filled);
        },
        |ui| FooterBar::buttons(buttons).show_footer(ui).into(),
    )
}

#[story(DropAction<u8>, covers = [Default], drive = Drive::Never)]
fn drop_action(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    DropAction::new(|_: &u8| true)
        .show(ui, |ui, _| {
            IconButton::new(IconId::new("trash")).label("Delete layer").show(ui);
        })
        .output
        .into()
}

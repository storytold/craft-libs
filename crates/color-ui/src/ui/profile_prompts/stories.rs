//! Stories of the profile prompts.

use super::{ProfileMismatchDialog, ProfileMode, ProfileQuestion};
use craft_ui::dialog::stories::hosted;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn shown(ui: &mut Ui, mut dialog: ProfileMismatchDialog) -> StoryOutput {
    let mut stack = DialogStack::default();
    stack.push(dialog.id(), None);
    let _ = dialog.show(ui, &stack);
    StoryOutput::default()
}

#[story(ProfileMismatchDialog, covers = [Default], drive = Drive::Never)]
fn profile_mismatch(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let question = ProfileQuestion::Mismatch { embedded: "Display P3".into(), working: "sRGB IEC61966-2.1".into() };
    shown(ui, ProfileMismatchDialog::new("photo.psd", ProfileMode::Rgb, question).rememberable(true))
}

/// A Missing Profile prompt for a scan that offers `names` to assign.
fn missing_with(names: &[&str]) -> ProfileMismatchDialog {
    let question = ProfileQuestion::Missing { working: "sRGB IEC61966-2.1".into() };
    ProfileMismatchDialog::new("photo.psd", ProfileMode::Rgb, question).profiles(names.iter().map(|name| (*name).to_owned()).collect())
}

#[story(ProfileMismatchDialog, covers = [], drive = Drive::Never)]
fn profile_missing_with_profiles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || missing_with(&["Adobe RGB (1998)", "Display P3", "ProPhoto RGB"]))
}

#[story(ProfileMismatchDialog, covers = [], drive = Drive::Never)]
fn profile_missing_with_long_profile_names(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || {
        missing_with(&[
            "Coated FOGRA39 (ISO 12647-2:2004) with a perceptual rendering intent and black point compensation",
            "Japan Color 2001 Coated",
        ])
    })
}

#[story(ProfileMismatchDialog, covers = [], drive = Drive::Never)]
fn profile_missing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let question = ProfileQuestion::Missing { working: "sRGB IEC61966-2.1".into() };
    shown(ui, ProfileMismatchDialog::new("photo.psd", ProfileMode::Rgb, question))
}

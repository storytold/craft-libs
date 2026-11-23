//! Stories of the profile prompts.

use super::{ProfileMismatchDialog, ProfileMode, ProfileQuestion};
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

#[story(ProfileMismatchDialog, covers = [], drive = Drive::Never)]
fn profile_missing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let question = ProfileQuestion::Missing { working: "sRGB IEC61966-2.1".into() };
    shown(ui, ProfileMismatchDialog::new("photo.psd", ProfileMode::Rgb, question))
}

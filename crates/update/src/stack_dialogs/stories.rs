//! Stories of the update dialogs.

use super::{UpdateCheckDialog, UpdatePromptDialog};
use crate::release::Release;
use crate::updater::Status;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn release() -> Release {
    Release { tag: "v2.0.0".into(), html_url: "https://example.org/releases/v2.0.0".into(), ..Release::default() }
}

#[story(UpdatePromptDialog, covers = [], drive = Drive::Never)]
fn prompt_download(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    UpdatePromptDialog::new(release(), "1.0.0", false).present(ui).0.into()
}

#[story(UpdatePromptDialog, covers = [], drive = Drive::Never)]
fn prompt_self_update(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    UpdatePromptDialog::new(release(), "1.0.0", true).present(ui).0.into()
}

#[story(UpdatePromptDialog, covers = [], drive = Drive::Never)]
fn prompt_installing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = UpdatePromptDialog::new(release(), "1.0.0", true);
    dialog.set_installing(true);
    dialog.present(ui).0.into()
}

#[story(UpdatePromptDialog, covers = [], drive = Drive::Never)]
fn prompt_failed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = UpdatePromptDialog::new(release(), "1.0.0", true);
    dialog.set_status(Some("Not enough disk space to install the update.".into()));
    dialog.present(ui).0.into()
}

#[story(UpdateCheckDialog, covers = [], drive = Drive::Never)]
fn check_checking(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    UpdateCheckDialog::new("1.0.0").present(ui).0.into()
}

#[story(UpdateCheckDialog, covers = [], drive = Drive::Never)]
fn check_up_to_date(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = UpdateCheckDialog::new("1.0.0");
    dialog.set_status(Status::UpToDate);
    dialog.present(ui).0.into()
}

#[story(UpdateCheckDialog, covers = [], drive = Drive::Never)]
fn check_available(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = UpdateCheckDialog::new("1.0.0").with_self_updating(true);
    dialog.set_available(Some(release()));
    dialog.present(ui).0.into()
}

#[story(UpdateCheckDialog, covers = [], drive = Drive::Never)]
fn check_failed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = UpdateCheckDialog::new("1.0.0");
    dialog.set_status(Status::Failed("Could not reach api.github.com: connection timed out.".into()));
    dialog.present(ui).0.into()
}

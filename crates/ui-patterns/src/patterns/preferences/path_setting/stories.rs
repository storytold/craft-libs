//! Stories of the path setting.

use super::PathSetting;
use crate::patterns::preferences::SettingsDraft;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_host::PickRequest;
use egui::Ui;

fn draft(path: &str) -> SettingsDraft {
    let mut draft = SettingsDraft::default();
    draft.set("integrations.readRoot", &path);
    draft
}

#[story(PathSetting<'static>, covers = [Default], drive = Drive::Never)]
fn path_setting_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let draft = data.value("empty", draft(""));
    PathSetting::new("Folder agents can open from", "integrations.readRoot", draft, PickRequest::folder("agents")).placeholder("None").show(ui).into()
}

#[story(PathSetting<'static>, covers = [], drive = Drive::Never)]
fn path_setting_filled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let draft = data.value("filled", draft("/photos/agents"));
    PathSetting::new("Folder agents can open from", "integrations.readRoot", draft, PickRequest::folder("agents")).show(ui).into()
}

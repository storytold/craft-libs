//! Stories of the version history panel and the preview banner.

use super::{VersionHistoryPanel, VersionPreviewBanner};
use crate::versions::{Branch, BranchId, BranchState, Version, VersionId};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const NOW: i64 = 10_000;

fn version(id: u64, title: &str, autosave: bool) -> Version {
    let created = i64::try_from(id).unwrap_or(0) * 1_000;
    Version { id: VersionId(id), title: title.into(), description: String::new(), created, author: "Ana".into(), autosave }
}

fn history() -> Vec<Version> {
    vec![version(1, "Draft", false), version(2, "", true), version(3, "", true), version(4, "Final", false)]
}

fn branches() -> Vec<Branch> {
    vec![Branch { id: BranchId(1), name: "Dark mode".into(), state: BranchState::Merged, created: 500, author: "Bo".into() }]
}

#[story(VersionHistoryPanel<'static>, covers = [Default, Many], drive = Drive::Never)]
fn version_panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    VersionHistoryPanel::new(&history(), &branches(), NOW).show(ui).into()
}

#[story(VersionHistoryPanel<'static>, covers = [Empty], drive = Drive::Never)]
fn version_panel_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    VersionHistoryPanel::new(&[], &[], NOW).show(ui).into()
}

#[story(VersionHistoryPanel<'static>, covers = [], drive = Drive::Never)]
fn version_panel_previewing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    VersionHistoryPanel::new(&history(), &branches(), NOW).previewing(VersionId(4)).show(ui).into()
}

#[story(VersionPreviewBanner<'static>, covers = [Default], drive = Drive::Never)]
fn version_banner(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let canvas = ui.available_rect_before_wrap();
    VersionPreviewBanner::new(&version(4, "Final", false), NOW).show(ui, canvas).into()
}

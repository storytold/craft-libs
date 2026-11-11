//! Stories of the font sources.

use std::sync::Arc;

use craft_host::{AppInfo, Host, ManualJobs};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::FontSources;
use crate::panel::fixtures::{SharedPanel, story_panel};

const APP: AppInfo = AppInfo { id: "font-sources-story", name: "Font sources story", version: "1.0" };

/// The user's index the second story adds.
const USER_INDEX: &str = "https://example.org/fonts/index.json";

fn sources(ui: &mut Ui, data: &mut StoryData, make: fn() -> SharedPanel) -> StoryOutput {
    let shared = data.value::<Option<SharedPanel>>("panel", None).get_or_insert_with(make).clone();
    let mut panel = shared.lock();
    FontSources::new(&mut panel).show(ui).into()
}

fn built_in_panel() -> SharedPanel {
    SharedPanel::new(story_panel())
}

fn user_panel() -> SharedPanel {
    let mut panel = story_panel();
    if panel.installer.add_user_source(USER_INDEX).is_ok() {
        panel.reload_index();
    }
    SharedPanel::new(panel)
}

/// A panel whose source download waits on a job that never runs.
fn fetching_panel() -> SharedPanel {
    let mut panel = story_panel();
    let host = Arc::new(Host::builder(APP).jobs(Arc::new(ManualJobs::new())).build());
    panel.fetch_sources(Some(&host));
    SharedPanel::new(panel)
}

#[story(FontSources<'static>, covers = [], drive = Drive::Never)]
fn built_in_sources(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sources(ui, data, built_in_panel)
}

#[story(FontSources<'static>, covers = [], drive = Drive::Never)]
fn a_user_source(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sources(ui, data, user_panel)
}

#[story(FontSources<'static>, covers = [], drive = Drive::Never)]
fn fetching(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sources(ui, data, fetching_panel)
}

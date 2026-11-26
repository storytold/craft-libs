//! Stories of the synced pair: the base look and the captioned variant.

use super::{Lead, SyncedViews, SyncedViewsState};
use crate::ui::fixtures::{DemoPages, PAPER};
use crate::ui::view::{PageView, PageViewState};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// Pages in each half's document: enough to scroll.
const PAGES: usize = 4;

fn pair(ui: &mut Ui, data: &mut StoryData, captions: Option<(&'static str, &'static str)>) -> StoryOutput {
    let (state, left, right) = data.value("pair", (SyncedViewsState::default(), PageViewState::default(), PageViewState::default()));
    let pages = DemoPages::new(PAGES);
    let mut views = SyncedViews::new("Compare", state);
    if let Some((old, new)) = captions {
        views = views.captions(old, new);
    }
    let out = views.halves(ui, (left, right), |ui, rect, state, half| {
        let label = if half == Lead::Left { "Old pages" } else { "New pages" };
        Some(PageView::new(label, &pages, state, PAPER).show_at(ui, rect, |_, _| ()))
    });
    out.edit.response.into()
}

#[story(SyncedViews<'static>, covers = [], drive = Drive::Never)]
fn plain_pair(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    pair(ui, data, None)
}

#[story(SyncedViews<'static>, covers = [], drive = Drive::Never)]
fn captioned_pair(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    pair(ui, data, Some(("Old file: report.pdf", "New file: report.pdf")))
}

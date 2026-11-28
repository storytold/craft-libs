//! Stories of `CanvasPanel`.

use super::CanvasPanel;
use crate::contract::Show as _;
use crate::display::Text;
use crate::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(CanvasPanel<'static>, covers = [Default], drive = Drive::Never)]
fn canvas_panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CanvasPanel::new()
        .tabs(|ui, _| {
            Text::new("Tabs").show(ui);
        })
        .show(ui, |ui, _| {
            Text::new("Document").show(ui);
        })
        .into()
}

/// The empty workspace: tabs with no document, so the canvas surround fills the document area.
#[story(CanvasPanel<'static>, covers = [Default], drive = Drive::Never)]
fn empty_canvas_panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CanvasPanel::new()
        .tabs(|ui, _| {
            Text::new("Tabs").show(ui);
        })
        .show(ui, |_, _| {})
        .into()
}

/// Large tabs: the strip is `size.bar.tab_large` high, the height the panel gives its tab pane.
#[story(CanvasPanel<'static>, covers = [], drive = Drive::Never)]
fn canvas_panel_with_large_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs = [DocumentTab::new(0_usize, "Poster"), DocumentTab::new(1, "Logo")];
    let shown = data.value("shown", 0_usize);
    let out = CanvasPanel::new()
        .large_tabs(true)
        .tabs(|ui, _| {
            for event in DocumentTabs::new("Documents", &tabs).selected(*shown).large(true).show(ui).events {
                if let DocumentTabsEvent::Selected(tab) = event {
                    *shown = tab;
                }
            }
        })
        .show(ui, |ui, _| {
            Text::new("Document").show(ui);
        });
    out.into()
}

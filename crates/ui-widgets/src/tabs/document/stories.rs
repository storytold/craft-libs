//! Stories of `DocumentTabs`.

use super::{DocumentTab, DocumentTabs};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The tabs every story shows: a modified one, two open ones and one still opening.
fn tabs() -> [DocumentTab<usize>; 4] {
    [
        DocumentTab::new(0, "Untitled-1").modified(true),
        DocumentTab::new(1, "Photo.psd"),
        DocumentTab::new(2, "Poster.psd"),
        DocumentTab::new(3, "Scan.tif").progress(Some(0.4)).detail("Opening").close_label("Cancel opening"),
    ]
}

#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn document_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs = tabs();
    let shown = *data.value("shown", 1_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).reorderable(true).tear_off(true).takes_files(true).show(ui);
    out.response.into()
}

#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn large_document_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs = tabs();
    let shown = *data.value("shown", 1_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).reorderable(true).takes_files(true).large(true).show(ui);
    out.response.into()
}

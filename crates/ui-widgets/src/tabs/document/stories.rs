//! Stories of `DocumentTabs`.

use super::{DocumentTab, DocumentTabs};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn document_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs = [DocumentTab::new(0, "Untitled-1").modified(true), DocumentTab::new(1, "Photo.psd"), DocumentTab::new(2, "Poster.psd")];
    let shown = *data.value("shown", 1_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).reorderable(true).tear_off(true).show(ui);
    out.response.into()
}

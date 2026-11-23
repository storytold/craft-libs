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

/// Many long document names exercise the strip's overflow behaviour.
#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn many_document_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs: Vec<_> = (0..12).map(|i| DocumentTab::new(i, format!("Document number {i}, scanned.pdf"))).collect();
    let shown = *data.value("shown", 0_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).reorderable(true).show(ui);
    out.response.into()
}

/// Hover text on a tab cut to fit, which shows its full name then the text, and on a short tab, which shows the text
/// alone. The long title is cut to fit the strip.
#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn document_tabs_with_tooltips(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const LONG: &str = "Quarterly report for the board of directors, final revision with every appendix, the audit letter, the \
                        budget tables and the minutes of each meeting held this year.pdf";
    let tabs = [
        DocumentTab::new(0, LONG).tooltip(format!("/home/u/Documents/Reports/{LONG}")),
        DocumentTab::new(1, "Memo.pdf").tooltip("/home/u/Documents/Memo.pdf"),
    ];
    let shown = *data.value("shown", 0_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).show(ui);
    out.response.into()
}

/// A tab that does not move: the last tab stays last, and no other tab is dragged or key-moved past it.
#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn document_tabs_with_fixed_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs =
        [DocumentTab::new(0, "Photo.psd"), DocumentTab::new(1, "Poster.psd"), DocumentTab::new(2, "Combine files").closable(false).movable(false)];
    let shown = *data.value("shown", 0_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).reorderable(true).show(ui);
    out.response.into()
}

#[story(DocumentTabs<'static, usize>, covers = [], drive = Drive::Never)]
fn large_document_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tabs = tabs();
    let shown = *data.value("shown", 1_usize);
    let out = DocumentTabs::new("Documents", &tabs).selected(shown).reorderable(true).takes_files(true).large(true).show(ui);
    out.response.into()
}

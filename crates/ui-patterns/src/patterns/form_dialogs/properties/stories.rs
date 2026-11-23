//! Stories of `PropertiesDialog`.

use super::{PropertiesDialog, PropertiesTab, PropertyRow};
use crate::contract::Bind;
use crate::patterns::form_dialogs::{DialogFields, Param, ParamKind, SharedRows};
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn tabs() -> Vec<PropertiesTab> {
    vec![
        PropertiesTab::new(
            "General",
            vec![
                PropertyRow::value("Kind", "Photoshop document"),
                PropertyRow::value("Size", "3000 x 2000 px"),
                PropertyRow::heading("Description"),
                PropertyRow::field(Param::new("title", "Title", ParamKind::text(""))),
                PropertyRow::field(Param::new("locked", "Locked", ParamKind::bool(false))),
            ],
        ),
        PropertiesTab::new("Colour", vec![PropertyRow::value("Mode", "RGB"), PropertyRow::field(Param::number("opacity", "Opacity", 100.0, 1.0, 0))]),
    ]
}

/// A properties dialog over `tabs`, open while the story's `open` value is on, so the gallery can close and reopen it.
/// The story's `values` are copied out for the pass and written back after it, as one story value is borrowed at a time.
fn shown(
    ui: &mut Ui,
    data: &mut StoryData,
    tabs: Vec<PropertiesTab>,
    configure: impl for<'a> FnOnce(PropertiesDialog<'a>) -> PropertiesDialog<'a>,
) -> StoryOutput {
    let mut values = data.value("values", DialogFields::new()).clone();
    let output = {
        let dialog = PropertiesDialog::new("properties", "Properties", tabs, Bind::bound(&mut values)).opened(Bind::bound(data.value("open", true)));
        configure(dialog).show(ui)
    };
    *data.value("values", DialogFields::new()) = values;
    output.into()
}

#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let read_only = *data.value("read_only", false);
    shown(ui, data, tabs(), |dialog| dialog.read_only(read_only))
}

#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let read_only = *data.value("read_only", true);
    shown(ui, data, tabs(), |dialog| dialog.read_only(read_only))
}

/// The editable tabs and a read-only one, shown first, whose values come from the file under a note.
#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let note = Label::from("Set by the application that made the file.");
    let file =
        PropertiesTab::new("File", vec![PropertyRow::value("Producer", "PdfCraft"), PropertyRow::value("PDF Version", "1.7")]).read_only(Some(note));
    let mut all = vec![file];
    all.extend(tabs());
    shown(ui, data, all, |dialog| dialog)
}

/// Rows shown with every tab, under the tab's rows, and an extra footer button.
#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog_extras(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shared = vec![PropertyRow::field(Param::new("applyAll", "Apply to all pages", ParamKind::bool(false)))];
    shown(ui, data, tabs(), |dialog| {
        dialog.shared(shared).shared_rows(SharedRows::Below).extra_buttons(vec![("metadata".to_owned(), Label::from("Additional Metadata…"))])
    })
}

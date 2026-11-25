//! Stories of `PropertiesDialog`.

use super::{PropertiesDialog, PropertiesTab, PropertyRow, TabLayout};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, clicked};
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

/// Tabs whose keys Reset Panel puts back: the first and the last are resettable, the middle one is not.
fn resettable_tabs() -> Vec<PropertiesTab> {
    vec![
        PropertiesTab::new(
            "Character",
            vec![
                PropertyRow::field(Param::number("size", "Size", 12.0, 1.0, 1)),
                PropertyRow::field(Param::new("title", "Title", ParamKind::text(""))),
            ],
        )
        .key("character")
        .resettable(),
        PropertiesTab::new("Notes", vec![PropertyRow::field(Param::new("note", "Note", ParamKind::text("")))]).key("notes"),
        PropertiesTab::new("Paragraph", vec![PropertyRow::field(Param::number("indent", "Indent", 0.0, 1.0, 0))]).key("paragraph").resettable(),
    ]
}

/// A properties dialog that starts closed; a button opens it.
fn closed_dialog(ui: &mut Ui, data: &mut StoryData, layout: TabLayout, tabs: Vec<PropertiesTab>) -> StoryOutput {
    let (open, values) = data.value("properties", (false, DialogFields::new()));
    if clicked(&Button::new("Open properties").show(ui)) {
        *open = true;
    }
    PropertiesDialog::new("properties", "Properties", tabs, Bind::bound(values)).tab_layout(layout).opened(Bind::bound(open)).show(ui).into()
}

#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog_tab_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    closed_dialog(ui, data, TabLayout::List, tabs())
}

#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog_resettable_tabs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    closed_dialog(ui, data, TabLayout::Strip, resettable_tabs())
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

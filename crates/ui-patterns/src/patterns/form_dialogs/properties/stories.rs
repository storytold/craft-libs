//! Stories of `PropertiesDialog`.

use super::{PropertiesDialog, PropertiesTab, PropertyRow, TabLayout};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, clicked};
use crate::patterns::form_dialogs::{DialogFields, Param, ParamKind};
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

#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let read_only = *data.value("read_only", false);
    let (values, values_at) = data.value_at("values", DialogFields::new());
    PropertiesDialog::new("Properties", tabs(), values_at.bind(values)).read_only(read_only).opened(Bind::given(true)).show(ui).into()
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
    PropertiesDialog::new("Properties", tabs, Bind::bound(values)).tab_layout(layout).opened(Bind::bound(open)).show(ui).into()
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
    let (values, values_at) = data.value_at("values", DialogFields::new());
    PropertiesDialog::new("Properties", tabs(), values_at.bind(values)).read_only(read_only).opened(Bind::given(true)).show(ui).into()
}

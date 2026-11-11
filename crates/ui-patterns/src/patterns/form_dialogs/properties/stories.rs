//! Stories of `PropertiesDialog`.

use super::{PropertiesDialog, PropertiesTab, PropertyRow};
use crate::contract::Bind;
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
    let values = data.value("values", DialogFields::new());
    PropertiesDialog::new("Properties", tabs(), Bind::bound(values)).read_only(read_only).opened(Bind::given(true)).show(ui).into()
}

#[story(PropertiesDialog<'static>, covers = [], drive = Drive::Never)]
fn properties_dialog_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let read_only = *data.value("read_only", true);
    let values = data.value("values", DialogFields::new());
    PropertiesDialog::new("Properties", tabs(), Bind::bound(values)).read_only(read_only).opened(Bind::given(true)).show(ui).into()
}

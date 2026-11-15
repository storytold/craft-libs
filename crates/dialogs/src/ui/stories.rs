//! Stories of the path field.

use super::PathField;
use craft_host::PickRequest;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PathField<'static>, covers = [Default], drive = Drive::Never)]
fn path_field_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PathField::new("Save to", Bind::bound(data.value("empty", String::new())), PickRequest::save("export")).show(ui).into()
}

#[story(PathField<'static>, covers = [], drive = Drive::Never)]
fn path_field_filled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PathField::new("Save to", Bind::bound(data.value("filled", "/photos/trip/sunrise.png".to_owned())), PickRequest::save("export")).show(ui).into()
}

#[story(PathField<'static>, covers = [], drive = Drive::Never)]
fn path_field_missing_file(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PathField::new("Open", Bind::bound(data.value("missing", "/photos/trip/missing.png".to_owned())), PickRequest::open("import"))
        .must_exist(true)
        .show(ui)
        .into()
}

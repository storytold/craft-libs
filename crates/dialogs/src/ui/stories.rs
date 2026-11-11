//! Stories of the path field and the file list picker.

use super::{FileList, FileListMode, FileListPicker, PathField};
use craft_host::PickRequest;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::path::PathBuf;

/// A name of the files the stories list.
const FILES: [&str; 3] = ["sunrise.png", "harbour.png", "market.png"];

fn photos() -> Vec<PathBuf> {
    FILES.iter().map(|name| PathBuf::from("/photos/trip").join(name)).collect()
}

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

#[story(FileListPicker<'static>, covers = [Many], drive = Drive::Never)]
fn file_list_picker_files(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = FileList::default();
    start.add(photos());
    let open = [PathBuf::from("/photos/trip/dusk.png")];
    FileListPicker::new("Files to load", Bind::bound(data.value("files", start)))
        .folder_mode(true)
        .open_files(&open)
        .info(&|_| Some("1200 × 800 px, 300 ppi, RGB".to_owned()))
        .show(ui)
        .into()
}

#[story(FileListPicker<'static>, covers = [Empty], drive = Drive::Never)]
fn file_list_picker_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    FileListPicker::new("Files to load", Bind::bound(data.value("empty", FileList::default()))).show(ui).into()
}

#[story(FileListPicker<'static>, covers = [], drive = Drive::Never)]
fn file_list_picker_folder(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = FileList { mode: FileListMode::Folder, folder: Some(PathBuf::from("/photos/trip")), ..FileList::default() };
    let files = photos();
    FileListPicker::new("Files to load", Bind::bound(data.value("folder", start))).folder_mode(true).folder_files(&files).show(ui).into()
}

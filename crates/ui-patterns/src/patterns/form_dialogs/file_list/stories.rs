//! Stories of the file list picker.

use super::{FileList, FileListMode, FileListPicker};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::path::PathBuf;

/// A name of the files the stories list.
const FILES: [&str; 3] = ["sunrise.png", "harbour.png", "market.png"];

fn photos() -> Vec<PathBuf> {
    FILES.iter().map(|name| PathBuf::from("/photos/trip").join(name)).collect()
}

#[story(FileListPicker<'static>, covers = [Many], drive = Drive::Never)]
fn file_list_picker_files(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = FileList::default();
    start.add(photos());
    let open = [PathBuf::from("/photos/trip/dusk.png")];
    FileListPicker::new("Files to load", data.bind("files", start))
        .folder_mode(true)
        .open_files(&open)
        .info(&|_| Some("1200 × 800 px, 300 ppi, RGB".to_owned()))
        .show(ui)
        .into()
}

#[story(FileListPicker<'static>, covers = [Empty], drive = Drive::Never)]
fn file_list_picker_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    FileListPicker::new("Files to load", data.bind("empty", FileList::default())).show(ui).into()
}

#[story(FileListPicker<'static>, covers = [], drive = Drive::Never)]
fn file_list_picker_folder(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = FileList { mode: FileListMode::Folder, folder: Some(PathBuf::from("/photos/trip")), ..FileList::default() };
    let files = photos();
    FileListPicker::new("Files to load", data.bind("folder", start)).folder_mode(true).folder_files(&files).show(ui).into()
}

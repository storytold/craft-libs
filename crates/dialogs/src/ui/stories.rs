//! Stories of the path field and the fallback file chooser.

use std::sync::Arc;

use super::{FileChooser, PathField};
use craft_browse::places::{FakeDirectories, FolderCache, StandardFolder};
use craft_host::{ManualJobs, PickRequest, Wake};
use craft_ui::dialog::stories::hosted_with;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PathField<'static>, covers = [Default], drive = Drive::Never)]
fn path_field_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PathField::new("Save to", data.bind("empty", String::new()), PickRequest::save("export")).show(ui).into()
}

#[story(PathField<'static>, covers = [], drive = Drive::Never)]
fn path_field_filled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PathField::new("Save to", data.bind("filled", "/photos/trip/sunrise.png".to_owned()), PickRequest::save("export")).show(ui).into()
}

#[story(PathField<'static>, covers = [], drive = Drive::Never)]
fn path_field_missing_file(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PathField::new("Open", data.bind("missing", "/photos/trip/missing.png".to_owned()), PickRequest::open("import")).must_exist(true).show(ui).into()
}

/// The folders the chooser stories browse: a home with documents, pictures, a hidden folder and an empty folder.
fn story_tree() -> FakeDirectories {
    let mut fake = FakeDirectories::new().standard_folder(StandardFolder::Home, "/home/u").folder("/home/u/Empty").folder("/home/u/.cache");
    for file in [
        "/home/u/Documents/Chapter 1.docx",
        "/home/u/Documents/Chapter 2.docx",
        "/home/u/Documents/Chapter 10.docx",
        "/home/u/Documents/notes.txt",
        "/home/u/Documents/.draft.docx",
        "/home/u/Documents/Research/sources.txt",
        "/home/u/Documents/cover.png",
        "/home/u/Pictures/sunrise.jpg",
    ] {
        fake = fake.file(file);
    }
    fake
}

/// A chooser for `request` over [`story_tree`] on the story's dialog host, its listing jobs run before each pass;
/// `cause` is the unavailable panel's message, when the story shows one.
fn chooser_story(ui: &mut Ui, data: &mut StoryData, name: &'static str, request: PickRequest, cause: Option<&'static str>) -> StoryOutput {
    let jobs = data.note("jobs", Arc::new(ManualJobs::new())).clone();
    let (running, waiting) = (jobs.clone(), jobs.clone());
    let ctx = ui.ctx().clone();
    let open = move || {
        let wake = Wake::new(move || ctx.request_repaint());
        FileChooser::new(request, cause.map(str::to_owned), FolderCache::new(Arc::new(story_tree()), jobs, wake))
    };
    let facts = hosted_with(ui, data, name, open, move |_| {
        running.run_all();
    });
    // A listing asked for in this pass runs before the next one.
    if waiting.waiting() > 0 {
        ui.ctx().request_repaint();
    }
    facts
}

#[story(FileChooser, covers = [Default], drive = Drive::Never)]
fn file_chooser_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let request =
        PickRequest::open("document").filter("Documents", &["docx", "txt"]).filter("Pictures", &["png", "jpg"]).directory("/home/u/Documents");
    chooser_story(ui, data, "open", request, None)
}

#[story(FileChooser, covers = [], drive = Drive::Never)]
fn file_chooser_save(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let request = PickRequest::save("document").filter("Word document", &["docx"]).file_name("Chapter 3.docx").directory("/home/u/Documents");
    chooser_story(ui, data, "save", request, None)
}

#[story(FileChooser, covers = [], drive = Drive::Never)]
fn file_chooser_empty_folder(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    chooser_story(ui, data, "empty", PickRequest::open("document").directory("/home/u/Empty"), None)
}

#[story(FileChooser, covers = [], drive = Drive::Never)]
fn file_chooser_unavailable(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let request = PickRequest::save("document").file_name("Chapter 3.docx");
    chooser_story(ui, data, "unavailable", request, Some("no file chooser portal and no zenity"))
}

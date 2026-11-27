//! `FileListPicker` and the Files row of `ParamRows`, against a file service that holds its picks until the test
//! answers them.

use craft_host::{AppInfo, Host, PickId, Picked, ScriptedFiles};
use craft_theme::Size;
use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, FileList, FileListPicker, Param, ParamRows};
use craft_ui::runtime::requests::ListingRequest;
use craft_ui::testing::{Fixture, trial};
use egui::Rect;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

fn host_with(files: &Arc<ScriptedFiles>) -> Arc<Host> {
    Arc::new(Host::builder(AppInfo { id: "test", name: "Test", version: "1.0" }).files(files.clone()).build())
}

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|n| PathBuf::from(format!("/photos/{n}"))).collect()
}

fn names(list: &FileList) -> Vec<String> {
    list.files.iter().map(|p| p.file_name().expect("a file name").to_string_lossy().into_owned()).collect()
}

/// An answer arriving while the list shows lands in the list once and is written to the binding once: the footer's
/// sizing pass takes and writes nothing.
#[trial]
fn browse_answer_is_taken_and_written_once() {
    let files = Arc::new(ScriptedFiles::held());
    let list = Rc::new(RefCell::new(FileList::default()));
    let changes = Rc::new(Cell::new(0usize));
    let ticket = Rc::new(Cell::new(None::<PickId>));
    let (held, counted, issued) = (Rc::clone(&list), Rc::clone(&changes), Rc::clone(&ticket));
    let mut fixture = Fixture::new(move |ui| {
        let mut list = held.borrow_mut();
        let out = FileListPicker::new("Files to load", Bind::bound(&mut *list)).show(ui);
        if out.browsing.is_some() {
            issued.set(out.browsing);
        }
        if out.changed {
            counted.set(counted.get() + 1);
        }
    });
    fixture.host(host_with(&files));
    fixture.settle();
    let browse = fixture.by_label("Browse…").expect("the Browse button");
    fixture.click(&browse);
    fixture.settle();
    changes.set(0);

    let id = ticket.get().expect("Browse issued a ticket");
    assert!(files.answer_to(id, Picked::Paths(paths(&["a.png", "b.png"]))), "the pick is open");
    fixture.idle(3);
    assert_eq!(names(&list.borrow()), ["a.png", "b.png"]);
    assert_eq!(changes.get(), 1, "the answer is written to the binding once");
}

/// A files row in folder mode asks for the listing of a folder once, when the folder changes; later passes ask for
/// nothing, and the files the host answers under `"{key}.folderFiles"` are shown.
#[trial]
fn a_folder_row_asks_for_its_listing_once_per_folder_and_shows_the_answer() {
    let files = Arc::new(ScriptedFiles::held());
    let values = Rc::new(RefCell::new(DialogFields::from_pairs([("photos.useFolder", true)])));
    let asked = Rc::new(RefCell::new(Vec::<ListingRequest>::new()));
    let (shown, seen) = (values.clone(), asked.clone());
    let params = vec![Param::files("photos", "Photos").folder_mode(true)];
    let mut fixture = Fixture::new(move |ui| {
        let out = ParamRows::new("Form", &params, Bind::bound(&mut *shown.borrow_mut())).show(ui);
        seen.borrow_mut().extend(out.listing);
    });
    fixture.host(host_with(&files));
    fixture.settle();
    assert!(asked.borrow().is_empty(), "no folder, no request");

    let browse = fixture.by_label("Browse…").expect("the Browse button");
    fixture.click(&browse);
    fixture.settle();
    assert!(files.answer(Picked::Paths(vec![PathBuf::from("/shots")])).is_some(), "Browse opened a folder pick");
    fixture.idle(3);
    let requests: Vec<(String, PathBuf)> = asked.borrow().iter().map(|r| (r.key.clone(), r.folder.clone())).collect();
    assert_eq!(requests, [("photos".to_owned(), PathBuf::from("/shots"))], "one request for the new folder");
    assert_eq!(values.borrow().text("photos.folder"), Some("/shots"));

    // The host's answer, written as the dialog does.
    let listed = ["dawn.png", "dusk.png"].map(|name| FieldValue::Text(format!("/shots/{name}")));
    values.borrow_mut().set("photos.folderFiles", FieldValue::List(listed.to_vec()));
    fixture.idle(4);
    assert_eq!(asked.borrow().len(), 1, "later passes ask for nothing");
    assert!(fixture.by_label("dawn.png").is_some() && fixture.by_label("dusk.png").is_some(), "the answered files are listed");
}

/// In a room shorter than its column of buttons, the picker keeps its list a positive height and shows each button
/// whole, one under another, inside the picker.
#[trial]
fn a_short_room_keeps_the_list_and_buttons_inside_the_picker() {
    let picker = Rc::new(Cell::new(Rect::NOTHING));
    let shown = Rc::clone(&picker);
    let mut list = FileList::default();
    list.add(paths(&["a.png", "b.png"]));
    let mut fixture = Fixture::new(move |ui| shown.set(FileListPicker::new("Files to load", Bind::bound(&mut list)).show(ui).response.rect));
    fixture.density(Size::Comfortable).room(egui::vec2(400.0, 60.0)).settle();

    let picker = picker.get();
    let files = fixture.by_label("Files").expect("the file list").rect;
    assert!(files.height() > 0.0, "the list keeps a positive height: {files:?}");
    assert!(picker.contains_rect(files), "the list stays inside the picker: {files:?} in {picker:?}");
    let mut above = f32::NEG_INFINITY;
    for name in ["Browse…", "Remove", "Add Open Files", "Sort By Name"] {
        let button = fixture.by_label(name).unwrap_or_else(|| panic!("the {name} button")).rect;
        assert!(button.height() > 0.0 && picker.contains_rect(button), "{name} shows whole inside the picker: {button:?} in {picker:?}");
        assert!(button.min.y >= above, "{name} stands under the button before it: {button:?}");
        above = button.max.y;
    }
}

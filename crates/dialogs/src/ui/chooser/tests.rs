//! Trials of the fallback chooser: the accept rules of WordCraft's built-in browser, driven through the dialog on a
//! `DialogHost`, and its place in the file service.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used, reason = "trials assert by panicking")]

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft_browse::places::{FakeDirectories, FolderCache, StandardFolder};
use craft_host::{Files as _, ManualJobs, PickRequest, Picked, Wake};
use craft_ui::dialog::{DialogHost, DialogResult};
use craft_ui::testing::{Fixture, trial};
use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};
use serde_json::Value;

use super::{ChooserWords, FileChooser};
use crate::Answer;
use crate::service::FileService;
use crate::ui::UnavailableWords;

/// The chooser's fields, as automation lists them; `Null` once it closed.
fn fields(host: &RefCell<DialogHost>) -> Value {
    host.borrow().open_dialogs().into_iter().find_map(|open| open.fields.filter(|fields| fields.get("listed").is_some())).unwrap_or(Value::Null)
}

/// `/d` with two folders and three files, one of them named with a leading space, and a home folder with a letter.
fn tree() -> FakeDirectories {
    FakeDirectories::new()
        .standard_folder(StandardFolder::Home, "/home/u")
        .file("/home/u/Letters/aunt.txt")
        .folder("/d/drafts")
        .folder("/d/Chapters")
        .file("/d/drafts/old.docx")
        .file("/d/ report.docx")
        .file("/d/report.docx")
}

/// A chooser on a dialog host over [`tree`], its jobs run at the start of every frame. Each click's frames run two
/// passes, and only the first has the input.
struct Chooser {
    host: Rc<RefCell<DialogHost>>,
    fake: Arc<FakeDirectories>,
    fixture: Fixture,
    twice: Rc<Cell<bool>>,
}

impl Chooser {
    fn open(request: PickRequest) -> Self {
        let fake = Arc::new(tree());
        let jobs = Arc::new(ManualJobs::new());
        let cache = FolderCache::new(fake.clone(), jobs.clone(), Wake::none());
        let host = Rc::new(RefCell::new(DialogHost::new()));
        host.borrow_mut().open(FileChooser::new(request, None, cache));
        let twice = Rc::new(Cell::new(false));
        let (shown, rerun) = (host.clone(), twice.clone());
        let mut fixture = Fixture::new(move |ui| {
            jobs.run_all();
            shown.borrow_mut().run(ui);
            if rerun.get() && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("the chooser's pass runs twice");
            }
        });
        fixture.idle(10);
        let chooser = Chooser { host, fake, fixture, twice };
        assert_eq!(chooser.field("listed"), Value::Bool(true), "the folder is listed");
        chooser
    }

    fn field(&self, name: &str) -> Value {
        fields(&self.host).get(name).cloned().unwrap_or(Value::Null)
    }

    fn click(&mut self, label: &str) {
        let target = self.fixture.by_label(label).unwrap_or_else(|| panic!("`{label}` is shown"));
        self.twice.set(true);
        self.fixture.click(&target);
        self.twice.set(false);
        self.fixture.idle(6);
    }

    /// Types `text` into the (empty) name field.
    fn type_name(&mut self, text: &str) {
        self.click("File name");
        self.fixture.type_text(text);
        self.fixture.idle(2);
    }

    /// The window lost the focus and has it back: the listing is read again.
    fn refocus(&mut self) {
        self.fixture.frame(vec![egui::Event::WindowFocused(false)]);
        self.fixture.frame(vec![egui::Event::WindowFocused(true)]);
        self.fixture.idle(6);
    }

    fn answer(&self) -> Option<DialogResult<Answer>> {
        self.host.borrow_mut().take::<FileChooser>()
    }
}

/// The folder field's text for `child` in `dir`, joined with the platform's separator (a backslash on Windows).
fn shown(dir: &str, child: &str) -> String {
    Path::new(dir).join(child).display().to_string()
}

fn picked(path: impl AsRef<Path>) -> Option<DialogResult<Answer>> {
    Some(DialogResult::Ok(Answer::Paths(vec![path.as_ref().to_path_buf()])))
}

/// A Save's answer of `path`, from a request that offers no file types.
fn saved(path: impl AsRef<Path>) -> Option<DialogResult<Answer>> {
    Some(DialogResult::Ok(Answer::Saved { path: path.as_ref().to_path_buf(), filter: None }))
}

// WordCraft's `save_adds_the_extension_and_asks_before_replacing`, `selecting_a_file_preserves_leading_spaces`,
// `a_selected_folder_never_replaces_the_typed_save_name` and `a_removed_selected_file_keeps_its_exact_save_path`, with
// `~` and "not found". The extension is the service's policy now, so the chooser answers the name as typed.
#[trial]
fn a_typed_folder_is_entered_and_a_new_name_is_saved_as_typed() {
    // A typed folder is entered and its name used up; a new name is answered as typed.
    let mut save = Chooser::open(PickRequest::save("document").directory("/d"));
    save.type_name("drafts");
    save.click("Save");
    assert_eq!((save.field("folder"), save.field("name")), (Value::from(shown("/d", "drafts")), Value::from("")), "the typed folder opens");
    save.type_name("chapter");
    save.click("Save");
    assert_eq!(save.answer(), saved("/d/drafts/chapter"), "a new name is saved to as typed");
}

#[trial]
fn a_listed_file_is_replaced_only_after_the_question() {
    // A listed file is replaced only after the question, asked once.
    let mut replace = Chooser::open(PickRequest::save("document").directory("/d/drafts"));
    replace.type_name("old.docx");
    replace.click("Save");
    assert_eq!(replace.answer(), None, "the chooser asks before replacing");
    replace.click("Replace");
    assert_eq!(replace.answer(), saved("/d/drafts/old.docx"));
}

#[trial]
fn a_selected_file_keeps_its_exact_name_for_open_and_for_the_save_question() {
    // A selected file keeps its exact name, leading space and all, for Open and for Save's question.
    let mut open = Chooser::open(PickRequest::open("document").directory("/d"));
    open.click(" report.docx");
    open.click("Open");
    assert_eq!(open.answer(), picked("/d/ report.docx"), "Open answers the selected file, not its trimmed sibling");
    let mut save = Chooser::open(PickRequest::save("document").file_name("Draft.docx").directory("/d"));
    save.click(" report.docx");
    assert_eq!(save.field("name"), Value::from(" report.docx"), "the selected file's name fills the field");
    save.click("Save");
    assert_eq!(save.answer(), None, "Save asks before replacing the selected file");
    save.click("Replace");
    assert_eq!(save.answer(), saved("/d/ report.docx"));
}

#[trial]
fn a_removed_selected_file_keeps_its_exact_save_path() {
    // A selected file removed meanwhile still names the exact path, and nothing is asked.
    let mut removed = Chooser::open(PickRequest::save("document").file_name("Draft.docx").directory("/d"));
    removed.click(" report.docx");
    removed.fake.remove_file("/d/ report.docx");
    removed.refocus();
    removed.click("Save");
    assert_eq!(removed.answer(), saved("/d/ report.docx"), "the removed selection keeps its path, not `report.docx`");
}

#[trial]
fn a_selected_folder_is_entered_and_never_becomes_the_save_name() {
    // A selected folder is entered and never becomes the save name; once it is gone, the typed name is saved to.
    let mut folder = Chooser::open(PickRequest::save("document").file_name("chapter.docx").directory("/d"));
    folder.click("Chapters");
    assert_eq!(folder.field("name"), Value::from("chapter.docx"), "a folder leaves the name alone");
    folder.click("Save");
    assert_eq!((folder.field("folder"), folder.field("name")), (Value::from(shown("/d", "Chapters")), Value::from("chapter.docx")));
    let mut gone = Chooser::open(PickRequest::save("document").file_name("chapter.docx").directory("/d"));
    gone.click("Chapters");
    gone.fake.remove_folder("/d/Chapters");
    gone.refocus();
    gone.click("Save");
    assert_eq!(gone.answer(), saved("/d/chapter.docx"), "the removed folder is not saved over as `Chapters.docx`");
}

#[trial]
fn the_home_root_and_a_typed_path_elsewhere_open() {
    // `~` is the Home root; a typed path elsewhere is listed, then opened.
    let mut home = Chooser::open(PickRequest::open("document").directory("/d"));
    home.type_name("~/Letters/aunt.txt");
    home.click("Open");
    assert_eq!(home.answer(), picked("/home/u/Letters/aunt.txt"));
}

#[trial]
fn a_missing_name_on_open_is_refused_and_said() {
    // A missing name on Open is refused and said.
    let mut missing = Chooser::open(PickRequest::open("document").directory("/d"));
    missing.type_name("missing.docx");
    missing.click("Open");
    assert_eq!(missing.answer(), None, "nothing is answered");
    let not_found = ChooserWords::default().not_found.fill(&[("name", "missing.docx")]);
    assert!(missing.fixture.by_label(&not_found).is_some(), "the chooser says what was not found");
}

/// No window: `pump_with(.., None::<&NoWindow>)`.
struct NoWindow;

impl HasWindowHandle for NoWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

impl HasDisplayHandle for NoWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

const NONE: Option<&NoWindow> = None;

// WordCraft's `no_system_dialog_falls_back_to_the_built_in_browser`: the service opens the chooser over the real
// folders, and its answer settles like a panel's.
#[trial]
fn an_unavailable_or_forced_pick_opens_the_chooser_and_settles_its_answer() {
    let dir = std::env::temp_dir().join(format!("craft-dialogs-chooser-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("report.docx"), b"x").unwrap();

    let (service, panels) = FileService::scripted(Wake::none(), [Answer::Unavailable("no portal".to_owned())]);
    let panels = Rc::new(RefCell::new(panels));
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let (pumped, shown) = (panels.clone(), host.clone());
    let mut fixture = Fixture::new(move |ui| {
        pumped.borrow_mut().pump_with(&mut shown.borrow_mut(), NONE);
        shown.borrow_mut().run(ui);
    });
    // The service lists the real folder on its own threads: wait until the chooser is listed.
    let listed = |fixture: &mut Fixture| {
        for _ in 0..400 {
            fixture.idle(1);
            if fields(&host).get("listed") == Some(&Value::Bool(true)) {
                // The chooser lays out its listing in the next passes: a click reads where the controls rest.
                fixture.settle();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the chooser listed the folder");
    };
    let click = |fixture: &mut Fixture, label: &str| {
        let target = fixture.by_label(label).unwrap_or_else(|| panic!("`{label}` is shown"));
        fixture.click(&target);
        fixture.idle(6);
    };

    // The panel answers Unavailable: the chooser opens with the cause, and a new name gets the suggested extension.
    let id = service.pick(PickRequest::save("document").directory(&dir).suggested_extension("docx"));
    listed(&mut fixture);
    assert!(host.borrow().is_open::<FileChooser>(), "the chooser stands in for the panel");
    assert!(fixture.by_label(&UnavailableWords::default().sentence("no portal")).is_some(), "it says why");
    click(&mut fixture, "File name");
    fixture.type_text("novel");
    click(&mut fixture, "Save");
    assert_eq!(service.take_answer(id), Some(Picked::Saved { path: dir.join("novel.docx"), file_type: None }), "the policy adds the extension");

    // A forced chooser: `report` is no listed file, so the chooser asks nothing; the policy's `report.docx` is,
    // so the service asks once.
    let id = service.pick(PickRequest::save("document").directory(&dir).suggested_extension("docx").fallback(true));
    listed(&mut fixture);
    click(&mut fixture, "File name");
    fixture.type_text("report");
    click(&mut fixture, "Save");
    assert_eq!(service.take_answer(id), None, "the Replace question waits");
    click(&mut fixture, "Replace");
    assert_eq!(service.take_answer(id), Some(Picked::Saved { path: dir.join("report.docx"), file_type: None }));

    // Cancel settles as Cancelled.
    let id = service.pick(PickRequest::open("document").directory(&dir).fallback(true));
    listed(&mut fixture);
    click(&mut fixture, "Cancel");
    assert_eq!(service.take_answer(id), Some(Picked::Cancelled));
    assert!(!host.borrow().is_open::<FileChooser>(), "the chooser closed");
    let _ = std::fs::remove_dir_all(&dir);
}

//! The path field and the picker ticket it keeps, against a file service that records its requests.

use craft_dialogs::ui::PathField;
use craft_host::{AppInfo, FileError, Files, Host, PickAnswer, PickId, PickRequest, Picked};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Records every request; answers are put in by the test.
#[derive(Default)]
struct StubFiles {
    state: Mutex<StubState>,
}

#[derive(Default)]
struct StubState {
    requests: Vec<PickRequest>,
    answered: Vec<PickAnswer>,
}

impl StubFiles {
    /// Answers ticket `id`; a ticket this service never issued is answered as a request of another feature.
    fn answer(&self, id: PickId, picked: Picked) {
        let mut state = self.state.lock().expect("the stub is not poisoned");
        let request = usize::try_from(id.0 - 1).ok().and_then(|at| state.requests.get(at).cloned()).unwrap_or_else(|| PickRequest::open("other"));
        state.answered.push(PickAnswer { id, request, picked });
    }
}

impl Files for StubFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.requests.push(request);
        PickId(state.requests.len() as u64)
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let at = state.answered.iter().position(|a| a.id == id)?;
        Some(state.answered.remove(at).picked)
    }

    fn forget(&self, id: PickId) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).answered.retain(|a| a.id != id);
    }

    fn pick_pending(&self) -> bool {
        false
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }

    fn write(&self, _path: &Path, _bytes: &[u8]) -> Result<(), FileError> {
        Err(FileError::Unsupported)
    }

    fn save(&self, request: PickRequest, _bytes: Vec<u8>) -> PickId {
        self.pick(request)
    }

    fn can_pick(&self) -> bool {
        true
    }

    fn browser_saves(&self) -> bool {
        false
    }

    fn last_folder(&self, _purpose: &str) -> Option<PathBuf> {
        None
    }
}

fn host_with(files: &Arc<StubFiles>) -> Arc<Host> {
    Arc::new(Host::builder(AppInfo { id: "test", name: "Test", version: "1.0" }).files(files.clone()).build())
}

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|n| PathBuf::from(format!("/photos/{n}"))).collect()
}

/// The path field's Browse queues a pick through the file service, and the answer lands in the field's path once.
#[trial]
fn path_field_browse_fills_the_path() {
    let files = Arc::new(StubFiles::default());
    let path = Rc::new(RefCell::new(String::new()));
    let sets = Rc::new(Cell::new(0usize));
    let ticket = Rc::new(Cell::new(None::<PickId>));
    let (held, counted, issued) = (Rc::clone(&path), Rc::clone(&sets), Rc::clone(&ticket));
    let mut fixture = Fixture::new(move |ui| {
        let mut path = held.borrow_mut();
        let out = PathField::new("PDF file", Bind::bound(&mut *path), PickRequest::open("import")).show(ui);
        if out.browsing.is_some() {
            issued.set(out.browsing);
        }
        if out.edit.is_some() {
            counted.set(counted.get() + 1);
        }
    });
    fixture.host(host_with(&files));
    fixture.settle();
    let browse = fixture.by_label("Browse…").expect("the Browse button");
    fixture.click(&browse);
    fixture.settle();
    sets.set(0);

    files.answer(ticket.get().expect("Browse issued a ticket"), Picked::Paths(paths(&["brochure.pdf"])));
    fixture.idle(3);
    assert_eq!(*path.borrow(), "/photos/brochure.pdf");
    assert_eq!(sets.get(), 1, "the answer lands once");
}

/// Picks `request` on a file service whose panel answers `answer`, and returns the answer with
/// whether the check box was ticked.
fn checked_answer(request: PickRequest, answer: craft_dialogs::Answer) -> (Picked, bool) {
    use craft_dialogs::service::FileService;
    let (service, mut panels) = FileService::scripted(craft_host::Wake::none(), [answer]);
    let id = service.pick(request);
    let mut host = craft_ui::dialog::DialogHost::new();
    panels.pump_with::<NoWindow>(&mut host, None);
    let answered = service.take_answer_checked(id).expect("the pick is answered");
    assert_eq!(service.take_answer_checked(id), None, "an answer is read once");
    answered
}

/// No window for the panels: the scripted service never shows one.
struct NoWindow;

impl raw_window_handle::HasWindowHandle for NoWindow {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::Unavailable)
    }
}

impl raw_window_handle::HasDisplayHandle for NoWindow {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Err(raw_window_handle::HandleError::Unavailable)
    }
}

/// An Open with a check box answers its files and whether the box was ticked; a ticked answer to a
/// request without a box, an unticked one, and a ticked answer naming no file all read unticked.
#[trial]
fn an_open_check_box_answers_whether_it_was_ticked() {
    use craft_dialogs::Answer;
    let sequence = || PickRequest::open("document").check_box("Image Sequence");
    let one = paths(&["frame_001.png"]);
    assert_eq!(checked_answer(sequence(), Answer::Checked(one.clone())), (Picked::Paths(one.clone()), true), "ticked");
    assert_eq!(checked_answer(sequence(), Answer::Paths(one.clone())), (Picked::Paths(one.clone()), false), "left off");
    assert_eq!(checked_answer(PickRequest::open("document"), Answer::Checked(one.clone())), (Picked::Paths(one.clone()), false), "no box asked for");
    assert_eq!(checked_answer(sequence(), Answer::Checked(Vec::new())), (Picked::Cancelled, false), "nothing picked");
    let many = paths(&["a.png", "b.png"]);
    let open_many = PickRequest::open_many("document").check_box("Image Sequence");
    assert_eq!(checked_answer(open_many, Answer::Checked(many.clone())), (Picked::Paths(many), true), "several files, ticked");
}

/// The scripted service of app tests answers the check box as queued, and reads it once; plain
/// `take_answer` reads the same paths.
#[trial]
fn scripted_files_answer_the_check_box() {
    use craft_host::ScriptedFiles;
    let one = paths(&["frame_001.png"]);
    let files = ScriptedFiles::new();
    files.queue_checked(Picked::Paths(one.clone()));
    files.queue(Picked::Paths(one.clone()));
    files.queue_checked(Picked::Paths(one.clone()));
    let ticked = files.pick(PickRequest::open("document").check_box("Image Sequence"));
    let plain = files.pick(PickRequest::open("document").check_box("Image Sequence"));
    let unasked = files.pick(PickRequest::open("document"));
    assert_eq!(files.take_answer_checked(ticked), Some((Picked::Paths(one.clone()), true)));
    assert_eq!(files.take_answer_checked(plain), Some((Picked::Paths(one.clone()), false)));
    assert_eq!(files.take_answer_checked(unasked), Some((Picked::Paths(one.clone()), false)), "no box asked for");
    assert_eq!(files.requests()[0].check_box.as_deref(), Some("Image Sequence"));

    let held = ScriptedFiles::held();
    let id = held.pick(PickRequest::open("document").check_box("Image Sequence"));
    assert_eq!(held.answer_checked(Picked::Paths(one.clone())), Some(id));
    assert_eq!(held.take_answer(id), Some(Picked::Paths(one)), "take_answer reads the same paths");
    assert_eq!(held.take_answer_checked(id), None);
}

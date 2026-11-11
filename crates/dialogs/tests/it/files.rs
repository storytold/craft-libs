//! The file list, the path field and the picker tickets they keep, against a file service that records its requests.

use craft_dialogs::ui::{FileList, FileListPicker};
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

fn names(list: &FileList) -> Vec<String> {
    list.files.iter().map(|p| p.file_name().expect("a file name").to_string_lossy().into_owned()).collect()
}

/// An answer arriving while the list shows lands in the list once and is written to the binding once: the footer's
/// sizing pass takes and writes nothing.
#[trial]
fn browse_answer_is_taken_and_written_once() {
    let files = Arc::new(StubFiles::default());
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

    files.answer(ticket.get().expect("Browse issued a ticket"), Picked::Paths(paths(&["a.png", "b.png"])));
    fixture.idle(3);
    assert_eq!(names(&list.borrow()), ["a.png", "b.png"]);
    assert_eq!(changes.get(), 1, "the answer is written to the binding once");
}

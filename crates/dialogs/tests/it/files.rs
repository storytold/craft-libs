//! The path field and the picker ticket it keeps, against a file service that records its requests.

use craft_dialogs::ui::PathField;
use craft_host::{AppInfo, FileError, Files, Host, PickAnswer, PickId, PickRequest, Picked, WriteIntent};
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

    fn pending_pick(&self) -> Option<craft_host::PendingPick> {
        None
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }

    fn write(&self, _path: &Path, _bytes: &[u8], _intent: WriteIntent) -> Result<bool, FileError> {
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

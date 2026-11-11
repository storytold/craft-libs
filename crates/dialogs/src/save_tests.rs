//! `craft_dialogs::save`, with a scripted `Files` and an inline `FileWatch`.

use craft_ui::testing::trial;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use craft_host::{FileError, FileFilter, Files, PickId, PickRequest, Picked};
use craft_platform::watch::FileWatch;

use crate::save::{SaveDocument, SaveFlow, SaveStep, SaveTarget};

/// Records every request; the test answers tickets by hand.
#[derive(Default)]
struct ScriptedFiles {
    requests: Mutex<Vec<(PickId, PickRequest)>>,
    answers: Mutex<HashMap<PickId, Picked>>,
}

impl ScriptedFiles {
    fn answer(&self, id: PickId, picked: Picked) {
        self.answers.lock().unwrap().insert(id, picked);
    }

    fn last_ticket(&self) -> PickId {
        self.requests.lock().unwrap().last().unwrap().0
    }

    fn last_request(&self) -> PickRequest {
        self.requests.lock().unwrap().last().unwrap().1.clone()
    }
}

impl Files for ScriptedFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let mut requests = self.requests.lock().unwrap();
        let id = PickId(requests.len() as u64 + 1);
        requests.push((id, request));
        id
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        self.answers.lock().unwrap().remove(&id)
    }

    fn forget(&self, id: PickId) {
        self.answers.lock().unwrap().remove(&id);
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

fn document(path: Option<&Path>, name: &str) -> SaveDocument<u32> {
    SaveDocument {
        key: 7,
        name: name.into(),
        path: path.map(Path::to_path_buf),
        extension: "pdf".into(),
        filters: vec![FileFilter::new("PDF", &["pdf"])],
        purpose: "document".into(),
        saved: None,
    }
}

fn watch() -> FileWatch<u32> {
    FileWatch::new(craft_host::Wake::none())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-save-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// After another writer changed the document's own file, Save asks; Replace writes, Save As opens
// the picker, Cancel cancels; a file the app wrote itself (written) never asks.
#[trial]
fn a_file_changed_on_disk_is_asked_about_once() {
    let dir = scratch("changed");
    let file = dir.join("a.pdf");
    std::fs::write(&file, b"one").unwrap();
    let (files, mut watch, mut flow) = (ScriptedFiles::default(), watch(), SaveFlow::new());
    watch.known_now(7, &file);
    std::fs::write(&file, b"another app wrote more").unwrap();
    let ask = SaveStep::AskChangedOnDisk { key: 7, path: file.clone() };

    assert_eq!(flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace), ask);
    assert!(flow.is_busy());
    assert_eq!(flow.replace(), SaveStep::Write { key: 7, path: file.clone(), target: SaveTarget::InPlace });
    flow.written(&mut watch, 7, &file);
    assert_eq!(
        flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace),
        SaveStep::Write { key: 7, path: file.clone(), target: SaveTarget::InPlace },
        "the app's own write is no change on disk"
    );
    flow.failed(&7);

    std::fs::write(&file, b"changed again, longer").unwrap();
    assert_eq!(flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace), ask);
    assert_eq!(flow.save_elsewhere(&files), SaveStep::Picking);
    let request = files.last_request();
    assert_eq!(request.directory.as_deref(), Some(dir.as_path()));
    let elsewhere = dir.join("b.pdf");
    files.answer(files.last_ticket(), Picked::Paths(vec![elsewhere.clone()]));
    assert_eq!(flow.take_pick(&files, &watch), Some(SaveStep::Write { key: 7, path: elsewhere, target: SaveTarget::As }));
    flow.failed(&7);

    assert_eq!(flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace), ask);
    assert_eq!(flow.cancel(&files), Some(SaveStep::Cancelled { key: 7 }));
    assert!(!flow.is_busy());
    std::fs::remove_dir_all(dir).ok();
}

// A write that finishes for another document (its save was cancelled) leaves a newer save alone.
#[trial]
fn a_late_write_does_not_free_another_save() {
    let (files, mut watch, mut flow) = (ScriptedFiles::default(), watch(), SaveFlow::new());
    assert_eq!(
        flow.start(&files, &watch, document(Some(Path::new("/x/a.pdf")), "a.pdf"), SaveTarget::InPlace),
        SaveStep::Write { key: 7, path: PathBuf::from("/x/a.pdf"), target: SaveTarget::InPlace }
    );
    assert_eq!(flow.cancel(&files), Some(SaveStep::Cancelled { key: 7 }));
    let mut other = document(None, "B");
    other.key = 8;
    assert_eq!(flow.start(&files, &watch, other, SaveTarget::As), SaveStep::Picking);
    let ticket = files.last_ticket();

    flow.written(&mut watch, 7, Path::new("/x/a.pdf"));
    assert!(flow.is_busy(), "the pick of the other document is still open");
    files.answer(ticket, Picked::Paths(vec![PathBuf::from("/x/b.pdf")]));
    assert_eq!(flow.take_pick(&files, &watch), Some(SaveStep::Write { key: 8, path: PathBuf::from("/x/b.pdf"), target: SaveTarget::As }));
    flow.written(&mut watch, 8, Path::new("/x/b.pdf"));
    assert!(!flow.is_busy());
}

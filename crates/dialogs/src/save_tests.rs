//! `craft_dialogs::save`, with a scripted `Files` and an inline `FileWatch`.

use craft_ui::testing::trial;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use craft_host::{FileError, FileFilter, Files, PickId, PickRequest, Picked, WriteIntent};
use craft_platform::watch::FileWatch;

use crate::save::{SaveDocument, SaveFlow, SaveStep, SaveTarget, save_dialog_folder};

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

fn document(path: Option<&Path>, name: &str) -> SaveDocument<u32> {
    SaveDocument {
        key: 7,
        name: name.into(),
        path: path.map(Path::to_path_buf),
        extension: "pdf".into(),
        filters: vec![FileFilter::new("PDF", &["pdf"])],
        purpose: "document".into(),
        saved: None,
        directory: None,
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
    assert_eq!(flow.replace(), SaveStep::Write { key: 7, path: file.clone(), target: SaveTarget::InPlace, file_type: None });
    flow.written(&mut watch, 7, &file);
    assert_eq!(
        flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace),
        SaveStep::Write { key: 7, path: file.clone(), target: SaveTarget::InPlace, file_type: None },
        "the app's own write is no change on disk"
    );
    flow.failed(&7);

    std::fs::write(&file, b"changed again, longer").unwrap();
    assert_eq!(flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace), ask);
    assert_eq!(flow.save_elsewhere(&files), SaveStep::Picking);
    let request = files.last_request();
    assert_eq!(request.directory.as_deref(), Some(dir.as_path()));
    let elsewhere = dir.join("b.pdf");
    files.answer(files.last_ticket(), Picked::Saved { path: elsewhere.clone(), file_type: Some(0) });
    assert_eq!(flow.take_pick(&files, &watch), Some(SaveStep::Write { key: 7, path: elsewhere, target: SaveTarget::As, file_type: Some(0) }));
    flow.failed(&7);

    assert_eq!(flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::InPlace), ask);
    assert_eq!(flow.cancel(&files), Some(SaveStep::Cancelled { key: 7 }));
    assert!(!flow.is_busy());

    let mut named = document(None, "C");
    named.directory = Some(dir.join("exports"));
    assert_eq!(flow.start(&files, &watch, named, SaveTarget::As), SaveStep::Picking);
    assert_eq!(files.last_request().directory, Some(dir.join("exports")), "the document's directory starts the picker");
    assert_eq!(flow.cancel(&files), Some(SaveStep::Cancelled { key: 7 }));
    std::fs::remove_dir_all(dir).ok();
}

// Tc-9 (4): Save As onto the document's own file writes the file type the panel picked, straight away or after the
// changed-on-disk question's Replace: the own-file path dropped the pick, so the document saved in its old type.
#[trial]
fn save_as_onto_the_own_file_keeps_the_picked_type() {
    let dir = scratch("own-type");
    let file = dir.join("a.pdf");
    std::fs::write(&file, b"one").unwrap();
    let (files, mut watch, mut flow) = (ScriptedFiles::default(), watch(), SaveFlow::new());
    watch.known_now(7, &file);
    let write = SaveStep::Write { key: 7, path: file.clone(), target: SaveTarget::As, file_type: Some(1) };
    for changed in [false, true] {
        if changed {
            std::fs::write(&file, b"another app wrote more").unwrap();
        }
        assert_eq!(flow.start(&files, &watch, document(Some(&file), "a.pdf"), SaveTarget::As), SaveStep::Picking);
        files.answer(files.last_ticket(), Picked::Saved { path: file.clone(), file_type: Some(1) });
        let step = flow.take_pick(&files, &watch);
        if changed {
            assert_eq!(step, Some(SaveStep::AskChangedOnDisk { key: 7, path: file.clone() }), "a changed file asks first");
            assert_eq!(flow.replace(), write, "Replace writes the picked type");
        } else {
            assert_eq!(step, Some(write.clone()), "the own file writes the picked type");
        }
        flow.written(&mut watch, 7, &file);
    }
    std::fs::remove_dir_all(dir).ok();
}

// A write that finishes for another document (its save was cancelled) leaves a newer save alone.
#[trial]
fn a_late_write_does_not_free_another_save() {
    let (files, mut watch, mut flow) = (ScriptedFiles::default(), watch(), SaveFlow::new());
    assert_eq!(
        flow.start(&files, &watch, document(Some(Path::new("/x/a.pdf")), "a.pdf"), SaveTarget::InPlace),
        SaveStep::Write { key: 7, path: PathBuf::from("/x/a.pdf"), target: SaveTarget::InPlace, file_type: None }
    );
    assert_eq!(flow.cancel(&files), Some(SaveStep::Cancelled { key: 7 }));
    let mut other = document(None, "B");
    other.key = 8;
    assert_eq!(flow.start(&files, &watch, other, SaveTarget::As), SaveStep::Picking);
    let ticket = files.last_ticket();

    flow.written(&mut watch, 7, Path::new("/x/a.pdf"));
    assert!(flow.is_busy(), "the pick of the other document is still open");
    files.answer(ticket, Picked::Saved { path: PathBuf::from("/x/b.pdf"), file_type: None });
    assert_eq!(
        flow.take_pick(&files, &watch),
        Some(SaveStep::Write { key: 8, path: PathBuf::from("/x/b.pdf"), target: SaveTarget::As, file_type: None })
    );
    flow.written(&mut watch, 8, Path::new("/x/b.pdf"));
    assert!(!flow.is_busy());
}

// An email attachment's temporary copy (Outlook's cache, Apple Mail's downloads) is no place to
// keep work: Save asks where, as for a new document, and the picker does not start in that folder.
// The paths need not exist: the mail check comes before any look at the disk.
#[trial]
fn mail_attachment_copies_always_ask_where_to_save() {
    let (files, watch, mut flow) = (ScriptedFiles::default(), watch(), SaveFlow::new());
    for copy in [
        r"C:\Users\me\AppData\Local\Microsoft\Windows\INetCache\Content.Outlook\AB12CD34\Quote.pdf",
        "/Users/me/Library/Containers/com.apple.mail/Data/Library/Mail Downloads/6F1C2A9E/booking.pdf",
    ] {
        assert_eq!(flow.start(&files, &watch, document(Some(Path::new(copy)), "Quote.pdf"), SaveTarget::InPlace), SaveStep::Picking, "{copy}");
        assert_eq!(files.last_request().directory, None, "{copy}");
        assert_eq!(flow.cancel(&files), Some(SaveStep::Cancelled { key: 7 }));
    }
    let ordinary = Path::new("/home/me/Documents/Quote.pdf");
    assert_eq!(
        flow.start(&files, &watch, document(Some(ordinary), "Quote.pdf"), SaveTarget::InPlace),
        SaveStep::Write { key: 7, path: ordinary.to_path_buf(), target: SaveTarget::InPlace, file_type: None }
    );
}

// Save As starts in the folder of the document's own file; a new document, a bare name, the root
// and a mail attachment's copy give the picker no folder.
#[trial]
fn the_save_picker_opens_in_the_documents_own_folder() {
    let (files, watch, mut flow) = (ScriptedFiles::default(), watch(), SaveFlow::new());
    let mut folder_of = |path: Option<&str>| {
        assert_eq!(flow.start(&files, &watch, document(path.map(Path::new), "f.pdf"), SaveTarget::As), SaveStep::Picking);
        let folder = files.last_request().directory;
        flow.cancel(&files);
        assert_eq!(folder, save_dialog_folder(path.map(Path::new)), "the picker starts where save_dialog_folder says");
        folder
    };
    assert_eq!(folder_of(Some("/home/a/plans/f.pdf")), Some(PathBuf::from("/home/a/plans")));
    assert_eq!(folder_of(None), None);
    assert_eq!(folder_of(Some("f.pdf")), None);
    assert_eq!(folder_of(Some("/")), None);
    assert_eq!(folder_of(Some(r"C:\Users\a\AppData\Local\Microsoft\Windows\INetCache\Content.Outlook\AB12\f.pdf")), None);
    assert_eq!(folder_of(Some("/Users/a/Library/Containers/com.apple.mail/Data/Library/Mail Downloads/UUID/f.pdf")), None);
    // Network shares follow the Windows rules for `Path::parent`, which only apply on Windows.
    #[cfg(windows)]
    {
        assert_eq!(folder_of(Some(r"\\server\share\plans\f.pdf")), Some(PathBuf::from(r"\\server\share\plans")));
        assert_eq!(folder_of(Some(r"\\server\share\")), None);
    }
}

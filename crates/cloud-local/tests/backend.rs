//! Drives [`craft_cloud_local::LocalBackend`] the way an app does: through
//! `craft_ui::cloud`'s own public functions, never by calling the backend
//! directly. That is the seam this crate exists to prove, so the tests stay
//! on the application's side of it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_cloud::{Entry, EntryKind};
use craft_cloud_local::LocalBackend;
use craft_ui::cloud::{self, Account, CloudApp, CloudFiles, Destination, DocumentKind, Job};

const NOTE: DocumentKind = DocumentKind { extensions: &["txt"], mimes: &["text/plain"], noun: "note", default_extension: "txt" };

/// The least a [`CloudApp`] can be. What it was told to open is recorded, so a
/// round trip can be checked by content and not only by the entry metadata
/// `craft_ui` itself remembers.
struct Harness {
    cloud: CloudFiles,
    share: craft_ui::share::ShareDialog,
    presence: craft_ui::presence::PresenceBar,
    opened: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    said: Arc<Mutex<Vec<String>>>,
}

impl Harness {
    fn new() -> Harness {
        Harness {
            cloud: CloudFiles::new(),
            share: craft_ui::share::ShareDialog::new(),
            presence: craft_ui::presence::PresenceBar::new(),
            opened: Arc::new(Mutex::new(Vec::new())),
            said: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl CloudApp for Harness {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> DocumentKind {
        NOTE
    }

    fn active(&self) -> Option<u64> {
        None
    }

    fn title(&self, _uid: u64) -> String {
        String::new()
    }

    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        self.opened.lock().unwrap().push((name.to_owned(), bytes));
        Ok(Some(1))
    }

    fn encode(&mut self, _uid: u64, _into: Destination<'_>) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn close(&mut self, _uid: u64) {}

    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, _painter: &egui::Painter, _rect: egui::Rect, _kind: EntryKind, _colour: egui::Color32) {}

    fn open_url(&self, _url: &str) {}

    fn say(&mut self, text: String) {
        self.said.lock().unwrap().push(text);
    }

    fn app_name(&self) -> &str {
        "Harness"
    }
}

fn wired(root: std::path::PathBuf) -> (Harness, egui::Context) {
    let mut app = Harness::new();
    app.cloud.connect(LocalBackend::new(root));
    (app, egui::Context::default())
}

/// Drains whatever the backend's worker threads have sent so far, until
/// `done` says the state the test is waiting for arrived or five seconds pass.
/// Five seconds is far more than a thread doing local file IO ever needs; it
/// only guards against a real bug leaving the test to hang forever.
fn settle(app: &mut Harness, ctx: &egui::Context, mut done: impl FnMut(&Harness) -> bool) {
    let start = Instant::now();
    loop {
        ctx.begin_pass(Default::default());
        cloud::pump(app, ctx);
        ctx.end_pass().textures_delta.clear();
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for the backend's worker thread");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn account_and_sign_in_report_the_same_fixed_identity_with_no_browser_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    cloud::start(&mut app);
    settle(&mut app, &ctx, |app| app.cloud().account.is_some());

    assert_eq!(
        app.cloud().account,
        Some(Account {
            display: "Local disk".to_owned(),
            capabilities: vec![craft_cloud::Capability::Files, craft_cloud::Capability::Etags],
            provider_linked: true
        })
    );
    // There was never an account to link, so nothing was ever offered.
    assert!(app.cloud().link.is_none());
}

#[test]
fn creating_a_document_writes_real_bytes_a_read_gets_back() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    cloud::submit(&app, Job::Create { uid: 1, folder: String::new(), name: "Minutes.txt".to_owned(), bytes: b"first draft".to_vec() });
    settle(&mut app, &ctx, |app| app.cloud().origins.contains_key(&1));

    let entry = app.cloud().origins.get(&1).unwrap().entry.clone();
    assert_eq!(entry.name, "Minutes.txt");
    assert_eq!(entry.id, "/Minutes.txt");
    assert_eq!(fs_read(dir.path(), "Minutes.txt"), b"first draft");

    cloud::submit(&app, Job::Read { entry, into: None });
    settle(&mut app, &ctx, |app| !app.opened.lock().unwrap().is_empty());

    let opened = app.opened.lock().unwrap().clone();
    assert_eq!(opened, vec![("Minutes.txt".to_owned(), b"first draft".to_vec())], "the read got back exactly what the create wrote");
}

#[test]
fn a_write_under_a_stale_version_is_refused_and_never_overwrites_the_newer_file() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    cloud::submit(&app, Job::Create { uid: 1, folder: String::new(), name: "Minutes.txt".to_owned(), bytes: b"v1".to_vec() });
    settle(&mut app, &ctx, |app| app.cloud().origins.contains_key(&1));
    let stale = app.cloud().origins.get(&1).unwrap().entry.clone();

    // Somebody else saves first, changing the file's etag without going
    // through this window at all.
    std::fs::write(dir.path().join("Minutes.txt"), b"somebody else's edit").unwrap();

    cloud::submit(&app, Job::Write { uid: 1, entry: stale, bytes: b"this window's edit".to_vec() });
    settle(&mut app, &ctx, |app| app.cloud().conflict.is_some());

    assert_eq!(app.cloud().conflict.as_ref().unwrap().uid, 1);
    assert_eq!(fs_read(dir.path(), "Minutes.txt"), b"somebody else's edit", "the refused write never touched the file");
}

#[test]
fn listing_the_root_reflects_a_file_nobody_went_through_this_backend_to_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Agenda.txt"), b"1. roll call").unwrap();
    std::fs::create_dir(dir.path().join("Archive")).unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    cloud::browse(&mut app, "");
    settle(&mut app, &ctx, |app| app.cloud().mine.loaded);

    let names: Vec<_> = app.cloud().mine.entries.iter().map(|entry| entry.name.clone()).collect();
    assert_eq!(names, vec!["Archive".to_owned(), "Agenda.txt".to_owned()], "folders first, then files, each alphabetised");
    let folder = app.cloud().mine.entries.iter().find(|entry| entry.name == "Archive").unwrap();
    assert_eq!(folder.kind, EntryKind::Folder);
    assert!(folder.size.is_none());
}

#[test]
fn nobody_has_shared_anything_with_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    cloud::load_shared(&mut app);
    settle(&mut app, &ctx, |app| app.cloud().shared.loaded);

    assert_eq!(app.cloud().shared.entries, Vec::new());
    assert!(app.cloud().shared.error.is_none(), "an empty share list is not a failure");
}

#[test]
fn a_conversation_is_a_fault_this_backend_is_honest_about() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    cloud::submit(&app, Job::Conversation("/Minutes.txt".to_owned()));
    settle(&mut app, &ctx, |app| app.cloud().status.is_some());

    assert_eq!(app.cloud().status, Some("this reference backend has no shared workspace to converse in".to_owned()));
}

#[test]
fn a_read_of_a_document_that_is_not_there_is_put_to_the_user_not_silently_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = wired(dir.path().to_path_buf());

    let ghost = Entry {
        id: "/ghost.txt".to_owned(),
        room: "local:/ghost.txt".to_owned(),
        name: "ghost.txt".to_owned(),
        path: "/ghost.txt".to_owned(),
        kind: EntryKind::File,
        size: None,
        modified: None,
        mime: None,
        etag: None,
        writable: true,
        shared_by: None,
    };
    cloud::submit(&app, Job::Read { entry: ghost, into: None });
    settle(&mut app, &ctx, |app| !app.said.lock().unwrap().is_empty());

    assert!(app.said.lock().unwrap()[0].contains("ghost.txt"), "the message names the document, not just that something failed");
}

fn fs_read(root: &std::path::Path, name: &str) -> Vec<u8> {
    std::fs::read(root.join(name)).unwrap()
}

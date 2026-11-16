//! The P1 from FredApps' re-review of #160: `resolve` only split on `/` and rejected a segment
//! equal to `..`; it did not constrain a Windows backslash, a drive letter or UNC prefix, or a
//! filesystem link, and `create` joined `name` onto the resolved folder with no validation of
//! `name` at all. Driven through `craft_ui::cloud`'s public functions the way an application
//! does, with a sentinel file and directory kept as a sibling of the configured root: every test
//! here must leave both untouched and unread.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_cloud::{Entry, EntryKind};
use craft_cloud_local::LocalBackend;
use craft_ui::cloud::{self, CloudApp, CloudFiles, Destination, DocumentKind, Job};

const NOTE: DocumentKind = DocumentKind { extensions: &["txt"], mimes: &["text/plain"], noun: "note", default_extension: "txt" };

/// A document the harness was told to open: its name and its bytes.
type OpenedDocs = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

struct Harness {
    cloud: CloudFiles,
    share: craft_ui::share::ShareDialog,
    presence: craft_ui::presence::PresenceBar,
    opened: OpenedDocs,
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

fn wired(root: PathBuf) -> (Harness, egui::Context) {
    let mut app = Harness::new();
    app.cloud.connect(LocalBackend::new(root));
    (app, egui::Context::default())
}

/// Drains whatever the backend's worker threads have sent so far, until `done` says the state
/// the test is waiting for arrived or five seconds pass. Five seconds is far more than a thread
/// doing local file IO ever needs; it only guards against a real bug leaving the test to hang.
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

fn said(app: &mut Harness, ctx: &egui::Context) -> Vec<String> {
    settle(app, ctx, |app| !app.said.lock().unwrap().is_empty());
    app.said.lock().unwrap().clone()
}

/// A tempdir holding the cloud root, plus a sentinel file and a sentinel directory that sit
/// beside it rather than inside it. Every test below must leave both exactly as they start.
struct Sentinels {
    // Kept only so the tempdir outlives every test that borrows the paths below; nothing reads
    // it back once the paths it was built from are in hand.
    _arena: tempfile::TempDir,
    root: PathBuf,
    sentinel_file: PathBuf,
    sentinel_dir: PathBuf,
}

fn sentinels() -> Sentinels {
    let arena = tempfile::tempdir().unwrap();
    let root = arena.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let sentinel_file = arena.path().join("sentinel.txt");
    std::fs::write(&sentinel_file, b"a sibling file this backend must never read or write").unwrap();
    let sentinel_dir = arena.path().join("sentinel-dir");
    std::fs::create_dir(&sentinel_dir).unwrap();
    std::fs::write(sentinel_dir.join("secret.txt"), b"a sibling directory this backend must never list").unwrap();
    Sentinels { _arena: arena, root, sentinel_file, sentinel_dir }
}

fn assert_sentinels_untouched(sentinels: &Sentinels) {
    assert_eq!(
        std::fs::read(&sentinels.sentinel_file).unwrap(),
        b"a sibling file this backend must never read or write",
        "the sentinel file outside the root was changed"
    );
    assert_eq!(
        std::fs::read(sentinels.sentinel_dir.join("secret.txt")).unwrap(),
        b"a sibling directory this backend must never list",
        "the sentinel directory's file outside the root was changed"
    );
}

/// Every one of these is a [`Job::Create`] `name` that must be refused before any byte reaches
/// disk: the review's own examples (a parent segment, an absolute path, and the three
/// Windows-shaped forms), none of which a Unix build's native parser alone would catch, and a
/// name that already is nothing but a Windows separator or drive prefix on top of that.
fn malicious_create_names() -> Vec<&'static str> {
    vec!["../sentinel.txt", "/sentinel.txt", "..\\sentinel.txt", "C:\\x", "C:evil", "\\\\server\\share\\x"]
}

#[test]
fn create_refuses_every_escaping_name_and_the_sentinels_stay_untouched() {
    for name in malicious_create_names() {
        let sentinels = sentinels();
        let (mut app, ctx) = wired(sentinels.root.clone());

        cloud::submit(&app, Job::Create { uid: 1, folder: String::new(), name: name.to_owned(), bytes: b"pwned".to_vec() });
        let messages = said(&mut app, &ctx);

        assert!(cloud::origin(&app).is_none(), "{name:?} must not be reported as created");
        assert!(!messages.is_empty(), "{name:?} must be refused, not silently dropped");
        assert_sentinels_untouched(&sentinels);
    }
}

/// A folder path for `Job::List` can carry the same Windows-shaped escape a `Create` name can;
/// `resolve` must refuse it through the identical [`confine_segment`] check, not a weaker one.
#[test]
fn listing_a_windows_shaped_folder_path_is_refused_on_every_platform() {
    let sentinels = sentinels();
    let (mut app, ctx) = wired(sentinels.root.clone());

    cloud::browse(&mut app, "..\\sentinel-dir");
    settle(&mut app, &ctx, |app| app.cloud().mine.loaded);

    assert!(app.cloud().mine.error.is_some(), "a backslash-separated folder path must be refused");
    assert!(app.cloud().mine.entries.is_empty());
    assert_sentinels_untouched(&sentinels);
}

// ------------------------------------------------------------------- link-based escapes
//
// `escape(&sentinels)` places one link named `escape` inside the cloud root, pointing out to
// `sentinels.sentinel_dir`: a symlink on Unix, a directory junction on Windows, which needs no
// elevated privilege, unlike a Windows symlink. The Linux gate runs the symlink half of every
// test below; CI's windows-latest lanes run the junction half. Either way `root/escape/secret.txt`
// is the same sentinel file reached through the link.

/// Nothing to clean up on Unix: `tempfile`'s own recursive removal of the arena directory
/// removes a plain symlink entry without following it. This exists only so `escape` returns the
/// same shape of RAII handle on every platform, rather than a bare `()` some lint would flag.
#[cfg(unix)]
struct Linked;

#[cfg(unix)]
fn escape(sentinels: &Sentinels) -> Linked {
    std::os::unix::fs::symlink(&sentinels.sentinel_dir, sentinels.root.join("escape")).unwrap();
    Linked
}

/// A directory junction, removed on drop before the tempdir it sits beside is cleaned up.
#[cfg(windows)]
struct Junction(PathBuf);

#[cfg(windows)]
impl Drop for Junction {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[cfg(windows)]
fn escape(sentinels: &Sentinels) -> Junction {
    use std::os::windows::process::CommandExt as _;
    let link = sentinels.root.join("escape");
    let target = &sentinels.sentinel_dir;
    assert!(link.is_absolute() && target.is_absolute());
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(target)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Junction(link)
}

#[cfg(any(unix, windows))]
fn leaked_entry() -> Entry {
    Entry {
        id: "/escape/secret.txt".to_owned(),
        room: "local:/escape/secret.txt".to_owned(),
        name: "secret.txt".to_owned(),
        path: "/escape/secret.txt".to_owned(),
        kind: EntryKind::File,
        size: None,
        modified: None,
        mime: None,
        etag: None,
        writable: true,
        shared_by: None,
    }
}

#[cfg(any(unix, windows))]
#[test]
fn listing_a_linked_folder_cannot_see_outside_the_root() {
    let sentinels = sentinels();
    let (mut app, ctx) = wired(sentinels.root.clone());
    let _escape = escape(&sentinels);

    cloud::browse(&mut app, "escape");
    settle(&mut app, &ctx, |app| app.cloud().mine.loaded);

    assert!(app.cloud().mine.error.is_some(), "listing through a symlink or junction out of the root must be refused");
    assert!(app.cloud().mine.entries.is_empty());
    assert_sentinels_untouched(&sentinels);
}

#[cfg(any(unix, windows))]
#[test]
fn reading_through_a_linked_folder_cannot_return_bytes_from_outside_the_root() {
    let sentinels = sentinels();
    let (mut app, ctx) = wired(sentinels.root.clone());
    let _escape = escape(&sentinels);

    cloud::submit(&app, Job::Read { entry: leaked_entry(), into: None });
    said(&mut app, &ctx);

    assert!(app.opened.lock().unwrap().is_empty(), "a read through a symlink or junction out of the root must not deliver bytes");
    assert_sentinels_untouched(&sentinels);
}

#[cfg(any(unix, windows))]
#[test]
fn writing_through_a_linked_folder_cannot_change_a_file_outside_the_root() {
    let sentinels = sentinels();
    let (mut app, ctx) = wired(sentinels.root.clone());
    let _escape = escape(&sentinels);

    cloud::submit(&app, Job::Write { uid: 1, entry: leaked_entry(), bytes: b"pwned".to_vec() });
    said(&mut app, &ctx);

    assert!(app.cloud().conflict.is_none(), "this must be refused outright, not treated as a stale-version conflict");
    assert_sentinels_untouched(&sentinels);
}

//! Confinement regressions for `crates/cloud-local`. The first round (FredApps' re-review) found
//! that `resolve` only split on `/` and rejected a segment equal to `..`, with no constraint on a
//! Windows backslash, a drive letter or UNC prefix, or a filesystem link, and that `create`
//! joined `name` onto the resolved folder with no validation of `name` at all. The second round
//! (benandez's re-review) reproduced three escapes the first fix still left open: a dangling
//! symlink, a check-then-use race on a swapped link, and a hard link sharing a file's inode.
//! Driven through `craft_ui::cloud`'s public functions the way an application does, with a
//! sentinel file and directory kept as a sibling of the configured root: every test here must
//! leave both untouched and unread.

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

/// Clears both event sinks, then waits for the next job to land in either one. Clearing first
/// matters for a test that submits several jobs in a loop: without it, a stale `said` from two
/// iterations ago would satisfy `settle` immediately, before the new job had actually answered.
fn said(app: &mut Harness, ctx: &egui::Context) -> Vec<String> {
    app.opened.lock().unwrap().clear();
    app.said.lock().unwrap().clear();
    settle(app, ctx, |app| !app.said.lock().unwrap().is_empty() || !app.opened.lock().unwrap().is_empty());
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

const SENTINEL_FILE_BYTES: &[u8] = b"a sibling file this backend must never read or write";
const SENTINEL_DIR_FILE_BYTES: &[u8] = b"a sibling directory this backend must never list";

fn sentinels() -> Sentinels {
    let arena = tempfile::tempdir().unwrap();
    let root = arena.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let sentinel_file = arena.path().join("sentinel.txt");
    std::fs::write(&sentinel_file, SENTINEL_FILE_BYTES).unwrap();
    let sentinel_dir = arena.path().join("sentinel-dir");
    std::fs::create_dir(&sentinel_dir).unwrap();
    std::fs::write(sentinel_dir.join("secret.txt"), SENTINEL_DIR_FILE_BYTES).unwrap();
    Sentinels { _arena: arena, root, sentinel_file, sentinel_dir }
}

fn assert_sentinels_untouched(sentinels: &Sentinels) {
    assert_eq!(std::fs::read(&sentinels.sentinel_file).unwrap(), SENTINEL_FILE_BYTES, "the sentinel file outside the root was changed");
    assert_eq!(
        std::fs::read(sentinels.sentinel_dir.join("secret.txt")).unwrap(),
        SENTINEL_DIR_FILE_BYTES,
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

// ------------------------------------------------------------------- link-based escapes
//
// `escape(&sentinels)` places one link named `escape` inside the cloud root, pointing out to
// `sentinels.sentinel_dir`: a symlink on Unix, a directory junction on Windows, which needs no
// elevated privilege, unlike a Windows symlink. The Linux gate runs the symlink half of every
// test below; CI's windows-latest lanes run the junction half. Either way `root/escape/...`
// reaches into the sentinel directory through the link.

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

/// An [`Entry`] stub naming `id`, for a [`Job::Read`] or [`Job::Write`] this file submits
/// directly rather than through an entry the backend itself handed back.
fn leaked_entry(id: &str, name: &str) -> Entry {
    Entry {
        id: id.to_owned(),
        room: format!("local:{id}"),
        name: name.to_owned(),
        path: id.to_owned(),
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

    cloud::submit(&app, Job::Read { entry: leaked_entry("/escape/secret.txt", "secret.txt"), into: None });
    said(&mut app, &ctx);

    assert!(app.opened.lock().unwrap().is_empty(), "a read through a symlink or junction out of the root must not deliver bytes");
    assert_sentinels_untouched(&sentinels);
}

/// benandez's finding 4: the original version of this test targeted the pre-existing
/// `secret.txt`, with `etag: None`. `write` refused that for an unrelated reason, a version
/// mismatch, before confinement ever ran, so the test passed whether or not confinement worked.
/// Targeting a name that does not exist yet under the link reaches the code that actually
/// matters, and the oracle (the file must never be created) is one confinement failing would
/// actually flip.
#[cfg(any(unix, windows))]
#[test]
fn writing_through_a_linked_folder_cannot_create_a_file_outside_the_root() {
    let sentinels = sentinels();
    let (mut app, ctx) = wired(sentinels.root.clone());
    let _escape = escape(&sentinels);

    cloud::submit(&app, Job::Write { uid: 1, entry: leaked_entry("/escape/not-yet-created.txt", "not-yet-created.txt"), bytes: b"pwned".to_vec() });
    said(&mut app, &ctx);

    assert!(!sentinels.sentinel_dir.join("not-yet-created.txt").exists(), "a write through a link must not create a file outside the root");
    assert_sentinels_untouched(&sentinels);
}

/// benandez's finding 5: nothing previously exercised a link in the `folder` a `Create` targets.
#[cfg(any(unix, windows))]
#[test]
fn creating_through_a_linked_folder_cannot_create_a_file_outside_the_root() {
    let sentinels = sentinels();
    let (mut app, ctx) = wired(sentinels.root.clone());
    let _escape = escape(&sentinels);

    cloud::submit(&app, Job::Create { uid: 1, folder: "escape".to_owned(), name: "new.txt".to_owned(), bytes: b"pwned".to_vec() });
    let messages = said(&mut app, &ctx);

    assert!(cloud::origin(&app).is_none(), "a create into a linked folder must not be reported as created");
    assert!(!messages.is_empty(), "a create into a linked folder must be refused, not silently dropped");
    assert!(!sentinels.sentinel_dir.join("new.txt").exists(), "a create through a link must not create a file outside the root");
    assert_sentinels_untouched(&sentinels);
}

// ------------------------------------------------------------------- Unix-only reproductions
//
// Directory junctions cannot be dangling (`mklink /J` requires the target to exist) and the
// race and hard-link reproductions below are exactly benandez's own repros, run on Linux only,
// as stated in that review. The structural fix (resolving through a `cap_std::fs::Dir`, and
// writing through a temporary file and rename) is the same code on every platform; these three
// just do not have a Windows-shaped equivalent worth a separate repro.

/// benandez's finding 1, reproduced exactly: `root/doc.txt` is a symlink to a file that does not
/// exist yet, outside the root. The old code's `confine_to_root` canonicalised the dangling link
/// to `NotFound`, walked up to `root`, accepted it, found no file to conflict with, and `write`
/// created the target for real, outside the root.
#[cfg(unix)]
#[test]
fn writing_through_a_dangling_symlink_cannot_create_a_file_outside_the_root() {
    let sentinels = sentinels();
    let outside_target = sentinels._arena.path().join("outside-planted.txt");
    std::os::unix::fs::symlink(&outside_target, sentinels.root.join("doc.txt")).unwrap();
    let (mut app, ctx) = wired(sentinels.root.clone());

    cloud::submit(&app, Job::Write { uid: 1, entry: leaked_entry("/doc.txt", "doc.txt"), bytes: b"pwned".to_vec() });
    said(&mut app, &ctx);

    assert!(!outside_target.exists(), "a write through a dangling symlink must not create its target outside the root");
    assert_sentinels_untouched(&sentinels);
}

/// benandez's finding 3, reproduced exactly: `root/hl` is a hard link sharing an inode with a
/// file outside the root. Reading it through the backend is legitimate (it is, from inside the
/// sandbox, an ordinary file), which is also how this test gets a real, matching etag to write
/// with, the same way a real caller would after opening the document. The old code's `write`
/// truncated the shared inode in place; `write_replacing` must instead replace `hl`'s own
/// directory entry, leaving the sentinel file's inode, and its other name, untouched.
#[cfg(unix)]
#[test]
fn writing_through_a_hard_link_replaces_the_entry_without_touching_the_shared_file() {
    let sentinels = sentinels();
    std::fs::hard_link(&sentinels.sentinel_file, sentinels.root.join("hl")).unwrap();
    let (mut app, ctx) = wired(sentinels.root.clone());

    cloud::submit(&app, Job::Read { entry: leaked_entry("/hl", "hl"), into: None });
    settle(&mut app, &ctx, |app| !app.opened.lock().unwrap().is_empty());
    let real_entry = app.cloud().origins.get(&1).expect("the read registered an origin for uid 1").entry.clone();

    // A successful write answers through `cloud.status`, not through `said` or `opened`, so it
    // needs its own wait condition rather than the `said` helper used everywhere else in this
    // file for a refusal. The read above already set `status` once (`Opened hl from the
    // cloud.`), so that has to be cleared first or this would observe the read's own status and
    // never actually wait for the write to finish.
    app.cloud_mut().status = None;
    cloud::submit(&app, Job::Write { uid: 1, entry: real_entry, bytes: b"replaced via rename, not truncated in place".to_vec() });
    settle(&mut app, &ctx, |app| app.cloud().status.is_some() || app.cloud().conflict.is_some() || !app.said.lock().unwrap().is_empty());
    assert!(app.cloud().conflict.is_none(), "the write through the hard-linked name should succeed, not conflict");
    assert!(
        app.said.lock().unwrap().is_empty(),
        "the write through the hard-linked name should succeed, not be refused: {:?}",
        app.said.lock().unwrap()
    );

    assert_eq!(
        std::fs::read(sentinels.root.join("hl")).unwrap(),
        b"replaced via rename, not truncated in place",
        "the write through the hard-linked name should still succeed"
    );
    assert_sentinels_untouched(&sentinels);
}

/// benandez's finding 2, reproduced at a smaller scale: a background thread swaps `root/d`
/// between a symlink into an inside folder and a symlink to the sentinel directory, as fast as
/// it can, with no coordination with the reads below. The old code's `confine_to_root` checked
/// one path string and then `fs::read` resolved that path again, a second, later lookup a swap
/// could land in between; out of 3000 such reads benandez's own run returned the outside file's
/// bytes 347 times. `cap_std::fs::Dir::open` resolves and opens `d` as one operation with no
/// such second lookup, so this must come back empty every time, not merely most of the time.
#[cfg(unix)]
#[test]
fn reading_through_a_link_being_swapped_never_observes_the_outside_target() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let sentinels = sentinels();
    let inside = sentinels.root.join("inside");
    std::fs::create_dir(&inside).unwrap();
    std::fs::write(inside.join("secret.txt"), b"inside bytes, always safe to read").unwrap();
    let link = sentinels.root.join("d");
    std::os::unix::fs::symlink(&inside, &link).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let stop = stop.clone();
        let link = link.clone();
        let outside = sentinels.sentinel_dir.clone();
        std::thread::spawn(move || {
            let temp = link.with_extension("swap");
            let mut toward_outside = false;
            while !stop.load(Ordering::Relaxed) {
                let target = if toward_outside { &outside } else { &inside };
                // A stale `temp` from a rename that did not happen (should not occur, but a
                // leftover `temp` would otherwise wedge every later iteration's `symlink` call,
                // silently stopping the swap rather than failing the test loudly) is cleared
                // first so this loop cannot get stuck.
                let _ = std::fs::remove_file(&temp);
                if std::os::unix::fs::symlink(target, &temp).is_ok() {
                    let _ = std::fs::rename(&temp, &link);
                }
                toward_outside = !toward_outside;
            }
        })
    };

    let (mut app, ctx) = wired(sentinels.root.clone());
    let mut leaked = false;
    for _ in 0..200 {
        cloud::submit(&app, Job::Read { entry: leaked_entry("/d/secret.txt", "secret.txt"), into: None });
        said(&mut app, &ctx);
        if let Some((_, bytes)) = app.opened.lock().unwrap().first() {
            leaked |= bytes.as_slice() == SENTINEL_DIR_FILE_BYTES;
        }
    }

    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();

    assert!(!leaked, "a read observed the outside target's bytes while the link was being swapped underneath it");
    assert_sentinels_untouched(&sentinels);
}

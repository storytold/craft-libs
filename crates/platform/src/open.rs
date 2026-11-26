//! OS requests reaching the app from outside, and the app handing files to the system.
//!
//! - [`FileIntake`]: one queue for the OS requests: files the OS asks the app to open (Finder
//!   double-click, Open With, a Dock drop, a second launch handing its paths to the first), files
//!   dropped on the window, and the OS asking the app to quit.
//! - `apple_events` (macOS, feature `apple-events`): the open-documents and quit Apple events,
//!   fed into a [`FileIntake`].
//! - [`natural_cmp`], [`same_file`]: the order files open in and the check that keeps an open
//!   file from opening twice.
//! - [`OPEN_FAILED`] and the other [`PlatformText`]s: the open errors every app words the same way.
//! - [`reveal`], [`open_external`]: show a file in the system file manager; open a file or URL
//!   with the system's handler.
//! - [`open_with`], [`is_application`]: open a file in a chosen application; whether a path is one.
//!
//! No egui: the app converts egui's dropped files into [`DroppedFile`]s itself.

use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use craft_host::Wake;

pub use craft_docpath::names::natural_cmp;

#[cfg(all(target_os = "macos", feature = "apple-events"))]
pub mod apple_events;

use crate::PlatformText;

/// "Couldn't open {name}: {error}", shown in the status bar and as an error notice.
pub const OPEN_FAILED: PlatformText = PlatformText { id: "open.failed", english: "Couldn't open {name}: {error}" };
/// "Couldn't place {name}: {error}", for a file dropped on the canvas.
pub const PLACE_FAILED: PlatformText = PlatformText { id: "open.placeFailed", english: "Couldn't place {name}: {error}" };
/// A path that isn't valid Unicode, when opening.
pub const PATH_NOT_UNICODE_OPEN: PlatformText =
    PlatformText { id: "open.pathNotUnicode", english: "{path}: the path is not valid Unicode; rename the file and open it again." };
/// A path that isn't valid Unicode, when saving.
pub const PATH_NOT_UNICODE_SAVE: PlatformText =
    PlatformText { id: "open.pathNotUnicodeSave", english: "{path}: the path is not valid Unicode; save under another name." };
/// [`reveal`] of a file that was moved or deleted.
pub const FILE_GONE: PlatformText = PlatformText { id: "open.fileGone", english: "The file no longer exists" };
/// [`reveal`] when no file manager answered.
pub const REVEAL_FAILED: PlatformText = PlatformText { id: "open.revealFailed", english: "Could not reveal the file in the system file manager" };
/// Every [`PlatformText`] of this module, for the missing-translation report ([`crate::texts`]).
pub const TEXTS: &[PlatformText] = &[OPEN_FAILED, PLACE_FAILED, PATH_NOT_UNICODE_OPEN, PATH_NOT_UNICODE_SAVE, FILE_GONE, REVEAL_FAILED];

/// A dropped file as the app got it. On the web there is no path, only the name and bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct DroppedFile {
    /// The file's path; `None` on the web.
    pub path: Option<PathBuf>,
    /// The file's name.
    pub name: String,
    /// The contents, when the platform hands them over instead of a path.
    pub bytes: Option<Arc<[u8]>>,
}

/// One batch of files to open.
#[derive(Clone, Debug, PartialEq)]
pub enum Intake {
    /// The OS asked to open these absolute paths: open them like File › Open, focusing a
    /// document that is already open instead of opening it again.
    Open(Vec<PathBuf>),
    /// Files dropped on the window, in drop order.
    Dropped {
        /// The dropped files.
        files: Vec<DroppedFile>,
    },
    /// The OS asked the app to quit.
    Quit,
    /// The OS asked to open these absolute paths for the app's own action.
    OpenFor {
        /// The files, absolute and sorted.
        paths: Vec<PathBuf>,
        /// The app's action name.
        action: String,
    },
    /// A file that arrived as bytes, with no path.
    Bytes {
        /// The file's name.
        name: String,
        /// The contents.
        bytes: Arc<[u8]>,
    },
}

/// The sending side of a [`FileIntake`]: `Send + Clone`, for the threads that hear the OS (the
/// macOS Apple Event handler, the single-instance socket of a second launch).
#[derive(Clone)]
pub struct IntakeSender {
    queue: Arc<Mutex<VecDeque<Intake>>>,
    wake: Wake,
}

// The threads that hear the OS keep a clone each.
const _: () = {
    const fn sendable<T: Send + Sync + Clone>() {}
    sendable::<IntakeSender>();
};

/// A source of OS requests that lives on the UI thread (the Apple-event inbox is not `Send`),
/// emptied into its [`IntakeSender`] by [`FileIntake::take`].
#[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
struct RegisteredDrain {
    queue: std::sync::Weak<Mutex<VecDeque<Intake>>>,
    drain: std::rc::Rc<dyn Fn(&IntakeSender)>,
}

#[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
thread_local! {
    static DRAINS: std::cell::RefCell<Vec<RegisteredDrain>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Keeps a drain registered on the thread that registered it; dropping it unregisters.
#[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
pub(crate) struct DrainRegistration {
    drain: std::rc::Rc<dyn Fn(&IntakeSender)>,
}

#[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
impl Drop for DrainRegistration {
    fn drop(&mut self) {
        DRAINS.with(|drains| drains.borrow_mut().retain(|d| !std::rc::Rc::ptr_eq(&d.drain, &self.drain)));
    }
}

impl IntakeSender {
    /// Wakes the UI without queueing anything: for a source that must not drain where it hears
    /// the OS (it may hold its own lock there) and is drained by [`FileIntake::take`] instead.
    #[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
    pub(crate) fn wake_ui(&self) {
        self.wake.wake();
    }

    /// Registers `drain`, which [`FileIntake::take`] runs on the calling thread before it returns,
    /// to move the requests a thread-bound source collected into this intake.
    #[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
    pub(crate) fn register_drain(&self, drain: impl Fn(&IntakeSender) + 'static) -> DrainRegistration {
        let drain: std::rc::Rc<dyn Fn(&IntakeSender)> = std::rc::Rc::new(drain);
        DRAINS.with(|drains| drains.borrow_mut().push(RegisteredDrain { queue: Arc::downgrade(&self.queue), drain: std::rc::Rc::clone(&drain) }));
        DrainRegistration { drain }
    }

    /// Queues an OS open request and wakes the UI (`wake` is the app's `request_repaint`).
    pub fn open(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.send(Intake::Open(ordered(paths)));
    }

    /// Queues an open request for the app's own `action`, made absolute and sorted as [`open`](Self::open) does, and
    /// wakes the UI. Empty input is ignored.
    pub fn open_for(&self, paths: Vec<PathBuf>, action: impl Into<String>) {
        if paths.is_empty() {
            return;
        }
        self.send(Intake::OpenFor { paths: ordered(paths), action: action.into() });
    }

    /// Queues a file that arrived as bytes, and wakes the UI. A file with neither a name nor bytes is ignored.
    pub fn open_bytes(&self, name: String, bytes: Arc<[u8]>) {
        if name.is_empty() && bytes.is_empty() {
            return;
        }
        self.send(Intake::Bytes { name, bytes });
    }

    /// Queues `batch` after every batch sent before it, then wakes the UI.
    fn send(&self, batch: Intake) {
        push(&self.queue, batch);
        self.wake.wake();
    }

    /// Queues files dropped on the window and wakes the UI; an empty list is ignored.
    pub fn dropped(&self, files: Vec<DroppedFile>) {
        if files.is_empty() {
            return;
        }
        push(&self.queue, Intake::Dropped { files });
        self.wake.wake();
    }

    /// Queues an OS quit request and wakes the UI.
    pub fn quit(&self) {
        push(&self.queue, Intake::Quit);
        self.wake.wake();
    }
}

/// `paths` made absolute and sorted by [`natural_cmp`] of the path text, the order an OS open request opens in.
fn ordered(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = paths.iter().map(|p| absolute(p)).collect();
    paths.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    paths
}

/// OS requests waiting for the app (files to open, files dropped, quit), read once per frame on
/// the UI thread (spec Q-S4F-7).
///
/// The app makes one at startup, gives [`sender`](Self::sender) to whatever hears the OS
/// (`apple_events::install` on macOS, its single-instance socket elsewhere), pushes drops
/// through a sender's [`dropped`](IntakeSender::dropped), and takes the requests with
/// [`take`](Self::take). An [`Intake::Open`] or [`Intake::OpenFor`] batch comes made absolute and sorted by
/// [`natural_cmp`] of the path text; the app focuses documents already open ([`same_file`]) and
/// opens the rest, then brings its window to the front (on Wayland, a user-attention request).
/// [`Intake::Quit`] closes the app the way its own Quit command does.
///
/// ```ignore
/// let (intake, sender) = FileIntake::new(Wake::new(move || ctx.request_repaint()));
/// #[cfg(target_os = "macos")]
/// let _events = craft_platform::open::apple_events::install(sender.clone());
/// // each frame:
/// intake.sender().dropped(ctx.input(|i| i.raw.dropped_files.iter().map(to_dropped).collect()));
/// for request in intake.take() { app.handle(request); }
/// ```
pub struct FileIntake {
    queue: Arc<Mutex<VecDeque<Intake>>>,
    sender: IntakeSender,
}

impl FileIntake {
    /// An intake and the sender that feeds it; `wake` runs when a batch arrives.
    pub fn new(wake: Wake) -> (FileIntake, IntakeSender) {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let sender = IntakeSender { queue: queue.clone(), wake };
        (FileIntake { queue, sender: sender.clone() }, sender)
    }

    /// Another sender for this intake.
    pub fn sender(&self) -> IntakeSender {
        self.sender.clone()
    }

    /// The requests in arrival order, emptied.
    // pc: crates/ui-egui/src/file_open.rs:157 open_paths (absolute, sorted)
    pub fn take(&self) -> Vec<Intake> {
        #[cfg(any(test, all(target_os = "macos", feature = "apple-events")))]
        {
            let mine: Vec<_> = DRAINS.with(|drains| {
                drains
                    .borrow()
                    .iter()
                    .filter(|d| d.queue.upgrade().is_some_and(|q| Arc::ptr_eq(&q, &self.queue)))
                    .map(|d| std::rc::Rc::clone(&d.drain))
                    .collect()
            });
            for drain in mine {
                drain(&self.sender);
            }
        }
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.drain(..).collect()
    }
}

/// Whether an open document's path `open` names the file at `path`. Equal paths match at once;
/// otherwise both are resolved on disk (`fs::canonicalize`), `path` once by the caller
/// (`canonical`) and `open` only when the cheap checks fail. A file moved or deleted since it was
/// opened matches only its old path as written. On wasm only equal paths match.
// pc: crates/ui-egui/src/file_open.rs:438 same_file, :425 canonical
pub fn same_file(open: &Path, path: &Path, canonical: Option<&Path>) -> bool {
    open == path || canonical.is_some_and(|c| open == c || resolve(open).is_some_and(|q| q == c))
}

/// `path` with symlinks, `.` and `..` resolved; `None` when it can't be (a missing file, or wasm).
#[cfg(not(target_arch = "wasm32"))]
fn resolve(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

#[cfg(target_arch = "wasm32")]
fn resolve(_path: &Path) -> Option<PathBuf> {
    None
}

/// `path` made absolute against the working directory; unchanged when that fails (and on wasm).
fn absolute(path: &Path) -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(a) = std::path::absolute(path) {
        return a;
    }
    path.to_path_buf()
}

fn push(queue: &Mutex<VecDeque<Intake>>, batch: Intake) {
    queue.lock().unwrap_or_else(PoisonError::into_inner).push_back(batch);
}

/// A path as text, or the message for one that isn't valid Unicode ([`PATH_NOT_UNICODE_SAVE`]
/// when `saving`, else [`PATH_NOT_UNICODE_OPEN`]) with `{path}` filled in lossily.
// pc: crates/ui-egui/src/file_open.rs:75 path_text
pub fn path_text(path: &Path, saving: bool) -> Result<String, String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        let line = if saving { PATH_NOT_UNICODE_SAVE } else { PATH_NOT_UNICODE_OPEN };
        line.english.replace("{path}", &path.to_string_lossy())
    })
}

/// Whether `reveal` can show a file on this platform.
pub const CAN_REVEAL: bool = cfg!(not(target_arch = "wasm32"));

/// Whether [`open_with`] can open a file in an application on this platform.
pub const CAN_OPEN_WITH: bool = cfg!(not(target_arch = "wasm32"));

/// Shows `path` selected in the system file manager: `open -R` on macOS, Explorer's
/// `/select,"<path>"` on Windows (raw arguments; a path containing `"` is refused), and on Linux
/// and FreeBSD the FileManager1 D-Bus call through `dbus-send`, falling back to `xdg-open` on the
/// folder. The path is made absolute first (so a name starting with `-` is never an option); a
/// path that no longer exists is [`FILE_GONE`]. Not on wasm.
///
/// It runs on the caller's thread and returns the real result. On Linux and FreeBSD it can block
/// up to the D-Bus reply timeout (2 s) before the fallback runs, so call it from the host's
/// `Jobs`, never from the UI thread.
// pdfcraft: crates/ui-egui/src/menus/tab_context.rs:51 reveal, :77 reveal_process, :92 reveal_via
#[cfg(not(target_arch = "wasm32"))]
pub fn reveal(path: &Path) -> Result<(), String> {
    // Absolute paths also keep a file name starting with '-' from becoming an option.
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Err(FILE_GONE.english.into());
    }
    let run = |program: &str, args: &[OsString]| reveal_process(program, args).output().map(|o| o.status.success()).map_err(|e| e.to_string());
    reveal_via(std::env::consts::OS, &path, run)
}

/// The process that runs `program` with `args`. Windows: the arguments go on the command line as
/// written, because Rust's own quoting of an argument with a space breaks Explorer's `/select,`.
#[cfg(not(target_arch = "wasm32"))]
fn reveal_process(program: &str, args: &[OsString]) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        for arg in args {
            command.raw_arg(arg);
        }
    }
    #[cfg(not(windows))]
    command.args(args);
    command
}

#[cfg(not(target_arch = "wasm32"))]
fn reveal_via(platform: &str, path: &Path, mut run: impl FnMut(&str, &[OsString]) -> Result<bool, String>) -> Result<(), String> {
    let (program, args) = reveal_command(platform, path)?;
    if run(program, &args).unwrap_or(false) {
        return Ok(());
    }
    if matches!(platform, "linux" | "freebsd") {
        // FileManager1 is optional; without it the containing folder opens.
        let parent = path.parent().ok_or("The file has no parent directory")?;
        if run("xdg-open", &[parent.as_os_str().into()])? {
            return Ok(());
        }
    }
    Err(REVEAL_FAILED.english.into())
}

#[cfg(not(target_arch = "wasm32"))]
fn reveal_command(platform: &str, path: &Path) -> Result<(&'static str, Vec<OsString>), String> {
    match platform {
        "windows" => {
            // Explorer reads `/select,"<object>"`: the path quoted after the comma. A Windows
            // path can't contain `"`, so the quotes can't be closed early; one that does is refused.
            if path.as_os_str().as_encoded_bytes().contains(&b'"') {
                return Err("The file name can't be shown in File Explorer".into());
            }
            let mut selection = OsString::from("/select,\"");
            selection.push(path);
            selection.push("\"");
            Ok(("explorer.exe", vec![selection]))
        }
        "macos" => Ok(("open", vec!["-R".into(), path.as_os_str().into()])),
        _ => {
            let mut uri = String::from("array:string:file://");
            for b in path.as_os_str().as_encoded_bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
                    uri.push(char::from(*b));
                } else {
                    use std::fmt::Write as _;
                    let _ = write!(uri, "%{b:02X}");
                }
            }
            Ok((
                "dbus-send",
                vec![
                    "--session".into(),
                    "--print-reply".into(),
                    "--reply-timeout=2000".into(),
                    "--dest=org.freedesktop.FileManager1".into(),
                    "/org/freedesktop/FileManager1".into(),
                    "org.freedesktop.FileManager1.ShowItems".into(),
                    uri.into(),
                    "string:".into(),
                ],
            ))
        }
    }
}

/// Opens `target` in a new browser tab with `window.open(target, "_blank", "noopener,noreferrer")`; a `null` return
/// is success. The one web opener. Not on native targets (see the native `open_external`).
#[cfg(all(target_arch = "wasm32", feature = "browser"))]
pub fn open_external(target: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("There is no browser window to open the link in")?;
    // With `noopener` the browser hands back no window, so `None` is the usual success.
    window
        .open_with_url_and_target_and_features(target, "_blank", "noopener,noreferrer")
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
}

/// Opens a file or an `http(s)` URL with the system's handler (`open`, `rundll32
/// url.dll,FileProtocolHandler`, `xdg-open`), without waiting for it. On wasm the browser
/// build's `open_external` opens the URL in a new tab instead.
///
/// No shell sees the target: on Windows `cmd` would run the part of a link after a `&`, and on
/// macOS and Linux a target that starts with `-` would be read as an option, so `--` precedes it.
// new: PdfCraft, LightCraft, VectorCraft and FilmCraft each shell out for this (N10.05)
#[cfg(not(target_arch = "wasm32"))]
pub fn open_external(target: &str) -> Result<(), String> {
    let mut command = open_command(target);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Opens `target` in the application at `app` (Edit Original's chosen application), never through a shell: on
/// macOS `open -a app target`, waiting at most 2 s for Launch Services to refuse; elsewhere `app` runs with
/// `target` as its only argument, without waiting. A path holding `"` is refused on Windows.
///
/// A missing `app` is [`ErrorKind::NotFound`](std::io::ErrorKind::NotFound), so a caller can fall back to its default
/// editor; a path that is not an application ([`is_application`]) is `InvalidInput`, and nothing is started.
// PLAN: the contract row sh.B2a says `Result<(), String>`; the app needs the `NotFound` kind, so the error is `io::Error`.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_with(app: &Path, target: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if cfg!(windows) && target.as_os_str().as_encoded_bytes().contains(&b'"') {
        return Err(Error::new(ErrorKind::InvalidInput, "the file name can't be passed to an application on Windows"));
    }
    std::fs::metadata(app)?;
    if !is_application(app) {
        return Err(Error::new(ErrorKind::InvalidInput, format!("{} isn't an application files can be opened in", app.display())));
    }
    launch(app, target)
}

/// `open` returns once Launch Services has the request, so a short wait reports an app it can't open (a folder named
/// `.app` that isn't a bundle). The wait is capped so a stalled `open` never holds the window: past it, the launch is
/// left to run.
#[cfg(target_os = "macos")]
fn launch(app: &Path, target: &Path) -> std::io::Result<()> {
    use std::time::{Duration, Instant};
    const WAIT: Duration = Duration::from_secs(2);
    let mut child = Command::new("open").arg("-a").arg(app).arg(target).stderr(Stdio::piped()).spawn()?;
    let started = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let mut why = String::new();
                if let Some(mut err) = child.stderr.take() {
                    // Best effort: without its message the exit status says why.
                    let _ = std::io::Read::read_to_string(&mut err, &mut why);
                }
                let why = why.trim();
                return Err(std::io::Error::other(if why.is_empty() { status.to_string() } else { why.to_owned() }));
            }
            None if started.elapsed() < WAIT => std::thread::sleep(Duration::from_millis(20)),
            None => {
                // Reaped off the UI thread once it ends.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
        }
    }
}

/// The editor itself runs, so waiting would last until it closes.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "macos")))]
fn launch(app: &Path, target: &Path) -> std::io::Result<()> {
    Command::new(app).arg(target).spawn().map(|_| ())
}

/// Whether `path` names an application the platform can launch: an `.app` folder on macOS, an `.exe` file on
/// Windows, an executable file elsewhere; `false` on wasm.
pub fn is_application(path: &Path) -> bool {
    if cfg!(target_arch = "wasm32") {
        return false;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if cfg!(target_os = "macos") {
        ext == "app" && path.is_dir()
    } else if cfg!(windows) {
        ext == "exe" && path.is_file()
    } else {
        executable_file(path)
    }
}

#[cfg(unix)]
fn executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_file(path: &Path) -> bool {
    path.is_file()
}

/// The command that opens `target`; see [`open_external`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_command(target: &str) -> Command {
    if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.args(["--", target]);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.args(["--", target]);
        c
    }
}

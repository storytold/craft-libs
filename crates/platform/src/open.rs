//! Files reaching the app from outside, and the app handing files to the system.
//!
//! - [`FileIntake`]: one queue for files the OS asks the app to open (Finder double-click, Open
//!   With, a Dock drop, a second launch handing its paths to the first) and files dropped on the
//!   window, with where they were dropped ([`DropTarget`]).
//! - [`natural_cmp`], [`same_file`]: the order files open in and the check that keeps an open
//!   file from opening twice.
//! - [`OPEN_FAILED`] and the other [`PlatformText`]s: the open errors every app words the same way.
//! - [`reveal`], [`open_external`]: show a file in the system file manager; open a file or URL
//!   with the system's handler.
//!
//! No egui: the app converts egui's dropped files into [`DroppedFile`]s and reads the pointer
//! position for [`DropTarget`] itself.

use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use craft_host::Wake;

pub use craft_docpath::names::natural_cmp;

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

/// Where files dropped on the window land. winit's drops carry no position, so the app finds it
/// from the pointer (PhotoCraft asks the OS while files hover); unknown counts as
/// [`Elsewhere`](DropTarget::Elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropTarget {
    /// The active document's canvas: each file is placed as a layer, one at a time, each in Free
    /// Transform (Photoshop's Place Embedded). Preset files (brushes, gradients) still open into
    /// their libraries.
    Canvas,
    /// The tab strip: opened with their tabs from this position.
    Tabs(usize),
    /// Anywhere else (panels, a dialog, no document): opened like File › Open.
    Elsewhere,
}

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
    /// Files dropped on the window, in drop order, with where they landed.
    Dropped {
        /// The dropped files.
        files: Vec<DroppedFile>,
        /// Where they landed.
        target: DropTarget,
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

impl IntakeSender {
    /// Queues an OS open request and wakes the UI (`wake` is the app's `request_repaint`).
    pub fn open(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let mut paths: Vec<PathBuf> = paths.iter().map(|p| absolute(p)).collect();
        paths.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        push(&self.queue, Intake::Open(paths));
        self.wake.wake();
    }
}

/// Files waiting to be opened, read once per frame on the UI thread (spec Q-S4F-7).
///
/// The app makes one at startup, gives [`sender`](Self::sender) to whatever hears the OS, pushes
/// drops from its frame with [`dropped`](Self::dropped), and takes the batches with
/// [`take`](Self::take). An [`Intake::Open`] batch comes made absolute and sorted by
/// [`natural_cmp`] of the path text; the app focuses documents already open ([`same_file`]) and
/// opens the rest, then brings its window to the front (on Wayland, a user-attention request).
///
/// ```ignore
/// let (intake, sender) = FileIntake::new(Wake::new(move || ctx.request_repaint()));
/// // macOS: the app's Apple Event handler calls `sender.open(paths)`; elsewhere its single-instance socket does.
/// // each frame:
/// intake.dropped(ctx.input(|i| i.raw.dropped_files.iter().map(to_dropped).collect()), target);
/// for batch in intake.take() { app.open_batch(batch); }
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

    /// Queues files dropped this frame; an empty list is ignored.
    pub fn dropped(&self, files: Vec<DroppedFile>, target: DropTarget) {
        if !files.is_empty() {
            push(&self.queue, Intake::Dropped { files, target });
        }
    }

    /// The batches in arrival order, emptied.
    // pc: crates/ui-egui/src/file_open.rs:157 open_paths (absolute, sorted)
    pub fn take(&self) -> Vec<Intake> {
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

/// Shows `path` selected in the system file manager: `open -R` on macOS, Explorer's
/// `/select,"<path>"` on Windows (raw arguments; a path containing `"` is refused), and on Linux
/// and FreeBSD the FileManager1 D-Bus call through `dbus-send` on a thread of its own, falling back
/// to `xdg-open` on the folder. The path is made absolute first (so a name starting with `-` is
/// never an option); a path that no longer exists is [`FILE_GONE`]. Not on wasm.
// pdfcraft: crates/ui-egui/src/menus/tab_context.rs:51 reveal, :77 reveal_process, :92 reveal_via
#[cfg(not(target_arch = "wasm32"))]
pub fn reveal(path: &Path) -> Result<(), String> {
    // Absolute paths also keep a file name starting with '-' from becoming an option.
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Err(FILE_GONE.english.into());
    }
    let platform = std::env::consts::OS;
    let run = |program: &str, args: &[OsString]| reveal_process(program, args).output().map(|o| o.status.success()).map_err(|e| e.to_string());
    if matches!(platform, "linux" | "freebsd") {
        // `dbus-send --print-reply` waits up to 2 s for the file manager (then xdg-open may run),
        // so it runs off the UI thread; a failure there has nobody to tell.
        return std::thread::Builder::new()
            .name("craft-reveal".into())
            .spawn(move || {
                let _ = reveal_via(platform, &path, run);
            })
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    reveal_via(platform, &path, run)
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

/// Opens a file or an `http(s)` URL with the system's handler (`open`, `rundll32
/// url.dll,FileProtocolHandler`, `xdg-open`), without waiting for it. Not on wasm (the browser
/// module opens URLs there).
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

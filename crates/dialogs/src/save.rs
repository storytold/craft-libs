//! File › Save and File › Save As with their guards (N10.07), on the file service's pickers
//! ([`Files`]) and the watcher's file versions ([`FileWatch`]).
//!
//! [`SaveFlow`] decides where a save writes and what to ask first; the app serializes and writes
//! (on a job when large) and reports back. One save runs at a time per flow. The guards:
//!
//! - **No file, or Save As**: the save picker opens in the document's own folder
//!   ([`save_dialog_folder`]), with the document's name and extension suggested. The file
//!   service applies the extension policy and asks about replacing a file the panel didn't ask
//!   about.
//! - **An email attachment's temporary copy** ([`is_mail_temp_copy`]): Save asks where to keep
//!   the document, as for a new one; that folder is no place to keep work.
//! - **The file changed on disk** since the document was opened or last saved (another app
//!   wrote it): Save asks first ([`SaveStep::AskChangedOnDisk`]); the app opens craft-ui's
//!   `StandardPrompt::changed_on_disk` and answers with [`SaveFlow::replace`] (Replace),
//!   [`SaveFlow::save_elsewhere`] (Save As, keeps both) or [`SaveFlow::cancel`]. Only when
//!   confirming the document's own file (storytold/photocraft#6596, storytold/pdfcraft#2166).
//!
//! It is egui-free: the questions are steps the app answers, as with craft-quit's `CloseGuard`,
//! which hands its `GuardStep::Save(doc)` to [`SaveFlow::start`] and hears back through
//! [`SaveFlow::written`] (`CloseGuard::saved`) or a cancelled step (`CloseGuard::save_failed`).

use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft_host::{FileFilter, Files, PickId, PickRequest};
use craft_platform::PlatformText;
use craft_platform::watch::FileWatch;

/// Where a save writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaveTarget {
    /// File › Save: the document's own file; asks for one when it has none.
    InPlace,
    /// File › Save As: always asks; the document takes the new file.
    As,
    /// File › Save a Copy: always asks; the document keeps its own file and stays unsaved.
    Copy,
}

/// The document a save is for, as the flow needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveDocument<D> {
    /// The app's key for the document; every step carries it back.
    pub key: D,
    /// The display name ("poster.psd", "Untitled-2"): the suggested file name, and `{name}` in
    /// messages.
    pub name: String,
    /// The document's own file; `None` for a new document or one opened from bytes.
    pub path: Option<PathBuf>,
    /// The extension a picked name without one gets, lowercase (`"pdf"`).
    pub extension: String,
    /// The picker's filters, the document's format first.
    pub filters: Vec<FileFilter>,
    /// The picker's remembered-folder purpose (`"document"`).
    pub purpose: String,
    /// The bytes the app last wrote or read for the document's file (PdfCraft's `saved_bytes`).
    /// When only the file's change time differs (Finder tags, LaunchServices), equal bytes are no
    /// change on disk, so Save doesn't ask. `None` asks on any difference.
    pub saved: Option<Arc<Vec<u8>>>,
}

/// What the app does next.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "a save step needs its answer"]
pub enum SaveStep<D> {
    /// The save picker is open; [`SaveFlow::take_pick`] gives the next step once the user has
    /// answered it (the native panel answers after the pass that opened it).
    Picking,
    /// `path` is the document's own file and another app changed it: ask, then answer with
    /// [`SaveFlow::replace`], [`SaveFlow::save_elsewhere`] or [`SaveFlow::cancel`].
    AskChangedOnDisk {
        /// The document.
        key: D,
        /// The document's file.
        path: PathBuf,
    },
    /// Write the document to `path` now; then call [`SaveFlow::written`] or
    /// [`SaveFlow::failed`]. With [`SaveTarget::As`] the document takes `path` as its file; with
    /// [`SaveTarget::Copy`] it keeps its own.
    Write {
        /// The document.
        key: D,
        /// Where to write.
        path: PathBuf,
        /// Whether the document takes `path` as its file or keeps its own.
        target: SaveTarget,
    },
    /// The user cancelled (the picker or the question); nothing was written.
    Cancelled {
        /// The document whose save was cancelled.
        key: D,
    },
    /// Another save of this flow is still running (a picker, a question or a write); nothing
    /// started. The app shows [`SAVE_IN_PROGRESS`].
    Busy,
}

/// The state of one save.
#[derive(Clone, Debug)]
enum Pending<D> {
    Picking { document: SaveDocument<D>, target: SaveTarget, ticket: PickId },
    Asking { document: SaveDocument<D>, target: SaveTarget, path: PathBuf },
    Writing { key: D, target: SaveTarget },
}

/// Save and Save As for an app's documents, one save at a time.
///
/// ```ignore
/// // File › Save
/// match saves.start(host.files(), &watch, document(doc), SaveTarget::InPlace) {
///     SaveStep::Write { key, path, target } => write_on_a_job(key, path, target),
///     SaveStep::AskChangedOnDisk { key, path } => { dialogs.open(StandardPrompt::changed_on_disk(name(key)).tag(key)); }
///     SaveStep::Busy => status(SAVE_IN_PROGRESS),
///     SaveStep::Picking | SaveStep::Cancelled { .. } => {}
/// }
/// // Each pass: the picker's answer.
/// if let Some(step) = saves.take_pick(host.files(), &watch) { handle(step) }
/// // The job finished:
/// saves.written(&mut watch, key, &path);
/// ```
#[derive(Debug)]
pub struct SaveFlow<D> {
    pending: Option<Pending<D>>,
}

impl<D> Default for SaveFlow<D> {
    fn default() -> Self {
        Self { pending: None }
    }
}

impl<D: Clone + Eq + Hash + Send + 'static> SaveFlow<D> {
    /// A flow with no save in progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a save. [`SaveTarget::InPlace`] of a document with a file (not a mail temporary
    /// copy) goes straight to the changed-on-disk check, then [`SaveStep::Write`]; anything else
    /// opens the save picker in [`save_dialog_folder`] with `name` (its extension made
    /// `extension`) suggested.
    // pdfcraft: crates/ui-egui/src/editing/save.rs:41 save_then, :125 save_doc_to_then
    // pc: crates/ui-egui/src/app/files.rs:165 save_dialog_flow
    pub fn start(&mut self, files: &dyn Files, watch: &FileWatch<D>, document: SaveDocument<D>, target: SaveTarget) -> SaveStep<D> {
        if self.pending.is_some() {
            return SaveStep::Busy;
        }
        match document.path.clone() {
            Some(path) if target == SaveTarget::InPlace && !is_mail_temp_copy(&path) => self.confirm(watch, document, target, path),
            _ => self.open_picker(files, document, target),
        }
    }

    /// The picker's answer, once it has come ([`Files::take_answer`] of the flow's own ticket;
    /// other features' answers stay where they are): the changed-on-disk check when the picked
    /// path is the document's own file, else [`SaveStep::Write`]; a cancelled pick is
    /// [`SaveStep::Cancelled`]. `None` while the picker is still open or when no pick is pending.
    /// Call it every pass while [`is_busy`](Self::is_busy).
    // pc: crates/ui-egui/src/app/files.rs:205 pick_save continuation
    pub fn take_pick(&mut self, files: &dyn Files, watch: &FileWatch<D>) -> Option<SaveStep<D>> {
        let Some(Pending::Picking { ticket, .. }) = &self.pending else { return None };
        let picked = files.take_answer(*ticket)?;
        let Some(Pending::Picking { document, target, .. }) = self.pending.take() else { return None };
        let Some(path) = picked.first_path() else { return Some(SaveStep::Cancelled { key: document.key }) };
        if document.path.as_deref() == Some(path.as_path()) {
            return Some(self.confirm(watch, document, target, path));
        }
        self.pending = Some(Pending::Writing { key: document.key.clone(), target });
        Some(SaveStep::Write { key: document.key, path, target })
    }

    /// The changed-on-disk question's Replace: write over the changed file. [`SaveStep::Busy`]
    /// when no such question is open.
    // pdfcraft: crates/ui-egui/src/editing/save.rs:141 overwrite_changed_file
    pub fn replace(&mut self) -> SaveStep<D> {
        let Some(Pending::Asking { document, target, path }) = self.pending.take() else { return SaveStep::Busy };
        self.pending = Some(Pending::Writing { key: document.key.clone(), target });
        SaveStep::Write { key: document.key, path, target }
    }

    /// The changed-on-disk question's Save As: leave the file on disk as it is and ask where to
    /// put this version.
    // pdfcraft: crates/ui-egui/src/editing/save.rs:150 save_changed_file_elsewhere
    pub fn save_elsewhere(&mut self, files: &dyn Files) -> SaveStep<D> {
        let Some(Pending::Asking { document, target, .. }) = self.pending.take() else { return SaveStep::Busy };
        let target = if target == SaveTarget::Copy { target } else { SaveTarget::As };
        self.open_picker(files, document, target)
    }

    /// Cancels the save in progress: [`SaveStep::Cancelled`] for its document; `None` when
    /// nothing was running. A pending pick's ticket is dropped with [`Files::forget`], so its
    /// answer never reaches another save. The app calls it when the changed-on-disk question
    /// or a format options dialog is cancelled, and when the document closes mid-save.
    pub fn cancel(&mut self, files: &dyn Files) -> Option<SaveStep<D>> {
        let key = match self.pending.take()? {
            Pending::Picking { document, ticket, .. } => {
                files.forget(ticket);
                document.key
            }
            Pending::Asking { document, .. } => document.key,
            Pending::Writing { key, .. } => key,
        };
        Some(SaveStep::Cancelled { key })
    }

    /// The write the last [`SaveStep::Write`] asked for succeeded: the flow is free again (only for
    /// the write of `key`; a finished write of another document leaves the pending save alone) and
    /// the watcher records the file's new version, so the app's own save never reads as a change
    /// on disk (not for [`SaveTarget::Copy`], whose file isn't the document's).
    pub fn written(&mut self, watch: &mut FileWatch<D>, key: D, path: &Path) {
        let target = match &self.pending {
            Some(Pending::Writing { key: running, target }) if *running == key => {
                let target = *target;
                self.pending = None;
                target
            }
            // Another document's write (its save was cancelled), or none of this flow's: the
            // save in progress, if any, is left alone.
            _ => SaveTarget::As,
        };
        if target != SaveTarget::Copy {
            watch.known_now(key, path);
        }
    }

    /// The write failed; the flow is free again. The app shows [`SAVE_FAILED`].
    pub fn failed(&mut self, key: &D) {
        if matches!(&self.pending, Some(Pending::Writing { key: running, .. }) if running == key) {
            self.pending = None;
        }
    }

    /// A save is running: a picker, a question or a write.
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }
}

impl<D: Clone + Eq + Hash + Send + 'static> SaveFlow<D> {
    /// Writes `path`, the document's own file, unless another app changed it.
    fn confirm(&mut self, watch: &FileWatch<D>, document: SaveDocument<D>, target: SaveTarget, path: PathBuf) -> SaveStep<D> {
        if watch.disk_differs(&document.key, &path, document.saved.as_deref().map(Vec::as_slice)) {
            let key = document.key.clone();
            self.pending = Some(Pending::Asking { document, target, path: path.clone() });
            return SaveStep::AskChangedOnDisk { key, path };
        }
        self.pending = Some(Pending::Writing { key: document.key.clone(), target });
        SaveStep::Write { key: document.key, path, target }
    }

    fn open_picker(&mut self, files: &dyn Files, document: SaveDocument<D>, target: SaveTarget) -> SaveStep<D> {
        let mut request =
            PickRequest::save(document.purpose.clone()).file_name(suggested_name(&document)).suggested_extension(document.extension.clone());
        request.filters = document.filters.clone();
        request.directory = save_dialog_folder(document.path.as_deref());
        let ticket = files.pick(request);
        self.pending = Some(Pending::Picking { document, target, ticket });
        SaveStep::Picking
    }
}

/// The name the picker suggests: the document's name with its extension made `extension`. A new
/// document's name ("Untitled-2", "v1.2 notes") gets the extension appended; a document with a
/// file has a real file name, so its extension is replaced.
fn suggested_name<D>(document: &SaveDocument<D>) -> String {
    if document.extension.is_empty() {
        return document.name.clone();
    }
    let suffix = format!(".{}", document.extension);
    let name =
        if document.path.is_some() { Path::new(&document.name).with_extension("").to_string_lossy().into_owned() } else { document.name.clone() };
    if name.to_ascii_lowercase().ends_with(&suffix) { name } else { format!("{name}{suffix}") }
}

/// Whether `path` is the temporary copy an email client opened an attachment from, so that Save
/// asks where to keep the document instead of writing there. Outlook opens one from its cache:
/// `INetCache/Content.Outlook/<random>/<file>` for classic Outlook (`Temporary Internet
/// Files/Content.Outlook/` on older Windows; exactly one folder, then the file) and
/// `Packages/Microsoft.OutlookForWindows_<id>` for the new Outlook; Apple Mail from
/// `Library/Containers/com.apple.mail/Data/Library/Mail Downloads/`. Matched on either separator
/// and in any case, so it is testable on every platform.
// pdfcraft: crates/ui-egui/src/editing/save.rs:226 is_mail_temp_copy
pub fn is_mail_temp_copy(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    // Outlook's cache holds `<marker><random folder>/<file>`: exactly one folder, then the file.
    let in_outlook_cache = |marker: &str| {
        p.split_once(marker)
            .is_some_and(|(_, rest)| matches!(rest.split('/').collect::<Vec<_>>()[..], [dir, file] if !dir.is_empty() && !file.is_empty()))
    };
    in_outlook_cache("/inetcache/content.outlook/")
        || in_outlook_cache("/temporary internet files/content.outlook/")
        || p.contains("/packages/microsoft.outlookforwindows_")
        || p.contains("/library/containers/com.apple.mail/data/library/mail downloads/")
}

/// The folder a save picker opens in for a document stored at `path`: the document's own
/// folder, as Acrobat and Foxit do. `None` (the picker's remembered folder) for a new document,
/// a path with no parent, or an email attachment's temporary copy. Right for UNC paths too
/// (`\\server\share\dir\f.pdf` gives `\\server\share\dir`).
// pdfcraft: crates/ui-egui/src/editing/save.rs:244 save_dialog_folder
pub fn save_dialog_folder(path: Option<&Path>) -> Option<PathBuf> {
    let path = path.filter(|p| !is_mail_temp_copy(p))?;
    let parent = path.parent()?;
    (!parent.as_os_str().is_empty()).then(|| parent.to_path_buf())
}

/// "Couldn't save {name}: {error}".
pub const SAVE_FAILED: PlatformText = PlatformText { id: "save.failed", english: "Couldn't save “{name}”: {error}" };
/// [`SaveStep::Busy`]. PhotoCraft's wording is for its format options dialogs ("Answer the TIFF,
/// Targa, DDS or JPEG Options before starting another save"); this is the general one.
pub const SAVE_IN_PROGRESS: PlatformText = PlatformText { id: "save.inProgress", english: "Finish the save in progress before starting another." };
/// The status line after a save; `{name}`.
pub const SAVED: PlatformText = PlatformText { id: "save.saved", english: "Saved “{name}”" };
/// Every [`PlatformText`] of the save flow, for the missing-translation report.
pub const TEXTS: &[PlatformText] = &[SAVE_FAILED, SAVE_IN_PROGRESS, SAVED];

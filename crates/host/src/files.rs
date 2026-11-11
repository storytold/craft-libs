//! [`Files`]: the file service of a [`Host`](crate::Host). Features pick, read and write files
//! through it; craft-dialogs implements it over the native panels, apps on wasm over the browser.
//!
//! Pickers never block the frame: [`Files::pick`] returns a [`PickId`] at once and the answer
//! arrives on a later pass through [`Files::take_answer`]. One picker shows at a time; later
//! requests wait in order. A request starts in its own [`PickRequest::directory`] when it names
//! one; otherwise the service starts it in the last folder it remembers for the
//! [`PickRequest::purpose`]. It applies the extension policy to a saved name before the app sees it.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a picker asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PickKind {
    /// One existing file.
    Open,
    /// Several existing files.
    OpenMany,
    /// A folder.
    Folder,
    /// A new or existing file to write.
    Save,
}

/// One entry of a picker's type list: a display name and its extensions, lowercase, without
/// the dot (`("JPEG", ["jpg", "jpeg"])`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    /// The name the picker shows.
    pub name: String,
    /// The extensions, lowercase, without the dot.
    pub extensions: Vec<String>,
}

impl FileFilter {
    /// A filter named `name`. Each extension loses a leading dot and is lowercased.
    pub fn new(name: impl Into<String>, extensions: &[&str]) -> Self {
        Self { name: name.into(), extensions: extensions.iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect() }
    }
}

/// One picker request. Built with the builder methods and handed to [`Files::pick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRequest {
    /// What to pick.
    pub kind: PickKind,
    /// Groups requests that share a remembered folder (`"document"`, `"export"`, `"swatches"`).
    /// The picker opens in the folder the last answered request of this purpose chose, unless the
    /// request names a [`directory`](PickRequest::directory).
    pub purpose: String,
    /// The picker's title, when the platform shows one.
    pub title: Option<String>,
    /// The file types to offer.
    pub filters: Vec<FileFilter>,
    /// Starting folder. It wins over the purpose's remembered folder; a request without one
    /// starts in the remembered folder.
    pub directory: Option<PathBuf>,
    /// Starting folder while the purpose has no remembered folder yet (an export that starts
    /// beside the document the first time, then where the last export went).
    pub default_directory: Option<PathBuf>,
    /// Suggested name for [`PickKind::Save`].
    pub file_name: Option<String>,
    /// The extension a Save answer gets when the user typed none (`"png"`), lowercase.
    pub suggested_extension: Option<String>,
}

impl PickRequest {
    /// A request of `kind` for `purpose`, with no title, filters, folder or name.
    pub fn new(kind: PickKind, purpose: impl Into<String>) -> Self {
        Self {
            kind,
            purpose: purpose.into(),
            title: None,
            filters: Vec::new(),
            directory: None,
            default_directory: None,
            file_name: None,
            suggested_extension: None,
        }
    }

    /// A request to choose one existing file.
    pub fn open(purpose: impl Into<String>) -> Self {
        Self::new(PickKind::Open, purpose)
    }

    /// A request to choose one or more existing files.
    pub fn open_many(purpose: impl Into<String>) -> Self {
        Self::new(PickKind::OpenMany, purpose)
    }

    /// A request to choose a folder.
    pub fn folder(purpose: impl Into<String>) -> Self {
        Self::new(PickKind::Folder, purpose)
    }

    /// A request to choose where to write a file.
    pub fn save(purpose: impl Into<String>) -> Self {
        Self::new(PickKind::Save, purpose)
    }

    /// Sets the picker's title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Adds a file type; see [`FileFilter::new`].
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(FileFilter::new(name, extensions));
        self
    }

    /// Sets the folder to start in, instead of the purpose's remembered folder.
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());
        self
    }

    /// Sets the folder to start in while the purpose has no remembered folder.
    pub fn default_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.default_directory = Some(directory.into());
        self
    }

    /// Sets the name a Save picker suggests.
    pub fn file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    /// Sets the extension a Save answer gets when the user typed none; a leading dot is dropped
    /// and the text is lowercased.
    pub fn suggested_extension(mut self, extension: impl Into<String>) -> Self {
        self.suggested_extension = Some(extension.into().trim_start_matches('.').to_ascii_lowercase());
        self
    }
}

/// The ticket of a pending pick; the answer carries it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PickId(pub u64);

/// What the user chose. On wasm a chosen file has no path the app could reopen, so an Open
/// answer carries the bytes too and the path is the browser's file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picked {
    /// The user cancelled.
    Cancelled,
    /// One or more paths. For [`PickKind::Save`] the extension policy has been applied, and the
    /// service has already asked about replacing an existing file the OS panel did not check.
    Paths(Vec<PathBuf>),
    /// Files read by the browser (wasm): name and contents.
    Contents(Vec<(String, Vec<u8>)>),
    /// No picker could be shown (no desktop portal or `zenity` on Linux, no window to attach to);
    /// the text is the technical cause, not a sentence for the user. A feature without a status
    /// line to report on treats it as nothing chosen.
    Unavailable(String),
}

impl Picked {
    /// The first chosen path; on wasm the first file's name. `None` when cancelled or unavailable.
    pub fn first_path(&self) -> Option<PathBuf> {
        match self {
            Picked::Paths(paths) => paths.first().cloned(),
            Picked::Contents(files) => files.first().map(|(name, _)| PathBuf::from(name)),
            Picked::Cancelled | Picked::Unavailable(_) => None,
        }
    }

    /// Every chosen path; on wasm the files' names. Empty when cancelled or unavailable.
    pub fn paths(&self) -> Vec<PathBuf> {
        match self {
            Picked::Paths(paths) => paths.clone(),
            Picked::Contents(files) => files.iter().map(|(name, _)| PathBuf::from(name)).collect(),
            Picked::Cancelled | Picked::Unavailable(_) => Vec::new(),
        }
    }
}

/// One answered pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickAnswer {
    /// The ticket of the pick.
    pub id: PickId,
    /// The request that was answered.
    pub request: PickRequest,
    /// What the user chose.
    pub picked: Picked,
}

/// Why a read or write failed. The message reads the same in every app (F15.06).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileError {
    /// The path does not exist. A conversion from `std::io::Error` leaves the path empty.
    NotFound(PathBuf),
    /// The path exists but the process may not use it. A conversion from `std::io::Error`
    /// leaves the path empty.
    PermissionDenied(PathBuf),
    /// The service cannot do this on the platform (write a path on wasm).
    Unsupported,
    /// Any other failure, with the system's message.
    Other(String),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::NotFound(p) => write!(f, "“{}” was not found", p.display()),
            FileError::PermissionDenied(p) => write!(f, "“{}” cannot be accessed", p.display()),
            FileError::Unsupported => f.write_str("not available on this platform"),
            FileError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FileError {}

impl From<std::io::Error> for FileError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => FileError::NotFound(PathBuf::new()),
            std::io::ErrorKind::PermissionDenied => FileError::PermissionDenied(PathBuf::new()),
            _ => FileError::Other(e.to_string()),
        }
    }
}

/// The file service. Pickers are queued and answered later; reads and writes are synchronous
/// (features run large ones through [`Jobs`](crate::Jobs)).
pub trait Files: Send + Sync {
    /// Queues a picker; the answer comes back from [`take_answer`](Files::take_answer) on a
    /// later pass. The ticket lets a feature match the answer to what it asked.
    fn pick(&self, request: PickRequest) -> PickId;
    /// The answer of ticket `id`, once it has come; it is removed. Other tickets' answers stay
    /// where they are, so a feature that never asks for its answer blocks no other feature.
    fn take_answer(&self, id: PickId) -> Option<Picked>;
    /// Drops ticket `id`: its answer, if it has come, and the answer that has yet to come. An
    /// owner that closes with a pick outstanding calls it.
    fn forget(&self, id: PickId);
    /// Whether a picker is on screen or waiting.
    fn pick_pending(&self) -> bool;
    /// Reads the whole file at `path`.
    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError>;
    /// Writes atomically where the platform allows (temp file and rename).
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError>;
    /// The folder the last answered pick of `purpose` chose.
    fn last_folder(&self, purpose: &str) -> Option<PathBuf>;
}

/// No pickers (every pick answers `Cancelled`); reads and writes go to the file system. The
/// default on native when the app installs no service; tests use it too.
#[derive(Default)]
pub struct NoPickFiles {
    next: Mutex<u64>,
    answered: Mutex<Vec<PickAnswer>>,
}

impl Files for NoPickFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        *next += 1;
        let id = PickId(*next);
        self.answered.lock().unwrap_or_else(|e| e.into_inner()).push(PickAnswer { id, request, picked: Picked::Cancelled });
        id
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut answered = self.answered.lock().unwrap_or_else(|e| e.into_inner());
        let at = answered.iter().position(|a| a.id == id)?;
        Some(answered.remove(at).picked)
    }

    fn forget(&self, id: PickId) {
        self.answered.lock().unwrap_or_else(|e| e.into_inner()).retain(|a| a.id != id);
    }

    fn pick_pending(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        std::fs::read(path).map_err(|e| with_path(e, path))
    }

    #[cfg(target_arch = "wasm32")]
    fn read(&self, _path: &Path) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError> {
        let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
        std::fs::write(&tmp, bytes).map_err(|e| with_path(e, path))?;
        std::fs::rename(&tmp, path).map_err(|e| with_path(e, path))
    }

    #[cfg(target_arch = "wasm32")]
    fn write(&self, _path: &Path, _bytes: &[u8]) -> Result<(), FileError> {
        Err(FileError::Unsupported)
    }

    fn last_folder(&self, _purpose: &str) -> Option<PathBuf> {
        None
    }
}

/// A file service that answers from a script, for tests of the features that ask for files. It
/// records every [`PickRequest`], keeps files in memory, and answers each pick in order from the
/// answers the test queued: the first queued answer goes to the first pick, and a pick with
/// nothing queued is `Cancelled`.
///
/// A test that expects no picker asserts [`requests`](ScriptedFiles::requests) is empty. Every
/// method takes `&self`, so a test keeps its own `Arc<ScriptedFiles>` and hands a clone to the
/// Host as the `Arc<dyn Files>`.
#[derive(Default)]
pub struct ScriptedFiles {
    state: Mutex<ScriptState>,
}

#[derive(Default)]
struct ScriptState {
    next: u64,
    asked: Vec<PickRequest>,
    /// Answers for the picks to come, first pick first.
    queued: VecDeque<Picked>,
    /// Picks stay open until the test answers them.
    held: bool,
    /// A Save is answered with the suggested name in the suggested folder.
    save_suggested: bool,
    /// Picks the test has not answered yet, oldest first.
    open: Vec<(PickId, PickRequest)>,
    answered: Vec<PickAnswer>,
    files: HashMap<PathBuf, Vec<u8>>,
}

impl ScriptedFiles {
    /// A service with nothing queued: every pick is `Cancelled`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A service that answers the picks with `answers`, in order.
    pub fn answering(answers: impl IntoIterator<Item = Picked>) -> Self {
        let files = Self::default();
        files.state().queued.extend(answers);
        files
    }

    /// A service whose picks stay open until the test answers them with
    /// [`answer`](ScriptedFiles::answer) or [`answer_to`](ScriptedFiles::answer_to): the state of
    /// a user who has the dialog on screen.
    pub fn held() -> Self {
        let files = Self::default();
        files.state().held = true;
        files
    }

    /// Answers every Save with the suggested name in the suggested folder; other picks follow the
    /// queue as before.
    pub fn saving_suggested(self) -> Self {
        self.state().save_suggested = true;
        self
    }

    /// Puts a file in memory, for [`Files::read`].
    pub fn with_file(self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Self {
        self.state().files.insert(path.into(), bytes.into());
        self
    }

    /// Queues the answer of the next pick that has none yet.
    pub fn queue(&self, picked: Picked) {
        self.state().queued.push_back(picked);
    }

    /// Every request asked so far, in order.
    pub fn requests(&self) -> Vec<PickRequest> {
        self.state().asked.clone()
    }

    /// How many picks the test has yet to answer (a [`held`](ScriptedFiles::held) service).
    pub fn open(&self) -> usize {
        self.state().open.len()
    }

    /// Answers the newest open pick; its ticket, or `None` when none is open.
    pub fn answer(&self, picked: Picked) -> Option<PickId> {
        let mut state = self.state();
        let (id, request) = state.open.pop()?;
        state.answered.push(PickAnswer { id, request, picked });
        Some(id)
    }

    /// Answers the open pick `id`; `false` when it is not open.
    pub fn answer_to(&self, id: PickId, picked: Picked) -> bool {
        let mut state = self.state();
        let Some(at) = state.open.iter().position(|(open, _)| *open == id) else { return false };
        let (_, request) = state.open.remove(at);
        state.answered.push(PickAnswer { id, request, picked });
        true
    }

    /// The bytes last written to `path`, or put there with [`with_file`](ScriptedFiles::with_file).
    pub fn file(&self, path: &Path) -> Option<Vec<u8>> {
        self.state().files.get(path).cloned()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ScriptState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The suggested name of a Save in its suggested folder.
    fn suggested(request: &PickRequest) -> Picked {
        let name = PathBuf::from(request.file_name.clone().unwrap_or_default());
        Picked::Paths(vec![request.directory.as_ref().map_or_else(|| name.clone(), |dir| dir.join(&name))])
    }
}

impl Files for ScriptedFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let mut state = self.state();
        state.next += 1;
        let id = PickId(state.next);
        let suggested = (state.save_suggested && request.kind == PickKind::Save).then(|| Self::suggested(&request));
        state.asked.push(request.clone());
        if state.held {
            state.open.push((id, request));
            return id;
        }
        let picked = suggested.or_else(|| state.queued.pop_front()).unwrap_or(Picked::Cancelled);
        state.answered.push(PickAnswer { id, request, picked });
        id
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut state = self.state();
        let at = state.answered.iter().position(|a| a.id == id)?;
        Some(state.answered.remove(at).picked)
    }

    fn forget(&self, id: PickId) {
        let mut state = self.state();
        state.answered.retain(|a| a.id != id);
        state.open.retain(|(open, _)| *open != id);
    }

    fn pick_pending(&self) -> bool {
        !self.state().open.is_empty()
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        self.file(path).ok_or_else(|| FileError::NotFound(path.to_owned()))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError> {
        self.state().files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn last_folder(&self, _purpose: &str) -> Option<PathBuf> {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn with_path(e: std::io::Error, path: &Path) -> FileError {
    match FileError::from(e) {
        FileError::NotFound(_) => FileError::NotFound(path.to_owned()),
        FileError::PermissionDenied(_) => FileError::PermissionDenied(path.to_owned()),
        other => other,
    }
}

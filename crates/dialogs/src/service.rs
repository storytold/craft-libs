//! [`FileService`]: craft-host's [`Files`] over the native panels of [`Dialogs`].
//!
//! The app builds the pair at startup, installs the service in the Host
//! (`Host::builder(..).files(Arc::new(service))`) and keeps the [`FilePanels`] on the main thread,
//! calling [`FilePanels::pump`] once per frame after its UI (the panel shows at the end of the
//! frame). The service is `Send + Sync`; the panels are not, because the native dialogs are not. Features never see the panels: they ask `host.files().pick(..)` and
//! read the answer from `host.files().take_answer(id)` on a later pass.
//!
//! What the user sees: Open and Folder as free-standing panels on macOS, Save as a sheet; the panel
//! starts in the request's own folder when it names one (Save As in the document's folder) and
//! otherwise in the last folder chosen for the same purpose; a Save that names an existing file the
//! OS panel did not ask about gets the Replace question (PhotoCraft `file_dialog.rs:193
//! replaces_unasked`), a native message box from [`FilePanels::pump`] or, with the `ui` feature, a
//! prompt on the app's `DialogHost` from `FilePanels::pump_with`; a typed name without an extension gets the suggested one and a new file's
//! uppercase extension is lowercased, while an existing file keeps its spelling (`file_dialog.rs:210 lowercased_extension`, `:365 saved_path`); the
//! steps are separate settings of the [`ExtensionPolicy`]. A name the panel left alone takes the
//! extension of the file type picked in the panel (`with_chosen_type`): the GTK file chooser keeps
//! the typed name when another type is picked, and the format follows the extension.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use craft_host::{FileError, FileFilter, Files, PendingPick, PickAnswer, PickId, PickRequest, Wake, WriteIntent};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[cfg(feature = "ui")]
use craft_browse::places::{FolderCache, SystemDirectories};
#[cfg(feature = "ui")]
use craft_host::ThreadJobs;
use craft_host::{PickKind, Picked};

#[cfg(feature = "ui")]
use crate::ui::FileChooser;
use crate::{Answer, Dialogs, FileDialog, Filter, Kind, Message, MessageButtons, MessageLevel, MessageResult, probe};

/// The words of the Replace question: a native message box, so they come from the app's catalog
/// (see `ReplaceLabels` in the `ui` feature) instead of `words!` at draw time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceWords {
    /// The box's title.
    pub title: String,
    /// The question, with `{name}` for the file name.
    pub question: String,
    /// The confirming button, the default.
    pub replace: String,
    /// The cancelling button.
    pub cancel: String,
}

impl Default for ReplaceWords {
    fn default() -> Self {
        Self {
            title: "Replace".into(), question: "“{name}” already exists. Replace it?".into(), replace: "Replace".into(), cancel: "Cancel".into()
        }
    }
}

fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|e| e.into_inner())
}

fn file_error(e: std::io::Error, path: &Path) -> FileError {
    match e.kind() {
        std::io::ErrorKind::NotFound => FileError::NotFound(path.to_owned()),
        std::io::ErrorKind::PermissionDenied => FileError::PermissionDenied(path.to_owned()),
        _ => FileError::Other(e.to_string()),
    }
}

/// Writes `bytes` to `path` atomically: the one write of [`Files::write`] and of a finished save.
// pc: apps/photocraft/src/services.rs:359 write_atomic
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    craft_atomic::atomic_write(path, bytes).map_err(|e| file_error(e, path))
}

/// The folder a picker starts in: the request's own folder, else the purpose's remembered one
/// (`last`), else the request's first-time folder. The native panels and the fallback chooser
/// both start here.
fn start_folder(request: &PickRequest, last: Option<&Path>) -> Option<PathBuf> {
    request.directory.clone().or_else(|| last.map(Path::to_path_buf)).or_else(|| request.default_directory.clone())
}

/// The name a Save panel starts with. Where the panel adds the filter's extension itself
/// (`adds_extension`, Windows), a lowercase extension is left off so the name does not end up
/// doubled; an uppercase extension is the user's spelling and stays.
// pc: apps/photocraft/src/services.rs:192 dialog_file_name
pub(crate) fn dialog_file_name(suggested: &str, adds_extension: bool) -> Option<String> {
    let path = Path::new(suggested);
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (Some(stem), Some(ext)) = (path.file_stem(), path.extension().and_then(|e| e.to_str())) else { return Some(name) };
    let bare = adds_extension && Path::new(stem).extension().is_none() && !ext.is_empty() && ext == ext.to_ascii_lowercase();
    Some(if bare { stem.to_string_lossy().into_owned() } else { name })
}

/// Whether `a` and `b` reach the same file (device and inode on Unix, final path elsewhere).
// pc: apps/photocraft/src/services.rs:370 same_file
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

/// Whether `a` and `b` name one entry in one folder: another spelling of a name on a file system
/// that ignores letter case is the same entry; a hard link is another entry.
// pc: apps/photocraft/src/services.rs:392 same_entry
fn same_entry(a: &Path, b: &Path) -> bool {
    let folder = |p: &Path| p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
    if !same_file(a, b) || !same_file(&folder(a), &folder(b)) {
        return false;
    }
    let (Some(x), Some(y)) = (a.file_name(), b.file_name()) else { return false };
    if x == y {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(folder(a)) else { return false };
    let listed: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    !(listed.iter().any(|n| n == x) && listed.iter().any(|n| n == y))
}

/// How a Save answer's extension is settled before the app sees it. The two steps are separate:
/// PhotoCraft with Lowercase Extension off still adds the suggested extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionPolicy {
    /// Lowercase an extension typed with capitals (`Photo.PNG` becomes `Photo.png`), unless the
    /// typed name is a file that already exists.
    pub lowercase: bool,
    /// Add the request's suggested extension to a name that has none.
    pub add_suggested: bool,
    /// Give a name the panel left alone the extension of the file type picked in the panel.
    pub picked_type: bool,
}

impl ExtensionPolicy {
    /// Keep the typed name as the panel answered it.
    pub const AS_TYPED: Self = Self { lowercase: false, add_suggested: false, picked_type: false };
}

impl Default for ExtensionPolicy {
    /// Lowercase, and add the suggested extension.
    fn default() -> Self {
        Self { lowercase: true, add_suggested: true, picked_type: true }
    }
}

/// An extension as compared: without the dot, in lower case.
pub(crate) fn normal(ext: &str) -> String {
    ext.trim_start_matches('.').to_ascii_lowercase()
}

/// `path` with the extension of the file type picked in the Save panel (`chosen`, its extensions
/// without the dot), where the panel left the name alone: the format follows the extension, and
/// the GTK file chooser keeps the typed name when another type is picked. Only a name that still
/// has the extension the panel opened with (`opened_with`, `Untitled-1.psd` with PNG picked) or
/// none gets the type's first extension, as PolyMeilex/rfd#341 does. An extension of the picked
/// type stays (`.psb` for PSD), and so does one that any of the request's `filters` lists
/// (`art.png` with the PSD type untouched): the user typed a type the app saves. Any other
/// extension (`scan.2024`) is part of the name, and the type's extension is added after it. With
/// no type picked the path is as typed. Extensions compare in any letter case.
// PLAN: fl.F9 says `&[Filter]`; `settled_save_path` holds a `PickRequest`, whose filters are `FileFilter`s
// pc: apps/photocraft/src/services.rs:161 with_chosen_type
pub(crate) fn with_chosen_type(path: &Path, opened_with: Option<&str>, chosen: &[String], filters: &[FileFilter]) -> PathBuf {
    let Some(first) = chosen.first() else { return path.to_path_buf() };
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let opened_ext = opened_with.and_then(|n| Path::new(n).extension()).and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let first = normal(first);
    let lists = |ext: &str| filters.iter().any(|f| f.extensions.iter().any(|e| normal(e) == ext));
    match ext {
        Some(ext) if chosen.iter().any(|e| normal(e) == ext) => path.to_path_buf(),
        Some(ext) if opened_ext.as_ref() == Some(&ext) => path.with_extension(first),
        Some(ext) if lists(&ext) => path.to_path_buf(),
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(".");
            name.push(first);
            PathBuf::from(name)
        }
    }
}

/// Which file type's extensions belong on the name the panel returned: the one it reported, or,
/// when the panel offered only one type, that one. It reports no pick then, and the name it was
/// given is the stem, so a bare `Swatches` has to come back as `Swatches.aco`. `None` when the
/// answer names no type the panel offered.
// pc: apps/photocraft/src/portal_save.rs:163 chosen_type
fn chosen_type(filters: &[FileFilter], picked: Option<usize>) -> Option<usize> {
    match picked {
        Some(i) if i < filters.len() => Some(i),
        None if filters.len() == 1 => Some(0),
        _ => None,
    }
}

/// Where a Save answer that would overwrite goes next: the panel already asked (macOS, Windows,
/// GTK and the portal all do), so only answers that still name an existing file on a platform
/// that did not ask get the question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReplaceGuard {
    /// Ask with a native message box when the panel did not.
    #[default]
    AskWhenUnasked,
    /// Never ask (the app asks with `StandardPrompt::replace_file` itself).
    Never,
}

/// One pending request: the ticket, what was asked, and whether the service is still settling the
/// answer (the replace question).
struct Pending {
    request: PickRequest,
    /// The owner dropped the ticket while its panel may still be on screen: the entry stays so the
    /// window stays held, and the panel's answer is dropped.
    forgotten: bool,
    /// The Save path waiting on the replace question's answer.
    awaiting_replace: Option<ReplaceQuestion>,
    /// What a [`Files::save`] writes to the settled path.
    bytes: Option<Vec<u8>>,
}

/// A Save answer that collides with an existing file after the extension policy: the path the
/// panel answered and the path it settled to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplaceQuestion {
    answered: PathBuf,
    settled: PathBuf,
    /// The file type the panel reported, handed back with the settled path.
    file_type: Option<usize>,
}

/// A replace question shown on the app's `DialogHost`: the pick it settles and the prompt's id,
/// once the host has accepted the prompt.
#[cfg(feature = "ui")]
struct AskedPrompt {
    id: PickId,
    question: ReplaceQuestion,
    dialog: Option<craft_ui::dialog::DialogId>,
}

/// A pick the fallback chooser answers on the app's `DialogHost`: the ticket, its request, why the
/// native dialog was not used (`None` when the request asked for the chooser), and the chooser's
/// id once it is open.
#[cfg(feature = "ui")]
struct AskedChooser {
    id: PickId,
    request: PickRequest,
    cause: Option<String>,
    dialog: Option<craft_ui::dialog::DialogId>,
}

/// The native file service: the Host's side. Requests go to the [`FilePanels`] over a channel;
/// answers come back through the shared state.
pub struct FileService {
    inner: Arc<Mutex<Inner>>,
    wake: Wake,
    to_panels: Sender<(PickId, PickRequest)>,
}

/// The main-thread side: owns the native panels and settles answers. Built with the service by
/// [`FileService::new`].
pub struct FilePanels {
    inner: Arc<Mutex<Inner>>,
    requests: Receiver<(PickId, PickRequest)>,
    dialogs: Dialogs<PickId>,
    extension_policy: ExtensionPolicy,
    replace_guard: ReplaceGuard,
    replace_words: ReplaceWords,
    /// Why no file dialog can open on this system, when the probe knows.
    backend_problem: fn() -> Option<String>,
    /// The replace questions [`pump_with`](FilePanels::pump_with) put on the app's dialog host.
    #[cfg(feature = "ui")]
    prompts: Vec<AskedPrompt>,
    /// The picks [`pump_with`](FilePanels::pump_with) answers with the fallback chooser, in order.
    #[cfg(feature = "ui")]
    choosers: Vec<AskedChooser>,
    /// Repaint request for the fallback chooser's folder jobs.
    #[cfg(feature = "ui")]
    wake: Wake,
}

struct Inner {
    next: u64,
    pending: HashMap<PickId, Pending>,
    answered: VecDeque<PickAnswer>,
    last_folder: HashMap<String, PathBuf>,
    /// The answers `Files::answer_next` gave, which the panels answer the next requests with.
    forced: VecDeque<Answer>,
}

impl Inner {
    /// Whether pick `id` is pending and its owner still waits for the answer.
    fn wanted(&self, id: &PickId) -> bool {
        self.pending.get(id).is_some_and(|p| !p.forgotten)
    }
}

impl FileService {
    /// `wake` requests a repaint from any thread (`ctx.request_repaint()`).
    // pc: apps/photocraft/src/services.rs:203 show_file_dialog; crates/ui-egui/src/file_dialog.rs:310 poll_file_dialog
    pub fn new(wake: Wake) -> (Self, FilePanels) {
        probe::start();
        let inner = Arc::new(Mutex::new(Inner {
            next: 0,
            pending: HashMap::new(),
            answered: VecDeque::new(),
            last_folder: HashMap::new(),
            forced: VecDeque::new(),
        }));
        let (to_panels, requests) = std::sync::mpsc::channel();
        let panels = FilePanels {
            inner: inner.clone(),
            requests,
            dialogs: Dialogs::new(wake.clone()),
            extension_policy: ExtensionPolicy::default(),
            replace_guard: ReplaceGuard::default(),
            replace_words: ReplaceWords::default(),
            backend_problem: probe::known_problem,
            #[cfg(feature = "ui")]
            prompts: Vec::new(),
            #[cfg(feature = "ui")]
            choosers: Vec::new(),
            #[cfg(feature = "ui")]
            wake: wake.clone(),
        };
        (Self { inner, wake, to_panels }, panels)
    }

    /// A service whose panels answer each pick with the next of `answers` instead of opening a
    /// native dialog, and answer [`Answer::Cancelled`] once the queue is empty. The answers pass
    /// through the same settling as native ones (extension completion, the Replace question), so an
    /// app test drives the real [`FilePanels::pump_with`] path, including
    /// [`FilePanels::open_questions`], without a window system.
    #[cfg(feature = "ui")]
    pub fn scripted(wake: Wake, answers: impl IntoIterator<Item = Answer>) -> (Self, FilePanels) {
        let (service, mut panels) = Self::new(wake);
        panels.backend_problem = || None;
        let mut answers = answers.into_iter().collect::<VecDeque<_>>();
        panels.dialogs.set_override(Some(Box::new(move |_| answers.pop_front().unwrap_or(Answer::Cancelled))));
        (service, panels)
    }

    /// Seeds the remembered folders (from the app's settings) before the first pick; the app reads
    /// them back with [`last_folders`](FileService::last_folders) to store them.
    pub fn with_last_folders(self, folders: HashMap<String, PathBuf>) -> Self {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).last_folder = folders;
        self
    }

    /// Every remembered folder by purpose, for the app's settings.
    pub fn last_folders(&self) -> HashMap<String, PathBuf> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).last_folder.clone()
    }
}

impl FilePanels {
    /// Sets how a Save answer without an extension is completed.
    pub fn extension_policy(mut self, policy: ExtensionPolicy) -> Self {
        self.extension_policy = policy;
        self
    }

    /// Runs `hook` around every native dialog's blocking show, so the app can mark the wait as waiting for the user.
    pub fn show_hook(mut self, hook: crate::ShowHook) -> Self {
        self.dialogs.set_show_hook(hook);
        self
    }

    /// Sets whether the panels ask the Replace question when the native panel did not.
    pub fn replace_guard(mut self, guard: ReplaceGuard) -> Self {
        self.replace_guard = guard;
        self
    }

    /// The Replace question's words; the app sets them again when the language changes.
    pub fn replace_words(mut self, words: ReplaceWords) -> Self {
        self.replace_words = words;
        self
    }

    /// [`replace_words`](FilePanels::replace_words) on panels the app already keeps.
    pub fn set_replace_words(&mut self, words: ReplaceWords) {
        self.replace_words = words;
    }

    /// [`extension_policy`](FilePanels::extension_policy) on panels the app already keeps, for a
    /// preference that changes while the app runs.
    pub fn set_extension_policy(&mut self, policy: ExtensionPolicy) {
        self.extension_policy = policy;
    }

    /// Shows the next queued panel and collects answers. Called once per frame on the main thread
    /// with the app window (`Some(frame)` in eframe) as the parent; `None` shows free-standing
    /// panels. Save answers go through the extension policy, then the replace guard (which may
    /// queue a message box and finish on a later pump), then the last folder is remembered and
    /// the answer becomes a [`PickAnswer`].
    // pc: crates/ui-egui/src/file_dialog.rs:310 poll_file_dialog, :349 finish_file_dialog, :365 saved_path
    pub fn pump<W: HasWindowHandle + HasDisplayHandle>(&mut self, parent: Option<&W>) {
        // Forced answers settle first, with no panel.
        for (id, answer) in self.start_requests() {
            if let Some(question) = self.finish(id, answer, false) {
                self.dialogs.ask_message(self.replace_message(&question.settled), id);
            }
        }
        self.cancel_forgotten();
        // A replace question queued by `finish` shows in the same pass, so the loop runs until no
        // answer is left to settle.
        loop {
            self.dialogs.show_pending(parent);
            let answers = self.dialogs.take_answers();
            if answers.is_empty() {
                break;
            }
            for (id, answer) in answers {
                if let Some(question) = self.finish(id, answer, true) {
                    self.dialogs.ask_message(self.replace_message(&question.settled), id);
                }
            }
        }
    }

    /// [`pump`](FilePanels::pump) with the replace question on the app's own dialog host: the
    /// question is a [`StandardPrompt::replace_file`](craft_ui::dialog::StandardPrompt::replace_file)
    /// the app's `DialogHost::show` draws, so `ui.dialog.confirm` and `ui.dialog.cancel` answer it
    /// like every prompt, and the answer is read on a later pump. A prompt of the app's own that is
    /// open makes the question wait its turn. The question is a prompt instance
    /// ([`StandardPrompt::instance`](craft_ui::dialog::StandardPrompt::instance)), so the app's
    /// `take::<StandardPrompt>()` and its own prompts leave its answer alone.
    ///
    /// A request with [`PickRequest::fallback`] set, and one whose native dialog answered
    /// [`Answer::Unavailable`], is answered by the in-app [`FileChooser`] on the same host instead,
    /// one chooser at a time; its answer is settled like a native one.
    // pc: crates/ui-egui/src/dialogs/replace_prompt.rs:23 open
    #[cfg(feature = "ui")]
    pub fn pump_with<W: HasWindowHandle + HasDisplayHandle>(&mut self, dialogs: &mut craft_ui::dialog::DialogHost, parent: Option<&W>) {
        self.collect_prompt_answers(dialogs);
        self.collect_chooser_answers(dialogs);
        while let Ok((id, request)) = self.requests.try_recv() {
            if let Some(answer) = self.forced_answer(id) {
                if let Some(question) = self.finish(id, answer, false) {
                    self.prompts.push(AskedPrompt { id, question, dialog: None });
                }
            } else if request.fallback {
                self.choosers.push(AskedChooser { id, request, cause: None, dialog: None });
            } else {
                self.ask_panel(id, &request);
            }
        }
        self.cancel_forgotten();
        loop {
            self.dialogs.show_pending(parent);
            let answers = self.dialogs.take_answers();
            if answers.is_empty() {
                break;
            }
            for (id, answer) in answers {
                // No native dialog could show: the chooser answers the pick, unless it was
                // forgotten or is settling a replace question.
                let answer = match answer {
                    Answer::Unavailable(why) => {
                        let unsettled =
                            lock(&self.inner).pending.get(&id).filter(|p| p.awaiting_replace.is_none() && !p.forgotten).map(|p| p.request.clone());
                        match unsettled {
                            Some(request) => {
                                self.choosers.push(AskedChooser { id, request, cause: Some(why), dialog: None });
                                continue;
                            }
                            None => Answer::Unavailable(why),
                        }
                    }
                    answer => answer,
                };
                if let Some(question) = self.finish(id, answer, true) {
                    self.prompts.push(AskedPrompt { id, question, dialog: None });
                }
            }
        }
        self.open_choosers(dialogs);
        self.open_prompts(dialogs);
    }

    /// Opens the next waiting chooser when none is open. Its folder source lists the file system
    /// on threads and repaints the app when a listing is in.
    #[cfg(feature = "ui")]
    fn open_choosers(&mut self, dialogs: &mut craft_ui::dialog::DialogHost) {
        if dialogs.get::<FileChooser>().is_some() {
            return;
        }
        let Some(asked) = self.choosers.iter_mut().find(|a| a.dialog.is_none()) else { return };
        let last = lock(&self.inner).last_folder.get(&asked.request.purpose).cloned();
        let mut request = asked.request.clone();
        request.directory = start_folder(&request, last.as_deref());
        let folders = FolderCache::new(Arc::new(SystemDirectories), Arc::new(ThreadJobs), self.wake.clone());
        asked.dialog = Some(dialogs.open(FileChooser::new(request, asked.cause.clone(), folders)));
    }

    /// Settles the picks whose chooser has finished: OK with its answer, anything else as
    /// `Cancelled`, the same for a chooser closed from outside. A pick forgotten before its
    /// chooser opened is dropped; one forgotten while it is open is dropped by `finish`.
    #[cfg(feature = "ui")]
    fn collect_chooser_answers(&mut self, dialogs: &mut craft_ui::dialog::DialogHost) {
        use craft_ui::dialog::DialogResult;
        for asked in std::mem::take(&mut self.choosers) {
            let answer = match asked.dialog {
                None => {
                    // No chooser is on screen for a forgotten pick, so nothing will answer it.
                    let mut inner = lock(&self.inner);
                    if inner.wanted(&asked.id) {
                        self.choosers.push(asked);
                    } else {
                        inner.pending.remove(&asked.id);
                    }
                    continue;
                }
                Some(dialog) => match dialogs.take_by_id::<FileChooser>(dialog) {
                    Some(DialogResult::Ok(answer)) => answer,
                    Some(_) => Answer::Cancelled,
                    None if dialogs.get_by_id::<FileChooser>(dialog).is_some() => {
                        // A forgotten pick has no one to answer: its chooser closes, as its
                        // native panel does (`cancel_forgotten`).
                        if !lock(&self.inner).wanted(&asked.id) {
                            dialogs.close_by_id(dialog);
                        }
                        self.choosers.push(asked);
                        continue;
                    }
                    None => Answer::Cancelled,
                },
            };
            if let Some(question) = self.finish(asked.id, answer, false) {
                self.prompts.push(AskedPrompt { id: asked.id, question, dialog: None });
            }
        }
    }

    /// The dialog ids of the Replace questions open on the app's dialog host, for an app that lists
    /// its open prompts to automation (`ui.inspect`) and answers them by id
    /// (`DialogHost::confirm_by_id`).
    #[cfg(feature = "ui")]
    pub fn open_questions(&self) -> impl Iterator<Item = craft_ui::dialog::DialogId> + '_ {
        self.prompts.iter().filter_map(|asked| asked.dialog)
    }

    /// Opens the replace questions the host has not accepted yet, one at a time: a prompt that is
    /// open (ours or the app's) keeps the input.
    #[cfg(feature = "ui")]
    fn open_prompts(&mut self, dialogs: &mut craft_ui::dialog::DialogHost) {
        use craft_ui::dialog::StandardPrompt;
        for asked in self.prompts.iter_mut().filter(|a| a.dialog.is_none()) {
            let name = |p: &Path| p.file_name().map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy()).into_owned();
            let prompt = StandardPrompt::replace_file(name(&asked.question.settled), Some(name(&asked.question.answered)))
                .instance(("craft-dialogs.replace", asked.id.0));
            match dialogs.open_unless_open(prompt) {
                Ok(dialog) => asked.dialog = Some(dialog),
                Err(_) => break,
            }
        }
    }

    /// Settles the picks whose replace prompt has finished. A prompt that is neither open nor
    /// holding a result was closed from outside (the app cleared its dialogs): the pick ends as
    /// `Cancelled`.
    #[cfg(feature = "ui")]
    fn collect_prompt_answers(&mut self, dialogs: &mut craft_ui::dialog::DialogHost) {
        use craft_ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
        for asked in std::mem::take(&mut self.prompts) {
            let replace = match asked.dialog {
                None => {
                    self.prompts.push(asked);
                    continue;
                }
                Some(dialog) => match dialogs.take_by_id::<StandardPrompt>(dialog) {
                    Some(DialogResult::Ok(answer)) => answer.choice == PromptChoice::Proceed,
                    Some(_) => false,
                    None if dialogs.get_by_id::<StandardPrompt>(dialog).is_some() => {
                        self.prompts.push(asked);
                        continue;
                    }
                    None => false,
                },
            };
            self.settle_replace(asked.id, replace);
        }
    }

    /// Drains the requests the service queued into the native panels' queue. A request that finds
    /// a forced answer waiting takes it instead of a panel; those answers are returned for the
    /// caller to settle.
    fn start_requests(&mut self) -> Vec<(PickId, Answer)> {
        let mut forced = Vec::new();
        while let Ok((id, request)) = self.requests.try_recv() {
            match self.forced_answer(id) {
                Some(answer) => forced.push((id, answer)),
                None => self.ask_panel(id, &request),
            }
        }
        forced
    }

    /// The oldest answer [`Files::answer_next`] gave, taken for pick `id`. A ticket forgotten
    /// before it got here takes none, which leaves the answer for the next request.
    fn forced_answer(&self, id: PickId) -> Option<Answer> {
        let mut inner = lock(&self.inner);
        if !inner.pending.contains_key(&id) {
            return None;
        }
        inner.forced.pop_front()
    }

    /// Queues the native panel of pick `id`.
    fn ask_panel(&mut self, id: PickId, request: &PickRequest) {
        let folder = lock(&self.inner).last_folder.get(&request.purpose).cloned();
        let dialog = self.file_dialog(request, folder.as_deref());
        self.dialogs.ask(dialog, id);
    }

    /// Closes the open panel of a pick that was forgotten (a closed document's portal Save).
    fn cancel_forgotten(&mut self) {
        let inner = &self.inner;
        self.dialogs.cancel_open_if(|id| !lock(inner).wanted(id));
    }

    /// Translates a request into the panel builder: kind, title, filters, `request.directory`
    /// (else the remembered folder, else `request.default_directory`) and the suggested name with
    /// its extension.
    // pc: apps/photocraft/src/services.rs:192 dialog_file_name, :203 show_file_dialog
    fn file_dialog(&self, request: &PickRequest, last_folder: Option<&Path>) -> FileDialog {
        let mut dialog = FileDialog::new(match request.kind {
            PickKind::Open => Kind::Open { multiple: false },
            PickKind::OpenMany => Kind::Open { multiple: true },
            PickKind::Folder => Kind::Folder,
            PickKind::Save => Kind::Save,
        });
        dialog.title = request.title.clone();
        dialog.filters = request.filters.iter().map(|f| Filter { name: f.name.clone(), extensions: f.extensions.clone() }).collect();
        dialog.directory = start_folder(request, last_folder);
        if request.kind == PickKind::Save {
            let adds_extension = cfg!(windows) && !request.filters.is_empty();
            dialog.file_name = request.file_name.as_deref().and_then(|name| dialog_file_name(name, adds_extension));
        }
        dialog
    }

    /// The saved path after the extension policy. First a name the panel left alone takes the
    /// extension of the picked type (`file_type`, an index into `request.filters`; when
    /// `picked_type`). Then `"Photo"` becomes `"Photo.png"` with a
    /// suggested `png` (when `add_suggested`); `"Photo.PNG"` becomes `"Photo.png"` (when
    /// `lowercase`) unless a file named `"Photo.PNG"` already exists; a path with another known
    /// extension is kept.
    ///
    /// An existing file keeps the spelling the panel answered: the panel has just asked whether to
    /// replace that entry, and `"PHOTO.png"` next to `"PHOTO.PNG"` on a file system that tells
    /// letter case apart is a second file, so the one the user picked would be left unchanged.
    // pc: crates/ui-egui/src/file_dialog.rs:210 lowercased_extension, :365 saved_path
    pub fn settled_save_path(policy: ExtensionPolicy, answered: &Path, request: &PickRequest, file_type: Option<usize>) -> PathBuf {
        let typed = if policy.picked_type {
            let chosen = chosen_type(&request.filters, file_type).and_then(|at| request.filters.get(at)).map_or(&[][..], |f| f.extensions.as_slice());
            with_chosen_type(answered, request.file_name.as_deref(), chosen, &request.filters)
        } else {
            answered.to_path_buf()
        };
        let suggested = request.suggested_extension.as_deref();
        match typed.extension().and_then(|e| e.to_str()) {
            Some(ext) if policy.lowercase && ext.bytes().any(|b| b.is_ascii_uppercase()) && !typed.exists() => {
                typed.with_extension(ext.to_ascii_lowercase())
            }
            Some(_) => typed,
            None => match suggested {
                Some(ext) if policy.add_suggested => typed.with_extension(ext),
                _ => typed,
            },
        }
    }

    /// Whether the settled path names an existing file the panel did not ask about: the panel
    /// asked when the settled path equals the answered path, so only an extension change (the
    /// policy's or the picked type's) can make a new collision.
    // pc: crates/ui-egui/src/file_dialog.rs:193 replaces_unasked
    pub fn replaces_unasked(answered: &Path, settled: &Path) -> bool {
        answered != settled && settled.exists() && !same_entry(answered, settled)
    }

    /// The replace question as a native message box with Replace (default) and Cancel.
    // pc: crates/ui-egui/src/dialogs/replace_prompt.rs (wording); shown natively here so it sits above the sheet
    fn replace_message(&self, settled: &Path) -> Message {
        let name = settled.file_name().map_or_else(|| settled.to_string_lossy(), |n| n.to_string_lossy());
        let words = &self.replace_words;
        Message {
            level: MessageLevel::Warning,
            title: words.title.clone(),
            description: craft_i18n::fmt(&words.question, &[("name", &name)]),
            buttons: MessageButtons::OkCancelCustom(words.replace.clone(), words.cancel.clone()),
        }
    }

    /// Turns a panel answer into the pick's answer: `Cancelled` stays, `Saved` goes through
    /// the policy and guard, `Unavailable` stays `Unavailable` with the cause, and a `Cancelled` on a
    /// system where the probe found no dialog backend becomes `Unavailable` with the probe's
    /// cause. A Save that needs the replace
    /// question stays pending and returns the question for the caller to ask. The answer of a
    /// forgotten pick is dropped. `from_panel` says a native panel answered, not the fallback
    /// chooser, whose Cancel is always the user's, nor a forced answer ([`Files::answer_next`]),
    /// whose Cancel is the caller's.
    fn finish(&mut self, id: PickId, answer: Answer, from_panel: bool) -> Option<ReplaceQuestion> {
        let Pending { request, forgotten, awaiting_replace, bytes } = lock(&self.inner).pending.remove(&id)?;
        if forgotten {
            return None;
        }
        let picked = match (answer, awaiting_replace) {
            (Answer::Message(result), Some(question)) => {
                let replace =
                    matches!(&result, MessageResult::Ok) || matches!(&result, MessageResult::Custom(label) if *label == self.replace_words.replace);
                if replace { Picked::Saved { path: question.settled, file_type: question.file_type } } else { Picked::Cancelled }
            }
            (Answer::Saved { path, filter }, _) => {
                let settled = Self::settled_save_path(self.extension_policy, &path, &request, filter);
                if self.replace_guard == ReplaceGuard::AskWhenUnasked && Self::replaces_unasked(&path, &settled) {
                    let question = ReplaceQuestion { answered: path, settled, file_type: filter };
                    lock(&self.inner).pending.insert(id, Pending { request, forgotten: false, awaiting_replace: Some(question.clone()), bytes });
                    return Some(question);
                }
                Picked::Saved { path: settled, file_type: filter }
            }
            (Answer::Paths(paths), _) if paths.is_empty() => Picked::Cancelled,
            (Answer::Paths(mut paths), _) => {
                if request.kind == PickKind::Save && paths.len() == 1 {
                    let path = paths.remove(0);
                    let settled = Self::settled_save_path(self.extension_policy, &path, &request, None);
                    if self.replace_guard == ReplaceGuard::AskWhenUnasked && Self::replaces_unasked(&path, &settled) {
                        let question = ReplaceQuestion { answered: path, settled, file_type: None };
                        lock(&self.inner).pending.insert(id, Pending { request, forgotten: false, awaiting_replace: Some(question.clone()), bytes });
                        return Some(question);
                    }
                    Picked::Paths(vec![settled])
                } else {
                    Picked::Paths(paths)
                }
            }
            (Answer::Unavailable(why), _) => Picked::Unavailable(why),
            // A panel that answers nothing on a system without a dialog backend did not get
            // cancelled; the probe knows why.
            (Answer::Cancelled, None) if from_panel => (self.backend_problem)().map_or(Picked::Cancelled, Picked::Unavailable),
            (Answer::Cancelled | Answer::Message(_), _) => Picked::Cancelled,
        };
        self.complete(id, request, picked, bytes);
        None
    }

    /// Ends the pick whose replace question has been answered: the settled path on Replace,
    /// `Cancelled` otherwise. A forgotten pick has nothing to settle.
    #[cfg(feature = "ui")]
    fn settle_replace(&mut self, id: PickId, replace: bool) {
        let Some(Pending { request, forgotten: false, awaiting_replace: Some(question), bytes }) = lock(&self.inner).pending.remove(&id) else {
            return;
        };
        let picked = if replace { Picked::Saved { path: question.settled, file_type: question.file_type } } else { Picked::Cancelled };
        self.complete(id, request, picked, bytes);
    }

    /// Writes a save's `bytes` to the settled path (a failed write answers `Failed` with the
    /// system's message), remembers the folder of a chosen path and queues the answer.
    fn complete(&self, id: PickId, request: PickRequest, picked: Picked, bytes: Option<Vec<u8>>) {
        let picked = match (picked, bytes) {
            (Picked::Saved { path, file_type }, Some(bytes)) => match write_file(&path, &bytes) {
                Ok(()) => Picked::Saved { path, file_type },
                Err(e) => Picked::Failed(e.to_string()),
            },
            (picked, _) => picked,
        };
        let mut inner = lock(&self.inner);
        if matches!(picked, Picked::Paths(_) | Picked::Saved { .. })
            && let Some(first) = picked.first_path()
        {
            let folder = if request.kind == PickKind::Folder { Some(first.as_path()) } else { first.parent() };
            if let Some(folder) = folder.filter(|f| !f.as_os_str().is_empty()) {
                inner.last_folder.insert(request.purpose.clone(), folder.to_path_buf());
            }
        }
        inner.answered.push_back(PickAnswer { id, request, picked });
    }
}

impl FileService {
    /// Allocates the ticket of `request`, counts it pending and queues it for the panels; a save's
    /// `bytes` wait in the pending entry for the settled path.
    // new: ticket allocation and queueing on `Dialogs::ask`
    fn ask(&self, request: PickRequest, bytes: Option<Vec<u8>>) -> PickId {
        let id = {
            let mut inner = lock(&self.inner);
            inner.next += 1;
            let id = PickId(inner.next);
            inner.pending.insert(id, Pending { request: request.clone(), forgotten: false, awaiting_replace: None, bytes });
            id
        };
        // The panels gone (the app is shutting down) leaves the ticket unanswered.
        let _ = self.to_panels.send((id, request));
        self.wake.wake();
        id
    }
}

impl Files for FileService {
    fn pick(&self, request: PickRequest) -> PickId {
        self.ask(request, None)
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut inner = lock(&self.inner);
        let at = inner.answered.iter().position(|a| a.id == id)?;
        inner.answered.remove(at).map(|a| a.picked)
    }

    // A ticket still pending stays in `pending`, marked: its panel may be on screen, `finish` drops the answer it gives
    // later, and the next pump closes its portal Save or chooser (`cancel_forgotten`, `collect_chooser_answers`).
    fn forget(&self, id: PickId) {
        let mut inner = lock(&self.inner);
        if let Some(pending) = inner.pending.get_mut(&id) {
            pending.forgotten = true;
        }
        inner.answered.retain(|a| a.id != id);
    }

    // Tickets count up, so the smallest id is the oldest: the pick on screen.
    fn pending_pick(&self) -> Option<PendingPick> {
        let inner = lock(&self.inner);
        let (_, pending) = inner.pending.iter().min_by_key(|(id, _)| **id)?;
        Some(PendingPick { request: pending.request.clone(), sheet: cfg!(target_os = "macos") && pending.request.kind == PickKind::Save })
    }

    // pc: apps/photocraft/src/services.rs:359 write_atomic (read is plain `std::fs::read`)
    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        std::fs::read(path).map_err(|e| file_error(e, path))
    }

    // A desktop app has no host page: every intent writes the path, and no page took the bytes.
    fn write(&self, path: &Path, bytes: &[u8], _intent: WriteIntent) -> Result<bool, FileError> {
        write_file(path, bytes)?;
        Ok(false)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// Queues a Save pick, then writes `bytes` to the settled path and answers `Saved` or
    /// `Failed`.
    fn save(&self, request: PickRequest, bytes: Vec<u8>) -> PickId {
        self.ask(request, Some(bytes))
    }

    fn can_pick(&self) -> bool {
        true
    }

    fn browser_saves(&self) -> bool {
        false
    }

    fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).last_folder.get(purpose).cloned()
    }

    /// Answers the next pick with `picked` instead of showing a panel, through the settling a native answer passes.
    /// `Picked::Contents`, `Failed` and `Downloaded` have no `Answer` form: they are `Err(FileError::Unsupported)`.
    fn answer_next(&self, picked: Picked) -> Result<(), FileError> {
        let answer = match picked {
            Picked::Paths(paths) => Answer::Paths(paths),
            Picked::Saved { path, .. } => Answer::Saved { path, filter: None },
            Picked::Cancelled => Answer::Cancelled,
            Picked::Unavailable(why) => Answer::Unavailable(why),
            // A native service reads paths; it has no file contents to hand over, and a write's
            // outcome is its own to report.
            Picked::Contents(_) | Picked::Failed(_) | Picked::Downloaded(_) => return Err(FileError::Unsupported),
        };
        lock(&self.inner).forced.push_back(answer);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod service_tests {
    //! Behaviour tests with `Dialogs::set_override` standing in for the panels.

    use craft_ui::testing::trial;
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft_host::Picked;
    use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

    use super::*;
    use crate::Request;

    struct NoWindow;

    impl HasWindowHandle for NoWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }

    impl HasDisplayHandle for NoWindow {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }

    const NONE: Option<&NoWindow> = None;

    type Log = Rc<RefCell<Vec<Request>>>;

    /// A service whose panels answer each request with the next of `answers`, recording the requests.
    fn service_with(answers: Vec<Answer>) -> (FileService, FilePanels, Log) {
        let (service, mut panels) = FileService::new(Wake::none());
        panels.backend_problem = || None;
        let log: Log = Rc::default();
        let seen = log.clone();
        let mut answers = answers.into_iter();
        panels.dialogs.set_override(Some(Box::new(move |request| {
            seen.borrow_mut().push(request.clone());
            answers.next().unwrap_or(Answer::Cancelled)
        })));
        (service, panels, log)
    }

    fn answered(service: &FileService, id: PickId) -> Picked {
        service.take_answer(id).expect("answered")
    }

    // An answer nobody takes does not block another ticket; a forgotten ticket's answer, early or
    // late, never shows up.
    #[trial]
    fn an_orphaned_answer_blocks_no_other_pick() {
        let (service, mut panels, _) = service_with(vec![Answer::Cancelled, Answer::Paths(vec![PathBuf::from("/x/b.png")]), Answer::Cancelled]);
        let orphan = service.pick(PickRequest::open("a"));
        let wanted = service.pick(PickRequest::open("b"));
        let late = service.pick(PickRequest::open("c"));
        service.forget(late);
        panels.pump(NONE);
        assert_eq!(answered(&service, wanted), Picked::Paths(vec![PathBuf::from("/x/b.png")]));
        assert!(service.take_answer(late).is_none());
        service.forget(orphan);
        assert!(service.take_answer(orphan).is_none());
    }

    /// Picks `answer` as a Save suggesting `png`, pumps, and returns the settled answer.
    fn saved_as(service: &FileService, panels: &mut FilePanels, policy: ExtensionPolicy) -> Picked {
        panels.set_extension_policy(policy);
        let id = service.pick(PickRequest::save("export").suggested_extension("png"));
        panels.pump(NONE);
        answered(service, id)
    }

    // Lowercasing and adding the suggested extension are separate steps; another known extension
    // is kept, and a policy that does neither keeps the typed name.
    #[trial]
    fn a_saved_path_is_settled_by_the_extension_policy() {
        let typed = |name: &str| Answer::Saved { path: PathBuf::from(name), filter: None };
        let (service, mut panels, _) = service_with(vec![
            typed("/x/Photo.PNG"),
            typed("/x/Photo"),
            typed("/x/Photo.JPG"),
            typed("/x/Photo.PNG"),
            typed("/x/Photo"),
            typed("/x/Photo.PNG"),
            typed("/x/Photo"),
        ]);
        let path = |p: &str| Picked::Saved { path: PathBuf::from(p), file_type: None };
        let both = ExtensionPolicy { lowercase: true, add_suggested: true, picked_type: false };
        assert_eq!(saved_as(&service, &mut panels, both), path("/x/Photo.png"), "only the extension is lowercased");
        assert_eq!(saved_as(&service, &mut panels, both), path("/x/Photo.png"), "a bare name gets the suggested extension");
        assert_eq!(saved_as(&service, &mut panels, both), path("/x/Photo.jpg"), "another known extension is kept, lowercased");
        let neither = ExtensionPolicy::AS_TYPED;
        assert_eq!(saved_as(&service, &mut panels, neither), path("/x/Photo.PNG"));
        assert_eq!(saved_as(&service, &mut panels, neither), path("/x/Photo"));
        let add_only = ExtensionPolicy { lowercase: false, add_suggested: true, picked_type: false };
        assert_eq!(saved_as(&service, &mut panels, add_only), path("/x/Photo.PNG"), "adding does not lowercase");
        assert_eq!(saved_as(&service, &mut panels, add_only), path("/x/Photo.png"));
    }

    // A forced answer answers the next pick with no panel, through the same settling: the folder
    // is remembered and a Save gets the extension policy. The pick after it reaches the panel, and
    // a forced cancel stays a cancel on a system without a dialog backend.
    #[trial]
    fn a_forced_answer_answers_the_next_pick_without_a_panel() {
        let (service, mut panels, log) = service_with(vec![]);
        let dir = PathBuf::from("/x/forced");
        assert_eq!(service.answer_next(Picked::Paths(vec![dir.join("a.pdf")])), Ok(()));
        let forced = service.pick(PickRequest::open("document"));
        panels.pump(NONE);
        assert_eq!(answered(&service, forced), Picked::Paths(vec![dir.join("a.pdf")]));
        assert!(log.borrow().is_empty(), "no request reached a panel");
        assert!(service.pending_pick().is_none());
        assert_eq!(service.last_folder("document"), Some(dir.clone()));

        let shown = service.pick(PickRequest::open("document"));
        panels.pump(NONE);
        assert_eq!(log.borrow().len(), 1, "the next pick reaches the panel");
        assert_eq!(answered(&service, shown), Picked::Cancelled);

        assert_eq!(service.answer_next(Picked::Paths(vec![dir.join("Report")])), Ok(()));
        let saved = service.pick(PickRequest::save("export").suggested_extension("pdf"));
        panels.pump(NONE);
        assert_eq!(answered(&service, saved), Picked::Paths(vec![dir.join("Report.pdf")]), "the extension policy completes it");
        assert_eq!(log.borrow().len(), 1);

        panels.backend_problem = || Some("no dialog backend".into());
        assert_eq!(service.answer_next(Picked::Cancelled), Ok(()));
        let cancelled = service.pick(PickRequest::open("document"));
        panels.pump(NONE);
        assert_eq!(answered(&service, cancelled), Picked::Cancelled);
        assert_eq!(service.answer_next(Picked::Contents(vec![])), Err(FileError::Unsupported));
    }

    // The panel asked about the file it answered; the extension change can name another existing
    // file, and that one is asked about here. Cancel ends the pick; an answer the panel already
    // asked about is not asked again.
    #[trial]
    fn a_replace_the_panel_did_not_ask_about_is_asked_here() {
        let dir = std::env::temp_dir().join(format!("craft-dialogs-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        std::fs::write(dir.join("Photo.png"), b"old").expect("existing file");
        let typed = |name: &str| Answer::Saved { path: dir.join(name), filter: None };
        let typed_as_png = Answer::Saved { path: dir.join("Photo"), filter: Some(0) };
        let (service, mut panels, log) = service_with(vec![
            typed("Photo"),
            Answer::Message(MessageResult::Cancel),
            typed("Photo.png"),
            typed_as_png,
            Answer::Message(MessageResult::Ok),
        ]);

        let bare = service.pick(PickRequest::save("export").suggested_extension("png"));
        panels.pump(NONE);
        assert_eq!(answered(&service, bare), Picked::Cancelled, "Cancel on the question ends the pick");
        assert_eq!(log.borrow().len(), 2, "the save panel, then the question");

        let asked = service.pick(PickRequest::save("export").suggested_extension("png"));
        panels.pump(NONE);
        assert_eq!(answered(&service, asked), Picked::Saved { path: dir.join("Photo.png"), file_type: None });
        assert_eq!(log.borrow().len(), 3, "no question for a path the panel asked about");

        let by_type = service.pick(PickRequest::save("export").filter("PNG", &["png"]));
        panels.pump(NONE);
        assert_eq!(
            answered(&service, by_type),
            Picked::Saved { path: dir.join("Photo.png"), file_type: Some(0) },
            "the picked type's rename names an existing file: asked, and the type is kept"
        );
        assert_eq!(log.borrow().len(), 5, "the save panel, then the question");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A save is a ticketed pick that writes: the answer names the written file once; a target
    // that cannot be written answers `Failed`, which names no path.
    #[trial]
    fn a_save_writes_the_bytes_once_and_answers_by_ticket() {
        let dir = std::env::temp_dir().join(format!("craft-dialogs-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let written = dir.join("Notes.txt");
        let unwritable = dir.join("missing").join("Notes.txt");
        let typed = |path: &PathBuf| Answer::Saved { path: path.clone(), filter: None };
        let (service, mut panels, _) = service_with(vec![typed(&written), typed(&unwritable)]);

        let saved = service.save(PickRequest::save("export"), b"hello".to_vec());
        assert!(service.pending_pick().is_some(), "the save waits for its panel");
        panels.pump(NONE);
        assert_eq!(service.take_answer(saved), Some(Picked::Saved { path: written.clone(), file_type: None }));
        assert_eq!(std::fs::read(&written).expect("the save wrote the file"), b"hello");
        assert_eq!(service.take_answer(saved), None, "an answer is read once");
        assert!(service.pending_pick().is_none());

        let failed = service.save(PickRequest::save("export"), b"lost".to_vec());
        panels.pump(NONE);
        let picked = answered(&service, failed);
        assert!(matches!(picked, Picked::Failed(_)), "{picked:?}");
        assert_eq!(picked.first_path(), None);
        assert!(!unwritable.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A closed document forgets its save: the portal Save on screen is cancelled, and the answer it
    // gives anyway is dropped without a write.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    #[trial]
    fn forgetting_a_pending_save_cancels_its_panel() {
        let late = std::env::temp_dir().join(format!("craft-dialogs-forgotten-{}.txt", std::process::id()));
        let (service, mut panels) = FileService::new(Wake::none());
        panels.backend_problem = || None;
        let cancel = crate::portal::Cancel::default();
        let shown = cancel.clone();
        let replies: Rc<RefCell<Vec<crate::Reply>>> = Rc::default();
        let kept = replies.clone();
        panels.dialogs.set_custom(move |_, reply| {
            kept.borrow_mut().push(reply);
            crate::native::Shown { cancel: Some(shown.clone()) }
        });

        let id = service.save(PickRequest::save("export"), b"late".to_vec());
        panels.pump(NONE);
        assert!(!cancel.is_cancelled(), "a save still wanted stays open");
        service.forget(id);
        panels.pump(NONE);
        assert!(cancel.is_cancelled(), "the forgotten save's panel is cancelled");

        let reply = replies.borrow_mut().remove(0);
        reply.send(Answer::Saved { path: late.clone(), filter: None });
        panels.pump(NONE);
        assert!(service.take_answer(id).is_none(), "the late answer stays dropped");
        assert!(!late.exists(), "nothing is written for a forgotten save");
    }

    // A closed document forgets its save: the fallback chooser on screen closes with the next
    // pass of the host, and nothing is answered for the ticket.
    #[cfg(feature = "ui")]
    #[trial]
    fn forgetting_a_pending_save_closes_its_chooser() {
        use craft_ui::testing::Fixture;
        let (service, mut panels) = FileService::scripted(Wake::none(), []);
        let host = Rc::new(RefCell::new(craft_ui::dialog::DialogHost::new()));
        let shown = host.clone();
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        let id = service.save(PickRequest::save("export").fallback(true), b"late".to_vec());
        panels.pump_with(&mut host.borrow_mut(), NONE);
        assert!(host.borrow().is_open::<FileChooser>(), "the chooser opens for the request");
        service.forget(id);
        panels.pump_with(&mut host.borrow_mut(), NONE);
        fixture.idle(2);
        panels.pump_with(&mut host.borrow_mut(), NONE);
        assert!(!host.borrow().is_open::<FileChooser>(), "the forgotten save's chooser is closed");
        assert!(service.take_answer(id).is_none());
    }

    // Saving over an existing `PHOTO.PNG` with Lowercase Extension on writes `PHOTO.PNG`, the
    // file the panel asked about, not a new `PHOTO.png` beside it (on a file system that tells
    // letter case apart) or a renamed one (on one that does not). A new name is still lowercased.
    #[trial]
    fn an_existing_file_keeps_its_extension_spelling() {
        let dir = std::env::temp_dir().join(format!("craft-dialogs-existing-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        std::fs::write(dir.join("PHOTO.PNG"), b"old").expect("existing file");
        let typed = |name: &str| Answer::Saved { path: dir.join(name), filter: None };
        let (service, mut panels, log) = service_with(vec![typed("PHOTO.PNG"), typed("Other.PNG")]);
        let lowercase = ExtensionPolicy::default();
        let saved = |name: &str| Picked::Saved { path: dir.join(name), file_type: None };

        assert_eq!(saved_as(&service, &mut panels, lowercase), saved("PHOTO.PNG"), "the existing file is replaced");
        assert_eq!(log.borrow().len(), 1, "no second question: the panel asked about this file");
        assert_eq!(saved_as(&service, &mut panels, lowercase), saved("Other.png"), "a new file is lowercased");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The rename rule: the picked type decides a bare name's extension, and a typed extension wins
    // unless it is the one the panel opened with.
    #[trial]
    fn the_picked_file_type_gives_a_bare_name_its_extension_and_a_typed_extension_wins() {
        let two = [FileFilter::new("Photoshop", &["psd", "psb"]), FileFilter::new("PNG", &["png"])];
        let only = [FileFilter::new("Swatches", &["aco"])];
        // (path, opened with, filters, picked type, expected path)
        type Case<'a> = (&'a str, Option<&'a str>, &'a [FileFilter], Option<usize>, &'a str);
        let table: [Case; 8] = [
            ("/s/Untitled", Some("Untitled.psd"), &two, Some(1), "/s/Untitled.png"),
            ("/s/Untitled.psd", Some("Untitled.psd"), &two, Some(1), "/s/Untitled.png"),
            ("/s/art.png", Some("art.psd"), &two, Some(0), "/s/art.png"),
            ("/s/art.PSB", Some("art.psd"), &two, Some(0), "/s/art.PSB"),
            ("/s/scan.2024", None, &two, Some(1), "/s/scan.2024.png"),
            ("/s/Swatches", None, &only, None, "/s/Swatches.aco"),
            ("/s/Swatches.aco", None, &only, None, "/s/Swatches.aco"),
            ("/s/free", None, &two, None, "/s/free"),
        ];
        for (path, opened, filters, picked, expected) in table {
            let chosen: &[String] = match chosen_type(filters, picked) {
                Some(at) => &filters[at].extensions,
                None => &[],
            };
            assert_eq!(with_chosen_type(Path::new(path), opened, chosen, filters), PathBuf::from(expected), "{path} with type {picked:?}");
        }
    }

    // A panel that reports the picked type gets its extension under the default policy; AS_TYPED keeps
    // the typed name, and the type is reported either way.
    #[trial]
    fn a_save_answer_reports_the_picked_type_and_as_typed_keeps_the_typed_name() {
        let answer = || Answer::Saved { path: PathBuf::from("/x/Photo"), filter: Some(1) };
        let (service, mut panels, _) = service_with(vec![answer(), answer()]);
        let request = || PickRequest::save("export").filter("Photoshop", &["psd"]).filter("PNG", &["png"]);
        let mut save = |policy: ExtensionPolicy| {
            panels.set_extension_policy(policy);
            let id = service.pick(request());
            panels.pump(NONE);
            answered(&service, id)
        };
        assert_eq!(save(ExtensionPolicy::default()), Picked::Saved { path: PathBuf::from("/x/Photo.png"), file_type: Some(1) });
        assert_eq!(save(ExtensionPolicy::AS_TYPED), Picked::Saved { path: PathBuf::from("/x/Photo"), file_type: Some(1) });
    }

    // The close gate reads `pending_pick`: the oldest ticket (a forgotten one while its panel may
    // still show), and still `Some` while the Replace question is open.
    #[trial]
    fn the_pending_pick_is_the_oldest_until_answered_and_holds_through_the_replace_question() {
        let (service, mut panels, _) = service_with(vec![]);
        assert!(service.pending_pick().is_none(), "nothing asked");
        let first = service.pick(PickRequest::open("a"));
        let second = service.pick(PickRequest::save("b"));
        let oldest = service.pending_pick().expect("a pick is pending");
        assert_eq!((oldest.request.purpose.as_str(), oldest.sheet), ("a", false), "the oldest ticket, an Open pick: no sheet");
        service.forget(first);
        assert_eq!(service.pending_pick().expect("still pending").request.purpose, "a", "a forgotten pick holds until its panel answers");
        panels.pump(NONE);
        assert!(service.pending_pick().is_none(), "both panels answered");
        assert!(service.take_answer(first).is_none(), "the forgotten answer is dropped");
        assert_eq!(answered(&service, second), Picked::Cancelled);

        let dir = std::env::temp_dir().join(format!("craft-dialogs-pending-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        std::fs::write(dir.join("Photo.png"), b"old").expect("existing file");
        let (service, mut panels) = FileService::new(Wake::none());
        panels.backend_problem = || None;
        let service = Rc::new(service);
        let during: Rc<RefCell<Option<Option<PendingPick>>>> = Rc::default();
        let (reader, slot) = (service.clone(), during.clone());
        let mut files = vec![Answer::Saved { path: dir.join("Photo"), filter: None }].into_iter();
        panels.dialogs.set_override(Some(Box::new(move |request| match request {
            Request::Message(_) => {
                *slot.borrow_mut() = Some(reader.pending_pick());
                Answer::Message(MessageResult::Cancel)
            }
            Request::File(_) => files.next().unwrap_or(Answer::Cancelled),
        })));
        let id = service.pick(PickRequest::save("export").suggested_extension("png"));
        panels.pump(NONE);
        let asked = during.borrow().clone().expect("the Replace question was asked").expect("the pick is still pending during it");
        assert_eq!((asked.request.kind, asked.sheet), (PickKind::Save, cfg!(target_os = "macos")), "a Save is a sheet on macOS only");
        assert!(service.pending_pick().is_none(), "answered");
        assert_eq!(answered(&service, id), Picked::Cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // `FileService::exists` checks the real file system, so batch writers on desktop see files already on disk.
    #[trial]
    fn exists_checks_the_file_system() {
        let dir = std::env::temp_dir().join(format!("craft-dialogs-exists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let present = dir.join("present.pdf");
        let missing = dir.join("missing.pdf");
        let _ = std::fs::remove_file(&present);
        let _ = std::fs::remove_file(&missing);
        let (service, _panels) = FileService::new(Wake::none());
        assert!(!service.exists(&present));
        assert_eq!(service.write(&present, b"data", WriteIntent::SaveAs), Ok(false));
        assert!(service.exists(&present));
        assert!(!service.exists(&missing));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

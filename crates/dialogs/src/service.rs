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
//! prompt on the app's `DialogHost` from `FilePanels::pump_with`; a typed name without an extension gets the suggested one and an uppercase
//! extension is lowercased (`file_dialog.rs:210 lowercased_extension`, `:365 saved_path`); the two
//! steps are separate settings of the [`ExtensionPolicy`].

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use craft_host::{FileError, Files, PickAnswer, PickId, PickRequest, Wake};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use craft_host::{PickKind, Picked};

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
    /// Lowercase an extension typed with capitals (`Photo.PNG` becomes `Photo.png`).
    pub lowercase: bool,
    /// Add the request's suggested extension to a name that has none.
    pub add_suggested: bool,
}

impl ExtensionPolicy {
    /// Keep the typed name as the panel answered it.
    pub const AS_TYPED: Self = Self { lowercase: false, add_suggested: false };
}

impl Default for ExtensionPolicy {
    /// Lowercase, and add the suggested extension.
    fn default() -> Self {
        Self { lowercase: true, add_suggested: true }
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
    /// The Save path waiting on the replace question's answer.
    awaiting_replace: Option<ReplaceQuestion>,
}

/// A Save answer that collides with an existing file after the extension policy: the path the
/// panel answered and the path it settled to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplaceQuestion {
    answered: PathBuf,
    settled: PathBuf,
}

/// A replace question shown on the app's `DialogHost`: the pick it settles and the prompt's id,
/// once the host has accepted the prompt.
#[cfg(feature = "ui")]
struct AskedPrompt {
    id: PickId,
    question: ReplaceQuestion,
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
}

struct Inner {
    next: u64,
    pending: HashMap<PickId, Pending>,
    answered: VecDeque<PickAnswer>,
    last_folder: HashMap<String, PathBuf>,
    /// A panel is on screen or queued (mirrored from the panels each pump).
    panel_busy: bool,
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
            panel_busy: false,
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
        self.start_requests();
        // A replace question queued by `finish` shows in the same pass, so the loop runs until no
        // answer is left to settle.
        loop {
            self.dialogs.show_pending(parent);
            let answers = self.dialogs.take_answers();
            if answers.is_empty() {
                break;
            }
            for (id, answer) in answers {
                if let Some(question) = self.finish(id, answer) {
                    self.dialogs.ask_message(self.replace_message(&question.settled), id);
                }
            }
        }
        self.mark_busy();
    }

    /// [`pump`](FilePanels::pump) with the replace question on the app's own dialog host: the
    /// question is a [`StandardPrompt::replace_file`](craft_ui::dialog::StandardPrompt::replace_file)
    /// the app's `DialogHost::show` draws, so `ui.dialog.confirm` and `ui.dialog.cancel` answer it
    /// like every prompt, and the answer is read on a later pump. A prompt of the app's own that is
    /// open makes the question wait its turn. The question is a prompt instance
    /// ([`StandardPrompt::instance`](craft_ui::dialog::StandardPrompt::instance)), so the app's
    /// `take::<StandardPrompt>()` and its own prompts leave its answer alone.
    // pc: crates/ui-egui/src/dialogs/replace_prompt.rs:23 open
    #[cfg(feature = "ui")]
    pub fn pump_with<W: HasWindowHandle + HasDisplayHandle>(&mut self, dialogs: &mut craft_ui::dialog::DialogHost, parent: Option<&W>) {
        self.collect_prompt_answers(dialogs);
        self.start_requests();
        loop {
            self.dialogs.show_pending(parent);
            let answers = self.dialogs.take_answers();
            if answers.is_empty() {
                break;
            }
            for (id, answer) in answers {
                if let Some(question) = self.finish(id, answer) {
                    self.prompts.push(AskedPrompt { id, question, dialog: None });
                }
            }
        }
        self.open_prompts(dialogs);
        self.mark_busy();
    }

    /// The dialog ids of the Replace questions open on the app's dialog host, for an app that lists
    /// its open prompts to automation (`ui.inspect`) and answers them by id
    /// (`DialogHost::confirm_by_id::<StandardPrompt>`).
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

    /// Drains the requests the service queued into the native panels' queue.
    fn start_requests(&mut self) {
        while let Ok((id, request)) = self.requests.try_recv() {
            let folder = lock(&self.inner).last_folder.get(&request.purpose).cloned();
            let dialog = self.file_dialog(&request, folder.as_deref());
            self.dialogs.ask(dialog, id);
        }
    }

    fn mark_busy(&self) {
        lock(&self.inner).panel_busy = self.dialogs.is_open() || self.dialogs.queued() > 0;
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
        dialog.directory = request.directory.clone().or_else(|| last_folder.map(Path::to_path_buf)).or_else(|| request.default_directory.clone());
        if request.kind == PickKind::Save {
            let adds_extension = cfg!(windows) && !request.filters.is_empty();
            dialog.file_name = request.file_name.as_deref().and_then(|name| dialog_file_name(name, adds_extension));
        }
        dialog
    }

    /// The saved path after the extension policy: `"Photo"` becomes `"Photo.png"` with a
    /// suggested `png` (when `add_suggested`); `"Photo.PNG"` becomes `"Photo.png"` (when
    /// `lowercase`); a path with another known extension is kept.
    // pc: crates/ui-egui/src/file_dialog.rs:210 lowercased_extension, :365 saved_path
    pub fn settled_save_path(policy: ExtensionPolicy, answered: &Path, suggested: Option<&str>) -> PathBuf {
        match answered.extension().and_then(|e| e.to_str()) {
            Some(ext) if policy.lowercase && ext.bytes().any(|b| b.is_ascii_uppercase()) => answered.with_extension(ext.to_ascii_lowercase()),
            Some(_) => answered.to_path_buf(),
            None => match suggested {
                Some(ext) if policy.add_suggested => answered.with_extension(ext),
                _ => answered.to_path_buf(),
            },
        }
    }

    /// Whether the settled path names an existing file the panel did not ask about: the panel
    /// asked when the settled path equals the answered path, so only an extension change can
    /// make a new collision.
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

    /// Turns a panel answer into the pick's answer: `Cancelled` stays, `Paths` for Save go through
    /// the policy and guard, `Unavailable` stays `Unavailable` with the cause, and a `Cancelled` on a
    /// system where the probe found no dialog backend becomes `Unavailable` with the probe's
    /// cause. A Save that needs the replace
    /// question stays pending and returns the question for the caller to ask.
    fn finish(&mut self, id: PickId, answer: Answer) -> Option<ReplaceQuestion> {
        let pending = lock(&self.inner).pending.remove(&id)?;
        let picked = match (answer, pending.awaiting_replace) {
            (Answer::Message(result), Some(question)) => {
                let replace =
                    matches!(&result, MessageResult::Ok) || matches!(&result, MessageResult::Custom(label) if *label == self.replace_words.replace);
                if replace { Picked::Paths(vec![question.settled]) } else { Picked::Cancelled }
            }
            (Answer::Paths(paths), _) if paths.is_empty() => Picked::Cancelled,
            (Answer::Paths(mut paths), _) => {
                if pending.request.kind == PickKind::Save {
                    let answered = paths.swap_remove(0);
                    let settled = Self::settled_save_path(self.extension_policy, &answered, pending.request.suggested_extension.as_deref());
                    if self.replace_guard == ReplaceGuard::AskWhenUnasked && Self::replaces_unasked(&answered, &settled) {
                        let question = ReplaceQuestion { answered, settled };
                        lock(&self.inner).pending.insert(id, Pending { request: pending.request, awaiting_replace: Some(question.clone()) });
                        return Some(question);
                    }
                    paths = vec![settled];
                }
                Picked::Paths(paths)
            }
            (Answer::Unavailable(why), _) => Picked::Unavailable(why),
            // A panel that answers nothing on a system without a dialog backend did not get
            // cancelled; the probe knows why.
            (Answer::Cancelled, None) => (self.backend_problem)().map_or(Picked::Cancelled, Picked::Unavailable),
            (Answer::Cancelled | Answer::Message(_), _) => Picked::Cancelled,
        };
        self.complete(id, pending.request, picked);
        None
    }

    /// Ends the pick whose replace question has been answered: the settled path on Replace,
    /// `Cancelled` otherwise. A forgotten pick has nothing to settle.
    #[cfg(feature = "ui")]
    fn settle_replace(&mut self, id: PickId, replace: bool) {
        let Some(Pending { request, awaiting_replace: Some(question) }) = lock(&self.inner).pending.remove(&id) else { return };
        self.complete(id, request, if replace { Picked::Paths(vec![question.settled]) } else { Picked::Cancelled });
    }

    /// Remembers the folder of a chosen path and queues the answer.
    fn complete(&self, id: PickId, request: PickRequest, picked: Picked) {
        let mut inner = lock(&self.inner);
        if let Picked::Paths(paths) = &picked
            && let Some(first) = paths.first()
        {
            let folder = if request.kind == PickKind::Folder { Some(first.as_path()) } else { first.parent() };
            if let Some(folder) = folder.filter(|f| !f.as_os_str().is_empty()) {
                inner.last_folder.insert(request.purpose.clone(), folder.to_path_buf());
            }
        }
        inner.answered.push_back(PickAnswer { id, request, picked });
    }
}

impl Files for FileService {
    // new: ticket allocation and queueing on `Dialogs::ask`
    fn pick(&self, request: PickRequest) -> PickId {
        let id = {
            let mut inner = lock(&self.inner);
            inner.next += 1;
            let id = PickId(inner.next);
            inner.pending.insert(id, Pending { request: request.clone(), awaiting_replace: None });
            id
        };
        // The panels gone (the app is shutting down) leaves the ticket unanswered.
        let _ = self.to_panels.send((id, request));
        self.wake.wake();
        id
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut inner = lock(&self.inner);
        let at = inner.answered.iter().position(|a| a.id == id)?;
        inner.answered.remove(at).map(|a| a.picked)
    }

    // A ticket still pending leaves `pending`, so the answer the panel gives later is dropped by `finish`.
    fn forget(&self, id: PickId) {
        let mut inner = lock(&self.inner);
        inner.pending.remove(&id);
        inner.answered.retain(|a| a.id != id);
    }

    fn pick_pending(&self) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.panel_busy || !inner.pending.is_empty()
    }

    // pc: apps/photocraft/src/services.rs:359 write_atomic (read is plain `std::fs::read`)
    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        std::fs::read(path).map_err(|e| file_error(e, path))
    }

    // pc: apps/photocraft/src/services.rs:359 write_atomic
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError> {
        craft_atomic::atomic_write(path, bytes).map_err(|e| file_error(e, path))
    }

    fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).last_folder.get(purpose).cloned()
    }
}

#[cfg(test)]
mod service_tests {
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
}

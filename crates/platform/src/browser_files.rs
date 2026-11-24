//! The browser's file service: picks and saves through the page.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use craft_host::{FileError, Files, PendingPick, PickId, PickKind, PickRequest, Picked, Wake, WriteIntent};
use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::browser::{DownloadOptions, download_bytes};

/// How much a browser pick may hand over.
#[derive(Clone, Copy, Debug)]
pub struct PickLimits {
    /// The most files one pick answers.
    pub files: usize,
    /// The most bytes of picked files held before they are taken.
    pub pending_bytes: usize,
}

impl PickLimits {
    /// The limits of production's web build: 2000 files and 256 MiB.
    pub const DEFAULT: PickLimits = PickLimits { files: 2000, pending_bytes: 256 << 20 };
}

/// How long a fallback download's object URL lives; the download keeps its own reference.
const REVOKE_AFTER_MS: i32 = 10_000;

/// Browser picks and saves. [`Files::can_pick`] answers false and [`Files::browser_saves`] true. The save picker
/// is reached through `js_sys::Reflect`, and a save it refuses falls back to one download.
pub struct BrowserFiles {
    state: Arc<Mutex<State>>,
    wake: Wake,
    limits: PickLimits,
    page_save: Option<PageSave>,
}

/// An embedding page's save ([`BrowserFiles::page_saves`]): given the file name, the bytes and whether the user asked
/// for Save As, `None` when no page hosts the app, else the page's answer.
pub type PageSave = Arc<PageSaveFn>;

/// The callback behind a [`PageSave`].
pub type PageSaveFn = dyn Fn(&str, &[u8], bool) -> Option<Result<(), String>> + Send + Sync;

/// Tickets of one service, shared with the tasks that answer them.
#[derive(Default)]
struct State {
    next: u64,
    /// Picks and saves whose task has not finished, forgotten or not: a save already writing
    /// still holds the page. Oldest first.
    running: Vec<(PickId, PickRequest)>,
    /// Running tickets whose owner gave them up; their answer is dropped when it comes.
    forgotten: Vec<PickId>,
    /// Answers not yet taken.
    answered: Vec<(PickId, Picked)>,
}

impl State {
    fn issue(&mut self) -> PickId {
        self.next = self.next.wrapping_add(1);
        PickId(self.next)
    }

    /// Records the answer of `id`, unless its owner forgot it.
    fn finish(&mut self, id: PickId, picked: Picked) {
        self.running.retain(|(r, _)| *r != id);
        if let Some(at) = self.forgotten.iter().position(|f| *f == id) {
            self.forgotten.swap_remove(at);
            return;
        }
        self.answered.push((id, picked));
    }

    /// Bytes of picked files answered and not yet taken.
    fn held_bytes(&self) -> usize {
        self.answered
            .iter()
            .filter_map(|(_, picked)| match picked {
                Picked::Contents(files) => Some(files.iter().map(|(_, bytes)| bytes.len()).fold(0, usize::saturating_add)),
                _ => None,
            })
            .fold(0, usize::saturating_add)
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Answers ticket `id` from a task and wakes the app to take it.
fn finish(state: &Mutex<State>, wake: &Wake, id: PickId, picked: Picked) {
    lock(state).finish(id, picked);
    wake.wake();
}

impl BrowserFiles {
    /// A service whose late answers call `wake`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self { state: Arc::new(Mutex::new(State::default())), wake: Wake::new(wake), limits: PickLimits::DEFAULT, page_save: None }
    }

    /// The service whose [`Files::write`] goes to the page embedding the app (a host that saves the document back
    /// itself, as a file manager's viewer does) whenever `save` answers; otherwise, and with no `save`, a write
    /// downloads the bytes.
    pub fn page_saves(self, save: impl Fn(&str, &[u8], bool) -> Option<Result<(), String>> + Send + Sync + 'static) -> Self {
        Self { page_save: Some(Arc::new(save)), ..self }
    }

    /// The service with `limits` in place of [`PickLimits::DEFAULT`].
    pub fn limits(self, limits: PickLimits) -> Self {
        Self { limits, ..self }
    }

    /// A ticket for `request` whose task is about to start.
    fn start(&self, request: PickRequest) -> PickId {
        let mut state = lock(&self.state);
        let id = state.issue();
        state.running.push((id, request));
        id
    }

    /// A ticket answered in the call itself.
    fn answer_now(&self, picked: Picked) -> PickId {
        let id = {
            let mut state = lock(&self.state);
            let id = state.issue();
            state.finish(id, picked);
            id
        };
        self.wake.wake();
        id
    }
}

/// The MIME type a picked file of `name` is read as.
fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("ott") => "application/vnd.oasis.opendocument.text-template",
        Some("fodt") => "application/vnd.oasis.opendocument.text-flat-xml",
        Some("rtf") => "application/rtf",
        Some("html") => "text/html",
        Some("md" | "txt") => "text/plain",
        Some("tex") => "application/x-tex",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}

/// What a rejected save picker answers: a cancel for `AbortError` (`error_name`), otherwise the `download` fallback's outcome.
/// A browser can expose the picker and still reject it (a cross-origin frame); cancelling must never start a
/// download (#1246).
pub(crate) fn rejected(error_name: &str, name: &str, download: impl FnOnce() -> Result<(), String>) -> Picked {
    if error_name == "AbortError" {
        return Picked::Cancelled;
    }
    downloaded(name, download())
}

/// The answer of a download of `name` that `result` says started or failed.
fn downloaded(name: &str, result: Result<(), String>) -> Picked {
    match result {
        Ok(()) => Picked::Downloaded(name.to_owned()),
        Err(error) => Picked::Failed(error),
    }
}

/// One download of `bytes` as `name`.
/// The name a write of `path` goes out under: its last part without control characters, `document` when that is empty.
fn write_name(path: &Path) -> String {
    let base: String = path.file_name().map(|n| n.to_string_lossy().chars().filter(|c| !c.is_control()).collect()).unwrap_or_default();
    let base = base.trim();
    if base.is_empty() { "document".to_owned() } else { base.to_owned() }
}

/// A write's route: the embedding page when `page` answers (`Ok(true)`), else `download` (`Ok(false)`).
pub(crate) fn page_or_download(
    page: Option<&PageSaveFn>,
    name: &str,
    bytes: &[u8],
    save_as: bool,
    download: impl FnOnce() -> Result<(), String>,
) -> Result<bool, FileError> {
    match page.and_then(|save| save(name, bytes, save_as)) {
        Some(answer) => answer.map(|()| true).map_err(FileError::Other),
        None => download().map(|()| false).map_err(FileError::Other),
    }
}

fn download(name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
    download_bytes(name, mime, bytes, DownloadOptions { revoke_after_ms: REVOKE_AFTER_MS }).map_err(|e| js_text(&e))
}

fn js_text(value: &JsValue) -> String {
    format!("{value:?}")
}

/// The JS error's `name` (`AbortError`), empty when it has none.
fn error_name(error: &JsValue) -> String {
    Reflect::get(error, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

/// The name a save goes out under: the last component of the request's name, `document` when blank.
fn save_name(request: &PickRequest) -> String {
    request
        .file_name
        .as_deref()
        .and_then(|n| Path::new(n).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "document".into())
}

/// The save picker's name for files of `ext`: the format's name, as its extension says it.
fn type_description(ext: &str) -> String {
    match ext {
        "htm" => "HTML".into(),
        "txt" => "Text".into(),
        e if !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()) => e.to_ascii_uppercase(),
        _ => "Document".into(),
    }
}

fn set(target: &JsValue, key: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(target, &JsValue::from_str(key), value).map(|_| ())
}

/// Opens `showSaveFilePicker` for `name`. `Ok(None)` when the browser has no such picker; an `Err` is the value the
/// call threw, whose name tells a cancel from a refusal. It must run in the call of the user's gesture: the picker
/// needs the page's transient activation.
fn open_save_picker(name: &str, ext: Option<&str>, mime: &str) -> Result<Option<Promise>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no browser window"))?;
    let Ok(picker) = Reflect::get(&window, &JsValue::from_str("showSaveFilePicker")) else { return Ok(None) };
    let Ok(picker) = picker.dyn_into::<Function>() else { return Ok(None) };
    let options = Object::new();
    set(&options, "suggestedName", &JsValue::from_str(name))?;
    if let Some(ext) = ext {
        let accept = Object::new();
        set(&accept, mime, &Array::of1(&JsValue::from_str(&format!(".{ext}"))))?;
        let kind = Object::new();
        set(&kind, "description", &JsValue::from_str(&type_description(ext)))?;
        set(&kind, "accept", &accept)?;
        set(&options, "types", &Array::of1(&kind))?;
    }
    let promise = picker.call1(&window, &options)?;
    promise.dyn_into::<Promise>().map(Some).map_err(|_| JsValue::from_str("showSaveFilePicker answered no promise"))
}

/// Calls `target.method(arg)` and awaits the promise it answers.
async fn await_call(target: &JsValue, method: &str, arg: Option<&JsValue>) -> Result<JsValue, String> {
    let function: Function =
        Reflect::get(target, &JsValue::from_str(method)).map_err(|e| js_text(&e))?.dyn_into().map_err(|_| format!("{method} is not a function"))?;
    let value = match arg {
        Some(arg) => function.call1(target, arg),
        None => function.call0(target),
    }
    .map_err(|e| js_text(&e))?;
    let promise: Promise = value.dyn_into().map_err(|_| format!("{method} answered no promise"))?;
    JsFuture::from(promise).await.map_err(|e| js_text(&e))
}

/// Writes `bytes` through a picked file handle; the file counts as saved only once it is closed.
async fn write_handle(handle: &JsValue, bytes: &[u8]) -> Result<(), String> {
    let stream = await_call(handle, "createWritable", None).await?;
    let data: JsValue = js_sys::Uint8Array::from(bytes).into();
    await_call(&stream, "write", Some(&data)).await?;
    await_call(&stream, "close", None).await?;
    Ok(())
}

impl Files for BrowserFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let many = match request.kind {
            PickKind::Open => false,
            PickKind::OpenMany => true,
            PickKind::Folder => return self.answer_now(Picked::Unavailable("the browser picks no folders".into())),
            PickKind::Save => return self.answer_now(Picked::Unavailable("the browser saves through Files::save".into())),
        };
        let mut dialog = rfd::AsyncFileDialog::new();
        if let Some(title) = &request.title {
            dialog = dialog.set_title(title.as_str());
        }
        for filter in &request.filters {
            dialog = dialog.add_filter(filter.name.as_str(), &filter.extensions);
        }
        let id = self.start(request);
        let (state, wake, limits) = (Arc::clone(&self.state), self.wake.clone(), self.limits);
        spawn_local(async move {
            let handles = if many { dialog.pick_files().await.unwrap_or_default() } else { dialog.pick_file().await.into_iter().collect() };
            let mut files = Vec::new();
            let mut this_pick = 0usize;
            for handle in handles.into_iter().take(limits.files) {
                let mut bytes = handle.read().await;
                // Several files waiting to be taken stay under a byte cap. A file over it answers empty, so
                // the feature reports that file on its own.
                if many {
                    let held = lock(&state).held_bytes().saturating_add(this_pick);
                    if held.saturating_add(bytes.len()) > limits.pending_bytes {
                        bytes = Vec::new();
                    }
                }
                this_pick = this_pick.saturating_add(bytes.len());
                files.push((handle.file_name(), bytes));
            }
            let picked = if files.is_empty() { Picked::Cancelled } else { Picked::Contents(files) };
            finish(&state, &wake, id, picked);
        });
        id
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut state = lock(&self.state);
        let at = state.answered.iter().position(|(a, _)| *a == id)?;
        Some(state.answered.remove(at).1)
    }

    fn forget(&self, id: PickId) {
        let mut state = lock(&self.state);
        state.answered.retain(|(a, _)| *a != id);
        if state.running.iter().any(|(r, _)| *r == id) && !state.forgotten.contains(&id) {
            state.forgotten.push(id);
        }
    }

    // The browser's pickers are never sheets: closing the page drops them.
    fn pending_pick(&self) -> Option<PendingPick> {
        lock(&self.state).running.first().map(|(_, request)| PendingPick { request: request.clone(), sheet: false })
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }

    /// `Ok(true)` when the embedding page saved the bytes, `Ok(false)` when they were downloaded.
    fn write(&self, path: &Path, bytes: &[u8], intent: WriteIntent) -> Result<bool, FileError> {
        let name = write_name(path);
        page_or_download(self.page_save.as_deref(), &name, bytes, intent == WriteIntent::SaveAs, || download(&name, mime_for(&name), bytes))
    }

    fn last_folder(&self, _purpose: &str) -> Option<PathBuf> {
        None
    }

    fn save(&self, request: PickRequest, bytes: Vec<u8>) -> PickId {
        let name = save_name(&request);
        let mime = mime_for(&name);
        let ext = Path::new(&name)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .or_else(|| request.suggested_extension.clone());
        let promise = match open_save_picker(&name, ext.as_deref(), mime) {
            Ok(Some(promise)) => promise,
            // No picker: the browser's download is the save.
            Ok(None) => return self.answer_now(downloaded(&name, download(&name, mime, &bytes))),
            // A picker that threw in the call is refused like one that rejected later.
            Err(error) => return self.answer_now(rejected(&error_name(&error), &name, || download(&name, mime, &bytes))),
        };
        let id = self.start(request);
        let (state, wake) = (Arc::clone(&self.state), self.wake.clone());
        spawn_local(async move {
            let picked = match JsFuture::from(promise).await {
                // A failed write or close is the save's failure; it never downloads.
                Ok(handle) => match write_handle(&handle, &bytes).await {
                    Ok(()) => Picked::Saved { path: PathBuf::from(&name), file_type: None },
                    Err(error) => Picked::Failed(error),
                },
                Err(error) => rejected(&error_name(&error), &name, || download(&name, mime, &bytes)),
            };
            finish(&state, &wake, id, picked);
        });
        id
    }

    fn can_pick(&self) -> bool {
        false
    }

    fn browser_saves(&self) -> bool {
        true
    }
}

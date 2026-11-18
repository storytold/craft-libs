//! The file service of a web build: pickers through a file input, saves through the host page or a download.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft_host::{FileError, Files, PickId, PickKind, PickRequest, Picked, Wake, WriteIntent};
use craft_platform::browser::{DownloadOptions, download_bytes};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys;

/// Answers by ticket: `None` while the browser's picker is open or its files are being read.
type Answers = Arc<Mutex<HashMap<PickId, Option<Picked>>>>;

/// The [`Files`] service of a web build. Open and OpenMany go through a file input to
/// [`Picked::Contents`] and a finished read wakes the app; Save answers the name at once and
/// `write` posts to the host page or downloads; Folder is unavailable.
pub struct BrowserFiles {
    wake: Wake,
    max_read: u64,
    next: AtomicU64,
    answers: Answers,
}

impl BrowserFiles {
    /// A service that wakes the app through `wake` when a picked file has been read. A picked file
    /// longer than `max_read` bytes is not copied in: its pick answers [`Picked::Unavailable`].
    pub fn new(wake: Wake, max_read: u64) -> Self {
        Self { wake, max_read, next: AtomicU64::new(0), answers: Arc::default() }
    }
}

impl Files for BrowserFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let id = PickId(self.next.fetch_add(1, Ordering::Relaxed) + 1);
        lock(&self.answers).insert(id, None);
        let now = match request.kind {
            PickKind::Save => Some(Picked::Paths(vec![PathBuf::from(save_name(&request))])),
            PickKind::Folder => Some(Picked::Unavailable("a browser has no folder picker".into())),
            PickKind::Open | PickKind::OpenMany => match open_input(&request) {
                Ok((input, settled)) => {
                    wasm_bindgen_futures::spawn_local(read_picked(input, settled, self.max_read, self.answers.clone(), self.wake.clone(), id));
                    None
                }
                Err(error) => Some(Picked::Unavailable(error)),
            },
        };
        if let Some(picked) = now {
            answer(&self.answers, &self.wake, id, picked);
        }
        id
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let mut answers = lock(&self.answers);
        if !matches!(answers.get(&id), Some(Some(_))) {
            return None;
        }
        answers.remove(&id).flatten()
    }

    fn forget(&self, id: PickId) {
        lock(&self.answers).remove(&id);
    }

    fn pick_pending(&self) -> bool {
        lock(&self.answers).values().any(Option::is_none)
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }

    fn write(&self, path: &Path, bytes: &[u8], intent: WriteIntent) -> Result<bool, FileError> {
        let name = crate::file_name(path.file_name().map(|n| n.to_string_lossy()).as_deref(), "document");
        if let Some(host) = crate::web::connected() {
            return host.post_save(&name, bytes, None, intent == WriteIntent::SaveAs).map(|()| true).map_err(FileError::Other);
        }
        download_bytes(&name, mime_type(&name), bytes, DownloadOptions::default()).map(|()| false).map_err(|e| FileError::Other(js_error(e)))
    }

    fn last_folder(&self, _purpose: &str) -> Option<PathBuf> {
        None
    }
}

fn lock(answers: &Mutex<HashMap<PickId, Option<Picked>>>) -> MutexGuard<'_, HashMap<PickId, Option<Picked>>> {
    answers.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stores `picked` for `id` and wakes the app to take it; a ticket forgotten meanwhile gets nothing.
fn answer(answers: &Mutex<HashMap<PickId, Option<Picked>>>, wake: &Wake, id: PickId, picked: Picked) {
    {
        let mut answers = lock(answers);
        let Some(slot) = answers.get_mut(&id) else { return };
        *slot = Some(picked);
    }
    wake.wake();
}

/// The name a Save pick answers: the suggested name, cleaned, with the suggested extension when it has none.
fn save_name(request: &PickRequest) -> String {
    let name = crate::file_name(request.file_name.as_deref(), "document");
    match &request.suggested_extension {
        Some(extension) if !extension.is_empty() && Path::new(&name).extension().is_none() => format!("{name}.{extension}"),
        _ => name,
    }
}

/// A hidden file input for `request`, already clicked, and the promise its `change` or `cancel`
/// event settles: the browser fires `cancel` at a file input when the user closes its picker
/// without choosing (HTML, "show the picker"), so a cancelled pick ends too.
fn open_input(request: &PickRequest) -> Result<(web_sys::HtmlInputElement, js_sys::Promise), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("the page has no document")?;
    let input: web_sys::HtmlInputElement =
        document.create_element("input").map_err(js_error)?.dyn_into().map_err(|_| "the element is not an input".to_string())?;
    input.set_type("file");
    input.set_multiple(request.kind == PickKind::OpenMany);
    let accept: Vec<String> = request.filters.iter().flat_map(|f| &f.extensions).map(|e| format!(".{e}")).collect();
    if !accept.is_empty() {
        input.set_accept(&accept.join(","));
    }
    input.set_hidden(true);
    // Some browsers show no picker for an input outside the document.
    let body = document.body().ok_or("the page has no body")?;
    body.append_child(&input).map_err(js_error)?;
    let settled = js_sys::Promise::new(&mut |resolve, _reject| {
        input.set_onchange(Some(&resolve));
        input.set_oncancel(Some(&resolve));
    });
    input.click();
    Ok((input, settled))
}

/// Waits for the picker to close, reads what was chosen, and answers ticket `id`.
async fn read_picked(input: web_sys::HtmlInputElement, settled: js_sys::Promise, max_read: u64, answers: Answers, wake: Wake, id: PickId) {
    let picked = match JsFuture::from(settled).await {
        Ok(_) => read_files(&input, max_read).await,
        Err(error) => Picked::Unavailable(js_error(error)),
    };
    input.remove();
    answer(&answers, &wake, id, picked);
}

/// The chosen files' names and contents; nothing chosen is `Cancelled`.
async fn read_files(input: &web_sys::HtmlInputElement, max_read: u64) -> Picked {
    let Some(list) = input.files() else { return Picked::Cancelled };
    let mut files = Vec::new();
    for at in 0..list.length() {
        let Some(file) = list.get(at) else { continue };
        let name = crate::file_name(Some(&file.name()), "document");
        // Checked before the copy: the file is untrusted and its bytes would come into memory whole.
        if file.size() > max_read as f64 {
            return Picked::Unavailable(format!("{name} is larger than {max_read} bytes"));
        }
        match JsFuture::from(file.array_buffer()).await {
            Ok(buffer) => files.push((name, js_sys::Uint8Array::new(&buffer).to_vec())),
            Err(error) => return Picked::Unavailable(js_error(error)),
        }
    }
    if files.is_empty() { Picked::Cancelled } else { Picked::Contents(files) }
}

fn js_error(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

/// The MIME type a download of `name` is offered as, from its extension.
fn mime_type(name: &str) -> &'static str {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "csv" => "text/csv",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "json" => "application/json",
        "xfdf" => "application/vnd.adobe.xfdf",
        "fdf" => "application/vnd.fdf",
        "rtf" => "application/rtf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

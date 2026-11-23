//! The file service of a web build: a deprecated wrapper over the platform browser service whose writes go to the host page or a download.

use std::path::{Path, PathBuf};

use craft_host::{FileError, Files, PickId, PickRequest, Picked, Wake, WriteIntent};
use craft_platform::browser::{DownloadOptions, download_bytes};
use craft_platform::browser_files::{BrowserFiles as PlatformFiles, PickLimits};
use wasm_bindgen::JsValue;

/// The [`Files`] service of a web build, kept for apps written against it: a thin wrapper over
/// [`craft_platform::browser_files::BrowserFiles`], which holds the picks and saves (one store).
/// Only `write` differs: it posts to the hosting page when one is connected, and downloads
/// otherwise.
#[deprecated(note = "use craft_platform::browser_files::BrowserFiles; host-page saves go through Files::save")]
pub struct BrowserFiles {
    inner: PlatformFiles,
}

#[allow(deprecated, reason = "the deprecated type's own impl")]
impl BrowserFiles {
    /// A service that wakes the app through `wake` when a pick has been answered. Picked files
    /// waiting to be taken stay under `max_read` bytes.
    pub fn new(wake: Wake, max_read: u64) -> Self {
        let limits = PickLimits { pending_bytes: usize::try_from(max_read).unwrap_or(usize::MAX), ..PickLimits::DEFAULT };
        Self { inner: PlatformFiles::new(move || wake.wake()).limits(limits) }
    }
}

#[allow(deprecated, reason = "the deprecated type's own impl")]
impl Files for BrowserFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        self.inner.pick(request)
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        self.inner.take_answer(id)
    }

    fn forget(&self, id: PickId) {
        self.inner.forget(id);
    }

    fn pick_pending(&self) -> bool {
        self.inner.pick_pending()
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        self.inner.read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8], intent: WriteIntent) -> Result<bool, FileError> {
        let name = crate::file_name(path.file_name().map(|n| n.to_string_lossy()).as_deref(), "document");
        if let Some(host) = crate::web::connected() {
            return host.post_save(&name, bytes, None, intent == WriteIntent::SaveAs).map(|()| true).map_err(FileError::Other);
        }
        download_bytes(&name, mime_type(&name), bytes, DownloadOptions::default()).map(|()| false).map_err(|e| FileError::Other(js_error(e)))
    }

    fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
        self.inner.last_folder(purpose)
    }

    fn save(&self, request: PickRequest, bytes: Vec<u8>) -> PickId {
        self.inner.save(request, bytes)
    }

    fn can_pick(&self) -> bool {
        self.inner.can_pick()
    }

    fn browser_saves(&self) -> bool {
        self.inner.browser_saves()
    }
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

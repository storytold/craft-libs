//! The browser's file service: picks and saves through the page.

use std::path::{Path, PathBuf};

use craft_host::{FileError, Files, PickId, PickRequest, Picked};

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

/// Browser picks and saves. [`Files::can_pick`] answers false and [`Files::browser_saves`] true. The save picker
/// is reached through `js_sys::Reflect`, and a save it refuses falls back to one download.
pub struct BrowserFiles {
    _limits: PickLimits,
}

impl BrowserFiles {
    /// A service whose late answers call `wake`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let _ = wake;
        todo!("pf.C-F6")
    }

    /// The service with `limits` in place of [`PickLimits::DEFAULT`].
    pub fn limits(self, limits: PickLimits) -> Self {
        let _ = (&self, limits);
        todo!("pf.C-F6")
    }
}

/// The MIME type a picked file of `name` is read as.
#[allow(dead_code)] // P3(platform-w5): pf.C-F6
fn mime_for(name: &str) -> &'static str {
    let _ = name;
    todo!("pf.C-F6")
}

/// What a rejected save picker answers: a cancel for `AbortError` (`error_name`), otherwise the `download` fallback's outcome.
#[allow(dead_code)] // P3(platform-w5): pf.C-F6
pub(crate) fn rejected(error_name: &str, download: impl FnOnce() -> Result<(), String>) -> Picked {
    let _ = (error_name, download);
    todo!("pf.C-F6")
}

impl Files for BrowserFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let _ = request;
        todo!("pf.C-F6")
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let _ = id;
        todo!("pf.C-F6")
    }

    fn forget(&self, id: PickId) {
        let _ = id;
        todo!("pf.C-F6")
    }

    fn pick_pending(&self) -> bool {
        todo!("pf.C-F6")
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        let _ = path;
        todo!("pf.C-F6")
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError> {
        let _ = (path, bytes);
        todo!("pf.C-F6")
    }

    fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
        let _ = purpose;
        todo!("pf.C-F6")
    }

    fn save(&self, request: PickRequest, bytes: Vec<u8>) -> PickId {
        let _ = (request, bytes);
        todo!("pf.C-F6")
    }

    fn can_pick(&self) -> bool {
        false
    }

    fn browser_saves(&self) -> bool {
        true
    }
}

//! The file service of a web build: a deprecated wrapper over the platform browser service whose writes go to the host page or a download.

use std::path::{Path, PathBuf};

use craft_host::{FileError, Files, PendingPick, PickId, PickRequest, Picked, Wake, WriteIntent};
use craft_platform::browser_files::{BrowserFiles as PlatformFiles, PickLimits};

/// The hosting page's save, for [`PlatformFiles::page_saves`]: posts `bytes` to the page `Host::connect` connected
/// (`None` when no page hosts the app). `craft_platform::browser_files::BrowserFiles::new(wake).page_saves(page_save)`
/// is a web build's file service whose writes go back to the embedding page.
pub fn page_save(name: &str, bytes: &[u8], save_as: bool) -> Option<Result<(), String>> {
    crate::web::connected().map(|host| host.post_save(name, bytes, None, save_as))
}

/// The [`Files`] service of a web build, kept for apps written against it: a thin wrapper over
/// [`craft_platform::browser_files::BrowserFiles`], which holds the picks and saves (one store).
/// Its writes go to the hosting page when one is connected ([`PlatformFiles::page_saves`]), and
/// download otherwise.
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
        let inner = PlatformFiles::new(move || wake.wake()).limits(limits).page_saves(page_save);
        Self { inner }
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

    fn pending_pick(&self) -> Option<PendingPick> {
        self.inner.pending_pick()
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        self.inner.read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8], intent: WriteIntent) -> Result<bool, FileError> {
        self.inner.write(path, bytes, intent)
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

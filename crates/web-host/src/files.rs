//! The file service of a web build: pickers through a file input, saves through the host page.

use std::path::{Path, PathBuf};

use craft_host::{FileError, Files, PickId, PickRequest, Picked, Wake};

/// The [`Files`] service of a web build. Open and OpenMany go through a file input to
/// [`Picked::Contents`] and a finished read wakes the app; Save answers the name at once and
/// `write` posts to the host page or downloads; Folder is unavailable.
pub struct BrowserFiles {
    #[allow(dead_code)] // P3(pdf-shell-w5): SH-C35
    wake: Wake,
}

impl BrowserFiles {
    /// A service that wakes the app through `wake` when a picked file has been read.
    pub fn new(wake: Wake) -> Self {
        let _ = wake;
        todo!("SH-C35")
    }
}

impl Files for BrowserFiles {
    fn pick(&self, request: PickRequest) -> PickId {
        let _ = request;
        todo!("SH-C35")
    }

    fn take_answer(&self, id: PickId) -> Option<Picked> {
        let _ = id;
        todo!("SH-C35")
    }

    fn forget(&self, id: PickId) {
        let _ = id;
        todo!("SH-C35")
    }

    fn pick_pending(&self) -> bool {
        todo!("SH-C35")
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
        let _ = path;
        todo!("SH-C35")
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError> {
        let _ = (path, bytes);
        todo!("SH-C35")
    }

    fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
        let _ = purpose;
        todo!("SH-C35")
    }
}

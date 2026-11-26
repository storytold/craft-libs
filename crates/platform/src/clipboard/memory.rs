//! An in-memory clipboard.

use std::path::PathBuf;

use super::backend::ClipboardBackend;
use super::convert::{parse_uri_list, rgba8_image};
use super::types::{BITMAP, ClipboardError, ClipboardImage, Flavor, RGBA8, URI_LIST};

/// The public in-memory clipboard for tests and hosts without one. It enforces no limits; the
/// [`Clipboard`](super::Clipboard) over it does.
#[derive(Default)]
pub struct MemoryClipboard {
    held: Vec<Flavor>,
}

impl MemoryClipboard {
    /// The held flavour that answers `mime`; [`BITMAP`] is any held `image/*`.
    fn find(&self, mime: &str) -> Option<&Flavor> {
        self.held.iter().find(|f| if mime == BITMAP { f.mime.starts_with("image/") } else { f.mime == mime })
    }
}

impl ClipboardBackend for MemoryClipboard {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        self.held = flavors.to_vec();
        Ok(0)
    }

    fn read(&mut self, mimes: &[&str], _limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        Ok(mimes.iter().find_map(|mime| self.find(mime)).cloned())
    }

    fn read_image(&mut self, _limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        Ok(self.find(RGBA8).and_then(|flavor| rgba8_image(&flavor.data)))
    }

    /// A held text/uri-list, one file:// URI per line, read as the event clipboard reads an offered one.
    fn read_files(&mut self, _limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let files = self.find(URI_LIST).map(|flavor| parse_uri_list(&String::from_utf8_lossy(&flavor.data)));
        Ok(files.filter(|files| !files.is_empty()))
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        mimes.iter().any(|mime| self.find(mime).is_some())
    }

    fn formats(&mut self) -> Vec<String> {
        self.held.iter().map(|flavor| flavor.mime.to_string()).collect()
    }
}

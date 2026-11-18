//! An in-memory clipboard.

use std::path::PathBuf;

use super::backend::ClipboardBackend;
use super::types::{ClipboardError, ClipboardImage, Flavor};

/// The public in-memory clipboard for tests and hosts without one.
#[derive(Default)]
pub struct MemoryClipboard {
    held: Vec<Flavor>,
}

impl ClipboardBackend for MemoryClipboard {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        let _ = (&self.held, flavors);
        todo!("pf.C-P10")
    }

    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        let _ = (mimes, limit);
        todo!("pf.C-P10")
    }

    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let _ = limit;
        todo!("pf.C-P10")
    }

    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let _ = limit;
        todo!("pf.C-P10")
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        let _ = mimes;
        todo!("pf.C-P10")
    }
}

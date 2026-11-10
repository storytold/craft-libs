//! Read-only reader for the subset of HDF5 that the reference app writes.
//!
//! Supported: superblock v0/v1, v1 object headers (with continuations), old-style groups (symbol tables),
//! datasets with compact, contiguous or chunked (v1 B-tree, 1-D, unfiltered) layout. Anything else is
//! reported as [`Hdf5Error::Unsupported`].

mod btree;
mod chunked;
mod dataset;
mod error;
mod group;
mod layout;
mod local_heap;
mod message;
mod namespace;
mod object;
mod object_header;
mod sizes;
mod source;
mod storage;
mod superblock;
mod symbol_table;

pub use error::Hdf5Error;
pub use source::{MemorySource, ReadAt};

use object::Object;
use sizes::Address;
#[cfg(not(target_arch = "wasm32"))]
use source::FileSource;
#[cfg(not(target_arch = "wasm32"))]
use std::fs::File;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use storage::Storage;
use superblock::Superblock;

/// An open HDF5 file. Reads happen lazily, so opening a 1 GB file is cheap.
pub struct Hdf5File {
    storage: Storage,
    root: Address,
}

/// One object in the file, with its absolute path (e.g. `/paint/document.bin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Group,
    /// A one-dimensional byte dataset of `len` bytes.
    Dataset {
        len: u64,
    },
}

impl Hdf5File {
    /// Opens a file on disk (not on wasm); it is read lazily, by positioned reads.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: &Path) -> Result<Self, Hdf5Error> {
        Self::from_source(Box::new(FileSource::new(File::open(path)?)?))
    }

    /// A whole file already in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Hdf5Error> {
        Self::from_source(Box::new(MemorySource::new(bytes)))
    }

    /// A file served by any random-access source.
    pub fn from_source(source: Box<dyn ReadAt>) -> Result<Self, Hdf5Error> {
        let superblock = Superblock::read(source.as_ref())?;
        let storage = Storage::new(source, superblock.base, superblock.sizes);
        Ok(Hdf5File { storage, root: superblock.root })
    }

    /// Every group and dataset, depth first, children in name order. The root group itself is not listed.
    pub fn entries(&self) -> Result<Vec<Entry>, Hdf5Error> {
        namespace::list(&self.storage, self.root)
    }

    /// The whole content of a byte dataset.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, Hdf5Error> {
        self.read_up_to(path, u64::MAX)
    }

    /// The first `max_len` bytes of a byte dataset (fewer if it is shorter). Used to read headers of large blobs.
    pub fn read_prefix(&self, path: &str, max_len: usize) -> Result<Vec<u8>, Hdf5Error> {
        self.read_up_to(path, max_len as u64)
    }

    fn read_up_to(&self, path: &str, max_len: u64) -> Result<Vec<u8>, Hdf5Error> {
        match namespace::resolve(&self.storage, self.root, path)? {
            Object::Dataset(dataset) => dataset.read(&self.storage, max_len),
            Object::Group(_) => Err(Hdf5Error::NotADataset(path.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn hdf5_file_is_send_and_sync() {
        assert_send_sync::<Hdf5File>();
    }

    #[test]
    fn garbage_is_not_hdf5() {
        let source = source::MemorySource(b"definitely not an HDF5 file".to_vec());
        assert!(matches!(Hdf5File::from_source(Box::new(source)), Err(Hdf5Error::NotHdf5)));
    }
}

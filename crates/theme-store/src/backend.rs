//! Where the store's files live: the suite folder, or memory.

use std::{collections::BTreeMap, io, path::PathBuf, sync::Arc, time::Duration};

#[cfg(not(target_arch = "wasm32"))]
use crate::disk::Disk;

/// What changes when a file is replaced: length, modification time and, on Unix, the inode
/// (every atomic write is a new file). In memory, a counter bumped on every write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileStamp {
    pub(crate) len: u64,
    pub(crate) modified_nanos: Option<u128>,
    pub(crate) identity: u64,
}

impl FileStamp {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A `library/*.json` file: the name before `.json` and its stamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryFile {
    pub stem: String,
    pub stamp: FileStamp,
}

// Only the disk backend fails these ways; web builds have none.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) enum WriteError {
    /// `selection.json` changed between the read under the lock and the rename: another
    /// writer ignored the lock (network file systems).
    Raced,
    Io(io::Error),
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) enum LockError {
    Busy,
    Io(io::Error),
}

/// Held for the duration of a write; the OS releases the lock when the file closes.
pub(crate) struct Lock {
    #[cfg(not(target_arch = "wasm32"))]
    _file: Option<std::fs::File>,
}

impl Lock {
    fn none() -> Self {
        Lock {
            #[cfg(not(target_arch = "wasm32"))]
            _file: None,
        }
    }
}

pub(crate) enum Backend {
    Memory(Memory),
    #[cfg(not(target_arch = "wasm32"))]
    Disk(Disk),
}

#[derive(Default)]
pub(crate) struct Memory {
    selection: Option<(Arc<[u8]>, u64)>,
    library: BTreeMap<String, (Arc<[u8]>, u64)>,
    /// Themes a delete in progress has set aside.
    hidden: BTreeMap<String, (Arc<[u8]>, u64)>,
    serial: u64,
}

impl Memory {
    fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }
}

fn capped(bytes: &[u8], cap: usize) -> Vec<u8> {
    bytes.get(..cap.saturating_add(1)).unwrap_or(bytes).to_vec()
}

fn mem_stamp(bytes: &[u8], serial: u64) -> FileStamp {
    FileStamp { len: bytes.len() as u64, modified_nanos: None, identity: serial }
}

impl Backend {
    pub fn dir(&self) -> Option<PathBuf> {
        match self {
            Backend::Memory(_) => None,
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => Some(d.dir.clone()),
        }
    }

    pub fn lock(&self, timeout: Duration) -> Result<Lock, LockError> {
        match self {
            Backend::Memory(_) => {
                let _ = timeout;
                Ok(Lock::none())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.lock(timeout).map(|f| Lock { _file: Some(f) }),
        }
    }

    /// At most `cap + 1` bytes, so the caller can tell an oversized file.
    pub fn read_selection(&self, cap: usize) -> io::Result<Option<Vec<u8>>> {
        match self {
            Backend::Memory(m) => Ok(m.selection.as_ref().map(|(b, _)| capped(b, cap))),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.read_capped(&d.selection_path(), cap),
        }
    }

    pub fn selection_stamp(&self) -> io::Result<Option<FileStamp>> {
        match self {
            Backend::Memory(m) => Ok(m.selection.as_ref().map(|(b, s)| mem_stamp(b, *s))),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.stamp(&d.selection_path()),
        }
    }

    pub fn write_selection(&mut self, bytes: &[u8], expected: Option<&[u8]>) -> Result<(), WriteError> {
        match self {
            Backend::Memory(m) => {
                let serial = m.next();
                m.selection = Some((Arc::from(bytes), serial));
                let _ = expected;
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.write_selection(bytes, expected),
        }
    }

    /// Renames a damaged `selection.json` to `selection.broken-<unix-ts>.json`.
    pub fn move_aside_selection(&mut self) -> io::Result<Option<PathBuf>> {
        match self {
            Backend::Memory(m) => {
                m.selection = None;
                Ok(None)
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.move_aside_selection().map(Some),
        }
    }

    /// Puts back a damaged `selection.json` moved aside by a write that then failed.
    pub fn restore_selection(&mut self, moved: &std::path::Path) -> io::Result<()> {
        match self {
            Backend::Memory(_) => {
                let _ = moved;
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.restore_selection(moved),
        }
    }

    pub fn list(&self) -> io::Result<Vec<LibraryFile>> {
        match self {
            Backend::Memory(m) => Ok(m.library.iter().map(|(stem, (b, s))| LibraryFile { stem: stem.clone(), stamp: mem_stamp(b, *s) }).collect()),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.list(),
        }
    }

    pub fn read_theme(&self, stem: &str, cap: usize) -> io::Result<Option<Vec<u8>>> {
        match self {
            Backend::Memory(m) => Ok(m.library.get(stem).map(|(b, _)| capped(b, cap))),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.read_capped(&d.theme_path(stem), cap),
        }
    }

    pub fn theme_stamp(&self, stem: &str) -> io::Result<Option<FileStamp>> {
        match self {
            Backend::Memory(m) => Ok(m.library.get(stem).map(|(b, s)| mem_stamp(b, *s))),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.stamp(&d.theme_path(stem)),
        }
    }

    /// Fails with `AlreadyExists` rather than replace a file.
    pub fn create_theme(&mut self, stem: &str, bytes: &[u8]) -> io::Result<()> {
        match self {
            Backend::Memory(m) => {
                if m.library.contains_key(stem) {
                    return Err(io::ErrorKind::AlreadyExists.into());
                }
                let serial = m.next();
                m.library.insert(stem.to_owned(), (Arc::from(bytes), serial));
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.create_theme(stem, bytes),
        }
    }

    /// Renames a theme to a hidden name that listings skip; `NotFound` when there is none.
    pub fn hide_theme(&mut self, stem: &str) -> io::Result<()> {
        match self {
            Backend::Memory(m) => {
                let entry = m.library.remove(stem).ok_or(io::ErrorKind::NotFound)?;
                m.hidden.insert(stem.to_owned(), entry);
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.hide_theme(stem),
        }
    }

    pub fn unhide_theme(&mut self, stem: &str) -> io::Result<()> {
        match self {
            Backend::Memory(m) => {
                let entry = m.hidden.remove(stem).ok_or(io::ErrorKind::NotFound)?;
                m.library.insert(stem.to_owned(), entry);
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.unhide_theme(stem),
        }
    }

    pub fn purge_hidden_theme(&mut self, stem: &str) -> io::Result<()> {
        match self {
            Backend::Memory(m) => {
                m.hidden.remove(stem);
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.purge_hidden_theme(stem),
        }
    }

    pub fn remove_theme(&mut self, stem: &str) -> io::Result<()> {
        match self {
            Backend::Memory(m) => match m.library.remove(stem) {
                Some(_) => Ok(()),
                None => Err(io::ErrorKind::NotFound.into()),
            },
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Disk(d) => d.remove_theme(stem),
        }
    }
}

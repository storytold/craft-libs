//! Pooled file [`ByteSource`]s: many clips can be open at once without holding a descriptor each.
//!
//! Past [`MAX_OPEN_FILES`] the least recently read file is closed and reopened by path on demand,
//! so a project with hundreds of sources does not hit the process file-descriptor limit (often 256
//! on macOS, 1024 on Linux).

#![cfg(not(target_arch = "wasm32"))]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::source::ByteSource;

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of byte source")
}

/// Open `path` for reading without blocking on a FIFO that was swapped in after the metadata check
/// (`O_NONBLOCK`; it changes nothing for a regular file, and the callers check the handle's type).
/// `O_NONBLOCK` has no std constant (no libc dependency here): it is 0o4000 on most Linux targets,
/// 0x80 on Linux MIPS, 0x4000 on Linux SPARC and 4 on the BSD family, and the open stays a plain
/// one on any other target.
fn open_nonblocking(path: &Path) -> io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(all(
            any(target_os = "linux", target_os = "android"),
            any(target_arch = "mips", target_arch = "mips32r6", target_arch = "mips64", target_arch = "mips64r6")
        ))]
        o.custom_flags(0x80);
        #[cfg(all(any(target_os = "linux", target_os = "android"), any(target_arch = "sparc", target_arch = "sparc64")))]
        o.custom_flags(0x4000);
        #[cfg(all(
            any(target_os = "linux", target_os = "android"),
            not(any(
                target_arch = "mips",
                target_arch = "mips32r6",
                target_arch = "mips64",
                target_arch = "mips64r6",
                target_arch = "sparc",
                target_arch = "sparc64"
            ))
        ))]
        o.custom_flags(0o4000);
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly"
        ))]
        o.custom_flags(0x4);
    }
    o.open(path)
}

fn not_a_file(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} is not a file", path.display()))
}

/// Files [`FileReader`]s keep open at most, together. Past this many, the least recently read file
/// is closed and its reader reopens it on demand.
pub const MAX_OPEN_FILES: usize = 128;

/// The open files of a set of [`FileReader`]s, least recently used first.
#[derive(Debug)]
pub(crate) struct FilePool {
    cap: usize,
    state: std::sync::Mutex<PoolState>,
}

#[derive(Debug, Default)]
struct PoolState {
    next_id: u64,
    open: std::collections::VecDeque<(u64, Arc<std::fs::File>)>,
}

impl FilePool {
    pub(crate) fn new(cap: usize) -> Self {
        Self { cap: cap.max(1), state: Default::default() }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn next_id(&self) -> u64 {
        let mut s = self.state();
        s.next_id = s.next_id.wrapping_add(1);
        s.next_id
    }

    /// The open file of reader `id`, now the most recently used.
    fn get(&self, id: u64) -> Option<Arc<std::fs::File>> {
        let mut s = self.state();
        let at = s.open.iter().position(|(i, _)| *i == id)?;
        let entry = s.open.remove(at)?;
        let file = entry.1.clone();
        s.open.push_back(entry);
        Some(file)
    }

    /// Keep `file` open for reader `id`, closing the least recently used files past the cap. A
    /// file another thread is reading from stays open until that read ends.
    fn insert(&self, id: u64, file: std::fs::File) -> Arc<std::fs::File> {
        let mut s = self.state();
        if let Some((_, open)) = s.open.iter().find(|(i, _)| *i == id) {
            return open.clone(); // another thread reopened it first
        }
        let file = Arc::new(file);
        s.open.push_back((id, file.clone()));
        while s.open.len() > self.cap {
            s.open.pop_front();
        }
        file
    }

    fn remove(&self, id: u64) {
        self.state().open.retain(|(i, _)| *i != id);
    }

    #[cfg(test)]
    fn open_count(&self) -> usize {
        self.state().open.len()
    }
}

static FILE_POOL: std::sync::LazyLock<Arc<FilePool>> = std::sync::LazyLock::new(|| Arc::new(FilePool::new(MAX_OPEN_FILES)));

/// A file on disk, read in place: only the requested ranges are read, so opening a clip costs its
/// index rather than its size. The file is kept open in a shared pool of at most
/// [`MAX_OPEN_FILES`] and reopened by path when the pool closed it.
#[derive(Debug)]
pub struct FileReader {
    path: PathBuf,
    len: u64,
    id: u64,
    pool: Arc<FilePool>,
}

impl FileReader {
    /// Open `path` in the process-wide pool.
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::open_in(path, &FILE_POOL)
    }

    pub(crate) fn open_in(path: &Path, pool: &Arc<FilePool>) -> io::Result<Self> {
        // checked before opening: opening a named pipe with no writer blocks the caller for good
        if !std::fs::metadata(path)?.is_file() {
            return Err(not_a_file(path));
        }
        let file = open_nonblocking(path)?;
        let m = file.metadata()?;
        if !m.is_file() {
            return Err(not_a_file(path));
        }
        let id = pool.next_id();
        pool.insert(id, file);
        Ok(Self { path: path.to_path_buf(), len: m.len(), id, pool: pool.clone() })
    }

    /// The open file, reopened when the pool closed it. A file now shorter than when it was opened
    /// (replaced or truncated) is refused rather than read as the old one; a longer one (still
    /// being written) is read up to the opened length.
    fn file(&self) -> io::Result<Arc<std::fs::File>> {
        if let Some(file) = self.pool.get(self.id) {
            return Ok(file);
        }
        let file = open_nonblocking(&self.path)?;
        let m = file.metadata()?;
        if !m.is_file() || m.len() < self.len {
            return Err(io::Error::other(format!("{} changed since it was opened", self.path.display())));
        }
        Ok(self.pool.insert(self.id, file))
    }
}

impl Drop for FileReader {
    fn drop(&mut self) {
        self.pool.remove(self.id);
    }
}

impl ByteSource for FileReader {
    fn len(&self) -> u64 {
        self.len
    }

    /// Positional reads: threads decoding different parts of the file do not share a cursor.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|e| e > self.len) {
            return Err(eof());
        }
        let file = self.file()?;
        file.read_at(offset, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_reader_reads_ranges_in_place() {
        let v: Vec<u8> = (0..=255).collect();
        let dir = craft_media_testkit::private_dir("file-reader");
        let path = dir.join("bytes.bin");
        std::fs::write(&path, &v).unwrap();
        let r = FileReader::open(&path).unwrap();
        assert_eq!(ByteSource::len(&r), 256);
        let mut b = [0u8; 3];
        r.read_at(4, &mut b).unwrap();
        assert_eq!(&b, &v[4..7]);
        // reads are positional: an earlier offset after a later one
        r.read_at(0, &mut b).unwrap();
        assert_eq!(&b, &v[0..3]);
        assert_eq!(r.read_at(254, &mut [0u8; 3]).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert!(FileReader::open(&dir).is_err(), "a directory is not a media file");
        assert_eq!(FileReader::open(&dir.join("missing")).unwrap_err().kind(), io::ErrorKind::NotFound);
        drop(r);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// More readers than the pool keeps open stay readable; past the cap the least recently read
    /// file is closed and reopened on demand, and a file truncated meanwhile is refused.
    #[test]
    fn more_file_readers_than_the_pool_holds_stay_readable() {
        let dir = craft_media_testkit::private_dir("file-pool");
        let pool = Arc::new(FilePool::new(2));
        let paths: Vec<_> = (0..5).map(|i| dir.join(format!("{i}.bin"))).collect();
        for (i, p) in paths.iter().enumerate() {
            std::fs::write(p, format!("file{i}-0123456789")).unwrap();
        }
        let readers: Vec<_> = paths.iter().map(|p| FileReader::open_in(p, &pool).unwrap()).collect();
        assert_eq!(pool.open_count(), 2);
        for _ in 0..2 {
            for (i, r) in readers.iter().enumerate() {
                let mut b = [0u8; 5];
                r.read_at(0, &mut b).unwrap();
                assert_eq!(b, *format!("file{i}").as_bytes());
                assert!(pool.open_count() <= 2);
            }
        }
        // reader 0 was closed by the later reads; its file is now shorter than when opened
        std::fs::write(&paths[0], b"short").unwrap();
        let mut b = [0u8; 5];
        assert!(readers[0].read_at(0, &mut b).is_err());
        drop(readers);
        assert_eq!(pool.open_count(), 0, "dropped readers close their files");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A named pipe with no writer would block `open` for good: refuse it first.
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let dir = craft_media_testkit::private_dir("file-fifo");
        let fifo = dir.join("pipe");
        if !std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            return; // no mkfifo here
        }
        assert!(FileReader::open(&fifo).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The open itself must not block on a FIFO swapped in after the path check (a FIFO with no writer).
    #[cfg(target_os = "linux")]
    #[test]
    fn opening_a_fifo_with_no_writer_does_not_block() {
        let dir = craft_media_testkit::private_dir("file-fifo-open");
        let fifo = dir.join("pipe");
        if std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            let f = super::open_nonblocking(&fifo).unwrap();
            assert!(!f.metadata().unwrap().is_file());
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

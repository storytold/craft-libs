//! Positioned reads from the bytes of an HDF5 file, so the reader never loads the whole file.
//!
//! [`MemorySource`] serves bytes already in memory (what a web build has); [`FileSource`] reads a file on disk and
//! exists only off wasm.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::File;
use std::io;

/// Random-access byte source. Reads take `&self`, so one source can serve several threads. Implement it to read a
/// project from storage of your own (a memory-mapped file, a blob); [`MemorySource`] covers bytes in memory.
pub trait ReadAt: Send + Sync {
    /// Total size in bytes.
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fills `buf` with the bytes starting at `offset`, or fails if they are not all there.
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

/// A file on disk, read with the platform's positioned-read call (no shared cursor, no lock).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct FileSource {
    file: File,
    len: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileSource {
    pub(crate) fn new(file: File) -> io::Result<Self> {
        let len = file.metadata()?.len();
        Ok(FileSource { file, len })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ReadAt for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    #[cfg(unix)]
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(&self.file, buf, offset)
    }

    #[cfg(windows)]
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match self.file.seek_read(buf, offset) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    // `seek_read` never reports more than it was given; a short count is the only way past
                    buf = std::mem::take(&mut buf).get_mut(n..).ok_or(io::ErrorKind::InvalidData)?;
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn read_exact_at(&self, _buf: &mut [u8], _offset: u64) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Bytes already in memory: a whole file the host has read (a web build's upload), or a synthetic file in tests.
pub struct MemorySource(pub(crate) Vec<u8>);

impl MemorySource {
    pub fn new(bytes: Vec<u8>) -> Self {
        MemorySource(bytes)
    }

    /// Hands the bytes back.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl ReadAt for MemorySource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        let src = start.checked_add(buf.len()).and_then(|end| self.0.get(start..end)).ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

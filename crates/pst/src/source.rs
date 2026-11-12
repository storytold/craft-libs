//! The file under the reader: any `Read + Seek`, read a page or a block at a time and never
//! whole. Every read is checked against the file's size before anything is allocated for it.

use std::io::{Read, Seek, SeekFrom};

use crate::error::{Cap, Error, Result};

/// The largest single read: a block ([MS-PST] 2.2.2.8: at most 8 KB).
pub(crate) const MAX_READ: usize = 8192;

pub(crate) struct Source<R> {
    inner: R,
    size: u64,
    /// Bytes read so far, and how many may be ([`crate::Limits::read_multiple`]).
    read: u64,
    allowed: u64,
}

impl<R: Read + Seek> Source<R> {
    pub(crate) fn new(mut inner: R, limits: &crate::Limits) -> Result<Self> {
        let size = inner.seek(SeekFrom::End(0))?;
        if size > limits.max_file_size {
            return Err(Error::Cap(Cap::FileSize));
        }
        let allowed = size.saturating_mul(limits.read_multiple).saturating_add(limits.read_floor);
        Ok(Source { inner, size, read: 0, allowed })
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    /// `len` bytes at `offset`, all of them inside the file.
    pub(crate) fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        if len > MAX_READ {
            return Err(Error::Corrupt("a read larger than a block"));
        }
        // Counted before it is checked, so that reads past the end aren't free either.
        self.read = self.read.saturating_add(len as u64);
        if self.read > self.allowed {
            return Err(Error::Cap(Cap::TotalRead));
        }
        let end = offset.checked_add(len as u64).ok_or(Error::OutOfBounds)?;
        if end > self.size {
            return Err(Error::OutOfBounds);
        }
        // Within the file's size, so a short read is the reader failing (the file cut short or
        // changed under the import), not damage to the file.
        let io = |e: std::io::Error| Error::Io(e.kind());
        self.inner.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut buf = vec![0u8; len];
        self.inner.read_exact(&mut buf).map_err(io)?;
        Ok(buf)
    }
}

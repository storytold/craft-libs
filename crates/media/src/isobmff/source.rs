//! Random-access byte sources for the demuxer.

use std::io;
use std::sync::Arc;

/// Random-access, read-only byte source (file, in-memory buffer, web Blob…).
///
/// Implementations must be able to serve reads at arbitrary offsets through `&self`.
pub trait ByteSource {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// Fill `buf` entirely with bytes starting at `offset`; fail with `UnexpectedEof` if past the end.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    /// True if the source holds no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of byte source")
}

impl ByteSource for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| eof())?;
        let end = start.checked_add(buf.len()).ok_or_else(eof)?;
        let src = self.get(start..end).ok_or_else(eof)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

impl ByteSource for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.as_slice().read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for Box<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ByteSource for std::fs::File {
    fn len(&self) -> u64 {
        self.metadata().map(|m| m.len()).unwrap_or(0)
    }
    #[cfg(unix)]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }
    #[cfg(windows)]
    fn read_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = self.seek_read(buf, offset)?;
            if n == 0 {
                return Err(eof());
            }
            offset += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = self;
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_reads() {
        let v = vec![1u8, 2, 3, 4];
        let mut b = [0u8; 2];
        v.read_at(1, &mut b).unwrap();
        assert_eq!(b, [2, 3]);
        assert!(v.read_at(3, &mut b).is_err());
        assert!(v.read_at(u64::MAX, &mut b).is_err());
        let a: Arc<[u8]> = Arc::from(v.clone());
        a.read_at(2, &mut b).unwrap();
        assert_eq!(b, [3, 4]);
        assert_eq!(ByteSource::len(&a), 4);
        let s: &[u8] = &v;
        assert_eq!(ByteSource::len(&s), 4);
    }
}

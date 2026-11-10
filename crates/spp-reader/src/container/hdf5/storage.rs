//! Bounds-checked reads of file structures at HDF5 addresses.

use super::error::Hdf5Error;
use super::sizes::{Address, Sizes};
use super::source::ReadAt;

/// Largest metadata structure (object header block, heap, B-tree node) we agree to load at once.
const MAX_STRUCTURE_LEN: u64 = 64 << 20;

/// The file bytes plus how addresses in it are encoded.
pub(crate) struct Storage {
    source: Box<dyn ReadAt>,
    base: u64,
    sizes: Sizes,
}

impl Storage {
    pub(crate) fn new(source: Box<dyn ReadAt>, base: u64, sizes: Sizes) -> Self {
        Storage { source, base, sizes }
    }

    pub(crate) fn sizes(&self) -> Sizes {
        self.sizes
    }

    /// Bytes from the base address to the end of the file: an upper bound for any stored object.
    pub(crate) fn capacity(&self) -> u64 {
        self.source.len().saturating_sub(self.base)
    }

    /// A metadata structure of `len` bytes at `addr`. `what` names it in error messages.
    pub(crate) fn read(&self, addr: Address, len: u64, what: &str) -> Result<Vec<u8>, Hdf5Error> {
        if len > MAX_STRUCTURE_LEN {
            return Err(Hdf5Error::Corrupt(format!("{what} at {addr:#x}: implausible size {len}")));
        }
        let mut buf = vec![0; len as usize];
        self.read_into(addr, &mut buf, what)?;
        Ok(buf)
    }

    /// Fills `buf` from `addr`, failing with [`Hdf5Error::Corrupt`] if that range is outside the file.
    pub(crate) fn read_into(&self, addr: Address, buf: &mut [u8], what: &str) -> Result<(), Hdf5Error> {
        let start = self.base.checked_add(addr);
        let end = start.and_then(|s| s.checked_add(buf.len() as u64));
        match (start, end) {
            (Some(start), Some(end)) if end <= self.source.len() => Ok(self.source.read_exact_at(buf, start)?),
            _ => Err(Hdf5Error::Corrupt(format!("{what} at {addr:#x} (+{} bytes) is outside the file", buf.len()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::source::MemorySource;

    #[test]
    fn out_of_range_reads_are_corrupt_not_io_errors() {
        let storage = Storage::new(Box::new(MemorySource(vec![1, 2, 3, 4])), 1, Sizes::new(8, 8).unwrap());
        assert_eq!(storage.read(1, 2, "x").unwrap(), vec![3, 4]);
        assert!(matches!(storage.read(2, 2, "x"), Err(Hdf5Error::Corrupt(_))));
        assert!(matches!(storage.read(u64::MAX, 1, "x"), Err(Hdf5Error::Corrupt(_))));
    }
}

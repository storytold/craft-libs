//! The superblock: where the file starts, how addresses are encoded and where the root group is.

use super::error::Hdf5Error;
use super::sizes::{Address, Sizes};
use super::source::ReadAt;
use super::symbol_table::SymbolEntry;
use crate::container::bytes::ByteReader;

const SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";

/// Enough for a version 0/1 superblock with 8-byte addresses (100 bytes).
const MAX_SUPERBLOCK_LEN: u64 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Superblock {
    /// Absolute file position that all addresses are relative to.
    pub base: u64,
    pub sizes: Sizes,
    /// Object header of the root group.
    pub root: Address,
}

impl Superblock {
    pub(crate) fn read(source: &dyn ReadAt) -> Result<Self, Hdf5Error> {
        let start = find_signature(source)?;
        let len = MAX_SUPERBLOCK_LEN.min(source.len() - start) as usize; // at most 128
        let mut buf = vec![0; len];
        source.read_exact_at(&mut buf, start)?;
        Self::parse(&buf, start)
    }

    /// Parses a superblock found at absolute position `start`; `data` begins with the signature.
    fn parse(data: &[u8], start: u64) -> Result<Self, Hdf5Error> {
        let mut r = ByteReader::new(data);
        r.skip(SIGNATURE.len())?;
        let version = r.u8()?;
        if version > 1 {
            return Err(Hdf5Error::Unsupported(format!("superblock version {version}")));
        }
        r.skip(4)?; // free-space, root group and shared header versions, reserved
        let sizes = Sizes::new(r.u8()?, r.u8()?)?;
        r.skip(1 + 2 + 2 + 4)?; // reserved, group leaf/internal K, consistency flags
        if version == 1 {
            r.skip(4)?; // indexed storage K, reserved
        }
        let base = sizes.address(&mut r)?.unwrap_or(start);
        for _ in 0..3 {
            sizes.address(&mut r)?; // free space, end of file, driver info
        }
        let root = SymbolEntry::parse(&mut r, sizes)?.header;
        Ok(Superblock { base, sizes, root })
    }
}

/// The signature is at offset 0 or at the next power of two from 512 on (when a user block precedes it).
fn find_signature(source: &dyn ReadAt) -> Result<u64, Hdf5Error> {
    let mut at = 0u64;
    while at.saturating_add(SIGNATURE.len() as u64) <= source.len() {
        let mut buf = [0u8; 8];
        source.read_exact_at(&mut buf, at)?;
        if &buf == SIGNATURE {
            return Ok(at);
        }
        at = if at == 0 { 512 } else { at.saturating_mul(2) };
    }
    Err(Hdf5Error::NotHdf5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::source::MemorySource;

    /// The first 128 bytes the reference app (and libhdf5's earliest format) writes.
    fn version0_superblock(root: u64) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        out.extend([0, 0, 0, 0, 0, 8, 8, 0, 4, 0, 16, 0, 0, 0, 0, 0]);
        out.extend(0u64.to_le_bytes()); // base
        out.extend(u64::MAX.to_le_bytes()); // free space
        out.extend(0x1000u64.to_le_bytes()); // end of file
        out.extend(u64::MAX.to_le_bytes()); // driver info
        out.extend(0u64.to_le_bytes()); // root entry: name offset
        out.extend(root.to_le_bytes());
        out.extend([0; 24]);
        out
    }

    #[test]
    fn parses_a_version_0_superblock() {
        let sb = Superblock::read(&MemorySource(version0_superblock(0x60))).unwrap();
        assert_eq!(sb.base, 0);
        assert_eq!(sb.root, 0x60);
        assert_eq!(sb.sizes, Sizes::new(8, 8).unwrap());
    }

    #[test]
    fn finds_the_signature_after_a_user_block() {
        let mut file = vec![0; 512];
        file.extend(version0_superblock(0x60));
        file[512 + 24..512 + 32].copy_from_slice(&512u64.to_le_bytes());
        let sb = Superblock::read(&MemorySource(file)).unwrap();
        assert_eq!(sb.base, 512);
    }

    #[test]
    fn rejects_newer_superblocks_and_non_hdf5_files() {
        let mut file = version0_superblock(0x60);
        file[8] = 2;
        assert!(matches!(Superblock::read(&MemorySource(file)), Err(Hdf5Error::Unsupported(_))));
        assert!(matches!(Superblock::read(&MemorySource(vec![0; 2000])), Err(Hdf5Error::NotHdf5)));
        assert!(matches!(Superblock::read(&MemorySource(vec![])), Err(Hdf5Error::NotHdf5)));
    }

    #[test]
    fn a_truncated_superblock_is_corrupt() {
        let file = version0_superblock(0x60)[..40].to_vec();
        assert!(matches!(Superblock::read(&MemorySource(file)), Err(Hdf5Error::Corrupt(_))));
    }
}

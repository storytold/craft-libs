//! Chunked storage of one-dimensional datasets: finding chunks in the v1 B-tree and copying them out.

use super::btree::{self, TreeKind};
use super::error::Hdf5Error;
use super::sizes::Address;
use super::storage::Storage;
use crate::container::bytes::ByteReader;

/// Chunk geometry of a one-dimensional dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChunkGrid {
    /// Bytes per (full) chunk.
    chunk_bytes: u64,
    element_size: u64,
}

/// A chunk B-tree key: stored chunk size, filter mask, then one offset per dimension (in elements).
struct ChunkKey {
    stored_size: u32,
    offset: u64,
}

/// Key size for a 1-D dataset: size, filter mask and two 8-byte offsets (data and element dimension).
const KEY_LEN: usize = 4 + 4 + 2 * 8;

impl ChunkKey {
    fn parse(key: &[u8]) -> Result<Self, Hdf5Error> {
        let mut r = ByteReader::new(key);
        let stored_size = r.u32_le()?;
        r.skip(4)?; // filter mask
        Ok(ChunkKey { stored_size, offset: r.u64_le()? })
    }
}

impl ChunkGrid {
    /// `dims` is the layout message's chunk shape: elements per chunk, then the element size.
    pub(crate) fn new(dims: &[u32], rank: usize, element_size: u32) -> Result<Self, Hdf5Error> {
        let &[chunk_elems, chunk_element_size] = dims else {
            return Err(Hdf5Error::Unsupported(format!("chunked dataset of rank {rank}")));
        };
        if rank != 1 {
            return Err(Hdf5Error::Corrupt(format!("rank {rank} dataset with 1-D chunks")));
        }
        if chunk_element_size != element_size || chunk_elems == 0 || element_size == 0 {
            return Err(Hdf5Error::Corrupt("chunk shape does not match the datatype".into()));
        }
        Ok(ChunkGrid { chunk_bytes: u64::from(chunk_elems) * u64::from(element_size), element_size: u64::from(element_size) })
    }

    /// Fills `out` with the first `out.len()` bytes of the dataset whose chunk B-tree is at `btree`.
    /// Only chunks overlapping that range are read; missing chunks leave zeros (the default fill value).
    pub(crate) fn read(&self, storage: &Storage, btree: Address, out: &mut [u8]) -> Result<(), Hdf5Error> {
        let want = out.len() as u64;
        let mut descend = |key: &[u8]| Ok(self.byte_start(&ChunkKey::parse(key)?)? < want);
        let mut visit = |key: &[u8], addr: Address| self.copy_chunk(storage, &ChunkKey::parse(key)?, addr, out);
        btree::walk(storage, btree, TreeKind::Chunk, KEY_LEN, &mut descend, &mut visit)
    }

    fn byte_start(&self, key: &ChunkKey) -> Result<u64, Hdf5Error> {
        key.offset.checked_mul(self.element_size).ok_or_else(|| Hdf5Error::Corrupt("chunk offset overflows".into()))
    }

    fn copy_chunk(&self, storage: &Storage, key: &ChunkKey, addr: Address, out: &mut [u8]) -> Result<(), Hdf5Error> {
        let start = self.byte_start(key)?;
        if start % self.chunk_bytes != 0 {
            return Err(Hdf5Error::Corrupt(format!("chunk offset {} is not on the chunk grid", key.offset)));
        }
        let total = out.len() as u64;
        if start >= total {
            return Ok(());
        }
        let len = (total - start).min(self.chunk_bytes);
        if u64::from(key.stored_size) < len {
            return Err(Hdf5Error::Corrupt(format!("chunk at {addr:#x} is smaller than its data")));
        }
        // Both bounds are at most `out.len()`, so they fit in usize.
        let range = start as usize..(start + len) as usize;
        let target = out.get_mut(range).ok_or_else(|| Hdf5Error::Corrupt(format!("chunk at {addr:#x} lies outside the dataset")))?;
        storage.read_into(addr, target, "data chunk")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::sizes::Sizes;
    use crate::container::hdf5::source::MemorySource;

    /// A leaf chunk B-tree at 0 for chunks of 4 bytes stored at 0x100 + 4·i, then the chunk data.
    fn file(chunks: u64) -> Vec<u8> {
        let mut out = b"TREE\x01\x00".to_vec();
        out.extend((chunks as u16).to_le_bytes());
        out.extend([0xff; 16]);
        let key = |offset: u64| [4u32.to_le_bytes(), [0; 4]].concat().into_iter().chain(offset.to_le_bytes()).chain([0; 8]);
        for i in 0..chunks {
            out.extend(key(4 * i));
            out.extend((0x100 + 4 * i).to_le_bytes());
        }
        out.extend(key(4 * chunks));
        out.resize(0x100, 0);
        out.extend((0..4 * chunks).map(|b| b as u8));
        out
    }

    fn read(file: Vec<u8>, len: usize) -> Result<Vec<u8>, Hdf5Error> {
        let storage = Storage::new(Box::new(MemorySource(file)), 0, Sizes::new(8, 8).unwrap());
        let mut out = vec![0; len];
        ChunkGrid::new(&[4, 1], 1, 1)?.read(&storage, 0, &mut out)?;
        Ok(out)
    }

    #[test]
    fn reads_a_partial_last_chunk() {
        assert_eq!(read(file(3), 10).unwrap(), (0..10).collect::<Vec<u8>>());
    }

    #[test]
    fn a_prefix_only_needs_its_chunks() {
        let mut data = file(3);
        data.truncate(0x104); // later chunks missing from the file
        assert_eq!(read(data, 3).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn rejects_multidimensional_chunks() {
        assert!(matches!(ChunkGrid::new(&[4, 4, 1], 2, 1), Err(Hdf5Error::Unsupported(_))));
    }
}

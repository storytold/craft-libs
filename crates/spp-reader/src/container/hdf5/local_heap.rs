//! Local heaps ("HEAP"): the name strings of an old-style group's members.

use super::error::Hdf5Error;
use super::sizes::Address;
use super::storage::Storage;
use crate::container::bytes::ByteReader;

/// The data segment of one local heap, loaded in full (it only holds member names).
pub(crate) struct LocalHeap {
    data: Vec<u8>,
}

impl LocalHeap {
    pub(crate) fn read(storage: &Storage, addr: Address) -> Result<Self, Hdf5Error> {
        let sizes = storage.sizes();
        let header_len = 8 + 2 * sizes.length_size() + sizes.offset_size();
        let header = storage.read(addr, header_len as u64, "local heap")?;
        let mut r = ByteReader::new(&header);
        if r.bytes(4)? != b"HEAP" {
            return Err(Hdf5Error::Corrupt(format!("no local heap at {addr:#x}")));
        }
        let version = r.u8()?;
        if version != 0 {
            return Err(Hdf5Error::Unsupported(format!("local heap version {version}")));
        }
        r.skip(3)?;
        let size = sizes.length(&mut r)?;
        sizes.length(&mut r)?; // free list head
        let data_addr = sizes.required_address(&mut r, "local heap data")?;
        let data = storage.read(data_addr, size, "local heap data")?;
        Ok(LocalHeap { data })
    }

    #[cfg(test)]
    pub(crate) fn from_data(data: Vec<u8>) -> Self {
        LocalHeap { data }
    }

    /// The null-terminated string at `offset` in the data segment.
    pub(crate) fn name_at(&self, offset: u64) -> Result<String, Hdf5Error> {
        let rest = usize::try_from(offset)
            .ok()
            .and_then(|at| self.data.get(at..))
            .ok_or_else(|| Hdf5Error::Corrupt(format!("name offset {offset} is outside the local heap")))?;
        let name = rest
            .iter()
            .position(|&b| b == 0)
            .and_then(|end| rest.get(..end))
            .ok_or_else(|| Hdf5Error::Corrupt(format!("name at heap offset {offset} is not terminated")))?;
        Ok(String::from_utf8_lossy(name).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::sizes::Sizes;
    use crate::container::hdf5::source::MemorySource;

    #[test]
    fn reads_names_from_a_heap() {
        let mut file = b"HEAP\0\0\0\0".to_vec();
        file.extend(16u64.to_le_bytes()); // data size
        file.extend(u64::MAX.to_le_bytes()); // no free list
        file.extend(32u64.to_le_bytes()); // data address
        file.extend(b"\0\0\0\0\0\0\0\0alpha\0\0\0");
        let storage = Storage::new(Box::new(MemorySource(file)), 0, Sizes::new(8, 8).unwrap());
        let heap = LocalHeap::read(&storage, 0).unwrap();
        assert_eq!(heap.name_at(8).unwrap(), "alpha");
        assert_eq!(heap.name_at(0).unwrap(), "");
    }

    #[test]
    fn bad_offsets_are_corrupt() {
        let heap = LocalHeap::from_data(b"ab".to_vec());
        assert!(matches!(heap.name_at(0), Err(Hdf5Error::Corrupt(_))));
        assert!(matches!(heap.name_at(99), Err(Hdf5Error::Corrupt(_))));
    }
}

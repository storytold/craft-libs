//! A dataset's shape and storage, and reading its bytes.

use super::chunked;
use super::error::Hdf5Error;
use super::layout::Layout;
use super::message::{Dataspace, Message};
use super::storage::Storage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dataset {
    space: Dataspace,
    element_size: u32,
    layout: Layout,
    filters: u8,
}

impl Dataset {
    /// Builds a dataset from its decoded header messages.
    pub(crate) fn from_messages(messages: Vec<Message>) -> Result<Self, Hdf5Error> {
        let (mut space, mut element_size, mut layout, mut filters) = (None, None, None, 0);
        for message in messages {
            match message {
                Message::Dataspace(s) => space = Some(s),
                Message::Datatype { size } => element_size = Some(size),
                Message::Layout(l) => layout = Some(l),
                Message::FilterPipeline { filters: n } => filters = n,
                _ => {}
            }
        }
        let missing = |what: &str| Hdf5Error::Corrupt(format!("dataset without a {what} message"));
        Ok(Dataset {
            space: space.ok_or_else(|| missing("dataspace"))?,
            element_size: element_size.ok_or_else(|| missing("datatype"))?,
            layout: layout.ok_or_else(|| missing("data layout"))?,
            filters,
        })
    }

    /// Size of the whole dataset in bytes.
    pub(crate) fn len(&self) -> Result<u64, Hdf5Error> {
        self.space
            .element_count()
            .and_then(|n| n.checked_mul(u64::from(self.element_size)))
            .ok_or_else(|| Hdf5Error::Corrupt("dataset size overflows".into()))
    }

    /// The first `max_len` bytes (all of them if the dataset is shorter).
    pub(crate) fn read(&self, storage: &Storage, max_len: u64) -> Result<Vec<u8>, Hdf5Error> {
        if self.filters > 0 {
            return Err(Hdf5Error::Unsupported("filter pipeline (compressed chunks)".into()));
        }
        let len = self.len()?.min(max_len);
        let len_usize = usize::try_from(len).map_err(|_| Hdf5Error::Unsupported("dataset larger than the address space".into()))?;
        if let Layout::Compact(data) = &self.layout {
            return data.get(..len_usize).map(<[u8]>::to_vec).ok_or_else(|| Hdf5Error::Corrupt("compact data is shorter than the dataset".into()));
        }
        // Rules out absurd allocations from corrupt sizes (sparse datasets beyond the file are not supported).
        if len > storage.capacity() {
            return Err(Hdf5Error::Corrupt(format!("dataset of {len} bytes is larger than the file")));
        }
        let mut out = vec![0; len_usize];
        match &self.layout {
            Layout::Compact(_) => {}
            Layout::Contiguous { addr: None, .. } => {}
            Layout::Contiguous { addr: Some(addr), size } => {
                if *size < len {
                    return Err(Hdf5Error::Corrupt("contiguous storage is shorter than the dataset".into()));
                }
                storage.read_into(*addr, &mut out, "contiguous dataset")?;
            }
            Layout::Chunked { btree: None, .. } => {}
            Layout::Chunked { btree: Some(btree), dims } => {
                let rank = self.space.dims.as_ref().map_or(0, Vec::len);
                let chunks = chunked::ChunkGrid::new(dims, rank, self.element_size)?;
                chunks.read(storage, *btree, &mut out)?;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::sizes::Sizes;
    use crate::container::hdf5::source::MemorySource;

    fn dataset(len: u64, layout: Layout) -> Dataset {
        let space = Dataspace { dims: Some(vec![len]) };
        Dataset::from_messages(vec![Message::Dataspace(space), Message::Datatype { size: 1 }, Message::Layout(layout)]).unwrap()
    }

    fn storage(file: Vec<u8>) -> Storage {
        Storage::new(Box::new(MemorySource(file)), 0, Sizes::new(8, 8).unwrap())
    }

    #[test]
    fn reads_contiguous_and_compact_prefixes() {
        let file = storage((0..100).collect());
        let contiguous = dataset(10, Layout::Contiguous { addr: Some(50), size: 10 });
        assert_eq!(contiguous.read(&file, u64::MAX).unwrap(), (50..60).collect::<Vec<u8>>());
        assert_eq!(contiguous.read(&file, 3).unwrap(), vec![50, 51, 52]);
        let compact = dataset(3, Layout::Compact(vec![7, 8, 9]));
        assert_eq!(compact.read(&file, 2).unwrap(), vec![7, 8]);
    }

    #[test]
    fn unwritten_storage_reads_as_zeros() {
        let file = storage(vec![1; 100]);
        let never_written = dataset(4, Layout::Contiguous { addr: None, size: 4 });
        assert_eq!(never_written.read(&file, u64::MAX).unwrap(), vec![0; 4]);
    }

    #[test]
    fn filters_and_oversized_datasets_are_rejected() {
        let file = storage(vec![1; 100]);
        let mut filtered = dataset(4, Layout::Contiguous { addr: Some(0), size: 4 });
        filtered.filters = 1;
        assert!(matches!(filtered.read(&file, 4), Err(Hdf5Error::Unsupported(_))));
        let huge = dataset(1 << 40, Layout::Contiguous { addr: Some(0), size: 1 << 40 });
        assert!(matches!(huge.read(&file, u64::MAX), Err(Hdf5Error::Corrupt(_))));
        assert_eq!(huge.read(&file, 2).unwrap(), vec![1, 1]);
    }

    #[test]
    fn a_dataset_needs_all_three_messages() {
        let result = Dataset::from_messages(vec![Message::Datatype { size: 1 }]);
        assert!(matches!(result, Err(Hdf5Error::Corrupt(_))));
    }
}

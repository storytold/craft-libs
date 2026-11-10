//! The data layout message (0x0008): where a dataset's bytes are stored.

use super::error::Hdf5Error;
use super::sizes::{Address, Sizes};
use crate::container::bytes::ByteReader;

/// Most dimensions HDF5 allows for a chunk (32 dataset dimensions plus the element dimension).
const MAX_CHUNK_DIMS: u8 = 33;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Layout {
    /// The bytes are inside the object header.
    Compact(Vec<u8>),
    /// One block; `None` if never written (reads as fill value).
    Contiguous { addr: Option<Address>, size: u64 },
    /// Fixed-size chunks indexed by a v1 B-tree; `None` if no chunk was ever written.
    /// `dims` are the chunk size in elements per dimension, followed by the element size in bytes.
    Chunked { btree: Option<Address>, dims: Vec<u32> },
}

impl Layout {
    pub(crate) fn parse(body: &[u8], sizes: Sizes) -> Result<Self, Hdf5Error> {
        let mut r = ByteReader::new(body);
        let version = r.u8()?;
        if !(3..=4).contains(&version) {
            return Err(Hdf5Error::Unsupported(format!("data layout message version {version}")));
        }
        match r.u8()? {
            0 => {
                let size = usize::from(r.u16_le()?);
                Ok(Layout::Compact(r.bytes(size)?.to_vec()))
            }
            1 => {
                let addr = sizes.address(&mut r)?;
                Ok(Layout::Contiguous { addr, size: sizes.length(&mut r)? })
            }
            2 if version == 3 => {
                let ndims = r.u8()?;
                if !(2..=MAX_CHUNK_DIMS).contains(&ndims) {
                    return Err(Hdf5Error::Corrupt(format!("chunk dimensionality {ndims}")));
                }
                let btree = sizes.address(&mut r)?;
                let dims = (0..ndims).map(|_| r.u32_le()).collect::<Result<Vec<_>, _>>()?;
                if dims.contains(&0) {
                    return Err(Hdf5Error::Corrupt("chunk dimension of size 0".into()));
                }
                Ok(Layout::Chunked { btree, dims })
            }
            2 => Err(Hdf5Error::Unsupported("chunk index of data layout version 4".into())),
            3 => Err(Hdf5Error::Unsupported("virtual dataset layout".into())),
            class => Err(Hdf5Error::Corrupt(format!("data layout class {class}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> Sizes {
        Sizes::new(8, 8).unwrap()
    }

    #[test]
    fn parses_the_three_classes() {
        assert_eq!(Layout::parse(&[3, 0, 2, 0, 7, 8], sizes()).unwrap(), Layout::Compact(vec![7, 8]));

        let mut contiguous = vec![3, 1];
        contiguous.extend(u64::MAX.to_le_bytes());
        contiguous.extend(10u64.to_le_bytes());
        assert_eq!(Layout::parse(&contiguous, sizes()).unwrap(), Layout::Contiguous { addr: None, size: 10 });

        let mut chunked = vec![3, 2, 2];
        chunked.extend(0x400u64.to_le_bytes());
        chunked.extend(65536u32.to_le_bytes());
        chunked.extend(1u32.to_le_bytes());
        assert_eq!(Layout::parse(&chunked, sizes()).unwrap(), Layout::Chunked { btree: Some(0x400), dims: vec![65536, 1] });
    }

    #[test]
    fn rejects_what_the_format_never_holds() {
        assert!(matches!(Layout::parse(&[4, 2, 0], sizes()), Err(Hdf5Error::Unsupported(_))));
        assert!(matches!(Layout::parse(&[2, 1, 0], sizes()), Err(Hdf5Error::Unsupported(_))));
        assert!(matches!(Layout::parse(&[3, 3], sizes()), Err(Hdf5Error::Unsupported(_))));
        assert!(matches!(Layout::parse(&[3, 2, 200], sizes()), Err(Hdf5Error::Corrupt(_))));
        assert!(matches!(Layout::parse(&[3, 0, 9, 0], sizes()), Err(Hdf5Error::Corrupt(_))));
    }
}

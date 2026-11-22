//! Decoding the object header messages this reader needs; everything else is [`Message::Other`].

use super::error::Hdf5Error;
use super::layout::Layout;
use super::object_header::RawMessage;
use super::sizes::{Address, Sizes};
use crate::container::bytes::ByteReader;

/// Most dimensions HDF5 allows for a dataspace.
const MAX_RANK: u8 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    Dataspace(Dataspace),
    /// Size in bytes of one element.
    Datatype {
        size: u32,
    },
    Layout(Layout),
    /// Number of filters (compression, …) applied to chunks.
    FilterPipeline {
        filters: u8,
    },
    /// An old-style group: its member B-tree and the local heap with member names.
    SymbolTable {
        btree: Address,
        heap: Address,
    },
    /// Link info, link or group info message: a group in the newer (1.8+) format.
    NewStyleGroup,
    Other,
}

/// Current extent of a dataset; `None` dimensions for a null dataspace (no elements at all).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dataspace {
    pub dims: Option<Vec<u64>>,
}

impl Dataspace {
    /// Number of elements (1 for a scalar), or `None` on overflow.
    pub(crate) fn element_count(&self) -> Option<u64> {
        match &self.dims {
            None => Some(0),
            Some(dims) => dims.iter().try_fold(1u64, |acc, &d| acc.checked_mul(d)),
        }
    }

    fn parse(body: &[u8], sizes: Sizes) -> Result<Self, Hdf5Error> {
        let mut r = ByteReader::new(body);
        let version = r.u8()?;
        let rank = r.u8()?;
        r.skip(1)?; // flags: max dims / permutation present (both follow the dims; not needed)
        let null = match version {
            1 => {
                r.skip(5)?;
                false
            }
            2 => r.u8()? == 2,
            _ => return Err(Hdf5Error::Unsupported(format!("dataspace message version {version}"))),
        };
        if rank > MAX_RANK {
            return Err(Hdf5Error::Corrupt(format!("dataspace rank {rank}")));
        }
        if null {
            return Ok(Dataspace { dims: None });
        }
        let dims = (0..rank).map(|_| sizes.length(&mut r)).collect::<Result<_, _>>()?;
        Ok(Dataspace { dims: Some(dims) })
    }
}

// Format data: HDF5 header message types (HDF5 file format specification, section IV.A.2).
const DATASPACE: u16 = 0x0001;
const LINK_INFO: u16 = 0x0002;
const DATATYPE: u16 = 0x0003;
const LINK: u16 = 0x0006;
const DATA_LAYOUT: u16 = 0x0008;
const GROUP_INFO: u16 = 0x000A;
const FILTER_PIPELINE: u16 = 0x000B;
const SYMBOL_TABLE: u16 = 0x0011;

impl Message {
    pub(crate) fn decode(raw: &RawMessage, sizes: Sizes) -> Result<Self, Hdf5Error> {
        let needs_body = matches!(raw.kind, DATASPACE | DATATYPE | DATA_LAYOUT | FILTER_PIPELINE);
        if needs_body && raw.is_shared() {
            return Err(Hdf5Error::Unsupported(format!("shared message of type {:#06x}", raw.kind)));
        }
        let mut r = ByteReader::new(&raw.body);
        Ok(match raw.kind {
            DATASPACE => Message::Dataspace(Dataspace::parse(&raw.body, sizes)?),
            DATATYPE => {
                r.skip(4)?; // class and version, class bit fields
                Message::Datatype { size: r.u32_le()? }
            }
            DATA_LAYOUT => Message::Layout(Layout::parse(&raw.body, sizes)?),
            FILTER_PIPELINE => {
                r.skip(1)?;
                Message::FilterPipeline { filters: r.u8()? }
            }
            SYMBOL_TABLE => Message::SymbolTable {
                btree: sizes.required_address(&mut r, "group B-tree")?,
                heap: sizes.required_address(&mut r, "group local heap")?,
            },
            LINK_INFO | LINK | GROUP_INFO => Message::NewStyleGroup,
            _ => Message::Other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(kind: u16, body: &[u8]) -> Result<Message, Hdf5Error> {
        let raw = RawMessage { kind, flags: 0, body: body.to_vec() };
        Message::decode(&raw, Sizes::new(8, 8).unwrap())
    }

    #[test]
    fn decodes_a_version_1_dataspace_with_max_dims() {
        let mut body = vec![1, 1, 1, 0, 0, 0, 0, 0];
        body.extend(970346u64.to_le_bytes());
        body.extend(u64::MAX.to_le_bytes());
        let Message::Dataspace(space) = decode(0x0001, &body).unwrap() else { panic!("not a dataspace") };
        assert_eq!(space.element_count(), Some(970346));
    }

    #[test]
    fn scalar_and_null_dataspaces() {
        let Message::Dataspace(scalar) = decode(0x0001, &[2, 0, 0, 0]).unwrap() else { panic!() };
        assert_eq!(scalar.element_count(), Some(1));
        let Message::Dataspace(null) = decode(0x0001, &[2, 0, 0, 2]).unwrap() else { panic!() };
        assert_eq!(null.element_count(), Some(0));
    }

    #[test]
    fn decodes_datatype_size_and_symbol_table() {
        assert_eq!(decode(0x0003, &[0x10, 0, 0, 0, 1, 0, 0, 0]).unwrap(), Message::Datatype { size: 1 });
        let mut body = 0x88u64.to_le_bytes().to_vec();
        body.extend(0x2a8u64.to_le_bytes());
        assert_eq!(decode(0x0011, &body).unwrap(), Message::SymbolTable { btree: 0x88, heap: 0x2a8 });
    }

    #[test]
    fn shared_datatypes_are_unsupported() {
        let raw = RawMessage { kind: 0x0003, flags: 2, body: vec![0; 16] };
        let result = Message::decode(&raw, Sizes::new(8, 8).unwrap());
        assert!(matches!(result, Err(Hdf5Error::Unsupported(_))));
    }
}

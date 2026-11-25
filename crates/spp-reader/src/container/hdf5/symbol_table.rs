//! Symbol table entries and the "SNOD" nodes that hold them (old-style group members).

use super::error::Hdf5Error;
use super::sizes::{Address, Sizes};
use super::storage::Storage;
use crate::container::bytes::ByteReader;

/// One group member: where its name is in the local heap and where its object header is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SymbolEntry {
    pub name_offset: u64,
    pub header: Address,
}

impl SymbolEntry {
    /// Encoded size: name offset, header address, cache type, reserved, 16-byte scratch pad.
    pub(crate) fn encoded_len(sizes: Sizes) -> usize {
        sizes.length_size() + sizes.offset_size() + 4 + 4 + 16
    }

    pub(crate) fn parse(r: &mut ByteReader, sizes: Sizes) -> Result<Self, Hdf5Error> {
        let name_offset = sizes.length(r)?;
        let header = sizes.required_address(r, "symbol table entry")?;
        r.skip(4 + 4 + 16)?;
        Ok(SymbolEntry { name_offset, header })
    }
}

/// Reads the entries of the symbol table node ("SNOD") at `addr`.
pub(crate) fn read_node(storage: &Storage, addr: Address) -> Result<Vec<SymbolEntry>, Hdf5Error> {
    let head = storage.read(addr, 8, "symbol table node")?;
    let mut r = ByteReader::new(&head);
    if r.bytes(4)? != b"SNOD" {
        return Err(Hdf5Error::Corrupt(format!("no symbol table node at {addr:#x}")));
    }
    let version = r.u8()?;
    if version != 1 {
        return Err(Hdf5Error::Unsupported(format!("symbol table node version {version}")));
    }
    r.skip(1)?;
    let count = r.u16_le()?;
    let entry_len = SymbolEntry::encoded_len(storage.sizes());
    let body = storage.read(addr.saturating_add(8), (entry_len * usize::from(count)) as u64, "symbol table node")?;
    let mut r = ByteReader::new(&body);
    (0..count).map(|_| SymbolEntry::parse(&mut r, storage.sizes())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::source::MemorySource;

    fn entry_bytes(name_offset: u64, header: u64) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(name_offset.to_le_bytes());
        out.extend(header.to_le_bytes());
        out.extend([0; 24]);
        out
    }

    #[test]
    fn reads_a_symbol_table_node() {
        let mut file = b"SNOD\x01\x00\x02\x00".to_vec();
        file.extend(entry_bytes(8, 0x100));
        file.extend(entry_bytes(16, 0x200));
        let storage = Storage::new(Box::new(MemorySource(file)), 0, Sizes::new(8, 8).unwrap());
        let entries = read_node(&storage, 0).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1], SymbolEntry { name_offset: 16, header: 0x200 });
    }

    #[test]
    fn rejects_a_wrong_signature() {
        let storage = Storage::new(Box::new(MemorySource(b"XNOD\x01\x00\x00\x00".to_vec())), 0, Sizes::new(8, 8).unwrap());
        assert!(matches!(read_node(&storage, 0), Err(Hdf5Error::Corrupt(_))));
    }
}

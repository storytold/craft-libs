//! Old-style groups: a B-tree of symbol table nodes, with member names in a local heap.

use super::btree::{self, TreeKind};
use super::error::Hdf5Error;
use super::local_heap::LocalHeap;
use super::sizes::Address;
use super::storage::Storage;
use super::symbol_table;

/// A group's index, as given by its symbol table message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Group {
    pub btree: Address,
    pub heap: Address,
}

/// One member of a group: its name and object header address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    pub name: String,
    pub header: Address,
}

impl Group {
    /// All members, sorted by name (byte order, as HDF5 sorts them).
    pub(crate) fn members(&self, storage: &Storage) -> Result<Vec<Member>, Hdf5Error> {
        let heap = LocalHeap::read(storage, self.heap)?;
        let mut members = Vec::new();
        let key_len = storage.sizes().length_size();
        let mut visit = |_: &[u8], node: Address| {
            for entry in symbol_table::read_node(storage, node)? {
                let name = heap.name_at(entry.name_offset)?;
                members.push(Member { name, header: entry.header });
            }
            Ok(())
        };
        btree::walk(storage, self.btree, TreeKind::Group, key_len, &mut |_| Ok(true), &mut visit)?;
        members.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(members)
    }

    /// The member called `name`, if any.
    pub(crate) fn member(&self, storage: &Storage, name: &str) -> Result<Option<Member>, Hdf5Error> {
        Ok(self.members(storage)?.into_iter().find(|m| m.name == name))
    }
}

//! Version 1 B-trees ("TREE"), the index behind old-style groups (type 0) and chunked datasets (type 1).
//!
//! Callers only see the leaf-level children with their left key; key meaning is up to them.

use super::error::Hdf5Error;
use super::sizes::Address;
use super::storage::Storage;
use crate::container::bytes::ByteReader;
use std::collections::HashSet;

/// What a B-tree indexes; the node type byte in every node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeKind {
    /// Leaf children are symbol table nodes.
    Group = 0,
    /// Leaf children are raw data chunks.
    Chunk = 1,
}

/// One child reference of a node, with the key that bounds it from below.
struct Child<'a> {
    key: &'a [u8],
    addr: Address,
}

/// A parsed node: its level (0 = leaf) and raw keys and child addresses.
struct Node {
    level: u8,
    keys: Vec<u8>,
    children: Vec<Address>,
    key_len: usize,
}

impl Node {
    fn read(storage: &Storage, addr: Address, kind: TreeKind, key_len: usize) -> Result<Self, Hdf5Error> {
        let sizes = storage.sizes();
        let header_len = 8 + 2 * sizes.offset_size();
        let header = storage.read(addr, header_len as u64, "B-tree node")?;
        let mut r = ByteReader::new(&header);
        if r.bytes(4)? != b"TREE" {
            return Err(Hdf5Error::Corrupt(format!("no B-tree node at {addr:#x}")));
        }
        let node_type = r.u8()?;
        if node_type != kind as u8 {
            return Err(Hdf5Error::Corrupt(format!("B-tree node at {addr:#x} has type {node_type}")));
        }
        let level = r.u8()?;
        let used = usize::from(r.u16_le()?);

        let body_len = (used + 1) * key_len + used * sizes.offset_size();
        let body = storage.read(addr.saturating_add(header_len as u64), body_len as u64, "B-tree node")?;
        let mut r = ByteReader::new(&body);
        let mut keys = Vec::with_capacity((used + 1) * key_len);
        let mut children = Vec::with_capacity(used);
        for _ in 0..used {
            keys.extend_from_slice(r.bytes(key_len)?);
            children.push(sizes.required_address(&mut r, "B-tree child")?);
        }
        keys.extend_from_slice(r.bytes(key_len)?);
        Ok(Node { level, keys, children, key_len })
    }

    fn children(&self) -> impl Iterator<Item = Child<'_>> {
        let keys = self.keys.chunks_exact(self.key_len);
        keys.zip(&self.children).map(|(key, &addr)| Child { key, addr })
    }
}

/// Asked with a child's left key whether to follow it.
pub(crate) type Descend<'f> = dyn FnMut(&[u8]) -> Result<bool, Hdf5Error> + 'f;

/// Called with each leaf-level child's left key and address.
pub(crate) type Visit<'f> = dyn FnMut(&[u8], Address) -> Result<(), Hdf5Error> + 'f;

/// Walks the tree at `root` in key order and calls `visit(key, address)` for every leaf-level child.
///
/// `key_len` (non-zero) is the encoded key size of this tree kind.
/// `descend(key)` is asked for every child (inner and leaf) before it is followed; returning false
/// skips it and everything below. Each node is read at most once, so cycles cannot loop.
pub(crate) fn walk(storage: &Storage, root: Address, kind: TreeKind, key_len: usize, descend: &mut Descend, visit: &mut Visit) -> Result<(), Hdf5Error> {
    let mut walker = Walker { storage, kind, key_len, seen: HashSet::new(), descend, visit };
    walker.node(root, None)
}

struct Walker<'s, 'f> {
    storage: &'s Storage,
    kind: TreeKind,
    key_len: usize,
    seen: HashSet<Address>,
    descend: &'f mut Descend<'f>,
    visit: &'f mut Visit<'f>,
}

impl Walker<'_, '_> {
    /// `expected_level` is one below the parent's; levels strictly decrease, which bounds the depth.
    fn node(&mut self, addr: Address, expected_level: Option<u8>) -> Result<(), Hdf5Error> {
        if !self.seen.insert(addr) {
            return Err(Hdf5Error::Corrupt(format!("B-tree node at {addr:#x} is referenced twice")));
        }
        let node = Node::read(self.storage, addr, self.kind, self.key_len)?;
        if expected_level.is_some_and(|level| level != node.level) {
            return Err(Hdf5Error::Corrupt(format!("B-tree node at {addr:#x} has the wrong level")));
        }
        for child in node.children() {
            if !(self.descend)(child.key)? {
                continue;
            }
            match node.level {
                0 => (self.visit)(child.key, child.addr)?,
                level => self.node(child.addr, Some(level - 1))?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::hdf5::sizes::Sizes;
    use crate::container::hdf5::source::MemorySource;

    /// A type-0 node with 8-byte keys `0, 1, 2, …` and the given children.
    fn node(level: u8, children: &[u64]) -> Vec<u8> {
        let mut out = b"TREE\0".to_vec();
        out.push(level);
        out.extend((children.len() as u16).to_le_bytes());
        out.extend(u64::MAX.to_le_bytes());
        out.extend(u64::MAX.to_le_bytes());
        for (i, child) in children.iter().enumerate() {
            out.extend((i as u64).to_le_bytes());
            out.extend(child.to_le_bytes());
        }
        out.extend((children.len() as u64).to_le_bytes());
        out
    }

    fn collect(file: Vec<u8>) -> Result<Vec<u64>, Hdf5Error> {
        let storage = Storage::new(Box::new(MemorySource(file)), 0, Sizes::new(8, 8).unwrap());
        let mut out = Vec::new();
        walk(&storage, 0, TreeKind::Group, 8, &mut |_| Ok(true), &mut |_, addr| {
            out.push(addr);
            Ok(())
        })?;
        Ok(out)
    }

    #[test]
    fn walks_two_levels_in_order() {
        let mut file = node(1, &[0x100, 0x200]);
        file.resize(0x100, 0);
        file.extend(node(0, &[7, 8]));
        file.resize(0x200, 0);
        file.extend(node(0, &[9]));
        assert_eq!(collect(file).unwrap(), vec![7, 8, 9]);
    }

    #[test]
    fn a_cycle_is_corrupt_not_endless() {
        assert!(matches!(collect(node(1, &[0])), Err(Hdf5Error::Corrupt(_))));
    }

    #[test]
    fn wrong_level_is_corrupt() {
        let mut file = node(2, &[0x100]);
        file.resize(0x100, 0);
        file.extend(node(0, &[7]));
        assert!(matches!(collect(file), Err(Hdf5Error::Corrupt(_))));
    }
}

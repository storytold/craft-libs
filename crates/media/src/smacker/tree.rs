//! The byte Huffman tree that Smacker's video tree data and audio chunks are both built on.

use crate::bitstream::{BitError, LsbBitReader};

/// Set on a branch node; the low bits are the index of its right child. The left child is the
/// next node.
const BRANCH: u16 = 0x8000;
/// A tree over byte values has at most 256 leaves, and so at most 511 nodes.
const MAX_NODES: usize = 511;

/// A Huffman tree whose leaves are bytes, stored in a Smacker bitstream in pre-order: a 1 bit for
/// a branch (followed by its left and then its right subtree), a 0 bit and eight value bits for a
/// leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteTree {
    nodes: Vec<u16>,
}

/// Why a [`ByteTree`] could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// The data ended inside the tree.
    Eof,
    /// The tree has more nodes than a tree over byte values can have.
    TooLarge,
}

impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TreeError::Eof => f.write_str("data ended inside a Huffman tree"),
            TreeError::TooLarge => f.write_str("Huffman tree has too many nodes"),
        }
    }
}

impl std::error::Error for TreeError {}

impl From<BitError> for TreeError {
    fn from(_: BitError) -> Self {
        TreeError::Eof
    }
}

impl ByteTree {
    /// The tree with the one leaf `value`: every symbol is `value` and takes no bits.
    pub fn single(value: u8) -> ByteTree {
        ByteTree { nodes: vec![u16::from(value)] }
    }

    /// Reads a tree as it is stored where a tree is optional: a flag bit, and if it is set the
    /// tree and one closing bit. Without the tree every symbol is 0.
    pub fn read_optional(r: &mut LsbBitReader<'_>) -> Result<ByteTree, TreeError> {
        if !r.read_bit()? {
            return Ok(ByteTree::single(0));
        }
        let tree = ByteTree::read(r)?;
        r.read_bit()?;
        Ok(tree)
    }

    /// Reads the nodes of a tree.
    pub fn read(r: &mut LsbBitReader<'_>) -> Result<ByteTree, TreeError> {
        let mut nodes = Vec::new();
        // branches whose left subtree is being read
        let mut open: Vec<usize> = Vec::new();
        loop {
            if nodes.len() >= MAX_NODES {
                return Err(TreeError::TooLarge);
            }
            if r.read_bit()? {
                open.push(nodes.len());
                nodes.push(BRANCH);
                continue;
            }
            nodes.push(r.read_bits(8)? as u16);
            // a leaf ends the left subtree of the innermost open branch, or the tree
            let Some(branch) = open.pop() else { return Ok(ByteTree { nodes }) };
            // the right subtree of that branch starts at the next node (below MAX_NODES)
            let right = nodes.len() as u16;
            if let Some(n) = nodes.get_mut(branch) {
                *n = BRANCH | right;
            }
        }
    }

    /// Decodes one symbol.
    #[inline]
    pub fn decode(&self, r: &mut LsbBitReader<'_>) -> Result<u8, BitError> {
        let mut at = 0usize;
        // every step moves to a later node, so the walk ends
        loop {
            let n = self.nodes.get(at).copied().ok_or(BitError::Eof)?;
            if n & BRANCH == 0 {
                return Ok(n as u8);
            }
            at = if r.read_bit()? { usize::from(n & !BRANCH) } else { at + 1 };
        }
    }
}

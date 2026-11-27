//! Block identity and share-aware byte counts.

use std::collections::HashSet;
use std::hash::Hash;
use std::mem::size_of;

use crate::SharedVec;
use crate::node::{Branch, Node};

/// The identity of one block of a list. Two lists hold the same block exactly when they report the
/// same id for it, while both are alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(usize);

/// One block of a list, as [`SharedVec::blocks`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    /// Whether the block holds elements (a leaf) rather than child blocks (a branch).
    pub is_leaf: bool,
    /// The number of elements (a leaf) or child blocks (a branch) it holds.
    pub slots: usize,
    /// The bytes of its allocation, as [`SharedVec::bytes_not_in`] counts them. Elements' own heap
    /// allocations are not included.
    pub bytes: usize,
}

/// The reference counts in front of an `Arc`'s value.
const ARC_HEADER: usize = 2 * size_of::<usize>();

fn node_bytes<T>(node: &Node<T>) -> usize {
    match node {
        Node::Leaf(leaf) => ARC_HEADER + size_of::<Vec<T>>() + leaf.capacity() * size_of::<T>(),
        Node::Branch(branch) => {
            ARC_HEADER + size_of::<Branch<T>>() + branch.lens.capacity() * size_of::<usize>() + branch.children.capacity() * size_of::<Node<T>>()
        }
    }
}

impl<T> SharedVec<T> {
    /// Every block of the list, the root first and each block before its children. An empty list
    /// has none.
    pub fn blocks(&self) -> Vec<Block> {
        let mut out = Vec::new();
        let mut stack: Vec<&Node<T>> = self.root.iter().collect();
        while let Some(node) = stack.pop() {
            let (is_leaf, slots) = match node {
                Node::Leaf(leaf) => (true, leaf.len()),
                Node::Branch(branch) => {
                    stack.extend(branch.children.iter().rev());
                    (false, branch.children.len())
                }
            };
            out.push(Block { id: BlockId(node.addr()), is_leaf, slots, bytes: node_bytes(node) });
        }
        out
    }

    /// The bytes of the blocks this list holds and `other` does not: what keeping both lists costs
    /// over keeping `other` alone. A list compared with an empty list counts all of its blocks; a
    /// list compared with a clone of itself counts none. Each block is counted once.
    ///
    /// The count covers the blocks themselves (element storage, child handles and lengths), not
    /// heap memory the elements own; [`bytes_not_in_with`](Self::bytes_not_in_with) adds that. A
    /// block's bytes are the sizes it asks the allocator for: the `Arc` counts, the block's fields
    /// and each buffer's capacity. The allocator's own rounding and bookkeeping are not included.
    pub fn bytes_not_in(&self, other: &Self) -> usize {
        let (total, _, _) = self.unshared(other);
        total
    }

    /// [`bytes_not_in`](Self::bytes_not_in) plus the heap bytes of the elements this list holds and
    /// `other` does not.
    ///
    /// `key` names an element's heap allocation (for an `Arc`, its address through `Arc::as_ptr`),
    /// and `payload` returns that allocation's bytes. An element is counted when it sits in a leaf
    /// that `other` does not hold and `other` has no element with the same key in the leaves it
    /// does not share with this list, so the unchanged elements of a copied leaf, and elements that
    /// an edit moved to another leaf, are not counted. Each key is counted once.
    ///
    /// The element count errs high: it never leaves out an element that `other` does not hold. An
    /// element that `other` holds only in leaves both lists share, and that this list also holds in
    /// a new leaf (a handle cloned into a second place, without removing it from the first), is
    /// counted. Finding it would mean reading every leaf of `other`.
    ///
    /// The walk visits only the blocks that the two lists do not share and their children, so after
    /// one edit it costs about one block per level of the tree.
    pub fn bytes_not_in_with<K, P>(&self, other: &Self, mut key: impl FnMut(&T) -> K, mut payload: P) -> usize
    where
        K: Hash + Eq,
        P: FnMut(&T) -> usize,
    {
        let (mut total, ours, theirs) = self.unshared(other);
        let theirs: HashSet<K> = theirs.iter().flat_map(|leaf| leaf.iter()).map(&mut key).collect();
        let mut counted: HashSet<K> = HashSet::new();
        for value in ours.iter().flat_map(|leaf| leaf.iter()) {
            let id = key(value);
            if !theirs.contains(&id) && counted.insert(id) {
                total += payload(value);
            }
        }
        total
    }

    /// The bytes of this list's blocks that `other` does not hold, those leaves of this list, and
    /// the leaves of `other` that this list does not hold.
    fn unshared<'a>(&'a self, other: &'a Self) -> (usize, Vec<&'a [T]>, Vec<&'a [T]>) {
        let mut total = 0;
        let mut ours: Vec<&Node<T>> = self.root.iter().collect();
        let mut theirs: Vec<&Node<T>> = other.root.iter().collect();
        let mut our_height = self.root.as_ref().map_or(0, Node::height);
        let mut their_height = other.root.as_ref().map_or(0, Node::height);
        let mut counted: HashSet<usize> = HashSet::new();
        let mut our_leaves = Vec::new();
        // Above the other tree's root, no block of ours can be one of its blocks.
        while our_height > their_height {
            ours = count_and_descend(ours, &HashSet::new(), &mut counted, &mut total, &mut our_leaves);
            our_height -= 1;
        }
        while their_height > our_height {
            theirs = descend(theirs, &HashSet::new());
            their_height -= 1;
        }
        // Level by level, blocks both trees hold at the same height are shared with everything
        // under them, and are left out on both sides. The last level holds the leaves.
        let mut their_leaves = Vec::new();
        while !ours.is_empty() {
            let their_set: HashSet<usize> = theirs.iter().map(|node| node.addr()).collect();
            let our_set: HashSet<usize> = ours.iter().map(|node| node.addr()).collect();
            ours = count_and_descend(ours, &their_set, &mut counted, &mut total, &mut our_leaves);
            if ours.is_empty() {
                for node in &theirs {
                    if let Node::Leaf(leaf) = node
                        && !our_set.contains(&node.addr())
                    {
                        their_leaves.push(leaf.as_slice());
                    }
                }
            }
            theirs = descend(theirs, &our_set);
        }
        (total, our_leaves, their_leaves)
    }
}

/// Counts each block of `level` that is not in `shared` (once each), keeps its leaves, and returns
/// the children of its branches.
fn count_and_descend<'a, T>(
    level: Vec<&'a Node<T>>,
    shared: &HashSet<usize>,
    counted: &mut HashSet<usize>,
    total: &mut usize,
    leaves: &mut Vec<&'a [T]>,
) -> Vec<&'a Node<T>> {
    let mut next = Vec::new();
    for node in level {
        let addr = node.addr();
        if shared.contains(&addr) || !counted.insert(addr) {
            continue;
        }
        *total += node_bytes(node);
        match node {
            Node::Leaf(leaf) => leaves.push(leaf.as_slice()),
            Node::Branch(branch) => next.extend(branch.children.iter()),
        }
    }
    next
}

/// The children of each block of `level` that is not in `shared`.
fn descend<'a, T>(level: Vec<&'a Node<T>>, shared: &HashSet<usize>) -> Vec<&'a Node<T>> {
    let mut next = Vec::new();
    for node in level {
        if let Node::Branch(branch) = node
            && !shared.contains(&node.addr())
        {
            next.extend(branch.children.iter());
        }
    }
    next
}

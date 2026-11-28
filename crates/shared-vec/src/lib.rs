#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! An ordered, indexed list whose clones share storage.
//!
//! [`SharedVec<T>`] holds its elements in blocks of up to 64, under a counted B-tree whose nodes
//! also hold up to 64 children. Blocks sit behind `Arc`, so cloning a list copies one handle, and an
//! edit to a list that shares its blocks with a clone copies only the blocks on the path to the
//! edited element: one leaf and one branch per level. A `Vec<T>` that is shared the same way
//! (`Arc<Vec<T>>` with `Arc::make_mut`) copies every element on each edit. An empty list does not
//! allocate.
//!
//! - **Reads:** [`get`](SharedVec::get), [`first`](SharedVec::first), [`last`](SharedVec::last),
//!   [`iter`](SharedVec::iter) (double-ended) and [`chunks`](SharedVec::chunks) (the blocks as
//!   slices).
//! - **Single edits**, O(log n): [`set`](SharedVec::set), [`get_mut`](SharedVec::get_mut),
//!   [`insert`](SharedVec::insert), [`remove`](SharedVec::remove), [`push`](SharedVec::push) and
//!   [`pop`](SharedVec::pop). A removal that leaves a block under half full also copies the
//!   neighbor it merges with or takes elements from.
//! - **Range edits:** [`append`](SharedVec::append), [`split_off`](SharedVec::split_off),
//!   [`truncate`](SharedVec::truncate), [`splice`](SharedVec::splice) and
//!   [`retain`](SharedVec::retain). A range edit that removes or inserts only a few elements makes
//!   them as single edits. One that changes more rebuilds the branches from the leaves: it copies
//!   O(n / 64) handles and keeps every leaf it does not cut or change. A rebuild is chosen only when
//!   the single edits could copy as many blocks, so the blocks a range edit copies stay within
//!   about one path per element it removes or inserts.
//! - **Accounting:** [`bytes_not_in`](SharedVec::bytes_not_in) counts the bytes of the blocks one
//!   list holds and another does not, for an undo history that keeps both;
//!   [`bytes_not_in_with`](SharedVec::bytes_not_in_with) adds the heap bytes of the elements
//!   that the other list does not hold.
//! - **Serde** (feature `serde`): a list reads and writes as a plain sequence, the same as a
//!   `Vec<T>`.
//!
//! If `T::clone` panics during an edit, the list stays memory-safe, but which elements it holds
//! afterwards is unspecified. A panic in the iterator passed to [`splice`](SharedVec::splice), or
//! in the closure passed to [`retain`](SharedVec::retain), leaves the list as it was.
//!
//! ```
//! use craft_shared_vec::SharedVec;
//!
//! let before: SharedVec<u32> = (0..10_000).collect();
//! let mut after = before.clone();
//! assert_eq!(after.set(5_000, 7), Ok(5_000));
//! assert_eq!(before.get(5_000), Some(&5_000));
//! assert_eq!(after.get(5_000), Some(&7));
//! // Only the blocks on the edited path are new; the edit copied a few hundred bytes, not 40 KB.
//! assert!(after.bytes_not_in(&before) < 4_000);
//! ```

mod bytes;
mod iter;
mod node;
#[cfg(feature = "serde")]
mod serde_impl;
#[cfg(test)]
mod tests;

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, RangeBounds};
use std::sync::Arc;

pub use bytes::{Block, BlockId};
pub use iter::{Chunks, IntoIter, Iter};
use node::{Leaves, Node};

/// An ordered, indexed list of `T` whose clones share their blocks (see the [crate] docs).
pub struct SharedVec<T> {
    /// `None` for an empty list, so an empty list does not allocate.
    root: Option<Node<T>>,
}

impl<T> SharedVec<T> {
    /// An empty list. It does not allocate.
    pub const fn new() -> Self {
        SharedVec { root: None }
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.root.as_ref().map_or(0, Node::len)
    }

    /// Whether the list has no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The element at `index`, or `None` past the end.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.root.as_ref()?.get(index)
    }

    /// The first element.
    pub fn first(&self) -> Option<&T> {
        self.get(0)
    }

    /// The last element.
    pub fn last(&self) -> Option<&T> {
        self.len().checked_sub(1).and_then(|last| self.get(last))
    }

    /// The elements in order. The iterator is double-ended and knows its length.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter::new(self.root.as_ref())
    }

    /// The blocks as slices, in order. Together they hold every element once.
    pub fn chunks(&self) -> Chunks<'_, T> {
        Chunks::new(self.root.as_ref())
    }

    /// Whether the two lists are the same tree: a clone that neither list has edited since, or two
    /// empty lists. Lists that compare equal need not be the same tree.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (None, None) => true,
            (Some(ours), Some(theirs)) => ours.addr() == theirs.addr(),
            _ => false,
        }
    }

    /// Removes every element.
    pub fn clear(&mut self) {
        self.root = None;
    }

    /// Whether `count` single edits copy no more blocks than a rebuild of the branches does. A
    /// single edit copies at most one block per level; a tree of `len` elements has at most about
    /// `len / (MIN * MIN)` branches, since every node other than the root holds at least `MIN`.
    fn few(&self, count: usize) -> bool {
        let levels = self.root.as_ref().map_or(0, Node::height) + 1;
        count.saturating_mul(levels) <= self.len() / (node::MIN * node::MIN)
    }
}

impl<T: Clone> SharedVec<T> {
    /// The element at `index`, to change in place, or `None` past the end. The blocks on its path
    /// that the list shares with a clone are copied first.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.root.as_mut()?.get_mut(index)
    }

    /// Replaces the element at `index` and returns the old one. Past the end, returns `value` as
    /// the error and leaves the list unchanged.
    pub fn set(&mut self, index: usize, value: T) -> Result<T, T> {
        match self.get_mut(index) {
            Some(slot) => Ok(std::mem::replace(slot, value)),
            None => Err(value),
        }
    }

    /// Inserts `value` at `index`, moving the elements after it one place on. An `index` past the
    /// length returns `value` as the error and leaves the list unchanged.
    pub fn insert(&mut self, index: usize, value: T) -> Result<(), T> {
        if index > self.len() {
            return Err(value);
        }
        let Some(root) = self.root.as_mut() else {
            self.root = Some(Node::Leaf(Arc::new(vec![value])));
            return Ok(());
        };
        let split = root.insert(index, value);
        if let Some(right) = split
            && let Some(left) = self.root.take()
        {
            self.root = Some(Node::Branch(Arc::new(node::Branch::new(vec![left, right]))));
        }
        Ok(())
    }

    /// Removes and returns the element at `index`, or `None` past the end.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        let removed = self.root.as_mut()?.remove(index)?;
        self.collapse_root();
        Some(removed)
    }

    /// Appends `value` at the end.
    pub fn push(&mut self, value: T) {
        let end = self.len();
        // An insertion at the length always succeeds.
        let _ = self.insert(end, value);
    }

    /// Removes and returns the last element.
    pub fn pop(&mut self) -> Option<T> {
        let last = self.len().checked_sub(1)?;
        self.remove(last)
    }

    /// Moves every element of `other` onto the end of this list, leaving `other` empty. Both lists'
    /// blocks are kept, except where a short block at the join has to merge with its neighbor.
    pub fn append(&mut self, other: &mut Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            std::mem::swap(self, other);
            return;
        }
        if self.few(other.len()) {
            for value in std::mem::take(other) {
                self.push(value);
            }
            return;
        }
        if other.few(self.len()) {
            let front: Vec<T> = std::mem::take(self).into();
            for value in front.into_iter().rev() {
                let _ = other.insert(0, value);
            }
            std::mem::swap(self, other);
            return;
        }
        let mut leaves = self.take_leaves();
        leaves.extend(other.take_leaves());
        self.root = node::build(leaves);
    }

    /// Splits the list at `at`: this list keeps `[0, at)` and the rest is returned. An `at` past the
    /// length returns an empty list.
    pub fn split_off(&mut self, at: usize) -> Self {
        let len = self.len();
        if at >= len {
            return SharedVec::new();
        }
        if self.few(len - at) {
            let mut back = Vec::with_capacity(len - at);
            for _ in at..len {
                back.extend(self.pop());
            }
            back.reverse();
            return back.into();
        }
        if self.few(at) {
            let mut front = Vec::with_capacity(at);
            for _ in 0..at {
                front.extend(self.remove(0));
            }
            return std::mem::replace(self, front.into());
        }
        let (front, back) = cut(self.take_leaves(), at);
        self.root = node::build(front);
        SharedVec { root: node::build(back) }
    }

    /// Keeps the first `len` elements and drops the rest. A `len` at or past the length leaves the
    /// list unchanged.
    pub fn truncate(&mut self, len: usize) {
        let old = self.len();
        if len >= old {
            return;
        }
        if self.few(old - len) {
            for _ in len..old {
                self.pop();
            }
            return;
        }
        let (front, _) = cut(self.take_leaves(), len);
        self.root = node::build(front);
    }

    /// Replaces the elements in `range` with `replace_with` and returns the removed elements in
    /// order. The range is clamped to the list: a start past the end inserts at the end.
    pub fn splice<R, I>(&mut self, range: R, replace_with: I) -> Vec<T>
    where
        R: RangeBounds<usize>,
        I: IntoIterator<Item = T>,
    {
        let len = self.len();
        let start = match range.start_bound() {
            Bound::Included(&start) => start,
            Bound::Excluded(&start) => start.saturating_add(1),
            Bound::Unbounded => 0,
        }
        .min(len);
        let end = match range.end_bound() {
            Bound::Included(&end) => end.saturating_add(1),
            Bound::Excluded(&end) => end,
            Bound::Unbounded => len,
        }
        .clamp(start, len);
        // The new elements are read before the list changes, so a panic while reading them leaves
        // the list as it was.
        let inserted: Vec<T> = replace_with.into_iter().collect();
        if self.few((end - start).saturating_add(inserted.len())) {
            let mut removed = Vec::with_capacity(end - start);
            for _ in start..end {
                removed.extend(self.remove(start));
            }
            for (offset, value) in inserted.into_iter().enumerate() {
                let _ = self.insert(start + offset, value);
            }
            return removed;
        }
        let (mut leaves, rest) = cut(self.take_leaves(), start);
        let (middle, back) = cut(rest, end - start);
        let removed: Vec<T> = middle.into_iter().flat_map(node::unwrap_vec).collect();
        node::chunk(inserted, &mut leaves);
        leaves.extend(back);
        self.root = node::build(leaves);
        removed
    }

    /// Keeps the elements for which `keep` returns `true`, in order. `keep` sees each element once,
    /// in order, before the list changes. Blocks that keep every element are kept as they are.
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut keep: F) {
        let dropped: Vec<usize> = self.iter().enumerate().filter(|(_, value)| !keep(value)).map(|(index, _)| index).collect();
        if dropped.is_empty() {
            return;
        }
        if self.few(dropped.len()) {
            for &index in dropped.iter().rev() {
                self.remove(index);
            }
            return;
        }
        let mut dropped = dropped.into_iter().peekable();
        let mut leaves = Vec::new();
        let mut start = 0;
        for leaf in self.leaves() {
            let end = start + leaf.len();
            if dropped.peek().is_some_and(|&index| index < end) {
                let mut kept = Vec::with_capacity(leaf.len());
                for (offset, value) in leaf.iter().enumerate() {
                    if dropped.next_if_eq(&(start + offset)).is_none() {
                        kept.push(value.clone());
                    }
                }
                if !kept.is_empty() {
                    leaves.push(Arc::new(kept));
                }
            } else {
                leaves.push(leaf);
            }
            start = end;
        }
        // Drop the old tree first, so leaves no other list holds can be merged without a copy.
        self.root = None;
        self.root = node::build(leaves);
    }

    /// Copies the elements into a `Vec`.
    pub fn to_vec(&self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.len());
        for chunk in self.chunks() {
            out.extend_from_slice(chunk);
        }
        out
    }

    fn leaves(&self) -> Vec<Arc<Vec<T>>> {
        let mut leaves = Vec::with_capacity(self.len().div_ceil(node::MIN));
        if let Some(root) = &self.root {
            root.collect_leaves(&mut leaves);
        }
        leaves
    }

    /// Empties the list and returns its leaves. A leaf that no clone holds is then held only by the
    /// returned run, so it can be merged or cut without a copy.
    fn take_leaves(&mut self) -> Vec<Arc<Vec<T>>> {
        let mut leaves = Vec::with_capacity(self.len().div_ceil(node::MIN));
        if let Some(root) = self.root.take() {
            root.collect_leaves(&mut leaves);
        }
        leaves
    }

    /// Replaces a root branch that has one child with that child, and an empty root with `None`.
    fn collapse_root(&mut self) {
        loop {
            let only = match &self.root {
                Some(Node::Branch(branch)) if branch.children.len() == 1 => branch.children.first().cloned(),
                _ => None,
            };
            match only {
                Some(child) => self.root = Some(child),
                None => break,
            }
        }
        if self.is_empty() {
            self.root = None;
        }
    }
}

impl<T> Clone for SharedVec<T> {
    /// Copies one handle; the clone shares every block.
    fn clone(&self) -> Self {
        SharedVec { root: self.root.clone() }
    }
}

impl<T> Default for SharedVec<T> {
    fn default() -> Self {
        SharedVec::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for SharedVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: PartialEq> PartialEq for SharedVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T: Eq> Eq for SharedVec<T> {}

impl<T: PartialOrd> PartialOrd for SharedVec<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.iter().partial_cmp(other.iter())
    }
}

impl<T: Ord> Ord for SharedVec<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.iter().cmp(other.iter())
    }
}

impl<T: Hash> Hash for SharedVec<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for value in self {
            value.hash(state);
        }
    }
}

impl<T: Clone> FromIterator<T> for SharedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut leaves = Vec::new();
        node::chunk(iter, &mut leaves);
        SharedVec { root: node::build(leaves) }
    }
}

impl<T: Clone> Extend<T> for SharedVec<T> {
    /// Pushes each element in turn, so the list keeps every block it had.
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.push(value);
        }
    }
}

impl<T: Clone> From<Vec<T>> for SharedVec<T> {
    fn from(values: Vec<T>) -> Self {
        values.into_iter().collect()
    }
}

impl<T: Clone> From<&[T]> for SharedVec<T> {
    fn from(values: &[T]) -> Self {
        values.iter().cloned().collect()
    }
}

impl<T: Clone> From<SharedVec<T>> for Vec<T> {
    fn from(list: SharedVec<T>) -> Self {
        list.into_iter().collect()
    }
}

impl<'a, T> IntoIterator for &'a SharedVec<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<T: Clone> IntoIterator for SharedVec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    /// Moves the elements out of blocks no other list holds, and clones them out of shared ones.
    fn into_iter(mut self) -> IntoIter<T> {
        IntoIter::new(self.take_leaves())
    }
}

/// The leaves before `at` and from `at` on, cutting the leaf that holds `at` in two.
fn cut<T: Clone>(leaves: Leaves<T>, at: usize) -> (Leaves<T>, Leaves<T>) {
    let mut front = Vec::new();
    let mut back = Vec::new();
    let mut start = 0;
    for leaf in leaves {
        let end = start + leaf.len();
        if end <= at {
            front.push(leaf);
        } else if start >= at {
            back.push(leaf);
        } else {
            let mut head = node::unwrap_vec(leaf);
            let tail = head.split_off(at - start);
            front.push(Arc::new(head));
            back.push(Arc::new(tail));
        }
        start = end;
    }
    (front, back)
}

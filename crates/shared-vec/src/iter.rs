//! Iterators over a [`SharedVec`](crate::SharedVec).

use std::iter::FusedIterator;
use std::sync::Arc;

use crate::node::{self, Node};

/// The levels a walk keeps, the root's included, so it handles trees up to height `DEPTH - 1`.
/// The root holds at least 2 slots and every other node at least 32, so a tree of height 11 holds
/// at least 2 * 32^11 = 2^56 elements in 2^51 leaves, far more than fit in memory.
const DEPTH: usize = 12;

/// The leaves under a node in order, or in reverse order, without allocating.
struct LeafWalk<'a, T> {
    /// The children still to visit at each level, from the root down; `levels[..depth]` are live.
    levels: [std::slice::Iter<'a, Node<T>>; DEPTH],
    depth: usize,
    reverse: bool,
}

impl<'a, T> LeafWalk<'a, T> {
    fn new(root: Option<&'a Node<T>>, reverse: bool) -> Self {
        let mut levels: [std::slice::Iter<'a, Node<T>>; DEPTH] = std::array::from_fn(|_| <&[Node<T>]>::default().iter());
        levels[0] = root.map_or(&[][..], std::slice::from_ref).iter();
        LeafWalk { levels, depth: 1, reverse }
    }

    fn next_leaf(&mut self) -> Option<&'a [T]> {
        loop {
            let level = self.levels.get_mut(self.depth.checked_sub(1)?)?;
            let node = if self.reverse { level.next_back() } else { level.next() };
            match node {
                None => self.depth -= 1,
                Some(Node::Branch(branch)) => {
                    *self.levels.get_mut(self.depth)? = branch.children.iter();
                    self.depth += 1;
                }
                Some(Node::Leaf(leaf)) if leaf.is_empty() => {}
                Some(Node::Leaf(leaf)) => return Some(leaf.as_slice()),
            }
        }
    }
}

impl<T> Clone for LeafWalk<'_, T> {
    fn clone(&self) -> Self {
        LeafWalk { levels: self.levels.clone(), depth: self.depth, reverse: self.reverse }
    }
}

/// The elements of a list in order, by reference. Made by [`SharedVec::iter`](crate::SharedVec::iter).
pub struct Iter<'a, T> {
    root: Option<&'a Node<T>>,
    /// The index of the next element from the front.
    front: usize,
    /// One past the index of the next element from the back.
    back: usize,
    /// The leaves after the one `front_leaf` came from.
    front_leaves: LeafWalk<'a, T>,
    /// The leaves before the one `back_leaf` came from.
    back_leaves: LeafWalk<'a, T>,
    /// The rest of the leaf that holds `front`.
    front_leaf: std::slice::Iter<'a, T>,
    /// The start of the leaf that holds `back - 1`, up to `back`.
    back_leaf: std::slice::Iter<'a, T>,
}

impl<'a, T> Iter<'a, T> {
    pub(crate) fn new(root: Option<&'a Node<T>>) -> Self {
        Iter {
            root,
            front: 0,
            back: root.map_or(0, Node::len),
            front_leaves: LeafWalk::new(root, false),
            back_leaves: LeafWalk::new(root, true),
            front_leaf: <&[T]>::default().iter(),
            back_leaf: <&[T]>::default().iter(),
        }
    }
}

impl<T> Clone for Iter<'_, T> {
    fn clone(&self) -> Self {
        Iter {
            root: self.root,
            front: self.front,
            back: self.back,
            front_leaves: self.front_leaves.clone(),
            back_leaves: self.back_leaves.clone(),
            front_leaf: self.front_leaf.clone(),
            back_leaf: self.back_leaf.clone(),
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.front >= self.back {
            return None;
        }
        if self.front_leaf.len() == 0 {
            self.front_leaf = self.front_leaves.next_leaf()?.iter();
        }
        let value = self.front_leaf.next()?;
        self.front += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.back - self.front;
        (left, Some(left))
    }

    /// Folds a leaf slice at a time, which runs as fast as folding a slice.
    fn fold<B, F: FnMut(B, &'a T) -> B>(self, init: B, mut f: F) -> B {
        match self.root {
            Some(root) => root.fold_range(self.front, self.back, init, &mut f),
            None => init,
        }
    }
}

impl<'a, T> DoubleEndedIterator for Iter<'a, T> {
    /// Folds a leaf slice at a time from the back.
    fn rfold<B, F: FnMut(B, &'a T) -> B>(self, init: B, mut f: F) -> B {
        match self.root {
            Some(root) => root.rfold_range(self.front, self.back, init, &mut f),
            None => init,
        }
    }

    fn next_back(&mut self) -> Option<&'a T> {
        if self.front >= self.back {
            return None;
        }
        if self.back_leaf.len() == 0 {
            self.back_leaf = self.back_leaves.next_leaf()?.iter();
        }
        let value = self.back_leaf.next_back()?;
        self.back -= 1;
        Some(value)
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}

impl<T> FusedIterator for Iter<'_, T> {}

/// The blocks of a list as slices, in order. Made by [`SharedVec::chunks`](crate::SharedVec::chunks).
pub struct Chunks<'a, T> {
    leaves: LeafWalk<'a, T>,
}

impl<'a, T> Chunks<'a, T> {
    pub(crate) fn new(root: Option<&'a Node<T>>) -> Self {
        Chunks { leaves: LeafWalk::new(root, false) }
    }
}

impl<T> Clone for Chunks<'_, T> {
    fn clone(&self) -> Self {
        Chunks { leaves: self.leaves.clone() }
    }
}

impl<'a, T> Iterator for Chunks<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<&'a [T]> {
        self.leaves.next_leaf()
    }
}

impl<T> FusedIterator for Chunks<'_, T> {}

/// The elements of a list in order, by value. Made by `SharedVec::into_iter`.
pub struct IntoIter<T> {
    leaves: std::vec::IntoIter<Arc<Vec<T>>>,
    current: std::vec::IntoIter<T>,
    left: usize,
}

impl<T> IntoIter<T> {
    pub(crate) fn new(leaves: Vec<Arc<Vec<T>>>) -> Self {
        let left = leaves.iter().map(|leaf| leaf.len()).sum();
        IntoIter { leaves: leaves.into_iter(), current: Vec::new().into_iter(), left }
    }
}

impl<T: Clone> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        loop {
            if let Some(value) = self.current.next() {
                self.left -= 1;
                return Some(value);
            }
            self.current = node::unwrap_vec(self.leaves.next()?).into_iter();
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl<T: Clone> ExactSizeIterator for IntoIter<T> {}

impl<T: Clone> FusedIterator for IntoIter<T> {}

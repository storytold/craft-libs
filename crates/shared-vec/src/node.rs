//! The counted B-tree under [`SharedVec`](crate::SharedVec).
//!
//! Every leaf is at the same depth. A leaf holds up to [`MAX`] elements and a branch up to [`MAX`]
//! children, each with its length. Every node except the root holds at least [`MIN`]. Blocks sit
//! behind `Arc`, so a clone of the tree shares all of them, and an edit copies only the blocks on
//! its path (`Arc::make_mut`).

use std::sync::Arc;

/// The most elements in a leaf, and the most children in a branch.
pub(crate) const MAX: usize = 64;
/// The fewest elements or children in a node other than the root.
pub(crate) const MIN: usize = MAX / 2;

/// A run of leaves in order.
pub(crate) type Leaves<T> = Vec<Arc<Vec<T>>>;

pub(crate) enum Node<T> {
    Leaf(Arc<Vec<T>>),
    Branch(Arc<Branch<T>>),
}

impl<T> Clone for Node<T> {
    fn clone(&self) -> Self {
        match self {
            Node::Leaf(leaf) => Node::Leaf(Arc::clone(leaf)),
            Node::Branch(branch) => Node::Branch(Arc::clone(branch)),
        }
    }
}

pub(crate) struct Branch<T> {
    /// The number of elements under this branch.
    pub(crate) len: usize,
    /// The number of elements under each child.
    pub(crate) lens: Vec<usize>,
    pub(crate) children: Vec<Node<T>>,
}

impl<T> Clone for Branch<T> {
    fn clone(&self) -> Self {
        Branch { len: self.len, lens: self.lens.clone(), children: self.children.clone() }
    }
}

impl<T> Branch<T> {
    pub(crate) fn new(children: Vec<Node<T>>) -> Self {
        let lens: Vec<usize> = children.iter().map(Node::len).collect();
        Branch { len: lens.iter().sum(), lens, children }
    }
}

impl<T> Node<T> {
    /// The number of elements under this node.
    pub(crate) fn len(&self) -> usize {
        match self {
            Node::Leaf(leaf) => leaf.len(),
            Node::Branch(branch) => branch.len,
        }
    }

    /// The number of elements (a leaf) or children (a branch) in this node.
    fn slots(&self) -> usize {
        match self {
            Node::Leaf(leaf) => leaf.len(),
            Node::Branch(branch) => branch.children.len(),
        }
    }

    /// The address of this node's block, which two trees share exactly when they hold the same block.
    pub(crate) fn addr(&self) -> usize {
        match self {
            Node::Leaf(leaf) => Arc::as_ptr(leaf).cast::<()>() as usize,
            Node::Branch(branch) => Arc::as_ptr(branch).cast::<()>() as usize,
        }
    }

    /// The number of levels below this node: 0 for a leaf.
    pub(crate) fn height(&self) -> usize {
        let mut height = 0;
        let mut node = self;
        while let Node::Branch(branch) = node {
            let Some(first) = branch.children.first() else { break };
            node = first;
            height += 1;
        }
        height
    }

    pub(crate) fn get(&self, mut index: usize) -> Option<&T> {
        let mut node = self;
        loop {
            match node {
                Node::Leaf(leaf) => return leaf.get(index),
                Node::Branch(branch) => {
                    let (child, rest) = locate(&branch.lens, index)?;
                    node = branch.children.get(child)?;
                    index = rest;
                }
            }
        }
    }

    /// Folds the elements `[start, end)` under this node in order, a leaf slice at a time.
    pub(crate) fn fold_range<'a, B, F: FnMut(B, &'a T) -> B>(&'a self, start: usize, end: usize, acc: B, f: &mut F) -> B {
        match self {
            Node::Leaf(leaf) => leaf.get(start..end.min(leaf.len())).unwrap_or_default().iter().fold(acc, f),
            Node::Branch(branch) => {
                let mut acc = acc;
                let mut offset = 0;
                for (child, &len) in branch.children.iter().zip(&branch.lens) {
                    if offset >= end {
                        break;
                    }
                    if offset + len > start {
                        acc = child.fold_range(start.saturating_sub(offset), (end - offset).min(len), acc, f);
                    }
                    offset += len;
                }
                acc
            }
        }
    }

    /// Folds the elements `[start, end)` under this node from the back, a leaf slice at a time.
    pub(crate) fn rfold_range<'a, B, F: FnMut(B, &'a T) -> B>(&'a self, start: usize, end: usize, acc: B, f: &mut F) -> B {
        match self {
            Node::Leaf(leaf) => leaf.get(start..end.min(leaf.len())).unwrap_or_default().iter().rfold(acc, f),
            Node::Branch(branch) => {
                let mut acc = acc;
                let mut offset = branch.len;
                for (child, &len) in branch.children.iter().zip(&branch.lens).rev() {
                    offset -= len;
                    if offset + len <= start {
                        break;
                    }
                    if offset < end {
                        acc = child.rfold_range(start.saturating_sub(offset), (end - offset).min(len), acc, f);
                    }
                }
                acc
            }
        }
    }

    /// Appends every leaf under this node, in order.
    pub(crate) fn collect_leaves(&self, out: &mut Vec<Arc<Vec<T>>>) {
        match self {
            Node::Leaf(leaf) => {
                if !leaf.is_empty() {
                    out.push(Arc::clone(leaf));
                }
            }
            Node::Branch(branch) => {
                for child in &branch.children {
                    child.collect_leaves(out);
                }
            }
        }
    }
}

impl<T: Clone> Node<T> {
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        match self {
            Node::Leaf(leaf) => {
                if index >= leaf.len() {
                    return None;
                }
                Arc::make_mut(leaf).get_mut(index)
            }
            Node::Branch(branch) => {
                let (child, rest) = locate(&branch.lens, index)?;
                Arc::make_mut(branch).children.get_mut(child)?.get_mut(rest)
            }
        }
    }

    /// Inserts `value` at `index` (at most this node's length). Returns the new right sibling when
    /// the node had to split.
    pub(crate) fn insert(&mut self, index: usize, value: T) -> Option<Node<T>> {
        match self {
            Node::Leaf(leaf) => {
                // Grow towards one past a full leaf, never past it.
                let target = (leaf.len() * 2).clamp(4, MAX + 1).max(leaf.len() + 1);
                if Arc::get_mut(leaf).is_none() {
                    // A shared leaf is copied once, straight into a block with room for the new element.
                    let mut copy = Vec::with_capacity(target);
                    copy.extend_from_slice(leaf);
                    *leaf = Arc::new(copy);
                }
                let leaf = Arc::make_mut(leaf);
                if leaf.len() == leaf.capacity() {
                    leaf.reserve_exact(target - leaf.len());
                }
                leaf.insert(index.min(leaf.len()), value);
                if leaf.len() > MAX {
                    let right = leaf.split_off(leaf.len() / 2);
                    return Some(Node::Leaf(Arc::new(right)));
                }
                None
            }
            Node::Branch(branch) => {
                let branch = Arc::make_mut(branch);
                let (child, rest) = locate_insert(&branch.lens, index);
                let split = branch.children.get_mut(child)?.insert(rest, value);
                branch.len += 1;
                if let Some(right) = split {
                    if let Some(left) = branch.children.get(child) {
                        let left_len = left.len();
                        if let Some(slot) = branch.lens.get_mut(child) {
                            *slot = left_len;
                        }
                    }
                    branch.lens.insert(child + 1, right.len());
                    branch.children.insert(child + 1, right);
                } else if let Some(slot) = branch.lens.get_mut(child) {
                    *slot += 1;
                }
                if branch.children.len() > MAX {
                    let half = branch.children.len() / 2;
                    let children = branch.children.split_off(half);
                    let lens = branch.lens.split_off(half);
                    let moved: usize = lens.iter().sum();
                    branch.len -= moved;
                    return Some(Node::Branch(Arc::new(Branch { len: moved, lens, children })));
                }
                None
            }
        }
    }

    /// Removes the element at `index`. A child left below [`MIN`] is merged with a sibling or
    /// takes some of its slots; this node itself may be left below [`MIN`] for its parent to fix.
    pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
        match self {
            Node::Leaf(leaf) => {
                if index >= leaf.len() {
                    return None;
                }
                Some(Arc::make_mut(leaf).remove(index))
            }
            Node::Branch(branch) => {
                let (child, rest) = locate(&branch.lens, index)?;
                let branch = Arc::make_mut(branch);
                let removed = branch.children.get_mut(child)?.remove(rest)?;
                branch.len -= 1;
                if let Some(slot) = branch.lens.get_mut(child) {
                    *slot -= 1;
                }
                if branch.children.get(child).is_some_and(|c| c.slots() < MIN) {
                    rebalance(branch, child);
                }
                Some(removed)
            }
        }
    }
}

/// The child that holds `index`, and the position of `index` in it.
fn locate(lens: &[usize], mut index: usize) -> Option<(usize, usize)> {
    for (child, &len) in lens.iter().enumerate() {
        if index < len {
            return Some((child, index));
        }
        index -= len;
    }
    None
}

/// The child an insertion at `index` goes into: like [`locate`], except that the end of the last
/// child belongs to it.
fn locate_insert(lens: &[usize], mut index: usize) -> (usize, usize) {
    let last = lens.len().saturating_sub(1);
    for (child, &len) in lens.iter().enumerate() {
        if index < len || child == last {
            return (child, index.min(len));
        }
        index -= len;
    }
    (0, 0)
}

/// Brings `branch.children[child]` back to at least [`MIN`] slots by merging it with a neighbor,
/// or, when both together are too many for one node, by sharing their slots evenly.
fn rebalance<T: Clone>(branch: &mut Branch<T>, child: usize) {
    if branch.children.len() < 2 {
        return;
    }
    let left = if child > 0 { child - 1 } else { child };
    let right = left + 1;
    if right >= branch.children.len() {
        return;
    }
    let right_node = branch.children.remove(right);
    branch.lens.remove(right);
    let Some(left_node) = branch.children.get_mut(left) else { return };
    let spill = join_pair(left_node, right_node);
    let left_len = left_node.len();
    if let Some(slot) = branch.lens.get_mut(left) {
        *slot = left_len;
    }
    if let Some(spill) = spill {
        branch.lens.insert(right, spill.len());
        branch.children.insert(right, spill);
    }
}

/// Moves the slots of `right` onto the end of `left`, two nodes of the same height. When that is
/// more than one node holds, splits the slots in half and returns the right half.
fn join_pair<T: Clone>(left: &mut Node<T>, right: Node<T>) -> Option<Node<T>> {
    match (left, right) {
        (Node::Leaf(left), Node::Leaf(right)) => {
            let left = Arc::make_mut(left);
            left.extend(unwrap_vec(right));
            (left.len() > MAX).then(|| {
                let half = left.len() / 2;
                Node::Leaf(Arc::new(left.split_off(half)))
            })
        }
        (Node::Branch(left), Node::Branch(right)) => {
            let left = Arc::make_mut(left);
            let right = Arc::try_unwrap(right).unwrap_or_else(|shared| (*shared).clone());
            left.len += right.len;
            left.lens.extend(right.lens);
            left.children.extend(right.children);
            (left.children.len() > MAX).then(|| {
                let half = left.children.len() / 2;
                let children = left.children.split_off(half);
                let lens = left.lens.split_off(half);
                let moved: usize = lens.iter().sum();
                left.len -= moved;
                Node::Branch(Arc::new(Branch { len: moved, lens, children }))
            })
        }
        // The two heights always match; a mismatch keeps both nodes as they are.
        (_, right) => Some(right),
    }
}

/// The elements of a leaf: moved out when no other tree holds it, cloned otherwise.
pub(crate) fn unwrap_vec<T: Clone>(leaf: Arc<Vec<T>>) -> Vec<T> {
    Arc::try_unwrap(leaf).unwrap_or_else(|shared| (*shared).clone())
}

/// Brings a run of leaves to [`MIN`]..=[`MAX`] elements each, keeping (sharing) every leaf that is
/// already in range and not next to one that is too small.
fn normalize<T: Clone>(leaves: Vec<Arc<Vec<T>>>) -> Vec<Arc<Vec<T>>> {
    let mut out: Vec<Arc<Vec<T>>> = Vec::with_capacity(leaves.len());
    let mut pending: Vec<T> = Vec::new();
    for leaf in leaves {
        if leaf.is_empty() {
            continue;
        }
        if pending.is_empty() && (MIN..=MAX).contains(&leaf.len()) {
            out.push(leaf);
            continue;
        }
        pending.extend(unwrap_vec(leaf));
        while pending.len() > MAX + MIN {
            let rest = pending.split_off(MAX);
            out.push(Arc::new(std::mem::replace(&mut pending, rest)));
        }
        if pending.len() >= MIN {
            push_split(&mut out, std::mem::take(&mut pending));
        }
    }
    if !pending.is_empty() {
        match out.pop() {
            Some(last) => {
                let mut merged = unwrap_vec(last);
                merged.extend(pending);
                push_split(&mut out, merged);
            }
            None => out.push(Arc::new(pending)),
        }
    }
    out
}

/// Pushes `elements` (at most `MAX + MIN` of them, at least `MIN` unless it is the only leaf) as one
/// leaf, or as two halves when that is more than a leaf holds.
fn push_split<T>(out: &mut Vec<Arc<Vec<T>>>, mut elements: Vec<T>) {
    if elements.len() > MAX {
        let right = elements.split_off(elements.len() / 2);
        out.push(Arc::new(elements));
        out.push(Arc::new(right));
    } else {
        out.push(Arc::new(elements));
    }
}

/// Builds a balanced tree from a run of leaves in order, or `None` when they hold no element.
pub(crate) fn build<T: Clone>(leaves: Vec<Arc<Vec<T>>>) -> Option<Node<T>> {
    let mut level: Vec<Node<T>> = normalize(leaves).into_iter().map(Node::Leaf).collect();
    while level.len() > 1 {
        let count = level.len();
        let groups = count.div_ceil(MAX);
        let mut next = Vec::with_capacity(groups);
        let mut nodes = level.into_iter();
        for group in 0..groups {
            let size = count / groups + usize::from(group < count % groups);
            next.push(Node::Branch(Arc::new(Branch::new(nodes.by_ref().take(size).collect()))));
        }
        level = next;
    }
    level.pop()
}

/// Checks the tree's shape: lengths add up, every node except the root holds [`MIN`]..=[`MAX`]
/// slots, a root branch holds at least two children, every leaf is at the same depth, and an empty
/// list has no root.
#[cfg(test)]
pub(crate) fn check<T>(root: &Option<Node<T>>) -> Result<(), String> {
    fn walk<T>(node: &Node<T>, is_root: bool, depth: usize, leaf_depth: &mut Option<usize>) -> Result<usize, String> {
        let slots = node.slots();
        let low = match (is_root, node) {
            (true, Node::Leaf(_)) => 0,
            (true, Node::Branch(_)) => 2,
            (false, _) => MIN,
        };
        if slots < low || slots > MAX {
            return Err(format!("{slots} slots at depth {depth}"));
        }
        match node {
            Node::Leaf(leaf) => {
                if leaf_depth.get_or_insert(depth) != &depth {
                    return Err(format!("leaf at depth {depth}, others at {leaf_depth:?}"));
                }
                Ok(leaf.len())
            }
            Node::Branch(branch) => {
                if branch.lens.len() != branch.children.len() {
                    return Err("lens and children differ in count".into());
                }
                let mut sum = 0;
                for (child, &len) in branch.children.iter().zip(&branch.lens) {
                    let actual = walk(child, false, depth + 1, leaf_depth)?;
                    if actual != len {
                        return Err(format!("child length {actual}, recorded {len}"));
                    }
                    sum += actual;
                }
                if sum != branch.len {
                    return Err(format!("branch length {}, children hold {sum}", branch.len));
                }
                Ok(sum)
            }
        }
    }
    match root {
        Some(root) if root.len() == 0 => Err("an empty root".into()),
        Some(root) => walk(root, true, 0, &mut None).map(|_| ()),
        None => Ok(()),
    }
}

/// Cuts a run of elements into full leaves.
pub(crate) fn chunk<T, I: IntoIterator<Item = T>>(items: I, out: &mut Vec<Arc<Vec<T>>>) {
    let mut items = items.into_iter();
    loop {
        let leaf: Vec<T> = items.by_ref().take(MAX).collect();
        if leaf.is_empty() {
            break;
        }
        let full = leaf.len() == MAX;
        out.push(Arc::new(leaf));
        if !full {
            break;
        }
    }
}

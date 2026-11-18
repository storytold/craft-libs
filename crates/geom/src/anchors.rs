//! The anchors of a subpath, held in shared chunks.
//!
//! [`Anchors`] keeps its anchors in chunks of at most [`CHUNK`] anchors, each behind an `Arc`.
//! Copying the list copies the table of chunks and shares the chunks themselves; changing an
//! anchor copies only the chunk that holds it when another copy still shares that chunk. An undo
//! step that keeps the version of a path before a one-anchor edit then keeps one chunk of its
//! own, however many anchors the path has. Inserting or removing an anchor copies the chunk it is
//! in and rebuilds the table, which has one entry per chunk.
//!
//! The list reads and writes as one flat sequence of anchors, as a `Vec<Anchor>` does.

use std::borrow::Cow;
use std::fmt;
use std::ops::{Index, IndexMut, RangeBounds};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::path::Anchor;

/// An [`Anchors`] list of the anchors given, as `vec!` makes a `Vec`.
#[macro_export]
macro_rules! anchors {
    ($($t:tt)*) => {
        $crate::Anchors::from(vec![$($t)*])
    };
}

/// Most anchors one chunk holds.
pub const CHUNK: usize = 256;

/// The anchors of a subpath: a sequence with the reads of a slice and the edits of a `Vec`, whose
/// copies share the chunks they have not changed.
#[derive(Clone, Default)]
pub struct Anchors {
    /// The chunks in order, none of them empty.
    chunks: Vec<Arc<Vec<Anchor>>>,
    /// Empty while every chunk but the last holds [`CHUNK`] anchors (anchor `i` is then in chunk
    /// `i / CHUNK`). Otherwise `ends[c]` is the number of anchors in chunks `0..=c`.
    ends: Vec<usize>,
    /// The number of anchors.
    len: usize,
}

impl Anchors {
    /// An empty list.
    pub const fn new() -> Self {
        Self { chunks: Vec::new(), ends: Vec::new(), len: 0 }
    }

    /// The number of anchors.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the list has no anchors.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The chunk that holds anchor `i` and its place in that chunk. For `i` at or past the end the
    /// chunk or the place is out of range.
    fn locate(&self, i: usize) -> (usize, usize) {
        if self.ends.is_empty() {
            (i / CHUNK, i % CHUNK)
        } else {
            let c = self.ends.partition_point(|&e| e <= i);
            let start = c.checked_sub(1).and_then(|p| self.ends.get(p)).copied().unwrap_or(0);
            (c, i.saturating_sub(start))
        }
    }

    /// Anchor `i`, or `None` past the end.
    pub fn get(&self, i: usize) -> Option<&Anchor> {
        if i >= self.len {
            return None;
        }
        let (c, o) = self.locate(i);
        self.chunks.get(c)?.get(o)
    }

    /// Anchor `i` to change, or `None` past the end. Copies the chunk that holds it when another
    /// list shares that chunk.
    pub fn get_mut(&mut self, i: usize) -> Option<&mut Anchor> {
        if i >= self.len {
            return None;
        }
        let (c, o) = self.locate(i);
        Arc::make_mut(self.chunks.get_mut(c)?).get_mut(o)
    }

    /// The first anchor.
    pub fn first(&self) -> Option<&Anchor> {
        self.chunks.first()?.first()
    }

    /// The last anchor.
    pub fn last(&self) -> Option<&Anchor> {
        self.chunks.last()?.last()
    }

    /// The first anchor to change.
    pub fn first_mut(&mut self) -> Option<&mut Anchor> {
        Arc::make_mut(self.chunks.first_mut()?).first_mut()
    }

    /// The last anchor to change.
    pub fn last_mut(&mut self) -> Option<&mut Anchor> {
        Arc::make_mut(self.chunks.last_mut()?).last_mut()
    }

    /// The anchors in order.
    pub fn iter(&self) -> Iter<'_> {
        let (front, back): (&[Anchor], &[Anchor]) = (&[], &[]);
        Iter { chunks: self.chunks.iter(), front: front.iter(), back: back.iter(), left: self.len }
    }

    /// The anchors in order, to change. Each chunk is copied, when another list shares it, as the
    /// iterator reaches it.
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        let chunk: fn(&mut Arc<Vec<Anchor>>) -> std::slice::IterMut<'_, Anchor> = |c| Arc::make_mut(c).iter_mut();
        self.chunks.iter_mut().flat_map(chunk)
    }

    /// The anchors as one slice: borrowed when they are in one chunk, copied otherwise.
    pub fn contiguous(&self) -> Cow<'_, [Anchor]> {
        match self.chunks.as_slice() {
            [] => Cow::Borrowed(&[]),
            [one] => Cow::Borrowed(one.as_slice()),
            _ => Cow::Owned(self.to_vec()),
        }
    }

    /// The anchors in `range` copied into a `Vec`, or `None` when the range is out of bounds or
    /// starts after it ends (as `slice::get` with a range).
    pub fn get_range(&self, range: impl RangeBounds<usize>) -> Option<Vec<Anchor>> {
        use std::ops::Bound;
        let start = match range.start_bound() {
            Bound::Included(&s) => s,
            Bound::Excluded(&s) => s.checked_add(1)?,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&e) => e.checked_add(1)?,
            Bound::Excluded(&e) => e,
            Bound::Unbounded => self.len,
        };
        (start <= end && end <= self.len).then(|| self.iter().skip(start).take(end - start).copied().collect())
    }

    /// The anchors copied into a `Vec`.
    pub fn to_vec(&self) -> Vec<Anchor> {
        let mut v = Vec::with_capacity(self.len);
        for c in &self.chunks {
            v.extend_from_slice(c);
        }
        v
    }

    /// The anchors as a `Vec`, without a copy when they are in one chunk no other list shares.
    pub fn into_vec(mut self) -> Vec<Anchor> {
        if self.chunks.len() == 1
            && let Some(one) = self.chunks.pop()
        {
            return Arc::try_unwrap(one).unwrap_or_else(|shared| (*shared).clone());
        }
        self.to_vec()
    }

    /// Change the anchors as a `Vec` (for edits that move many of them, such as reversing).
    pub fn with_vec<R>(&mut self, f: impl FnOnce(&mut Vec<Anchor>) -> R) -> R {
        let mut v = std::mem::take(self).into_vec();
        let r = f(&mut v);
        *self = Self::from(v);
        r
    }

    /// Rebuild `ends` and `len` after the chunks changed, dropping empty chunks.
    fn reindex(&mut self) {
        self.chunks.retain(|c| !c.is_empty());
        self.len = self.chunks.iter().map(|c| c.len()).sum();
        let n = self.chunks.len();
        let uniform = self.chunks.iter().enumerate().all(|(k, c)| if k + 1 == n { c.len() <= CHUNK } else { c.len() == CHUNK });
        self.ends.clear();
        if !uniform {
            let mut total = 0;
            self.ends.extend(self.chunks.iter().map(|c| {
                total += c.len();
                total
            }));
        }
    }

    /// Add `a` at the end.
    pub fn push(&mut self, a: Anchor) {
        match self.chunks.last_mut() {
            Some(last) if last.len() < CHUNK => Arc::make_mut(last).push(a),
            _ => {
                self.chunks.push(Arc::new(vec![a]));
                if !self.ends.is_empty() {
                    self.ends.push(self.len);
                }
            }
        }
        self.len += 1;
        if let Some(end) = self.ends.last_mut() {
            *end = self.len;
        }
    }

    /// Remove the last anchor.
    pub fn pop(&mut self) -> Option<Anchor> {
        let last = self.chunks.last_mut()?;
        let a = Arc::make_mut(last).pop();
        if last.is_empty() {
            self.chunks.pop();
            self.ends.pop();
        }
        self.len = self.len.saturating_sub(1);
        if let Some(end) = self.ends.last_mut() {
            *end = self.len;
        }
        a
    }

    /// Insert `a` before anchor `i`, or at the end when `i` is the length or more.
    pub fn insert(&mut self, i: usize, a: Anchor) {
        if i >= self.len {
            self.push(a);
            return;
        }
        let (c, o) = self.locate(i);
        let Some(chunk) = self.chunks.get_mut(c) else { return };
        let chunk = Arc::make_mut(chunk);
        chunk.insert(o.min(chunk.len()), a);
        if chunk.len() > CHUNK {
            let tail = chunk.split_off(chunk.len() / 2);
            self.chunks.insert(c + 1, Arc::new(tail));
        }
        self.reindex();
    }

    /// Remove anchor `i`, or `None` past the end.
    pub fn try_remove(&mut self, i: usize) -> Option<Anchor> {
        if i >= self.len {
            return None;
        }
        let (c, o) = self.locate(i);
        let chunk = Arc::make_mut(self.chunks.get_mut(c)?);
        let a = (o < chunk.len()).then(|| chunk.remove(o));
        // A chunk left small joins the next one when the two fit in one chunk.
        if chunk.len() < CHUNK / 4
            && let Some(next) = self.chunks.get(c + 1)
            && self.chunks.get(c).map_or(0, |c| c.len()) + next.len() <= CHUNK
        {
            let next = self.chunks.remove(c + 1);
            if let Some(chunk) = self.chunks.get_mut(c) {
                Arc::make_mut(chunk).extend_from_slice(&next);
            }
        }
        self.reindex();
        a
    }

    /// Remove anchor `i`. Panics past the end, as `Vec::remove` does.
    pub fn remove(&mut self, i: usize) -> Anchor {
        match self.try_remove(i) {
            Some(a) => a,
            None => {
                let none: &[Anchor] = &[];
                none[i]
            }
        }
    }

    /// Keep the first `n` anchors.
    pub fn truncate(&mut self, n: usize) {
        if n >= self.len {
            return;
        }
        let (c, o) = self.locate(n);
        self.chunks.truncate(c + 1);
        if let Some(last) = self.chunks.last_mut() {
            Arc::make_mut(last).truncate(o);
        }
        self.reindex();
    }

    /// Remove every anchor.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Add the anchors of `s` at the end.
    pub fn extend_from_slice(&mut self, s: &[Anchor]) {
        self.extend(s.iter().copied());
    }

    /// Keep the anchors for which `f` is true.
    pub fn retain(&mut self, f: impl FnMut(&Anchor) -> bool) {
        self.with_vec(|v| v.retain(f));
    }

    /// Keep the anchors for which `f` is true, letting `f` change them.
    pub fn retain_mut(&mut self, f: impl FnMut(&mut Anchor) -> bool) {
        self.with_vec(|v| v.retain_mut(f));
    }

    /// Reverse the order of the anchors.
    pub fn reverse(&mut self) {
        self.with_vec(|v| v.reverse());
    }

    /// Rotate the anchors `k` places towards the start.
    pub fn rotate_left(&mut self, k: usize) {
        self.with_vec(|v| v.rotate_left(k));
    }

    /// Rotate the anchors `k` places towards the end.
    pub fn rotate_right(&mut self, k: usize) {
        self.with_vec(|v| v.rotate_right(k));
    }

    /// Remove the anchors in `range` and return them.
    pub fn drain(&mut self, range: impl RangeBounds<usize>) -> std::vec::IntoIter<Anchor> {
        self.with_vec(|v| v.drain(range).collect::<Vec<_>>()).into_iter()
    }

    /// Replace the anchors in `range` with those of `with`, and return the anchors replaced.
    pub fn splice(&mut self, range: impl RangeBounds<usize>, with: impl IntoIterator<Item = Anchor>) -> std::vec::IntoIter<Anchor> {
        self.with_vec(|v| v.splice(range, with).collect::<Vec<_>>()).into_iter()
    }

    /// The pairs of anchors in the same place in `self` and `other`, with that place, in order.
    /// When the two are chunked alike, the chunks they share in the same place are skipped (their
    /// anchors are the same); otherwise every pair is visited. Stops at the end of the shorter list.
    pub fn pairs_not_shared<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = (usize, &'a Anchor, &'a Anchor)> + 'a {
        let alike = self.len == other.len && self.chunks.len() == other.chunks.len() && self.ends == other.ends;
        let chunked = alike.then(|| {
            let starts = self.chunks.iter().scan(0, |start, c| {
                let at = *start;
                *start += c.len();
                Some(at)
            });
            starts
                .zip(self.chunks.iter().zip(&other.chunks))
                .filter(|(_, (a, b))| !Arc::ptr_eq(a, b))
                .flat_map(|(at, (a, b))| a.iter().zip(b.iter()).enumerate().map(move |(o, (x, y))| (at + o, x, y)))
        });
        let flat = (!alike).then(|| self.iter().zip(other.iter()).enumerate().map(|(i, (a, b))| (i, a, b)));
        chunked.into_iter().flatten().chain(flat.into_iter().flatten())
    }

    /// Each chunk's address and capacity (in anchors): two lists hold a chunk in common when they
    /// give the same address. For memory estimates.
    pub fn chunk_allocations(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.chunks.iter().map(|c| (Arc::as_ptr(c) as usize, c.capacity()))
    }

    /// The bytes the list's own tables hold on the heap (the chunk table and the running totals),
    /// without the chunks.
    pub fn table_bytes(&self) -> usize {
        self.chunks.capacity() * size_of::<Arc<Vec<Anchor>>>() + self.ends.capacity() * size_of::<usize>()
    }
}

/// [`Anchors::iter_mut`]'s iterator.
pub type IterMut<'a> = std::iter::FlatMap<
    std::slice::IterMut<'a, Arc<Vec<Anchor>>>,
    std::slice::IterMut<'a, Anchor>,
    fn(&mut Arc<Vec<Anchor>>) -> std::slice::IterMut<'_, Anchor>,
>;

/// [`Anchors::iter`]'s iterator.
#[derive(Clone)]
pub struct Iter<'a> {
    /// The chunks neither end has reached.
    chunks: std::slice::Iter<'a, Arc<Vec<Anchor>>>,
    /// What is left of the chunk the front is in.
    front: std::slice::Iter<'a, Anchor>,
    /// What is left of the chunk the back is in.
    back: std::slice::Iter<'a, Anchor>,
    /// The anchors left.
    left: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Anchor;

    fn next(&mut self) -> Option<&'a Anchor> {
        loop {
            if let Some(a) = self.front.next() {
                self.left -= 1;
                return Some(a);
            }
            match self.chunks.next() {
                Some(c) => self.front = c.iter(),
                None => {
                    let a = self.back.next()?;
                    self.left -= 1;
                    return Some(a);
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }

    fn nth(&mut self, mut n: usize) -> Option<&'a Anchor> {
        // Whole chunks are skipped without visiting their anchors.
        loop {
            let f = self.front.len();
            if n < f {
                self.left -= n + 1;
                return self.front.nth(n);
            }
            n -= f;
            self.left -= f;
            self.front = Default::default();
            match self.chunks.as_slice().first() {
                Some(c) if c.len() <= n => {
                    n -= c.len();
                    self.left -= c.len();
                    self.chunks.next();
                }
                Some(_) => {
                    if let Some(c) = self.chunks.next() {
                        self.front = c.iter();
                    }
                }
                None => {
                    let b = self.back.len();
                    if n < b {
                        self.left -= n + 1;
                        return self.back.nth(n);
                    }
                    self.left -= b;
                    self.back = Default::default();
                    return None;
                }
            }
        }
    }

    fn count(self) -> usize {
        self.left
    }

    fn last(mut self) -> Option<&'a Anchor> {
        self.next_back()
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    fn next_back(&mut self) -> Option<&'a Anchor> {
        loop {
            if let Some(a) = self.back.next_back() {
                self.left -= 1;
                return Some(a);
            }
            match self.chunks.next_back() {
                Some(c) => self.back = c.iter(),
                None => {
                    let a = self.front.next_back()?;
                    self.left -= 1;
                    return Some(a);
                }
            }
        }
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl std::iter::FusedIterator for Iter<'_> {}

impl Index<usize> for Anchors {
    type Output = Anchor;

    /// Anchor `i`. Panics past the end, as indexing a slice does.
    fn index(&self, i: usize) -> &Anchor {
        let (c, o) = self.locate(i);
        &self.chunks[c][o]
    }
}

impl IndexMut<usize> for Anchors {
    /// Anchor `i` to change ([`Anchors::get_mut`]). Panics past the end, as indexing a slice does.
    fn index_mut(&mut self, i: usize) -> &mut Anchor {
        let (c, o) = self.locate(i);
        &mut Arc::make_mut(&mut self.chunks[c])[o]
    }
}

impl From<Vec<Anchor>> for Anchors {
    fn from(v: Vec<Anchor>) -> Self {
        if v.is_empty() {
            return Self::new();
        }
        let len = v.len();
        let chunks = if len <= CHUNK { vec![Arc::new(v)] } else { v.chunks(CHUNK).map(|c| Arc::new(c.to_vec())).collect() };
        Self { chunks, ends: Vec::new(), len }
    }
}

impl From<&[Anchor]> for Anchors {
    fn from(s: &[Anchor]) -> Self {
        Self::from(s.to_vec())
    }
}

impl<const N: usize> From<[Anchor; N]> for Anchors {
    fn from(s: [Anchor; N]) -> Self {
        Self::from(Vec::from(s))
    }
}

impl From<Anchors> for Vec<Anchor> {
    fn from(a: Anchors) -> Self {
        a.into_vec()
    }
}

impl FromIterator<Anchor> for Anchors {
    fn from_iter<I: IntoIterator<Item = Anchor>>(it: I) -> Self {
        Self::from(it.into_iter().collect::<Vec<_>>())
    }
}

impl Extend<Anchor> for Anchors {
    fn extend<I: IntoIterator<Item = Anchor>>(&mut self, it: I) {
        for a in it {
            self.push(a);
        }
    }
}

impl<'a> Extend<&'a Anchor> for Anchors {
    fn extend<I: IntoIterator<Item = &'a Anchor>>(&mut self, it: I) {
        self.extend(it.into_iter().copied());
    }
}

impl<'a> IntoIterator for &'a Anchors {
    type Item = &'a Anchor;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Anchors {
    type Item = &'a mut Anchor;
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        self.iter_mut()
    }
}

impl IntoIterator for Anchors {
    type Item = Anchor;
    type IntoIter = std::vec::IntoIter<Anchor>;

    fn into_iter(self) -> std::vec::IntoIter<Anchor> {
        self.into_vec().into_iter()
    }
}

impl PartialEq for Anchors {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl PartialEq<[Anchor]> for Anchors {
    fn eq(&self, other: &[Anchor]) -> bool {
        self.len == other.len() && self.iter().eq(other.iter())
    }
}

impl PartialEq<&[Anchor]> for Anchors {
    fn eq(&self, other: &&[Anchor]) -> bool {
        *self == **other
    }
}

impl PartialEq<Vec<Anchor>> for Anchors {
    fn eq(&self, other: &Vec<Anchor>) -> bool {
        *self == **other
    }
}

impl<const N: usize> PartialEq<[Anchor; N]> for Anchors {
    fn eq(&self, other: &[Anchor; N]) -> bool {
        *self == other[..]
    }
}

impl PartialEq<Anchors> for [Anchor] {
    fn eq(&self, other: &Anchors) -> bool {
        *other == *self
    }
}

impl PartialEq<Anchors> for Vec<Anchor> {
    fn eq(&self, other: &Anchors) -> bool {
        *other == **self
    }
}

impl fmt::Debug for Anchors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Serialize for Anchors {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Anchors {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::<Anchor>::deserialize(d).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use kurbo::Point;

    use super::*;

    fn line(n: usize) -> Vec<Anchor> {
        (0..n).map(|i| Anchor::corner(Point::new(i as f64, (i % 7) as f64))).collect()
    }

    /// Chunks of `a` that `b` does not hold, with their capacity in anchors.
    fn own_chunks(a: &Anchors, b: &Anchors) -> Vec<usize> {
        let theirs: Vec<usize> = b.chunk_allocations().map(|(id, _)| id).collect();
        a.chunk_allocations().filter(|(id, _)| !theirs.contains(id)).map(|(_, cap)| cap).collect()
    }

    #[test]
    fn editing_one_anchor_copies_one_chunk_whatever_the_length() {
        for n in [10_000, 200_000] {
            let before = Anchors::from(line(n));
            let mut after = before.clone();
            after[n / 2].p = Point::new(-1.0, -1.0);
            if let Some(a) = after.get_mut(n / 2 + 1) {
                a.h_out = Point::new(-2.0, -2.0);
            }
            let copied = own_chunks(&after, &before);
            assert_eq!(copied.len(), 1, "{n} anchors: one chunk copied");
            assert!(copied.iter().all(|&cap| cap <= CHUNK), "{n} anchors: {copied:?}");
            assert_eq!(own_chunks(&before, &after).len(), 1, "{n} anchors: the old version keeps one chunk of its own");
            // The old version is as it was; the new one differs only at the two anchors.
            assert_eq!(before, line(n));
            let mut expected = line(n);
            expected[n / 2].p = Point::new(-1.0, -1.0);
            expected[n / 2 + 1].h_out = Point::new(-2.0, -2.0);
            assert_eq!(after, expected);
        }
    }

    #[test]
    fn inserting_and_removing_copy_only_the_chunk_touched() {
        let n = 200_000;
        let before = Anchors::from(line(n));
        let mut after = before.clone();
        let extra = Anchor::corner(Point::new(0.5, 0.5));
        after.insert(n / 2, extra);
        assert!(own_chunks(&after, &before).len() <= 2, "the chunk split in two");
        assert_eq!(after.len(), n + 1);
        assert_eq!(after[n / 2], extra);
        assert_eq!(after[n / 2 + 1], line(n)[n / 2]);
        assert_eq!(after.remove(n / 2), extra);
        assert_eq!(after, line(n));
        assert!(own_chunks(&after, &before).len() <= 2);
        assert_eq!(before, line(n));
    }

    #[test]
    fn acts_as_a_vec_through_edits() {
        // A run of edits applied to an `Anchors` and to a `Vec` leaves the two the same, across
        // chunk splits, joins and the uniform and non-uniform layouts.
        let mut v = line(700);
        let mut a = Anchors::from(v.clone());
        let mut seed = 7u64;
        let mut next = |m: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % m.max(1)
        };
        for step in 0..3000 {
            let x = Anchor::corner(Point::new(step as f64, 0.25));
            match next(8) {
                0 | 1 => {
                    let i = next(v.len() + 1);
                    v.insert(i, x);
                    a.insert(i, x);
                }
                2 | 3 if !v.is_empty() => {
                    let i = next(v.len());
                    assert_eq!(v.remove(i), a.remove(i));
                }
                4 => {
                    v.push(x);
                    a.push(x);
                }
                5 => assert_eq!(v.pop(), a.pop()),
                6 if !v.is_empty() => {
                    let i = next(v.len());
                    v[i].h_in = x.p;
                    a[i].h_in = x.p;
                }
                _ => {
                    let keep = v.len().saturating_sub(next(3));
                    v.truncate(keep);
                    a.truncate(keep);
                }
            }
            assert_eq!(a.len(), v.len(), "step {step}");
            if step % 50 == 0 {
                assert_eq!(a, v, "step {step}");
                for i in 0..v.len() {
                    assert_eq!(a.get(i), v.get(i), "step {step}, anchor {i}");
                }
            }
        }
        assert_eq!(a, v);
        assert_eq!(a.iter().rev().copied().collect::<Vec<_>>(), v.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(a.get(v.len()), None);
    }

    #[test]
    fn iterators_skip_and_count_as_a_slice_does() {
        let v = line(1000);
        let a = Anchors::from(v.clone());
        for skip in [0, 1, 255, 256, 257, 999, 1000, 1001] {
            assert_eq!(a.iter().nth(skip), v.get(skip), "nth {skip}");
            assert_eq!(a.iter().skip(skip).count(), v.iter().skip(skip).count(), "skip {skip}");
            assert_eq!(a.iter().skip(skip).len(), v.iter().skip(skip).len(), "len after {skip}");
        }
        let mut both = a.iter();
        let mut vs = v.iter();
        for k in 0..1000 {
            let (x, y) = if k % 3 == 0 { (both.next_back(), vs.next_back()) } else { (both.next(), vs.next()) };
            assert_eq!(x, y, "step {k}");
            assert_eq!(both.len(), vs.len());
        }
        assert_eq!(both.next(), None);
        assert_eq!(a.iter().last(), v.last());
        let mut m = a.clone();
        m.iter_mut().for_each(|x| x.p.x += 1.0);
        assert!(m.iter().zip(&v).all(|(x, y)| x.p.x == y.p.x + 1.0));
        assert_eq!(a, v, "changing a copy left the original alone");
    }

    #[test]
    fn reads_and_writes_as_a_flat_list() {
        let v = line(600);
        let a = Anchors::from(v.clone());
        assert_eq!(serde_json::to_string(&a).ok(), serde_json::to_string(&v).ok());
        let back: Anchors = serde_json::from_str(&serde_json::to_string(&v).unwrap_or_default()).unwrap_or_default();
        assert_eq!(back, v);
        assert_eq!(format!("{a:?}"), format!("{v:?}"));
    }

    #[test]
    fn pairs_not_shared_skip_the_chunks_both_hold() {
        let before = Anchors::from(line(10_000));
        let mut after = before.clone();
        after[5000].p = Point::new(-3.0, 0.0);
        let pairs: Vec<_> = before.pairs_not_shared(&after).collect();
        assert!(pairs.len() <= CHUNK);
        let changed: Vec<usize> = pairs.iter().filter(|(_, a, b)| a != b).map(|(i, _, _)| *i).collect();
        assert_eq!(changed, [5000]);
        assert!(pairs.iter().all(|&(i, a, b)| *a == before[i] && *b == after[i]));
        // Chunked differently: every pair.
        let mut moved = before.clone();
        moved.insert(0, Anchor::corner(Point::ZERO));
        moved.remove(0);
        assert_eq!(before.pairs_not_shared(&moved).count(), 10_000);
    }
}

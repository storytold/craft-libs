//! [`ViewHistory`]: Go Back and Go Forward through the places a window has jumped between, in the
//! order it showed them, like a browser's Back and Forward (InDesign's Layout › Go Back / Go
//! Forward). Session state, not document history.

use kurbo::Point;

/// Where a view stood: what it showed (`key`: a page, an artboard, a document) and the view's
/// centre and zoom there.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ViewPlace<K> {
    /// What the view showed.
    pub key: K,
    /// The document point at the canvas's centre.
    pub center: Point,
    /// Screen points per document unit ([`CanvasView::zoom`](super::CanvasView::zoom)).
    pub zoom: f64,
}

impl<K> ViewPlace<K> {
    /// The view of `key` centred on `center` at `zoom`.
    pub const fn new(key: K, center: Point, zoom: f64) -> Self {
        Self { key, center, zoom }
    }
}

/// The places a window jumped between, oldest first, with the one it shows now. The app records
/// a jump (a page picked in the Pages panel, Go to Page, a link followed) with
/// [`leave`](Self::leave) before it and [`visit`](Self::visit) after; scrolling within a place is
/// not a jump. Bounded to [`LIMIT`](Self::LIMIT) entries.
///
/// ```
/// use craft_behavior::canvas::{ViewHistory, ViewPlace};
/// use kurbo::Point;
/// let mut history = ViewHistory::default();
/// let at = |key| ViewPlace::new(key, Point::ZERO, 1.0);
/// history.visit(at(1));
/// history.leave(at(1));
/// history.visit(at(2));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ViewHistory<K> {
    entries: Vec<ViewPlace<K>>,
    at: usize,
}

impl<K> Default for ViewHistory<K> {
    fn default() -> Self {
        Self { entries: Vec::new(), at: 0 }
    }
}

impl<K: Clone + PartialEq> ViewHistory<K> {
    /// Entries kept; the oldest go first.
    pub const LIMIT: usize = 256;

    /// The view is about to jump away from `place`: its entry keeps the centre and zoom the
    /// jump left it at, so Go Back returns there.
    pub fn leave(&mut self, place: ViewPlace<K>) {
        match self.entries.get_mut(self.at) {
            Some(entry) => *entry = place,
            None => self.visit(place),
        }
    }

    /// The view jumped to `place`. Visiting after Go Back drops the Go Forward entries; visiting
    /// the place already current only updates it.
    pub fn visit(&mut self, place: ViewPlace<K>) {
        if let Some(entry) = self.entries.get_mut(self.at)
            && entry.key == place.key
        {
            *entry = place;
            return;
        }
        self.entries.truncate(self.at.saturating_add(1));
        self.entries.push(place);
        if self.entries.len() > Self::LIMIT {
            self.entries.remove(0);
        }
        self.at = self.entries.len() - 1;
    }

    /// The nearest entry in the direction of `step` whose key still exists and differs from the
    /// current one.
    fn target(&self, step: isize, exists: impl Fn(&K) -> bool) -> Option<usize> {
        let current = self.entries.get(self.at).map(|e| &e.key);
        let usable = |i: &usize| self.entries.get(*i).is_some_and(|e| Some(&e.key) != current && exists(&e.key));
        if step < 0 { (0..self.at).rev().find(usable) } else { (self.at.saturating_add(1)..self.entries.len()).find(usable) }
    }

    /// Go Back or Go Forward: the place to show, skipping places that no longer exist. Going
    /// does not record the place it leaves.
    pub fn go(&mut self, step: isize, exists: impl Fn(&K) -> bool) -> Option<ViewPlace<K>> {
        let i = self.target(step, exists)?;
        self.at = i;
        self.entries.get(i).cloned()
    }
}

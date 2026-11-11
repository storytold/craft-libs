//! An outline being drawn by a selection or freeform tool, and how it closes: [`OutlineDraft`].
//!
//! The Lasso, the Polygonal Lasso and the Freeform Pen, Select and Mask's Polygonal Lasso, and
//! Content-Aware Fill's and Liquify's lassos share this one state machine. It takes plain input
//! (document points, the modifiers, the zoom), so a canvas tool feeds it from its pointer and key
//! calls and paints [`points`](OutlineDraft::points) itself.

use craft_keymap::Mods;
use kurbo::Point;

use super::lock::{Directions, round_to_direction};

/// Which tool's rules an [`OutlineDraft`] follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DraftKind {
    /// A drag draws and the release closes; there are no straight segments and ⌥ changes nothing
    /// (Content-Aware Fill's and Liquify's Freeze lassos).
    Freehand,
    /// The Lasso and the Freeform Pen (Photoshop's): a drag draws freehand; ⌥ held after the press
    /// switches to straight segments, where each click adds a vertex and a drag draws freehand
    /// again; releasing ⌥ with the button up closes. ⌥ held at the press means subtract (the
    /// tool's operation), not straight segments, until it is released and pressed again; with
    /// nothing selected it starts straight segments at once.
    #[default]
    Mixed,
    /// The Polygonal Lasso: each click adds a vertex, ⇧ (without ⌥) turns the new segment to a
    /// multiple of 45° from the last vertex, and a drag with ⌥ held draws freehand.
    Polygonal,
}

/// How near a press must be to close an outline, in screen points. The caller passes the hit
/// tokens: `size.hit.target` for `close` and `size.hit.slop` for `same_point`; a tool with its own
/// reach (Select and Mask's Polygonal Lasso closes within 6) passes its own numbers. The draft
/// divides them by [`DraftPoint::zoom`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DraftReach {
    /// A press this near the first vertex closes an outline of 3 or more points (a grab: the
    /// user aims at the start).
    pub close: f64,
    /// A press this near the last vertex is the same click again and closes (a hand's jitter, not
    /// an aiming radius: a vertex placed further away is the next vertex, however short the
    /// segment).
    pub same_point: f64,
}

impl DraftReach {
    /// A reach closing within `close` screen points of the first vertex and treating a press
    /// within `same_point` of the last vertex as the same click.
    pub const fn new(close: f64, same_point: f64) -> Self {
        Self { close, same_point }
    }
}

/// One pointer sample for an [`OutlineDraft`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DraftPoint {
    /// The pointer in document units .
    pub at: Point,
    /// The modifiers held at this sample.
    pub mods: Mods,
    /// Screen points per document unit: the
    /// [`DraftReach`] radii are divided by it.
    pub zoom: f64,
}

impl DraftPoint {
    /// A sample at `at` (document units) with `mods` held, at `zoom` screen points per unit.
    pub fn new(at: Point, mods: Mods, zoom: f64) -> Self {
        Self { at, mods, zoom }
    }
}

/// What a call on an [`OutlineDraft`] did. Calls that change nothing return `None`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DraftEvent {
    /// A press began an outline. `intent` is the modifiers the tool resolves its operation from
    /// ([`SelectionOperation::resolve`](super::selection_op::SelectionOperation::resolve)): the press's own,
    /// with ⌥ dropped when ⌥ started straight segments because nothing was selected.
    #[non_exhaustive]
    Started {
        /// The modifiers the tool resolves its operation from.
        intent: Mods,
    },
    /// The outline or its rubber band changed: a vertex added or retracted, freehand points
    /// added, the pointer moved. The tool repaints.
    Changed,
    /// The outline is done and the draft is empty again. `points` has 3 or more points (an
    /// outline with fewer gives [`Cancelled`](Self::Cancelled)); `straight` holds the indices of
    /// the points whose incoming segment is straight (an ⌥-click or a polygon vertex), for a path
    /// that keeps them straight (the Freeform Pen).
    #[non_exhaustive]
    Closed {
        /// The outline's points, at least 3.
        points: Vec<Point>,
        /// Indices of the points whose incoming segment is straight.
        straight: Vec<usize>,
    },
    /// The outline was dropped: Esc, the last vertex retracted, a document change, or a close
    /// with fewer than 3 points. The tool reports nothing to the app.
    Cancelled,
}

/// The pointer state machine of an outline being drawn, shared by every lasso.
///
/// The tool calls, per pass, in order:
/// - [`for_document`](Self::for_document) with the active document's key, before any input (an
///   outline survives neither a document change nor, through the tool's
///   [`cancel`](Self::cancel), a tool change);
/// - [`press`](Self::press), [`drag`](Self::drag) (button down), [`hover`](Self::hover) (button
///   up) and [`release`](Self::release) from the host's pointer calls, and
///   [`mods_changed`](Self::mods_changed) whenever the held modifiers change (between
///   two samples);
/// - [`close`](Self::close) on Enter or a double-click
///   (a click count of 2);
/// - [`retract`](Self::retract) on Backspace or Delete with no modifier, and on a secondary click
///   for [`DraftKind::Polygonal`], only while [`takes_retract`](Self::takes_retract) (the tool
///   keeps those keys from Edit › Clear then);
/// - [`cancel`](Self::cancel) on Esc, a tool change or the window losing focus.
///
/// Every input is in document units; the close radii are screen points ([`DraftReach`]) over the
/// sample's zoom. A point equal to the last one is not added. The draft reports what happened
/// ([`DraftEvent`]) and never touches the document: the tool turns a [`DraftEvent::Closed`] into
/// its own event (`SelectEvent::Commit`).
#[derive(Clone, Debug)]
pub struct OutlineDraft {
    kind: DraftKind,
    reach: DraftReach,
    /// The outline's points; the first is the press that began it.
    points: Vec<Point>,
    /// Indices into `points` whose incoming segment is straight.
    straight: Vec<usize>,
    /// The pointer's last sample: the rubber band's free end.
    cursor: Option<Point>,
    /// The primary button is down.
    down: bool,
    /// Straight segments are on (Mixed: ⌥ latched; Polygonal: always, except during an ⌥-drag).
    polygonal: bool,
    /// ⌥ was held at the press that began the outline and has not been released since.
    alt_from_press: bool,
    /// The document key the outline began under.
    document: Option<u64>,
}

impl OutlineDraft {
    /// An empty draft following `kind`'s rules, closing within `reach`.
    pub fn new(kind: DraftKind, reach: DraftReach) -> Self {
        Self {
            kind,
            reach,
            points: Vec::new(),
            straight: Vec::new(),
            cursor: None,
            down: false,
            polygonal: false,
            alt_from_press: false,
            document: None,
        }
    }

    /// The rules this draft follows.
    pub fn kind(&self) -> DraftKind {
        self.kind
    }

    /// Changes the close radii; an open outline keeps its points.
    pub fn set_reach(&mut self, reach: DraftReach) {
        self.reach = reach;
    }

    /// Records `key` as the active document's. An open outline begun under another key is
    /// dropped ([`DraftEvent::Cancelled`]); an empty draft forgets the key it had.
    pub fn for_document(&mut self, key: u64) -> Option<DraftEvent> {
        let stale = self.is_open() && self.document.is_some_and(|d| d != key);
        self.document = Some(key);
        if stale {
            self.reset();
            return Some(DraftEvent::Cancelled);
        }
        None
    }

    /// The primary button went down at `at`; `has_selection` is whether the document has a
    /// selection (a path tool passes false: it has nothing to subtract from).
    ///
    /// - No outline: one begins at `at` ([`DraftEvent::Started`]). [`DraftKind::Mixed`]: ⌥ held
    ///   with `has_selection` false starts straight segments and drops ⌥ from the intent; ⌥ held
    ///   with a selection stays in the intent (subtract) and the outline is freehand.
    ///   [`DraftKind::Polygonal`]: the press is the first vertex, and ⌥ with `has_selection`
    ///   false is dropped from the intent too.
    /// - An open outline of 3 or more points: a press within `reach.close / zoom` of the first
    ///   point, or within `reach.same_point / zoom` of the last, closes it
    ///   ([`DraftEvent::Closed`]).
    /// - Otherwise, in straight-segment mode (Mixed with ⌥ latched, Polygonal): the press adds a
    ///   vertex and its index goes into [`straight`](Self::straight) ([`DraftEvent::Changed`]).
    ///   Polygonal with ⇧ and not ⌥: the vertex is turned to the nearest multiple of 45° about the
    ///   last vertex, keeping its distance (in document space, so view rotation and mirroring do
    ///   not skew it); the first vertex is never turned.
    ///
    /// A point that isn't finite is ignored (`None`).
    pub fn press(&mut self, at: DraftPoint, has_selection: bool) -> Option<DraftEvent> {
        if !at.at.x.is_finite() || !at.at.y.is_finite() {
            return None;
        }
        if !self.is_open() {
            let alt_starts_straight = at.mods.alt && !has_selection;
            let intent = if alt_starts_straight { Mods { alt: false, ..at.mods } } else { at.mods };
            self.points.push(at.at);
            self.cursor = Some(at.at);
            self.down = true;
            self.polygonal = match self.kind {
                DraftKind::Freehand => false,
                DraftKind::Mixed => alt_starts_straight,
                DraftKind::Polygonal => true,
            };
            self.alt_from_press = self.kind == DraftKind::Mixed && intent.alt;
            return Some(DraftEvent::Started { intent });
        }
        if self.kind == DraftKind::Mixed
            && let Some(closed @ DraftEvent::Closed { .. }) = self.mods_changed(at.mods)
        {
            return Some(closed);
        }
        if self.hits_start(at) {
            return self.close();
        }
        if !self.polygonal {
            return None;
        }
        let mut vertex = at.at;
        if self.kind == DraftKind::Polygonal
            && at.mods.shift
            && !at.mods.alt
            && let Some(&last) = self.points.last()
        {
            vertex = turn_to_45(last, vertex);
        }
        self.cursor = Some(vertex);
        self.down = true;
        if self.append(vertex) {
            self.straight.push(self.points.len() - 1);
        }
        Some(DraftEvent::Changed)
    }

    /// The pointer moved to `at` with the button down. Freehand and Mixed: the point is added
    /// (a freehand point, not in [`straight`](Self::straight)). Polygonal: added only while ⌥
    /// is held (an ⌥-drag draws freehand); otherwise only the rubber band follows. Returns
    /// [`DraftEvent::Changed`] when the outline or the rubber band moved.
    pub fn drag(&mut self, at: DraftPoint) -> Option<DraftEvent> {
        if !self.down {
            return self.hover(at);
        }
        if !self.is_open() || !at.at.x.is_finite() || !at.at.y.is_finite() {
            return None;
        }
        if self.kind != DraftKind::Polygonal || at.mods.alt {
            self.append(at.at);
        }
        self.cursor = Some(at.at);
        Some(DraftEvent::Changed)
    }

    /// The pointer moved to `at` with the button up: only the rubber band follows (hover moves
    /// never become vertices). [`DraftEvent::Changed`] while an outline is open.
    pub fn hover(&mut self, at: DraftPoint) -> Option<DraftEvent> {
        if !self.is_open() || !at.at.x.is_finite() || !at.at.y.is_finite() {
            return None;
        }
        self.cursor = Some(at.at);
        Some(DraftEvent::Changed)
    }

    /// The primary button went up at `at`. Freehand: closes. Mixed: closes unless straight
    /// segments are on (then the outline waits for the next click). Polygonal: the outline
    /// waits. A release that closes an outline of fewer than 3 points cancels it.
    pub fn release(&mut self, at: DraftPoint) -> Option<DraftEvent> {
        if !self.is_open() || !self.down {
            return None;
        }
        self.down = false;
        if at.at.x.is_finite() && at.at.y.is_finite() {
            self.cursor = Some(at.at);
        }
        if self.polygonal {
            return Some(DraftEvent::Changed);
        }
        self.close()
    }

    /// The held modifiers changed. Mixed only: ⌥ pressed after the press (or pressed again after
    /// being released, when it was held at the press) turns straight segments on; with the
    /// button down the pointer's position is first added as a vertex. ⌥ released with the button
    /// up closes the outline; with the button down freehand drawing goes on.
    pub fn mods_changed(&mut self, mods: Mods) -> Option<DraftEvent> {
        if self.kind != DraftKind::Mixed || !self.is_open() {
            return None;
        }
        if self.alt_from_press && !mods.alt {
            self.alt_from_press = false;
        }
        let was = self.polygonal;
        let now = mods.alt && !self.alt_from_press;
        if was && !now && !self.down {
            return self.close();
        }
        if !was
            && now
            && self.down
            && let Some(cursor) = self.cursor
        {
            self.append(cursor);
        }
        self.polygonal = now;
        (was != now).then_some(DraftEvent::Changed)
    }

    /// Removes the last vertex while [`takes_retract`](Self::takes_retract); removing the only
    /// one cancels the outline. Indices in [`straight`](Self::straight) past the end go too.
    /// `None` when the outline doesn't take a retract (the button is down, or no outline).
    pub fn retract(&mut self) -> Option<DraftEvent> {
        if !self.takes_retract() {
            return None;
        }
        if self.points.len() <= 1 {
            self.reset();
            return Some(DraftEvent::Cancelled);
        }
        self.points.pop();
        let len = self.points.len();
        self.straight.retain(|&i| i < len);
        Some(DraftEvent::Changed)
    }

    /// Closes an open outline (Enter, a double-click): [`DraftEvent::Closed`] with 3 or more
    /// points, else [`DraftEvent::Cancelled`]. `None` with no outline.
    pub fn close(&mut self) -> Option<DraftEvent> {
        if !self.is_open() {
            return None;
        }
        let points = std::mem::take(&mut self.points);
        let straight = std::mem::take(&mut self.straight);
        self.reset();
        Some(if points.len() >= 3 { DraftEvent::Closed { points, straight } } else { DraftEvent::Cancelled })
    }

    /// Drops an open outline: [`DraftEvent::Cancelled`]; `None` with no outline.
    pub fn cancel(&mut self) -> Option<DraftEvent> {
        if !self.is_open() {
            return None;
        }
        self.reset();
        Some(DraftEvent::Cancelled)
    }

    /// The outline's points so far, in document units; empty with no outline.
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// Indices into [`points`](Self::points) whose incoming segment is straight.
    pub fn straight(&self) -> &[usize] {
        &self.straight
    }

    /// The rubber band from the last vertex to the pointer while straight segments are on and
    /// the button is up, for the tool to paint; `None` otherwise.
    pub fn rubber_band(&self) -> Option<[Point; 2]> {
        if !self.polygonal || self.down {
            return None;
        }
        Some([*self.points.last()?, self.cursor?])
    }

    /// Whether a press at `at` would close the outline (the tool's close cursor or ring): the
    /// same test as [`press`](Self::press).
    fn hits_start(&self, at: DraftPoint) -> bool {
        if self.points.len() < 3 {
            return false;
        }
        let zoom = at.zoom.max(0.01);
        let near = |q: Point, reach: f64| (q - at.at).hypot() < reach / zoom;
        near(self.points[0], self.reach.close) || self.points.last().is_some_and(|&last| near(last, self.reach.same_point))
    }

    /// Whether an outline is open.
    pub fn is_open(&self) -> bool {
        !self.points.is_empty()
    }

    /// Whether the primary button is down on an open outline.
    pub fn is_down(&self) -> bool {
        self.down && self.is_open()
    }

    /// Whether [`retract`](Self::retract) acts: an outline is open and the button is up (between
    /// clicks). A freehand stroke with the button held is never retracted.
    pub fn takes_retract(&self) -> bool {
        self.is_open() && !self.down
    }

    /// Adds `p` unless it equals the last point; whether it was added.
    fn append(&mut self, p: Point) -> bool {
        if self.points.last() == Some(&p) {
            return false;
        }
        self.points.push(p);
        true
    }

    /// Back to an empty draft (the document key stays).
    fn reset(&mut self) {
        self.points.clear();
        self.straight.clear();
        self.cursor = None;
        self.down = false;
        self.polygonal = false;
        self.alt_from_press = false;
    }
}

/// `point` turned about `from` to the nearest multiple of 45°, keeping its distance.
pub fn turn_to_45(from: Point, point: Point) -> Point {
    let delta = point - from;
    let length = delta.hypot();
    if !length.is_finite() || length == 0.0 {
        return point;
    }
    let angle = round_to_direction(delta.y.atan2(delta.x), Directions::Eight);
    Point::new(from.x + length * angle.cos(), from.y + length * angle.sin())
}

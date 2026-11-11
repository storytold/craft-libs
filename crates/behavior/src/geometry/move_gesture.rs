//! A move by drag: direction lock, stroke lock and snapping along a ray.
//!
//! - [`MoveGesture`]: the Move tool's rules as a value the tool owns. ⇧ locks the offset from the
//!   press to 4 or 8 directions ([`lock_direction`]) and ⌥ held at the press copies once the
//!   pointer has travelled [`ConstrainStyle::copy_reach`]. The gesture reports values; the tool
//!   owns the gesture id, the label, the value before and the cancel, so the duplicate and the
//!   move are one undo step from the tool's mark at the press.
//! - [`StrokeLock`]: ⇧ during a painting stroke keeps it on a 0°, 45° or 90° line from the
//!   stroke's last point.
//!
//! There is no library Move tool: picking what moves and changing the document are the app's.
//! A tool passes `ToolContext::slop` for both distances of the [`ConstrainStyle`]: a stroke
//! width is not a distance, so no token feeds either.
//!
//! ```no_run
//! use craft_behavior::geometry::{ConstrainStyle, Directions, MoveGesture};
//! use craft_keymap::Mods;
//! use kurbo::Point;
//! # fn demo(slop: f64, doc: Point, mods: Mods, zoom: f64) {
//! // `CanvasTool::press`: the tool takes its undo mark and gesture id.
//! let mut drag = MoveGesture::press(&ConstrainStyle::new(slop, slop), doc, mods, Directions::Four);
//! // `CanvasTool::drag`, each sample:
//! if let Some(value) = drag.follow(doc, mods, zoom) {
//!     let _ = value.delta; // an update event with `value`
//! }
//! // `CanvasTool::release`:
//! let value = drag.release(doc, mods, zoom);
//! let _ = value.changed(); // a commit event, `changed` false means no undo step
//! # }
//! ```
//! `CanvasTool::cancel` drops the gesture and restores the tool's mark, which also takes back a
//! copy.

use craft_keymap::Mods;
use kurbo::{Point, Vec2};

use super::lock::{Directions, lock_direction};

/// The distances of the move and stroke rules, in screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ConstrainStyle {
    /// Screen points an ⌥-drag travels before it copies.
    pub copy_reach: f64,
    /// Screen points a ⇧ stroke travels from its origin before its direction locks.
    pub stroke_lock: f64,
}

impl ConstrainStyle {
    /// A style with the two distances; a tool passes `ToolContext::slop` for both.
    pub const fn new(copy_reach: f64, stroke_lock: f64) -> Self {
        Self { copy_reach, stroke_lock }
    }
}

/// The value of a move drag.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MoveValue {
    /// The offset from the press in document units, after the ⇧ lock.
    pub delta: Vec2,
    /// True from the first value after the pointer travelled
    /// [`copy_reach`](ConstrainStyle::copy_reach) screen points with ⌥ held at the press, and for
    /// the rest of the drag. The tool duplicates on the first value that carries it. ⌥ pressed
    /// after the press does nothing.
    pub copy: bool,
}

impl MoveValue {
    /// Whether the drag changes anything: an offset or a copy. A press and release without
    /// travel is unchanged and takes no undo step.
    pub fn changed(&self) -> bool {
        self.delta != Vec2::ZERO || self.copy
    }
}

/// A Move-tool drag in progress: the press, whether ⌥ was held at it, and the last locked
/// offset. The tool keeps it between events and drops it after [`release`](Self::release) or on
/// cancel.
#[derive(Clone, Debug)]
pub struct MoveGesture {
    start: Point,
    directions: Directions,
    copy_reach: f64,
    alt_at_press: bool,
    value: MoveValue,
    /// The last locked offset, for the hysteresis; `None` while ⇧ is up.
    locked: Option<Vec2>,
}

impl MoveGesture {
    /// Starts a drag at `start` (document units) with the keys held at the press.
    pub fn press(style: &ConstrainStyle, start: Point, mods: Mods, directions: Directions) -> MoveGesture {
        MoveGesture { start, directions, copy_reach: style.copy_reach, alt_at_press: mods.alt, value: MoveValue::default(), locked: None }
    }

    /// The pointer moved to `at` (document units) with `mods` at `zoom` screen points per
    /// document unit. ⇧ locks the offset to the gesture's directions with hysteresis from the
    /// last locked offset; without ⇧ the offset is free and the lock is forgotten. `copy` turns
    /// on when ⌥ was held at the press and `|delta| × zoom ≥ copy_reach`. Returns the value when
    /// it changed since the last call, else `None`.
    pub fn follow(&mut self, at: Point, mods: Mods, zoom: f64) -> Option<MoveValue> {
        let next = self.value_at(at, mods, zoom);
        if next == self.value {
            return None;
        }
        self.value = next;
        Some(next)
    }

    /// The release at `at`: the same lock as [`follow`](Self::follow) applies, so a release where
    /// the last value was ends there.
    pub fn release(mut self, at: Point, mods: Mods, zoom: f64) -> MoveValue {
        self.value_at(at, mods, zoom)
    }

    /// The value of the last [`follow`](Self::follow): the offset and whether it copies.
    pub fn value(&self) -> MoveValue {
        self.value
    }

    /// The value at `at`, updating the remembered lock; `copy` stays on once on.
    fn value_at(&mut self, at: Point, mods: Mods, zoom: f64) -> MoveValue {
        let d = at - self.start;
        let delta = if mods.shift {
            let locked = lock_direction(d, self.locked, self.directions);
            self.locked = Some(locked);
            locked
        } else {
            self.locked = None;
            d
        };
        let copy = self.value.copy || (self.alt_at_press && delta.hypot() * zoom >= self.copy_reach);
        MoveValue { delta, copy }
    }
}

/// ⇧ during a painting stroke (every painting tool, the Freeform Pen; which tools is the app's
/// list): the stroke stays on a line at 0°, 45° or 90° from where the lock began.
///
/// The lock begins at the stroke's last point when ⇧ is down (the stroke's start, or where ⇧ went
/// down mid-stroke). Inside [`stroke_lock`](ConstrainStyle::stroke_lock) screen points of it the
/// point stays at the origin; past them the direction is the nearest 45° ray and holds until ⇧ is
/// released.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeLock {
    reach: f64,
    origin: Option<Point>,
    direction: Option<Vec2>,
}

impl StrokeLock {
    /// No lock yet; `style` gives the screen distance before the direction locks.
    pub fn new(style: &ConstrainStyle) -> Self {
        Self { reach: style.stroke_lock, origin: None, direction: None }
    }

    /// The stroke point for the pointer at `p` (document units), `last` the stroke's last point,
    /// `zoom` screen points per document unit. Without `shift` the point passes through and the
    /// lock is dropped. With it: the lock begins at `last` if none runs; within `reach / zoom` of
    /// the origin the origin comes back; the first point past it fixes the direction (nearest
    /// multiple of 45°, as [`lock_direction`] with [`Directions::Eight`]); every later point is projected on that line.
    pub fn constrain(&mut self, last: Point, p: Point, shift: bool, zoom: f64) -> Point {
        if !shift {
            self.reset();
            return p;
        }
        let origin = *self.origin.get_or_insert(last);
        if self.direction.is_none() {
            if (p - origin).hypot() < self.reach / zoom.max(1e-3) {
                return origin;
            }
            let locked = lock_direction(p - origin, None, Directions::Eight);
            let len = locked.hypot();
            self.direction = (len.is_finite() && len > 0.0).then(|| locked / len);
        }
        match self.direction {
            Some(dir) => origin + dir * (p - origin).dot(dir),
            None => origin,
        }
    }

    /// Whether a direction is locked.
    pub fn locked(&self) -> bool {
        self.direction.is_some()
    }

    /// Forget the lock (the stroke ended).
    pub fn reset(&mut self) {
        self.origin = None;
        self.direction = None;
    }
}

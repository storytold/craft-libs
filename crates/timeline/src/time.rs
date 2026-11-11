//! Exact media time: [`Tick`], [`TimeRange`] and the time-to-pixel map [`TimeView`].
//!
//! [`TICKS_PER_SECOND`] (Premiere's tick) divides evenly into every broadcast frame duration
//! (23.976 … 120 fps, NTSC rates included) and every sample duration of 8 kHz … 384 kHz, so
//! FilmCraft's frames, EffectCraft's comp frames and SoundCraft's samples all convert exactly
//! (`Tick::from_units(samples, 48_000)`). Every conversion is total: a zero or negative rate gives
//! zero, never a division-by-zero panic.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks per second.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// How far past the end of the content a timeline view reaches: ten minutes, as Premiere's
/// Timeline does (on a 6 s sequence its view stops with 00:10:06 at the right edge). The view
/// cannot be scrolled past it and the scroll bar spans it.
pub const VIEW_EXTENT_PAST_END: Tick = Tick(600 * TICKS_PER_SECOND);

/// A point or a duration in time, in ticks. Arithmetic saturates at the `i64` limits;
/// representable results stay exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    /// Time zero.
    pub const ZERO: Tick = Tick(0);
    /// The latest time any widget accepts; a quarter of the storage range, so sums of two stay
    /// representable.
    pub const MAX: Tick = Tick(i64::MAX / 4);
    /// The earliest time any widget accepts; the negative counterpart of [`Tick::MAX`].
    pub const MIN: Tick = Tick(i64::MIN / 4);

    /// `s` seconds, rounded to the nearest tick.
    // filmcraft: crates/time/src/lib.rs:33 (`(s * TICKS_PER_SECOND).round()`; a non-finite `s`
    // gives zero here, FilmCraft's cast saturates).
    pub fn from_seconds_f64(s: f64) -> Tick {
        Tick(if s.is_finite() { (s * TICKS_PER_SECOND as f64).round() as i64 } else { 0 })
    }

    /// The time in seconds.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Exact conversion from a count of `units` at `per_second` (samples at 48,000; frames at a
    /// whole rate). Zero when `per_second` is zero.
    // filmcraft: crates/time/src/lib.rs:40 (i128 product, checked division, clamped to i64).
    pub fn from_units(units: i64, per_second: i64) -> Tick {
        Tick(clamp_i64((i128::from(units) * i128::from(TICKS_PER_SECOND)).checked_div(i128::from(per_second)).unwrap_or(0)))
    }

    /// The count of whole units at `per_second` before this time (floor, also for negative times).
    // filmcraft: crates/time/src/lib.rs:44 (`div_euclid`).
    pub fn to_units_floor(self, per_second: i64) -> i64 {
        clamp_i64((i128::from(self.0) * i128::from(per_second)).div_euclid(i128::from(TICKS_PER_SECOND)))
    }

    /// The time of a rational timestamp `pts × num / den` seconds (a container timebase). Zero
    /// when `den` is zero; a product past `i128` saturates to the sign's limit.
    // filmcraft: crates/time/src/lib.rs:48.
    pub fn from_rational(pts: i64, num: i64, den: i64) -> Tick {
        if den == 0 {
            return Tick::ZERO;
        }
        let Some(t) = (i128::from(pts) * i128::from(num)).checked_mul(i128::from(TICKS_PER_SECOND)) else {
            return Tick(if (pts < 0) ^ (num < 0) ^ (den < 0) { i64::MIN } else { i64::MAX });
        };
        Tick(clamp_i64(t.div_euclid(i128::from(den))))
    }

    /// The inverse of [`Tick::from_rational`], floored to the timebase.
    // filmcraft: crates/time/src/lib.rs:59.
    pub fn to_rational_floor(self, num: i64, den: i64) -> i64 {
        clamp_i64((i128::from(self.0) * i128::from(den)).checked_div_euclid(i128::from(num) * i128::from(TICKS_PER_SECOND)).unwrap_or(0))
    }

    /// The inverse of [`Tick::from_rational`], rounded to the nearest unit (halves up): containers
    /// store timestamps rounded to their timebase, so flooring misses frames stamped half a unit
    /// late.
    // filmcraft: crates/time/src/lib.rs:67.
    pub fn to_rational_round(self, num: i64, den: i64) -> i64 {
        let n = i128::from(self.0) * i128::from(den);
        let unit = i128::from(num) * i128::from(TICKS_PER_SECOND);
        let (n, unit) = if unit < 0 { (-n, -unit) } else { (n, unit) };
        let (Some(q), Some(r)) = (n.checked_div_euclid(unit), n.checked_rem_euclid(unit)) else {
            return 0;
        };
        clamp_i64(if r * 2 >= unit { q + 1 } else { q })
    }

    /// Scaled by `num / den`, floored; zero when `den` is zero.
    // filmcraft: crates/time/src/lib.rs:89.
    pub fn mul_ratio(self, num: i64, den: i64) -> Tick {
        Tick(clamp_i64((i128::from(self.0) * i128::from(num)).checked_div_euclid(i128::from(den)).unwrap_or(0)))
    }

    /// The absolute value; saturates at the maximum.
    pub fn abs(self) -> Tick {
        Tick(self.0.saturating_abs())
    }

    /// The earlier of two times.
    pub fn min(self, o: Tick) -> Tick {
        Tick(self.0.min(o.0))
    }

    /// The later of two times.
    pub fn max(self, o: Tick) -> Tick {
        Tick(self.0.max(o.0))
    }

    /// Clamped to `lo..=hi`; never panics when the bounds cross (`hi` wins).
    pub fn clamp(self, lo: Tick, hi: Tick) -> Tick {
        Tick(self.0.max(lo.0).min(hi.0))
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, o: Tick) -> Tick {
        Tick(self.0.saturating_add(o.0))
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, o: Tick) -> Tick {
        Tick(self.0.saturating_sub(o.0))
    }
}

impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(self.0.saturating_neg())
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_add(o.0);
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

/// A half-open range `[start, start + duration)`. A zero or negative duration is empty and
/// overlaps nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TimeRange {
    /// The first tick in the range.
    pub start: Tick,
    /// The length; zero or negative makes the range empty.
    pub duration: Tick,
}

impl TimeRange {
    /// A range from `start` lasting `duration`.
    pub fn new(start: Tick, duration: Tick) -> Self {
        Self { start, duration }
    }

    /// A range from `start` up to, not including, `end`.
    pub fn from_bounds(start: Tick, end: Tick) -> Self {
        Self { start, duration: end - start }
    }

    /// The first tick after the range.
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }

    /// Whether the range has zero or negative duration.
    pub fn is_empty(&self) -> bool {
        self.duration.0 <= 0
    }

    /// Whether `t` lies in `[start, end)`.
    pub fn contains(&self, t: Tick) -> bool {
        t >= self.start && t < self.end()
    }

    /// Whether both ranges are non-empty and share at least one tick.
    pub fn overlaps(&self, o: &TimeRange) -> bool {
        !self.is_empty() && !o.is_empty() && self.start < o.end() && o.start < self.end()
    }

    /// The common part, `None` when it is empty.
    pub fn intersect(&self, o: &TimeRange) -> Option<TimeRange> {
        let s = self.start.max(o.start);
        let e = self.end().min(o.end());
        (e > s).then(|| TimeRange::from_bounds(s, e))
    }
}

/// How a timeline shows time: the time at its left edge and its zoom in points per second. The
/// app keeps one per sequence (Premiere's sequence tabs each keep theirs) and saves it beside the
/// document, never in its undo history.
///
/// Screen x is `left + (t − start) × points_per_second`, where `left` is the x of the time
/// area's left edge (right of the track headers).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimeView {
    /// The time at the left edge of the time area.
    pub start: Tick,
    /// The zoom in points a second, between [`TimeView::MIN_ZOOM`] and [`TimeView::MAX_ZOOM`].
    pub points_per_second: f64,
}

impl Default for TimeView {
    /// 40 points a second from zero, FilmCraft's new-sequence zoom (`state.rs:223`).
    fn default() -> Self {
        Self { start: Tick::ZERO, points_per_second: 40.0 }
    }
}

impl TimeView {
    /// The most zoomed-out view: 0.05 points a second (FilmCraft `layout.rs:110`).
    pub const MIN_ZOOM: f64 = 0.05;
    /// The most zoomed-in view: 24,000 points a second, a few points per frame at 120 fps.
    pub const MAX_ZOOM: f64 = 24_000.0;

    /// A view with its left edge at `start` and `points_per_second` of zoom.
    pub fn new(start: Tick, points_per_second: f64) -> Self {
        Self { start, points_per_second }
    }

    /// The screen x of `t`.
    pub fn x_of(&self, t: Tick, left: f32) -> f32 {
        left + ((t - self.start).seconds() * self.points_per_second) as f32
    }

    /// The time under screen x.
    // filmcraft: crates/ui-egui/src/panels/timeline/layout.rs:29 (`scroll + (x - left) / pps`).
    pub fn time_at(&self, x: f32, left: f32) -> Tick {
        if !(self.points_per_second > 0.0 && self.points_per_second.is_finite() && x.is_finite() && left.is_finite()) {
            return self.start;
        }
        self.start + Tick::from_seconds_f64(f64::from(x - left) / self.points_per_second)
    }

    /// Zoom by `factor` keeping `anchor` under the same x; when the anchor is outside the
    /// `width`-point view the middle of the view stays instead. The zoom is clamped to
    /// [`MIN_ZOOM`](Self::MIN_ZOOM)…[`MAX_ZOOM`](Self::MAX_ZOOM) and `start` never goes below
    /// zero.
    // filmcraft: crates/ui-egui/src/panels/timeline/layout.rs:108 `zoom_about`.
    pub fn zoom_about(&mut self, factor: f64, anchor: Tick, width: f32) {
        if !factor.is_finite() || !(self.points_per_second > 0.0 && self.points_per_second.is_finite()) {
            return;
        }
        let old = self.points_per_second;
        let new = (old * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let x = ((anchor - self.start).seconds() * old) as f32;
        let x = if (0.0..=width).contains(&x) { x } else { width / 2.0 };
        self.points_per_second = new;
        self.start = (anchor - Tick::from_seconds_f64(f64::from(x) / new)).max(Tick::ZERO);
    }

    /// Where `start` goes so `t` shows in a view `width` points wide: `t` 5 % of the width from
    /// the left edge, never before zero. `None` when `t` is in view already or a number is not
    /// finite. Explicit navigation (Home, a marker, an edit point) reveals the playhead with it
    /// even when the time did not change (FilmCraft #1188).
    // filmcraft: crates/ui-egui/src/panels/timeline/layout.rs:87 `scroll_to_reveal`.
    pub fn scroll_to_reveal(&self, t: Tick, width: f32) -> Option<Tick> {
        let pps = self.points_per_second;
        if !(pps > 0.0 && pps.is_finite() && width.is_finite()) {
            return None;
        }
        let px = (t - self.start).seconds() * pps;
        (px < 0.0 || px > f64::from(width)).then(|| (t - Tick::from_seconds_f64(f64::from(width) * 0.05 / pps)).max(Tick::ZERO))
    }

    /// The latest `start` a view `width` points wide may have over content ending at `end`:
    /// [`VIEW_EXTENT_PAST_END`] past it, less the view's own span; zero for a non-positive zoom.
    // filmcraft: crates/ui-egui/src/panels/timeline/layout.rs:73 `timeline_extent`, `:78`
    // `max_scroll`.
    pub fn max_start(&self, end: Tick, width: f32) -> Tick {
        let pps = self.points_per_second;
        if !(pps > 0.0 && pps.is_finite() && width.is_finite()) {
            return Tick::ZERO;
        }
        (end.max(Tick::ZERO) + VIEW_EXTENT_PAST_END - Tick::from_seconds_f64(f64::from(width) / pps)).max(Tick::ZERO)
    }

    /// The view that shows `range` in `width` points with `margin` to spare
    /// (`timeline.fit_margin`), After Effects' fit (Shift+= toggles it with the frame zoom).
    // effectcraft: crates/ui-egui/src/panels/timeline.rs:608 `fit_pps` (`(width - 10) /
    // duration`, at least 0.01 s).
    pub fn fit(range: TimeRange, width: f32, margin: f32) -> TimeView {
        let span = range.duration.seconds().max(0.01);
        let points_per_second = (f64::from(width - margin).max(1.0) / span).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        TimeView { start: range.start, points_per_second }
    }

    /// This view with every number brought into a usable range (zoom 0.001…100,000, start
    /// 0…10,000,000 s), or `None` when a number is not finite. Views come from files, so nothing
    /// in them is trusted.
    // filmcraft: crates/project/src/view.rs:21 `SequenceView::checked`.
    pub fn checked(self) -> Option<TimeView> {
        if !self.points_per_second.is_finite() {
            return None;
        }
        Some(TimeView {
            start: self.start.clamp(Tick::ZERO, Tick(MAX_START_SECONDS * TICKS_PER_SECOND)),
            points_per_second: self.points_per_second.clamp(0.001, 100_000.0),
        })
    }
}

/// The latest start [`TimeView::checked`] accepts, in seconds.
const MAX_START_SECONDS: i64 = 10_000_000;

/// Narrows after applying the storage bounds, never by a wrapping cast.
fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

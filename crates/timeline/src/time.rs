//! Exact media time: craft-time's [`Tick`], [`TimeRange`] and frame and timecode primitives,
//! re-exported so `craft_timeline::time::*` paths keep working, and the timeline's time-to-pixel
//! map [`TimeView`].

pub use crate::timecode::format_feet_frames;
pub use craft_time::{
    FrameRate, ParseError, SAMPLE_RATES, TICKS_PER_SECOND, Tick, TimeDisplay, TimeRange, fields_to_frames, format_timecode_frames, frames_to_fields,
    parse_time_display_relative as parse_time_display, parse_timecode_relative as parse_timecode,
};

/// How far past the end of the content a timeline view reaches: ten minutes, as Premiere's
/// Timeline does (on a 6 s sequence its view stops with 00:10:06 at the right edge). The view
/// cannot be scrolled past it and the scroll bar spans it.
pub const VIEW_EXTENT_PAST_END: Tick = Tick(600 * TICKS_PER_SECOND);

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

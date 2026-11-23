//! Stroke ordering and an animation schedule for replaying ink in the order it was drawn.
//!
//! The schedule is pure time: the caller owns the clock (a frame loop hands over its delta) and
//! paints what [`Replay::visible`] returns. Strokes are ordered by [`Stroke::order`]; when no
//! stroke records an order (every order zero, as Office ink does) the document order stands. The
//! sort is stable, so equal orders keep the order the caller supplied.

use crate::stroke::{Stroke, StrokePoint};

/// Sheet points drawn per second by default.
pub const DEFAULT_SPEED: f64 = 400.0;
/// Seconds between strokes by default.
pub const DEFAULT_GAP: f64 = 0.12;
/// No stroke appears instantly.
pub const MIN_STROKE_SECS: f64 = 0.15;
/// No stroke takes longer than this, however long it is.
pub const MAX_STROKE_SECS: f64 = 2.5;
/// Most strokes one replay animates.
pub const MAX_STROKES: usize = 2_000;
/// Most points one stroke contributes.
pub const MAX_POINTS: usize = 20_000;
/// Longest gap between strokes accepted, in seconds.
pub const MAX_GAP: f64 = 60.0;
/// Longest single stroke accepted, in seconds.
pub const MAX_DURATION: f64 = 3_600.0;
/// Largest stroke length considered, in the caller's coordinate units, so a hostile stroke
/// cannot make the schedule infinite.
const MAX_LENGTH: f64 = 1e12;

/// How a replay is timed. [`Options::default`] is the speed a drawing app shows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Options {
    /// Stroke length drawn per second, in the caller's coordinate units.
    pub speed: f64,
    /// Seconds between one stroke's end and the next one's start.
    pub gap: f64,
    /// Shortest time a stroke takes.
    pub min_duration: f64,
    /// Longest time a stroke takes.
    pub max_duration: f64,
    /// Most strokes animated; later ones are left out.
    pub max_strokes: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { speed: DEFAULT_SPEED, gap: DEFAULT_GAP, min_duration: MIN_STROKE_SECS, max_duration: MAX_STROKE_SECS, max_strokes: MAX_STROKES }
    }
}

impl Options {
    /// The options with every value usable: non-finite numbers take their defaults, negative
    /// durations and gaps become zero, and the ranges are put back in order. The bounds also keep
    /// the schedule's arithmetic finite for a hostile caller.
    pub fn sanitized(self) -> Options {
        let fallback = Options::default();
        let speed = if self.speed.is_finite() && self.speed > 0.0 { self.speed.clamp(1e-6, 1e12) } else { fallback.speed };
        let gap = if self.gap.is_finite() && self.gap >= 0.0 { self.gap.min(MAX_GAP) } else { fallback.gap };
        let min_duration =
            if self.min_duration.is_finite() && self.min_duration >= 0.0 { self.min_duration.min(MAX_DURATION) } else { fallback.min_duration };
        let max_duration =
            if self.max_duration.is_finite() && self.max_duration >= 0.0 { self.max_duration.min(MAX_DURATION) } else { fallback.max_duration };
        let (min_duration, max_duration) = if min_duration <= max_duration { (min_duration, max_duration) } else { (max_duration, min_duration) };
        Options { speed, gap, min_duration, max_duration, max_strokes: self.max_strokes.min(MAX_STROKES) }
    }
}

/// One stroke's place in a replay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    /// The stroke's index in the slice the replay was built from.
    pub stroke: usize,
    /// When the stroke starts, in seconds from the replay's beginning.
    pub start: f64,
    /// How long the stroke takes.
    pub duration: f64,
}

/// The order strokes are replayed in.
///
/// [`Stroke::order`] decides when any stroke records one; document order stands otherwise. The
/// sort is stable, so strokes with equal orders keep their input order.
pub fn draw_order(strokes: &[Stroke]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..strokes.len()).collect();
    if strokes.iter().any(|stroke| stroke.order != 0) {
        order.sort_by_key(|&index| strokes.get(index).map_or(0, |stroke| stroke.order));
    }
    order
}

/// A stroke's length, the sum of its segment lengths. Non-finite segments add nothing and the
/// total is capped, so a hostile stroke cannot make the schedule infinite.
pub fn path_length(points: &[StrokePoint]) -> f64 {
    points.windows(2).filter(|pair| pair.iter().all(|p| p.x.is_finite() && p.y.is_finite())).fold(0.0, |total, pair| {
        let (a, b) = (pair[0], pair[1]);
        (total + (b.x - a.x).hypot(b.y - a.y)).min(MAX_LENGTH)
    })
}

/// A replay in progress: the schedule, the clock and whether it is running.
#[derive(Clone, Debug, PartialEq)]
pub struct Replay {
    timings: Vec<Timing>,
    total: f64,
    clock: f64,
    playing: bool,
}

impl Replay {
    /// A schedule for `strokes`, or `None` when there is nothing to replay.
    pub fn new(strokes: &[Stroke], options: Options) -> Option<Replay> {
        let options = options.sanitized();
        let mut timings = Vec::new();
        let mut start = 0.0;
        for index in draw_order(strokes).into_iter().take(options.max_strokes) {
            let Some(stroke) = strokes.get(index) else { continue };
            let length = path_length(&stroke.points);
            let duration = (length / options.speed).clamp(options.min_duration, options.max_duration);
            if !duration.is_finite() {
                continue;
            }
            timings.push(Timing { stroke: index, start, duration });
            start += duration + options.gap;
        }
        if timings.is_empty() {
            return None;
        }
        let total = timings.last().map_or(0.0, |timing| timing.start + timing.duration);
        Some(Replay { timings, total, clock: 0.0, playing: true })
    }

    /// The schedule, in replay order.
    pub fn timings(&self) -> &[Timing] {
        &self.timings
    }

    /// The time each stroke takes, ending with the last stroke's end.
    pub fn total_duration(&self) -> f64 {
        self.total
    }

    /// Seconds into the replay.
    pub fn clock(&self) -> f64 {
        self.clock
    }

    /// Whether the clock is still moving.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Whether the clock has reached the end.
    pub fn is_finished(&self) -> bool {
        self.clock >= self.total
    }

    /// Advances the clock by `dt` seconds, stopping at the end. A negative or non-finite delta
    /// does nothing.
    pub fn tick(&mut self, dt: f64) {
        if !self.playing || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.clock = (self.clock + dt).min(self.total);
        if self.is_finished() {
            self.playing = false;
        }
    }

    /// Puts the clock at `seconds` into the replay, bounded to the schedule.
    pub fn seek(&mut self, seconds: f64) {
        self.clock = if seconds.is_finite() { seconds.clamp(0.0, self.total) } else { 0.0 };
        self.playing = self.clock < self.total;
    }

    /// Plays from the current clock.
    pub fn play(&mut self) {
        self.playing = !self.is_finished();
    }

    /// Stops the clock where it is.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Starts over from the beginning.
    pub fn restart(&mut self) {
        self.clock = 0.0;
        self.playing = true;
    }

    /// The stroke's timing, when the replay covers it.
    pub fn timing(&self, stroke: usize) -> Option<&Timing> {
        self.timings.iter().find(|timing| timing.stroke == stroke)
    }

    /// The part of `points` visible now, in drawing order: nothing before the stroke starts,
    /// then a growing prefix, interpolated at the head. Non-finite points are skipped and at most
    /// [`MAX_POINTS`] are considered. An interpolated point keeps the previous point's pressure.
    pub fn visible(&self, stroke: usize, points: &[StrokePoint]) -> Vec<StrokePoint> {
        let Some(timing) = self.timing(stroke) else { return Vec::new() };
        if self.clock < timing.start {
            return Vec::new();
        }
        let fraction = if timing.duration > 0.0 { ((self.clock - timing.start) / timing.duration).clamp(0.0, 1.0) } else { 1.0 };
        if fraction <= 0.0 {
            return Vec::new();
        }
        let finite: Vec<StrokePoint> = points.iter().copied().filter(|p| p.x.is_finite() && p.y.is_finite()).take(MAX_POINTS).collect();
        if fraction >= 1.0 {
            return finite;
        }
        let Some(first) = finite.first().copied() else { return Vec::new() };
        let mut remaining = path_length(&finite) * fraction;
        let mut out = Vec::new();
        out.push(first);
        for pair in finite.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let segment = (b.x - a.x).hypot(b.y - a.y);
            if segment <= remaining {
                out.push(b);
                remaining -= segment;
            } else {
                let t = if segment > 0.0 { remaining / segment } else { 0.0 };
                out.push(StrokePoint { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, pressure: a.pressure });
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::{Brush, Color};

    fn stroke(points: &[[f64; 2]], order: u32) -> Stroke {
        let points = points.iter().map(|p| StrokePoint::new(p[0], p[1])).collect();
        Stroke { order, ..Stroke::new(points, Brush::default(), Color::BLACK) }
    }

    #[test]
    fn draw_order_uses_recorded_orders_and_keeps_document_order_otherwise() {
        let plain = [stroke(&[[0.0, 0.0]], 0), stroke(&[[1.0, 0.0]], 0), stroke(&[[2.0, 0.0]], 0)];
        assert_eq!(draw_order(&plain), vec![0, 1, 2]);
        let ordered = [stroke(&[[0.0, 0.0]], 2), stroke(&[[1.0, 0.0]], 0), stroke(&[[2.0, 0.0]], 1)];
        assert_eq!(draw_order(&ordered), vec![1, 2, 0]);
        let tied = [stroke(&[[0.0, 0.0]], 1), stroke(&[[1.0, 0.0]], 0), stroke(&[[2.0, 0.0]], 1)];
        assert_eq!(draw_order(&tied), vec![1, 0, 2], "equal orders keep input order");
    }

    #[test]
    fn the_schedule_is_monotonic_and_starts_in_draw_order() {
        let strokes = [stroke(&[[0.0, 0.0], [100.0, 0.0]], 1), stroke(&[[0.0, 0.0], [10.0, 0.0]], 0)];
        let replay = Replay::new(&strokes, Options::default()).expect("a replay");
        let timings = replay.timings();
        assert_eq!(timings.iter().map(|t| t.stroke).collect::<Vec<_>>(), vec![1, 0]);
        assert_eq!(timings[0].start, 0.0);
        assert!(timings[1].start >= timings[0].start + timings[0].duration);
        assert!(timings[0].duration >= MIN_STROKE_SECS);
        assert!(timings.iter().all(|t| t.duration <= MAX_STROKE_SECS));
        assert_eq!(replay.total_duration(), timings[1].start + timings[1].duration);
        // A zero-length stroke still takes the minimum time.
        assert_eq!(timings[0].duration, MIN_STROKE_SECS);
    }

    #[test]
    fn hostile_options_and_points_are_bounded() {
        let strokes = [stroke(&[[0.0, 0.0], [1.0, 0.0]], 0)];
        let options = Options { speed: f64::NAN, gap: -1.0, min_duration: f64::INFINITY, max_duration: 0.0, max_strokes: usize::MAX };
        let options = options.sanitized();
        assert!(options.speed.is_finite() && options.gap >= 0.0);
        assert!(options.min_duration <= options.max_duration);
        assert_eq!(options.max_strokes, MAX_STROKES);
        assert!(Replay::new(&[], Options::default()).is_none());
        let replay = Replay::new(&strokes, options).expect("a replay");
        assert!(replay.total_duration().is_finite());
        assert_eq!(path_length(&[StrokePoint::new(f64::NAN, 0.0), StrokePoint::new(1.0, 1.0)]), 0.0);
    }

    #[test]
    fn the_clock_stops_at_the_end_and_seek_is_bounded() {
        let strokes = [stroke(&[[0.0, 0.0], [40.0, 0.0]], 0)];
        let mut replay = Replay::new(&strokes, Options { speed: 100.0, gap: 0.0, ..Options::default() }).expect("a replay");
        assert!(replay.is_playing());
        replay.tick(f64::NAN);
        assert_eq!(replay.clock(), 0.0);
        replay.tick(-1.0);
        assert_eq!(replay.clock(), 0.0);
        replay.tick(10.0);
        assert_eq!(replay.clock(), replay.total_duration());
        assert!(replay.is_finished() && !replay.is_playing());
        replay.seek(f64::INFINITY);
        assert_eq!(replay.clock(), 0.0);
        replay.seek(1e9);
        assert_eq!(replay.clock(), replay.total_duration());
        replay.restart();
        assert_eq!(replay.clock(), 0.0);
        assert!(replay.is_playing());
        replay.pause();
        assert!(!replay.is_playing());
    }

    #[test]
    fn a_stroke_is_revealed_progressively_and_interpolated() {
        let strokes = [stroke(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], 0)];
        let mut replay =
            Replay::new(&strokes, Options { speed: 100.0, gap: 0.0, min_duration: 0.0, max_duration: 10.0, ..Options::default() }).expect("a replay");
        assert_eq!(replay.visible(0, &strokes[0].points), Vec::new(), "before any time passes");
        replay.seek(replay.total_duration() / 2.0);
        let half = replay.visible(0, &strokes[0].points);
        assert!(half.len() >= 2);
        let length: f64 = half.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum();
        assert!((length - 100.0).abs() < 1e-6, "{length}");
        replay.seek(replay.total_duration());
        assert_eq!(replay.visible(0, &strokes[0].points), strokes[0].points);
        assert!(replay.visible(1, &strokes[0].points).is_empty(), "a stroke the replay does not cover");
    }
}

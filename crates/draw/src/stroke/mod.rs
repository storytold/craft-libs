//! The stroke style of a path: width, caps, joins, alignment and dashes, without egui.
//!
//! [`StrokeStyle`] is the value an app's engine and the stroke editor (`craft_draw::ui::stroke`)
//! share: PhotoCraft's `ShapeStroke` without its paint, with PhotoCraft's limits
//! ([`StrokeStyle::validate`]) and presets ([`StrokeStyle::with_line_style`]). Colour and paint
//! stay with the app (S7-shapes' paints list).

use std::fmt;
use std::ops::RangeInclusive;

use kurbo::{BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, PathEl, PathSeg, Point, QuadBez, Vec2};
use serde::{Deserialize, Serialize};

#[cfg(test)]
pub(crate) mod dash_tests;

/// The shortest run (between corners and path ends) and the smallest pattern total `Corners` fits.
const MIN_RUN: f64 = 1e-6;
/// The length and sine below which segments and tangents count as degenerate.
const EPS: f64 = 1e-9;
/// The accuracy of arc lengths, in document units.
const ARCLEN_ACCURACY: f64 = 1e-6;
/// The distance within which two dash ends count as the same point, so a dash across a corner joins.
const JOIN_EPS: f64 = 1e-6;
/// The most dashes one call produces; a path that would need more stays solid.
const MAX_OUTPUT_DASHES: usize = 200_000;

/// Where the stroke sits on the path. Only a closed path aligns; an open one is always centred.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeAlign {
    /// The stroke lies inside the path.
    Inside,
    /// The stroke is centred on the path.
    #[default]
    Center,
    /// The stroke lies outside the path.
    Outside,
}

/// The shape at the ends of an open path and of each dash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineCap {
    /// Butt / Flat: the stroke ends at the end point.
    #[default]
    Butt,
    /// A semicircle past the end point.
    Round,
    /// The stroke goes on half its width past the end point.
    Square,
}

/// The shape of the stroke where two segments meet at a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineJoin {
    /// A sharp point, cut off past [`StrokeStyle::miter_limit`].
    #[default]
    Miter,
    /// A circular arc around the corner point.
    Round,
    /// The corner is cut off flat.
    Bevel,
}

/// The name of a dash pattern, as the stroke editor's presets and its opener show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineStyle {
    /// No dashes.
    Solid,
    /// Dashes [`StrokeStyle::DASHED`].
    Dashed,
    /// Dashes [`StrokeStyle::DOTTED`] with round caps.
    Dotted,
    /// Any other pattern.
    Custom,
}

/// The unit of the dash lengths and the dash offset. It changes what the numbers mean, never the
/// stored numbers: the stroke editor shows it as the fields' suffix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DashUnit {
    /// Multiples of the stroke width (PhotoCraft, Photoshop).
    #[default]
    Widths,
    /// Document units (VectorCraft).
    Document,
}

/// Why [`StrokeStyle::validate`] refuses a style. The `Display` text is PhotoCraft's engine
/// message, for logs and the app's own errors; the stroke editor shows its own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrokeError {
    /// The width is not finite or outside [`StrokeStyle::WIDTH_RANGE`].
    Width,
    /// The opacity is not finite or outside 0..=1.
    Opacity,
    /// The miter limit is not finite or outside [`StrokeStyle::MITER_LIMIT_RANGE`].
    MiterLimit,
    /// The dash offset is not finite or its size is over [`StrokeStyle::DASH_OFFSET_MAX`].
    DashOffset,
    /// More than [`StrokeStyle::MAX_DASHES`] lengths, or a length not finite or outside
    /// [`StrokeStyle::DASH_RANGE`].
    DashPattern,
    /// A pattern whose lengths add up to less than [`StrokeStyle::DASH_TOTAL_MIN`].
    DashTotal,
}

impl fmt::Display for StrokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // pc: crates/doc/src/vector.rs:270-290 `ShapeStroke::validate`, the messages verbatim.
        f.write_str(match self {
            StrokeError::Width => "stroke width must be finite and between 0 and 1000000 px",
            StrokeError::Opacity => "stroke opacity must be between 0 and 100%",
            StrokeError::MiterLimit => "miter limit must be between 1 and 500",
            StrokeError::DashOffset => "dash offset must be finite and within ±1000000 widths",
            StrokeError::DashPattern => "dash pattern must contain at most 32 finite lengths between 0 and 10000 widths",
            StrokeError::DashTotal => "dash pattern total must be at least 0.01 widths",
        })
    }
}

impl std::error::Error for StrokeError {}

/// How dashes sit along a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum DashAlign {
    /// The dash and gap lengths as given, from the dash offset.
    #[default]
    Exact,
    /// Illustrator's "align dashes to corners and path ends": every run between corners and path
    /// ends holds a whole number of periods, and a dash is centred on each corner and end.
    Corners,
}

/// How a path is stroked, except its paint: width, alignment, caps, corners, dashes and opacity.
/// The default is PhotoCraft's `ShapeStroke::default()` without the paint: width 3, centred, butt
/// caps, miter corners with limit 100, solid, opacity 1, dashes in widths.
///
/// Serialised in camelCase with every field optional (missing fields take the default), so an
/// app keeps it in its settings and documents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StrokeStyle {
    /// The width in document units.
    pub width: f32,
    /// Where the stroke lies relative to the path.
    pub align: StrokeAlign,
    /// The shape of open ends.
    pub cap: LineCap,
    /// The shape of corners.
    pub join: LineJoin,
    /// The miter limit as a multiple of the width (Photoshop's 100 mitres nearly every corner).
    pub miter_limit: f32,
    /// Dash and gap lengths, alternating, in [`dash_unit`](Self::dash_unit); empty is solid.
    pub dashes: Vec<f32>,
    /// Where the pattern starts along the path, in [`dash_unit`](Self::dash_unit).
    pub dash_offset: f32,
    /// What a dash or gap length counts in.
    pub dash_unit: DashUnit,
    /// How dashes sit along the path. [`DashAlign::Corners`] needs the caller to stroke
    /// [`dashed`](Self::dashed)'s path solid: [`to_kurbo`](Self::to_kurbo) keeps the `Exact` pattern.
    pub dash_align: DashAlign,
    /// 0..=1.
    pub opacity: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        // pc: crates/doc/src/vector.rs:251-264 `ShapeStroke::default`.
        Self {
            width: 3.0,
            align: StrokeAlign::Center,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 100.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
            dash_unit: DashUnit::Widths,
            dash_align: DashAlign::Exact,
            opacity: 1.0,
        }
    }
}

impl StrokeStyle {
    /// The widths [`validate`](Self::validate) accepts.
    pub const WIDTH_RANGE: RangeInclusive<f32> = 0.0..=1_000_000.0;
    /// The miter limits [`validate`](Self::validate) accepts.
    pub const MITER_LIMIT_RANGE: RangeInclusive<f32> = 1.0..=500.0;
    /// The dash and gap lengths [`validate`](Self::validate) accepts.
    pub const DASH_RANGE: RangeInclusive<f32> = 0.0..=10_000.0;
    /// The largest dash offset, either way.
    pub const DASH_OFFSET_MAX: f32 = 1_000_000.0;
    /// The most dash and gap lengths a pattern holds (16 pairs).
    pub const MAX_DASHES: usize = 32;
    /// The smallest total of a dashed pattern.
    pub const DASH_TOTAL_MIN: f32 = 0.01;
    /// The Dashed preset's pattern.
    pub const DASHED: [f32; 2] = [4.0, 2.0];
    /// The Dotted preset's pattern (with round caps).
    pub const DOTTED: [f32; 2] = [0.0, 2.0];

    /// Whether an engine accepts the style. PhotoCraft's rules, checked in this order, the first
    /// failure being the error: width finite and in [`WIDTH_RANGE`](Self::WIDTH_RANGE); opacity
    /// finite and in 0..=1; miter limit finite and in
    /// [`MITER_LIMIT_RANGE`](Self::MITER_LIMIT_RANGE); dash offset finite with `abs() <=`
    /// [`DASH_OFFSET_MAX`](Self::DASH_OFFSET_MAX); at most [`MAX_DASHES`](Self::MAX_DASHES)
    /// lengths, each finite and in [`DASH_RANGE`](Self::DASH_RANGE); a non-empty pattern whose sum
    /// is at least [`DASH_TOTAL_MIN`](Self::DASH_TOTAL_MIN). The limits are the same in either
    /// [`DashUnit`].
    pub fn validate(&self) -> Result<(), StrokeError> {
        // pc: crates/doc/src/vector.rs:270-290 `ShapeStroke::validate`.
        if !self.width.is_finite() || !Self::WIDTH_RANGE.contains(&self.width) {
            return Err(StrokeError::Width);
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(StrokeError::Opacity);
        }
        if !self.miter_limit.is_finite() || !Self::MITER_LIMIT_RANGE.contains(&self.miter_limit) {
            return Err(StrokeError::MiterLimit);
        }
        if !self.dash_offset.is_finite() || self.dash_offset.abs() > Self::DASH_OFFSET_MAX {
            return Err(StrokeError::DashOffset);
        }
        if self.dashes.len() > Self::MAX_DASHES || self.dashes.iter().any(|d| !d.is_finite() || !Self::DASH_RANGE.contains(d)) {
            return Err(StrokeError::DashPattern);
        }
        if !self.dashes.is_empty() && self.dashes.iter().sum::<f32>() < Self::DASH_TOTAL_MIN {
            return Err(StrokeError::DashTotal);
        }
        Ok(())
    }

    /// The pattern's name: empty dashes are [`LineStyle::Solid`]; exactly
    /// [`DASHED`](Self::DASHED) is [`LineStyle::Dashed`]; exactly [`DOTTED`](Self::DOTTED) with
    /// [`LineCap::Round`] is [`LineStyle::Dotted`]; anything else is [`LineStyle::Custom`]
    /// (`[0, 2]` with another cap too). The dash offset plays no part.
    pub fn line_style(&self) -> LineStyle {
        // pc: crates/ui-egui/src/tools/shape_stroke_ui.rs:53-63 `StrokeOptions::label`.
        if self.dashes.is_empty() {
            LineStyle::Solid
        } else if self.dashes == Self::DASHED {
            LineStyle::Dashed
        } else if self.dashes == Self::DOTTED && self.cap == LineCap::Round {
            LineStyle::Dotted
        } else {
            LineStyle::Custom
        }
    }

    /// This style with the preset `style`: the dash offset goes to 0 and the dashes become empty
    /// (Solid), [`DASHED`](Self::DASHED), or [`DOTTED`](Self::DOTTED) with the cap set to
    /// [`LineCap::Round`]. Every other field is kept. [`LineStyle::Custom`] returns the style
    /// unchanged.
    #[must_use]
    pub fn with_line_style(&self, style: LineStyle) -> StrokeStyle {
        // pc: crates/ui-egui/src/tools/shape_stroke_ui.rs:65-75 `StrokeOptions::preset`.
        let mut out = self.clone();
        match style {
            LineStyle::Custom => return out,
            LineStyle::Solid => out.dashes = Vec::new(),
            LineStyle::Dashed => out.dashes = Self::DASHED.to_vec(),
            LineStyle::Dotted => {
                out.dashes = Self::DOTTED.to_vec();
                out.cap = LineCap::Round;
            }
        }
        out.dash_offset = 0.0;
        out
    }

    /// The dashes as (dash, gap) pairs in order. An odd-length pattern's last pair has no gap
    /// (`None`), as PhotoCraft's editor shows it (one Dash field in the last row).
    pub fn dash_pairs(&self) -> Vec<(f32, Option<f32>)> {
        // pc: crates/ui-egui/src/tools/shape_stroke_ui/editor.rs:201-211 (`dashes.chunks_mut(2)`).
        self.dashes.chunks(2).map(|c| (c[0], c.get(1).copied())).collect()
    }

    /// This style with its first `pairs.len()` pairs replaced by `pairs` and every later entry of
    /// the pattern kept, so a host that shows three pairs (VectorCraft, Photoshop) never loses
    /// pairs four and on. A `None` gap drops the gap when it replaces the pattern's last pair and
    /// keeps the stored gap anywhere else. Pairs past the stored ones are appended. The result is
    /// not validated.
    #[must_use]
    pub fn with_dash_pairs(&self, pairs: &[(f32, Option<f32>)]) -> StrokeStyle {
        // new: entries a field row does not show stay unchanged (step7-pen "Stroke editor", Dashes).
        let mut out = self.clone();
        for (i, &(dash, gap)) in pairs.iter().enumerate() {
            let at = i * 2;
            if at >= out.dashes.len() {
                out.dashes.push(dash);
                out.dashes.extend(gap);
                continue;
            }
            out.dashes[at] = dash;
            let is_last = at + 1 >= out.dashes.len();
            match (gap, is_last) {
                (Some(g), false) => out.dashes[at + 1] = g,
                (Some(g), true) => out.dashes.push(g),
                (None, true) => {}
                (None, false) => {
                    // The stored gap stays unless this pair is the pattern's last.
                    if at + 2 >= out.dashes.len() {
                        out.dashes.truncate(at + 1);
                    }
                }
            }
        }
        out
    }

    /// The path cut into its dashes, lengths in document units as [`to_kurbo`](Self::to_kurbo)
    /// scales them times `length_scale` (1 for a document; a preview passes its sample-to-style
    /// width ratio). A solid style returns the path, as does a style that fails
    /// [`validate`](Self::validate) and a pattern that would give more than 200,000 dashes.
    ///
    /// [`DashAlign::Exact`] dashes from the dash offset. [`DashAlign::Corners`] ignores the
    /// offset: each run between corners and path ends is fitted to a whole number of periods and
    /// starts and ends in the middle of a dash, so a dash is centred on each corner and each end
    /// of an open path.
    pub fn dashed(&self, path: &kurbo::BezPath, length_scale: f64) -> kurbo::BezPath {
        if self.dashes.is_empty() || self.validate().is_err() {
            return path.clone();
        }
        let scale = self.dash_scale() * length_scale;
        let pattern: Vec<f64> = self.dashes.iter().map(|&d| f64::from(d) * scale).collect();
        // `!(x > y)` also refuses a NaN total.
        if pattern.iter().sum::<f64>().partial_cmp(&MIN_RUN) != Some(std::cmp::Ordering::Greater) {
            return path.clone();
        }
        let cut = match self.dash_align {
            DashAlign::Exact => dash_exact(path, f64::from(self.dash_offset) * scale, &pattern),
            DashAlign::Corners => dash_corners(path, &pattern),
        };
        cut.unwrap_or_else(|| path.clone())
    }

    /// The style as a kurbo stroke, for an engine and the editor's line samples: width, join,
    /// miter limit and the cap at both ends as they are; dash lengths and offset multiplied by the
    /// width under [`DashUnit::Widths`], taken as they are under [`DashUnit::Document`]. Alignment
    /// and opacity are not part of a kurbo stroke: an engine strokes an aligned closed path twice
    /// as wide and clips to its inside ([`StrokeAlign::Inside`]) or outside
    /// ([`StrokeAlign::Outside`]), and applies the opacity itself. The pattern is the
    /// [`DashAlign::Exact`] one: a [`DashAlign::Corners`] caller strokes the path of
    /// [`dashed`](Self::dashed) solid.
    pub fn to_kurbo(&self) -> kurbo::Stroke {
        // new: kurbo `Stroke::new(width).with_join(..).with_miter_limit(..).with_caps(..)
        // .with_dashes(offset, pattern)`. PhotoCraft strokes with its own engine
        // (crates/ui-egui/src/tools/shape_stroke_ui/editor.rs:329-343 renders through it).
        let scale = self.dash_scale();
        let cap = match self.cap {
            LineCap::Butt => kurbo::Cap::Butt,
            LineCap::Round => kurbo::Cap::Round,
            LineCap::Square => kurbo::Cap::Square,
        };
        let join = match self.join {
            LineJoin::Miter => kurbo::Join::Miter,
            LineJoin::Round => kurbo::Join::Round,
            LineJoin::Bevel => kurbo::Join::Bevel,
        };
        kurbo::Stroke::new(f64::from(self.width))
            .with_join(join)
            .with_miter_limit(f64::from(self.miter_limit))
            .with_caps(cap)
            .with_dashes(f64::from(self.dash_offset) * scale, self.dashes.iter().map(|&d| f64::from(d) * scale))
    }

    /// What one stored dash length is in document units.
    fn dash_scale(&self) -> f64 {
        match self.dash_unit {
            DashUnit::Widths => f64::from(self.width),
            DashUnit::Document => 1.0,
        }
    }
}

/// `path` dashed by `pattern` from `offset`; `None` when it would give too many dashes.
fn dash_exact(path: &BezPath, offset: f64, pattern: &[f64]) -> Option<BezPath> {
    let mut out = BezPath::new();
    let mut count = 0usize;
    for el in kurbo::dash(path.iter(), offset, pattern) {
        if matches!(el, PathEl::MoveTo(_)) {
            count += 1;
            if count > MAX_OUTPUT_DASHES {
                return None;
            }
        }
        out.push(el);
    }
    Some(out)
}

/// `path` dashed with every run between corners and path ends fitted to whole periods; `None` when
/// it would give too many dashes.
fn dash_corners(path: &BezPath, pattern: &[f64]) -> Option<BezPath> {
    let mut pattern = pattern.to_vec();
    // An odd pattern repeats with its roles swapped, as kurbo reads it.
    if pattern.len() % 2 == 1 {
        pattern.extend_from_within(..);
    }
    let period: f64 = pattern.iter().sum();
    let mut out = BezPath::new();
    let mut count = 0usize;
    for (segs, closed) in subpaths(path) {
        for dash in fit_subpath(segs, closed, &pattern, period, &mut count)? {
            for el in dash {
                out.push(el);
            }
        }
    }
    Some(out)
}

/// The subpaths of `path` as segments, each with whether it closes.
fn subpaths(path: &BezPath) -> Vec<(Vec<PathSeg>, bool)> {
    let mut out = Vec::new();
    let mut segs: Vec<PathSeg> = Vec::new();
    let (mut start, mut cur) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !segs.is_empty() {
                    out.push((std::mem::take(&mut segs), false));
                }
                (start, cur) = (p, p);
            }
            PathEl::LineTo(p) => {
                segs.push(PathSeg::Line(Line::new(cur, p)));
                cur = p;
            }
            PathEl::QuadTo(a, p) => {
                segs.push(PathSeg::Quad(QuadBez::new(cur, a, p)));
                cur = p;
            }
            PathEl::CurveTo(a, b, p) => {
                segs.push(PathSeg::Cubic(CubicBez::new(cur, a, b, p)));
                cur = p;
            }
            PathEl::ClosePath => {
                if cur.distance(start) > EPS {
                    segs.push(PathSeg::Line(Line::new(cur, start)));
                }
                if !segs.is_empty() {
                    out.push((std::mem::take(&mut segs), true));
                }
                cur = start;
            }
        }
    }
    if !segs.is_empty() {
        out.push((segs, false));
    }
    out
}

/// The unit direction of `seg` at its end or start; zero for a degenerate segment.
fn tangent(seg: &PathSeg, at_end: bool) -> Vec2 {
    let pts: Vec<Point> = match seg {
        PathSeg::Line(l) => vec![l.p0, l.p1],
        PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
        PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    };
    let direction = if at_end {
        let anchor = pts[pts.len() - 1];
        pts[..pts.len() - 1].iter().rev().map(|p| anchor - *p).find(|d| d.hypot() > EPS)
    } else {
        let anchor = pts[0];
        pts[1..].iter().map(|p| *p - anchor).find(|d| d.hypot() > EPS)
    };
    direction.map_or(Vec2::ZERO, Vec2::normalize)
}

/// Whether the path turns where `a` ends and `b` starts.
fn is_kink(a: &PathSeg, b: &PathSeg) -> bool {
    let (ta, tb) = (tangent(a, true), tangent(b, false));
    if ta.hypot() < EPS || tb.hypot() < EPS {
        return false;
    }
    ta.cross(tb).abs() > EPS || ta.dot(tb) < 0.0
}

fn end_point(el: &PathEl) -> Option<Point> {
    match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(p),
        PathEl::ClosePath => None,
    }
}

fn ends_at(dash: &[PathEl], p: Point) -> bool {
    dash.last().and_then(end_point).is_some_and(|q| q.distance(p) < JOIN_EPS)
}

fn starts_at(dash: &[PathEl], p: Point) -> bool {
    dash.first().and_then(end_point).is_some_and(|q| q.distance(p) < JOIN_EPS)
}

/// The dashes of one subpath, each as path elements from a `MoveTo`. Every run between kinks (and
/// the path's ends) is stretched to whole periods and dashed from the middle of the first dash, so
/// the two half dashes that meet at a corner are joined into one dash; a closed subpath joins its
/// last dash to its first the same way. A closed subpath without a kink is one run starting at its
/// first point. `None` when `count` passes [`MAX_OUTPUT_DASHES`].
fn fit_subpath(segs: Vec<PathSeg>, closed: bool, pattern: &[f64], period: f64, count: &mut usize) -> Option<Vec<Vec<PathEl>>> {
    let segs: Vec<PathSeg> = segs.into_iter().filter(|s| s.arclen(ARCLEN_ACCURACY) > EPS).collect();
    let n = segs.len();
    if n == 0 {
        return Some(Vec::new());
    }
    let kink_after = |i: usize| (closed || i + 1 < n) && is_kink(&segs[i], &segs[(i + 1) % n]);
    let first = if closed { (0..n).find(|&i| kink_after(i)).map_or(0, |i| (i + 1) % n) } else { 0 };
    let origin = segs[first].start();

    let mut runs: Vec<Vec<PathSeg>> = Vec::new();
    let mut current: Vec<PathSeg> = Vec::new();
    for step in 0..n {
        let i = (first + step) % n;
        current.push(segs[i]);
        if step + 1 < n && kink_after(i) {
            runs.push(std::mem::take(&mut current));
        }
    }
    runs.push(current);

    let mut dashes: Vec<Vec<PathEl>> = Vec::new();
    for run in &runs {
        let length: f64 = run.iter().map(|s| s.arclen(ARCLEN_ACCURACY)).sum();
        if length <= MIN_RUN {
            continue;
        }
        let periods = (length / period).round().max(1.0);
        let fit = length / (periods * period);
        let scaled: Vec<f64> = pattern.iter().map(|p| p * fit).collect();
        let begin = run[0].start();
        let mut path = BezPath::new();
        path.move_to(begin);
        for seg in run {
            path.push(seg.as_path_el());
        }
        let mut fresh: Vec<Vec<PathEl>> = Vec::new();
        for el in kurbo::dash(path.iter(), scaled[0] / 2.0, &scaled) {
            if matches!(el, PathEl::MoveTo(_)) {
                *count += 1;
                if *count > MAX_OUTPUT_DASHES {
                    return None;
                }
                fresh.push(vec![el]);
            } else if let Some(dash) = fresh.last_mut() {
                dash.push(el);
            }
        }
        // `kurbo::dash` holds back a subpath's first dash, to join it with a dash that wraps through a `ClosePath`,
        // and emits it last. A run never closes, so its first dash goes back to the front.
        if fresh.len() > 1 && fresh.last().is_some_and(|dash| starts_at(dash, begin)) {
            fresh.rotate_right(1);
        }
        let mut fresh = fresh.into_iter().peekable();
        if let Some(prev) = dashes.last_mut()
            && ends_at(prev, begin)
            && fresh.peek().is_some_and(|d| starts_at(d, begin))
            && let Some(head) = fresh.next()
        {
            prev.extend(head.into_iter().skip(1));
        }
        dashes.extend(fresh);
    }
    if closed && dashes.len() > 1 && dashes.last().is_some_and(|d| ends_at(d, origin)) && starts_at(&dashes[0], origin) {
        let head = dashes.remove(0);
        if let Some(last) = dashes.last_mut() {
            last.extend(head.into_iter().skip(1));
        }
    }
    Some(dashes)
}

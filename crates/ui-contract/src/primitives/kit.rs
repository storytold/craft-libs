//! The content-paint kit (5.4): the recurring shapes of document and plot painting, read from tokens, so an
//! allowlisted content painter draws them through one implementation. The shape functions are pure; the
//! `ContentPainterCx` methods add their shapes to its painter.

use super::content::ContentPainterCx;
use super::draw::{Dashes, PaintShape, StrokeWidth};
use super::rule::{Rule, RuleKind};
use super::surface::Surface;
use crate::style::{Corners, DataColor, RuleLook, SurfaceKind, UiTokens, ink_color, rgba_color, rule_look, surface_style};
use craft_behavior::{Axis, CanvasView, kurbo};
use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use std::f32::consts::TAU;

/// Which edge of a rect a tick row sits on, and so which way its marks point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TickEdge {
    /// Marks rise from the bottom edge, positions run along x.
    Bottom,
    /// Marks hang from the top edge, positions run along x.
    Top,
    /// Marks grow from the left edge, positions run along y.
    Left,
    /// Marks grow from the right edge, positions run along y.
    Right,
}

/// How long a tick is: the rank it has among its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TickRank {
    /// The shortest, `space.gap.tight`.
    Minor,
    /// Halfway between two major ticks, `size.ruler.tick.mid`.
    Mid,
    /// The longest, `space.gap.inline`.
    Major,
}

/// One tick: where it sits along its edge and how long it is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Tick {
    /// The position along the edge, in points from the rect's origin on that axis.
    pub at: f32,
    /// The tick's length class.
    pub rank: TickRank,
}

impl Tick {
    /// A tick at `at` of the length class `rank`.
    pub const fn new(at: f32, rank: TickRank) -> Self {
        Tick { at, rank }
    }
}

/// The cells of the transparency checkerboard: their side and the two colours. A field left unset takes the
/// theme's checker (`size.checker` and the overlay checker colours); the transparency grid is an app setting, so an
/// app sets any of them from its preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Checker {
    /// The side of a cell, in points.
    pub size: Option<f32>,
    /// The colour of the cells that fill the rect.
    pub light: Option<DataColor>,
    /// The colour of the cells that sit on the light ones.
    pub dark: Option<DataColor>,
}

impl Checker {
    /// The transparency grid preference's default, which an app shows until the user picks another: medium cells
    /// of 8 points, light colours (white and 80 % grey). The gallery's story documents show it.
    pub const PREFERENCE_DEFAULT: Checker =
        Checker::new().size(8.0).colors(DataColor::new(Color32::from_rgb(255, 255, 255)), DataColor::new(Color32::from_rgb(204, 204, 204)));

    /// The theme's checker.
    pub const fn new() -> Self {
        Checker { size: None, light: None, dark: None }
    }

    /// Cells of `size` points.
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// The light and dark colours.
    #[must_use]
    pub const fn colors(mut self, light: DataColor, dark: DataColor) -> Self {
        self.light = Some(light);
        self.dark = Some(dark);
        self
    }

    /// The checkerboard over `rect`, drawn where `clip` shows it: the light colour fills it and the dark cells sit
    /// on top, from the rect's top-left corner.
    pub fn shapes(&self, tokens: &UiTokens, rect: Rect, clip: Rect) -> Vec<Shape> {
        let visible = rect.intersect(clip);
        if visible.width() <= 0.0 || visible.height() <= 0.0 {
            return Vec::new();
        }
        let side = self.size.unwrap_or(tokens.size.checker).max(f32::EPSILON);
        let light = self.light.map_or_else(|| rgba_color(tokens.color.overlay.checker_light), |color| color.0);
        let dark = self.dark.map_or_else(|| rgba_color(tokens.color.overlay.checker_dark), |color| color.0);
        let mut shapes = vec![Shape::rect_filled(visible, 0.0, light)];
        let cells = |from: f32, to: f32, origin: f32| {
            let at = |edge: f32, round: fn(f32) -> f32| round((edge - origin) / side).max(0.0) as usize;
            at(from, f32::floor)..at(to, f32::ceil)
        };
        for row in cells(visible.top(), visible.bottom(), rect.top()) {
            for column in cells(visible.left(), visible.right(), rect.left()).filter(|column| (column + row) % 2 == 0) {
                let min = rect.min + vec2(column as f32 * side, row as f32 * side);
                let cell = Rect::from_min_size(min, Vec2::splat(side)).intersect(visible);
                shapes.push(Shape::rect_filled(cell, 0.0, dark));
            }
        }
        shapes
    }
}

/// The theme's transparency checkerboard over `rect`, drawn where `clip` shows it, with cells of `size.checker`.
pub fn checker(tokens: &UiTokens, rect: Rect, clip: Rect) -> Vec<Shape> {
    Checker::new().shapes(tokens, rect, clip)
}

/// A colour ramp over `rect`: the strip shades from stop to stop along `axis` (`Axis::X` runs left to right, `Axis::Y`
/// top to bottom). Each stop is its place along the strip, from 0 at the start to 1 at the end, and its colour. Stops
/// are taken in order; a place outside 0 to 1 is clamped, and a ramp whose first stop is after 0 or whose last is
/// before 1 holds that colour out to the edge. The colours blend as the renderer blends mesh vertices, in sRGB.
pub fn ramp(rect: Rect, stops: &[(f32, DataColor)], axis: Axis) -> Vec<Shape> {
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return Vec::new();
    }
    let mut marks: Vec<(f32, Color32)> = stops.iter().filter(|(at, _)| at.is_finite()).map(|(at, color)| (at.clamp(0.0, 1.0), color.0)).collect();
    let (Some(&(first_at, first)), Some(&(last_at, last))) = (marks.first(), marks.last()) else { return Vec::new() };
    if first_at > 0.0 {
        marks.insert(0, (0.0, first));
    }
    if last_at < 1.0 {
        marks.push((1.0, last));
    }
    let vertical = axis == Axis::Y;
    let (from, to) = if vertical { (rect.left_top(), rect.left_bottom()) } else { (rect.left_top(), rect.right_top()) };
    let across = if vertical { vec2(rect.width(), 0.0) } else { vec2(0.0, rect.height()) };
    let mut mesh = Mesh::default();
    for (index, (at, color)) in marks.iter().enumerate() {
        let on = from + (to - from) * *at;
        mesh.colored_vertex(on, *color);
        mesh.colored_vertex(on + across, *color);
        if let Some(base) = (index as u32).checked_sub(1).map(|previous| previous * 2) {
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    vec![Shape::mesh(mesh)]
}

/// Most lines a grid draws along one axis.
pub const MAX_LINES: usize = 10_000;

/// The smallest step, in screen points, at which a grid axis draws.
const MIN_STEP: f64 = 6.0;

/// The smallest major step, in screen points, at which crossings draw.
const CROSSING_MIN: f64 = 8.0;

/// Minor lines' share of the major colour's alpha.
const MINOR_SHARE: f32 = 0.45;

/// The radius of a dot, in screen points.
const DOT_RADIUS: f32 = 0.6;

/// The lines of one axis: positions along it, in the grid's space.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GridAxis {
    /// The minor lines.
    pub minor: Vec<f64>,
    /// The major lines.
    pub major: Vec<f64>,
}

impl GridAxis {
    /// The lines of one axis between `lo` and `hi`: majors every `step` from `origin`, minors between them when the
    /// minor step is at least [`MIN_STEP`] screen points at `zoom`. An axis whose major step is under `MIN_STEP`
    /// draws nothing; at most [`MAX_LINES`] lines are given.
    fn spaced(step: f64, subdivisions: u32, origin: f64, (lo, hi): (f64, f64), zoom: f64) -> GridAxis {
        if !(step.is_finite() && step > 0.0 && zoom.is_finite() && zoom > 0.0) || step * zoom < MIN_STEP {
            return GridAxis::default();
        }
        let subdivided = subdivisions > 1 && step / f64::from(subdivisions) * zoom >= MIN_STEP;
        let pitch = if subdivided { step / f64::from(subdivisions) } else { step };
        let every = if subdivided { i64::from(subdivisions) } else { 1 };
        let (first, last) = (((lo - origin) / pitch).ceil(), ((hi - origin) / pitch).floor());
        let mut lines = GridAxis::default();
        if !(first.is_finite() && last.is_finite()) || last < first {
            return lines;
        }
        for i in (first as i64..=last as i64).take(MAX_LINES) {
            let at = origin + i as f64 * pitch;
            if i.rem_euclid(every) == 0 { lines.major.push(at) } else { lines.minor.push(at) }
        }
        lines
    }
}

/// The lines of a grid, as [`grid_lines`] draws them: vertical lines (by x) and horizontal lines (by y), each
/// running across `extent`. The positions are in document units under `view`, or in screen points without one.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GridLines {
    /// The vertical lines.
    pub x: GridAxis,
    /// The horizontal lines.
    pub y: GridAxis,
    /// Where the lines run; `None` when the grid draws nothing.
    pub extent: Option<kurbo::Rect>,
    /// Whether the major spacing is wide enough for crossings on both axes.
    pub crossings: bool,
    /// The view that maps the positions to the screen.
    pub view: Option<CanvasView>,
}

impl GridLines {
    /// The lines of a grid in document units: majors every `spacing` from `origin`, `subdivisions` steps to a major
    /// (1 or 0 is no minor lines), across `extent` as `view` shows it.
    pub fn document(view: &CanvasView, extent: kurbo::Rect, spacing: kurbo::Vec2, origin: kurbo::Point, subdivisions: u32) -> Self {
        let subdivisions = subdivisions.max(1);
        let x = GridAxis::spaced(spacing.x, subdivisions, origin.x, (extent.x0, extent.x1), view.zoom_x());
        let y = GridAxis::spaced(spacing.y, subdivisions, origin.y, (extent.y0, extent.y1), view.zoom);
        let crossings = spacing.x * view.zoom_x() >= CROSSING_MIN && spacing.y * view.zoom >= CROSSING_MIN;
        GridLines { x, y, extent: Some(extent), crossings, view: Some(*view) }
    }

    /// The lines over `rect` that `clip` shows, every `spacing` points from the rect's top-left corner, all major.
    /// Lines closer than a device pixel are drawn a pixel apart.
    pub fn screen(rect: Rect, spacing: Vec2, clip: Rect, pixels_per_point: f32) -> Self {
        let visible = rect.intersect(clip);
        if visible.width() < 0.0 || visible.height() < 0.0 {
            return GridLines::default();
        }
        let pixel = 1.0 / pixels_per_point.max(f32::EPSILON);
        let along = |spacing: f32, from: f32, lo: f32, hi: f32| {
            let mut axis = GridAxis::default();
            if spacing > 0.0 {
                let step = spacing.max(pixel);
                let first = ((lo - from) / step).ceil().max(0.0);
                axis.major = (0..MAX_LINES).map(|n| from + (first + n as f32) * step).take_while(|at| *at <= hi).map(f64::from).collect();
            }
            axis
        };
        let extent = kurbo::Rect::new(visible.left().into(), visible.top().into(), visible.right().into(), visible.bottom().into());
        GridLines {
            x: along(spacing.x, rect.left(), visible.left(), visible.right()),
            y: along(spacing.y, rect.top(), visible.top(), visible.bottom()),
            extent: Some(extent),
            crossings: false,
            view: None,
        }
    }
}

/// How the lines of a grid are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GridStroke {
    /// An unbroken line.
    #[default]
    Solid,
    /// Dashes of `stroke.dash` with gaps of `stroke.dash_gap`.
    Dashed,
    /// Dots `stroke.dash_gap` apart.
    Dots,
}

/// The colours a grid's lines take; an unset one is the theme's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GridInk {
    /// The major lines' colour: the overlay grid colour by default.
    pub major: Option<DataColor>,
    /// The minor lines' colour: the major colour at a share of its alpha by default.
    pub minor: Option<DataColor>,
    /// How the lines are drawn.
    pub stroke: GridStroke,
}

impl GridInk {
    /// The theme's colours and solid lines.
    pub const fn new() -> Self {
        GridInk { major: None, minor: None, stroke: GridStroke::Solid }
    }

    /// The major lines in `color`.
    #[must_use]
    pub const fn major(mut self, color: DataColor) -> Self {
        self.major = Some(color);
        self
    }

    /// The minor lines in `color`.
    #[must_use]
    pub const fn minor(mut self, color: DataColor) -> Self {
        self.minor = Some(color);
        self
    }

    /// The lines drawn as `stroke`.
    #[must_use]
    pub const fn stroke(mut self, stroke: GridStroke) -> Self {
        self.stroke = stroke;
        self
    }
}

/// What the lines of a grid read from the tokens: the line colour and width, and the dash and gap lengths.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridLook {
    color: Color32,
    width: f32,
    dash: f32,
    gap: f32,
}

impl GridLook {
    /// The grid look of `tokens`.
    pub fn of(tokens: &UiTokens) -> Self {
        let look = rule_look(tokens, RuleKind::Grid);
        GridLook { color: look.color, width: look.width, dash: tokens.stroke.dash, gap: tokens.stroke.dash_gap }
    }
}

/// The lines of `grid` as `ink` draws them: minor lines first, then the major ones, each running across
/// the grid's extent and mapped through its view.
pub fn grid_lines(tokens: &UiTokens, grid: &GridLines, ink: &GridInk, pixels_per_point: f32) -> Vec<Shape> {
    grid_lines_with(&GridLook::of(tokens), grid, ink, pixels_per_point)
}

/// [`grid_lines`] from a look read earlier.
pub fn grid_lines_with(look: &GridLook, grid: &GridLines, ink: &GridInk, pixels_per_point: f32) -> Vec<Shape> {
    let Some(extent) = grid.extent else { return Vec::new() };
    let major = ink.major.map_or(look.color, |color| color.0);
    let minor = ink.minor.map_or_else(|| major.gamma_multiply(MINOR_SHARE), |color| color.0);
    let screen = |x: f64, y: f64| match &grid.view {
        Some(view) => view.to_screen(kurbo::Point::new(x, y)),
        None => pos2(x as f32, y as f32),
    };
    let mut shapes = Vec::new();
    for (color, xs, ys) in [(minor, &grid.x.minor, &grid.y.minor), (major, &grid.x.major, &grid.y.major)] {
        let ends =
            xs.iter().map(|&x| (screen(x, extent.y0), screen(x, extent.y1))).chain(ys.iter().map(|&y| (screen(extent.x0, y), screen(extent.x1, y))));
        for (a, b) in ends {
            match ink.stroke {
                GridStroke::Solid => shapes.extend(Rule::shapes(&RuleLook { color, width: look.width }, a, b, pixels_per_point)),
                GridStroke::Dashed => shapes.extend(dashed(&[a, b], Dashes::new(look.dash, look.gap), look.width, color)),
                GridStroke::Dots => shapes.extend(Shape::dotted_line(&[a, b], color, look.gap, DOT_RADIUS)),
            }
        }
    }
    shapes
}

/// The tick marks of `ticks` on one edge of `rect` that `clip` shows, `stroke.thin` lines of `text.faint`: minor ones
/// `gap.tight` long, mid ones `size.ruler.tick.mid`, major ones across the whole depth of `rect`, beside their labels.
pub fn ticks(tokens: &UiTokens, rect: Rect, edge: TickEdge, ticks: &[Tick], clip: Rect, pixels_per_point: f32) -> Vec<Shape> {
    let look = RuleLook { color: rgba_color(tokens.color.text.faint), width: tokens.stroke.thin };
    let depth = match edge {
        TickEdge::Bottom | TickEdge::Top => rect.height(),
        TickEdge::Left | TickEdge::Right => rect.width(),
    };
    let (minor, mid, major) = (tokens.space.gap.tight, tokens.size.ruler.tick.mid, depth);
    ticks
        .iter()
        .filter_map(|tick| {
            let length = match tick.rank {
                TickRank::Minor => minor,
                TickRank::Mid => mid,
                TickRank::Major => major,
            };
            let (from, to) = match edge {
                TickEdge::Bottom => (pos2(rect.left() + tick.at, rect.bottom()), pos2(rect.left() + tick.at, rect.bottom() - length)),
                TickEdge::Top => (pos2(rect.left() + tick.at, rect.top()), pos2(rect.left() + tick.at, rect.top() + length)),
                TickEdge::Left => (pos2(rect.left(), rect.top() + tick.at), pos2(rect.left() + length, rect.top() + tick.at)),
                TickEdge::Right => (pos2(rect.right(), rect.top() + tick.at), pos2(rect.right() - length, rect.top() + tick.at)),
            };
            clip.intersects(Rect::from_two_pos(from, to)).then(|| Rule::shapes(&look, from, to, pixels_per_point))
        })
        .flatten()
        .collect()
}

/// The line that marks a shape on any image: a `thick` halo under a `medium` line, in the overlay colours.
pub fn outline(tokens: &UiTokens, shape: &PaintShape) -> Vec<Shape> {
    outline_in(tokens, std::slice::from_ref(shape), rgba_color(tokens.color.overlay.line))
}

/// The outline of [`outline`] with the line in `line`, a colour the user picked (a Navigator proxy colour), over the
/// overlay halo. Every halo is drawn under every line, so the edges of one mark that meet or cross stay unbroken.
pub fn outline_in(tokens: &UiTokens, shapes: &[PaintShape], line: Color32) -> Vec<Shape> {
    let halo = Stroke::new(tokens.stroke.thick, rgba_color(tokens.color.overlay.halo));
    let line = Stroke::new(tokens.stroke.medium, line);
    let halos = shapes.iter().flat_map(|shape| stroked(shape, halo));
    halos.chain(shapes.iter().flat_map(|shape| stroked(shape, line))).collect()
}

/// A dashed line along `points` in `color`, `width` thick, with the lengths and phase of `pattern`. The pattern
/// continues across the segments; the corners are not closed. The one dashed-line implementation: `Draw::dashed` and
/// the painter's `dashed` call it.
pub fn dashed(points: &[Pos2], pattern: Dashes, width: f32, color: Color32) -> Vec<Shape> {
    Shape::dashed_line_with_offset(points, Stroke::new(width, color), &[pattern.dash], &[pattern.gap], pattern.phase)
}

/// Longer than any line that fits the view: only its part in view is dashed.
const LONG_EDGE: f32 = 8192.0;

/// The ants' geometry on a display of some device pixels per point, at one moment of the march.
#[derive(Clone, Copy)]
struct Ants {
    ppp: f32,
    /// Line thickness, in device pixels.
    width: i64,
    /// Dash length, in device pixels.
    dash: i64,
    /// How far the pattern has marched, in device pixels.
    phase: i64,
}

impl Ants {
    /// Ants of `stroke.dash` line widths, a line width being a whole number of device pixels (one at 100 percent,
    /// two up to 200 percent), marched `phase` points.
    fn new(tokens: &UiTokens, ppp: f32, phase: f32) -> Self {
        let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
        // A hair over a whole scale (a windowing system's rounding) is still that scale.
        let width = ((ppp - 0.01).ceil() as i64).clamp(1, 8);
        let dash = (tokens.stroke.dash.round().max(1.0) as i64) * width;
        let phase = if phase.is_finite() { (f64::from(phase) * f64::from(ppp)).round().rem_euclid((2 * dash) as f64) as i64 } else { 0 };
        Ants { ppp, width, dash, phase }
    }

    /// The dark runs of `[lo, hi)` along one pixel row (or column) of a band. A pixel is dark where
    /// `(s + phase) mod 2 * dash < dash`, `s` being its position along the edge plus `off`, `width` times its position
    /// across it: a dash ends on a slant in a band thicker than a pixel.
    fn runs(&self, lo: i64, hi: i64, off: i64) -> impl Iterator<Item = (i64, i64)> + '_ {
        let period = 2 * self.dash;
        let first = lo - (lo + off + self.phase).rem_euclid(period);
        std::iter::successors(Some(first), move |s| Some(s + period))
            .take_while(move |&s| s < hi)
            .map(move |s| (s.max(lo), (s + self.dash).min(hi)))
            .filter(|(s, e)| e > s)
    }
}

/// Marching ants along `points` (closed when `closed`) inside `clip`: bands of the marquee colour with dark dashes in
/// the halo colour, shifted by `phase` points along the path. The pattern is fixed to the screen's device pixels, so
/// each horizontal or vertical edge is a band of whole pixels (one at 100 percent, two up to 200 percent) at any
/// fractional scale and one mesh of pixel-aligned quads; a slanted edge stays a line shape.
pub fn marching_ants(tokens: &UiTokens, points: &[Pos2], closed: bool, phase: f32, clip: Rect, pixels_per_point: f32) -> Vec<Shape> {
    let (light, dark) = (rgba_color(tokens.color.overlay.marquee), rgba_color(tokens.color.overlay.halo));
    let ants = Ants::new(tokens, pixels_per_point, phase);
    let Ants { ppp, width: w, dash, .. } = ants;
    let mut mesh = egui::Mesh::default();
    let mut slanted = Vec::new();
    // Device pixels to a rectangle in points.
    let mut quad = |x0: i64, y0: i64, x1: i64, y1: i64, color: Color32| {
        let rect = Rect::from_min_max(pos2(x0 as f32 / ppp, y0 as f32 / ppp), pos2(x1 as f32 / ppp, y1 as f32 / ppp));
        mesh.add_colored_rect(rect, color);
    };
    // The view in device pixels, a dash period wider: a band is cut to it so a long edge costs what its visible part
    // does, and the pattern is fixed to the screen so the cut does not move it.
    let view = clip.expand(2.0 * dash as f32 / ppp);
    let (vx0, vy0) = ((view.min.x * ppp).floor() as i64, (view.min.y * ppp).floor() as i64);
    let (vx1, vy1) = ((view.max.x * ppp).ceil() as i64, (view.max.y * ppp).ceil() as i64);
    // The first pixel of a band across its boundary: centred on it, the odd one after it.
    let half = w / 2;
    let back = points.first().zip(points.last()).filter(|_| closed && points.len() > 2).map(|(first, last)| (*last, *first));
    let lines = points.windows(2).map(|pair| (pair[0], pair[1])).chain(back);
    for (pa, pb) in lines {
        if !(pa.is_finite() && pb.is_finite() && clip.intersects(Rect::from_two_pos(pa, pb).expand(1.0))) {
            continue;
        }
        let (pa, pb) = if pa.distance(pb) > LONG_EDGE {
            let Some(inside) = clip_line(view, pa, pb) else { continue };
            inside
        } else {
            (pa, pb)
        };
        let (da, db) = (pa * ppp, pb * ppp);
        let (ax, ay, bx, by) = (da.x.round() as i64, da.y.round() as i64, db.x.round() as i64, db.y.round() as i64);
        if (ax, ay) == (bx, by) {
            continue;
        }
        if ay == by {
            // Rows `y0..y0 + w`, from one end's band to the other's, so edges meet at corners.
            let (x0, x1) = ((ax.min(bx) - half).max(vx0), (ax.max(bx) - half + w).min(vx1));
            let y0 = ay - half;
            if x1 <= x0 {
                continue;
            }
            quad(x0, y0, x1, y0 + w, light);
            for y in y0..y0 + w {
                for (s, e) in ants.runs(x0, x1, w * y) {
                    quad(s, y, e, y + 1, dark);
                }
            }
        } else if ax == bx {
            let (y0, y1) = ((ay.min(by) - half).max(vy0), (ay.max(by) - half + w).min(vy1));
            let x0 = ax - half;
            if y1 <= y0 {
                continue;
            }
            quad(x0, y0, x0 + w, y1, light);
            for x in x0..x0 + w {
                for (s, e) in ants.runs(y0, y1, w * x) {
                    quad(x, s, x + 1, e, dark);
                }
            }
        } else {
            let len = da.distance(db);
            let dir = (db - da) / len;
            let stroke = |color| Stroke::new(w as f32 / ppp, color);
            slanted.push(Shape::line_segment([da / ppp, db / ppp], stroke(light)));
            // Phased by screen position, like the straight edges, so dashes line up across joins.
            let period = (2 * dash) as f32;
            let mut t = -(da.x.floor() + da.y.floor() + ants.phase as f32).rem_euclid(period);
            while t < len {
                let (s0, s1) = (t.max(0.0), (t + dash as f32).min(len));
                if s1 > s0 {
                    slanted.push(Shape::line_segment([(da + dir * s0) / ppp, (da + dir * s1) / ppp], stroke(dark)));
                }
                t += period;
            }
        }
    }
    if !mesh.is_empty() {
        slanted.push(Shape::mesh(mesh));
    }
    slanted
}

/// The part of the segment `a`-`b` inside `r`, clipped in f64: a zoomed view can put endpoints far away, where f32
/// interpolation loses viewport-sized differences.
fn clip_line(r: Rect, a: Pos2, b: Pos2) -> Option<(Pos2, Pos2)> {
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, ax - f64::from(r.min.x)), (dx, f64::from(r.max.x) - ax), (-dy, ay - f64::from(r.min.y)), (dy, f64::from(r.max.y) - ay)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    let at = |t: f64| pos2((ax + dx * t) as f32, (ay + dy * t) as f32);
    (t0 <= t1).then(|| (at(t0), at(t1)))
}

/// The insertion mark of a drop from `from` to `to`.
pub fn drop_mark(tokens: &UiTokens, from: Pos2, to: Pos2, pixels_per_point: f32) -> Vec<Shape> {
    Rule::shapes(&rule_look(tokens, RuleKind::DropMark), from, to, pixels_per_point)
}

/// The points of the ellipse with centre `center` and conjugate half-axes `axes` (a rotated or skewed circle is
/// a pair of axes), counter-clockwise from the first axis, with a count that keeps the chord error under a
/// fraction of a point at any size. The first point is not repeated at the end.
pub fn ellipse(center: Pos2, axes: [Vec2; 2]) -> Vec<Pos2> {
    // Chord error of a regular n-gon on radius r is r (1 - cos(pi / n)); n = pi / acos(1 - e / r) keeps it under e.
    const MAX_CHORD_ERROR: f32 = 0.1;
    const MIN_POINTS: usize = 16;
    const MAX_POINTS: usize = 720;
    let radius = axes[0].length().max(axes[1].length());
    let count = if radius <= MAX_CHORD_ERROR {
        MIN_POINTS
    } else {
        ((std::f32::consts::PI / (1.0 - MAX_CHORD_ERROR / radius).acos()).ceil() as usize).clamp(MIN_POINTS, MAX_POINTS)
    };
    (0..count)
        .map(|i| {
            let angle = TAU * i as f32 / count as f32;
            center + axes[0] * angle.cos() + axes[1] * angle.sin()
        })
        .collect()
}

fn stroked(shape: &PaintShape, stroke: Stroke) -> Vec<Shape> {
    match shape {
        PaintShape::Rect(rect, radius) => vec![Shape::rect_stroke(*rect, *radius, stroke, egui::StrokeKind::Middle)],
        PaintShape::Circle(center, radius) => vec![Shape::circle_stroke(*center, *radius, stroke)],
        PaintShape::Polygon(points) => vec![Shape::closed_line(points.clone(), stroke)],
        PaintShape::Path(points) => vec![Shape::line(points.clone(), stroke)],
    }
}

impl ContentPainterCx<'_> {
    fn ppp(&self) -> f32 {
        self.painter.pixels_per_point()
    }

    /// Paints the part `uv` of `texture` into `rect`, untinted, with corners of `radius` points (zero for square ones).
    pub fn picture(&self, texture: egui::TextureId, rect: Rect, uv: Rect, radius: f32) {
        crate::runtime::egui::textures::paint(&self.painter, texture, rect, uv, super::textures::UNTINTED, UiTokens::corner(radius));
    }

    /// Paints the transparency checkerboard over `rect`.
    pub fn checker(&self, rect: Rect) {
        self.painter.extend(checker(self.tokens, rect, self.painter.clip_rect()));
    }

    /// Paints a colour ramp over `rect` along `axis`; see [`ramp`].
    pub fn ramp(&self, rect: Rect, stops: &[(f32, DataColor)], axis: Axis) {
        self.painter.extend(ramp(rect, stops, axis));
    }

    /// Paints the well of a plot over `rect` and returns the rect inside its edge, where the plot draws.
    pub fn plot_frame(&self, rect: Rect) -> Rect {
        let style = surface_style(self.tokens, SurfaceKind::Well);
        self.painter.extend(Surface::shapes(&style, None, rect, Corners::None, self.direction, self.ppp()));
        rect.shrink(self.tokens.stroke.thin)
    }

    /// Paints a surface of `kind` over `rect`; [`Self::ink_on`] is the ink text and glyphs read on it.
    pub fn surface(&self, rect: Rect, kind: SurfaceKind, corners: Corners) {
        let style = surface_style(self.tokens, kind);
        self.painter.extend(Surface::shapes(&style, None, rect, corners, self.direction, self.ppp()));
    }

    /// The ink text and glyphs read on a surface of `kind`.
    pub fn ink_on(&self, kind: SurfaceKind) -> Color32 {
        ink_color(self.tokens, kind.ink())
    }

    /// Paints the lines of `grid` in `ink`; see [`grid_lines`].
    pub fn grid_lines(&self, grid: &GridLines, ink: &GridInk) {
        self.painter.extend(grid_lines(self.tokens, grid, ink, self.ppp()));
    }

    /// Paints the checkerboard `checker` over `rect`.
    pub fn checker_with(&self, checker: &Checker, rect: Rect) {
        self.painter.extend(checker.shapes(self.tokens, rect, self.painter.clip_rect()));
    }

    /// Paints tick marks on one edge of `rect`.
    pub fn ticks(&self, rect: Rect, edge: TickEdge, marks: &[Tick]) {
        self.painter.extend(ticks(self.tokens, rect, edge, marks, self.painter.clip_rect(), self.ppp()));
    }

    /// Paints a halo-and-line outline of `shape`.
    pub fn outline(&self, shape: &PaintShape) {
        self.painter.extend(outline(self.tokens, shape));
    }

    /// Paints the halo-and-line outline of `shapes` with the line in `line`; see [`outline_in`].
    pub fn outline_in(&self, shapes: &[PaintShape], line: Color32) {
        self.painter.extend(outline_in(self.tokens, shapes, line));
    }

    /// Paints a dashed line along `points`; see [`dashed`].
    pub fn dashed(&self, points: &[Pos2], pattern: Dashes, width: StrokeWidth, color: Color32) {
        self.painter.extend(dashed(points, pattern, width.points(self.tokens, self.ppp()), color));
    }

    /// Paints marching ants along `points`, shifted `phase` points; see [`marching_ants`].
    pub fn marching_ants(&self, points: &[Pos2], closed: bool, phase: f32) {
        self.painter.extend(marching_ants(self.tokens, points, closed, phase, self.painter.clip_rect(), self.ppp()));
    }

    /// The device pixels per point of the surface being painted.
    pub fn pixels_per_point(&self) -> f32 {
        self.ppp()
    }

    /// Paints the insertion mark of a drop.
    pub fn drop_mark(&self, from: Pos2, to: Pos2) {
        self.painter.extend(drop_mark(self.tokens, from, to, self.ppp()));
    }
}

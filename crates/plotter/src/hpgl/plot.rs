//! The HP-GL/2 instructions played on a model of the plotter: the coordinate system (scaling
//! points, user units, rotation), the pen (position, up or down, number, width, color), the line
//! and fill attributes and the polygon buffer. What the pen draws is collected as shapes in
//! rotated plotter units, y up.

use std::collections::HashMap;

use kurbo::{Affine, BezPath, PathEl, Point, Shape as _, Vec2};

use super::lex::{ETX, Lexer, Pe, PeItem};

/// Plotter units per millimeter (0.025 mm each).
pub const PLU_PER_MM: f64 = 40.0;
/// The hard-clip limits taken for P1 and P2 when the file sets none (plotter units): 11 × 8.5 in.
pub const HARD_CLIP: Point = Point::new(11176.0, 8636.0);
/// The pen width after IN, in millimeters.
const DEFAULT_WIDTH_MM: f64 = 0.35;
/// The default line pattern length, in percent of the P1-P2 diagonal.
const DEFAULT_PATTERN: f64 = 4.0;
/// How far from the origin art may lie (plotter units, twice the range the format defines).
pub const LIMIT: f64 = 16_777_216.0;
/// The widest pen (plotter units): 409.6 mm, the least a device must take before it draws its own
/// widest pen instead.
const MAX_WIDTH: f64 = 16_384.0;
/// The most objects an import makes.
const MAX_SHAPES: usize = 1 << 20;
/// The most path elements an import makes.
const MAX_ELEMENTS: usize = if cfg!(test) { 200_000 } else { 4_000_000 };
/// The most pens a palette holds.
const MAX_PENS: i64 = 32_768;

pub(super) const LABELS: &str = "labels were left out";
pub(super) const SYMBOLS: &str = "symbols drawn at plotted points (symbol mode) were left out";
pub(super) const RASTER: &str = "raster images in the file were left out";
pub(super) const HATCHES: &str = "hatched and patterned fills are filled solid";
pub(super) const WINDOW: &str = "the input window doesn't clip the art";
pub(super) const PAGES: &str = "the art of later pages is drawn over the first page";
pub(super) const FAR_AWAY: &str = "objects far outside the drawing were left out";
pub(super) const TOO_MANY: &str = "the file has more than 1,048,576 objects: the rest are left out";
pub(super) const TOO_MANY_POINTS: &str = "the file has more than 4,000,000 path points: the rest are left out";

/// The instructions polygon mode plays (the others are ignored there).
const POLYGON_MODE: &[[u8; 2]] =
    &[*b"AA", *b"AR", *b"AT", *b"BP", *b"BR", *b"BZ", *b"CI", *b"CO", *b"DF", *b"IN", *b"PA", *b"PD", *b"PE", *b"PM", *b"PR", *b"PU", *b"RT"];

/// The colors of pens 0 to 7 until PC sets others.
const PALETTE: [[u8; 3]; 8] = [[255, 255, 255], [0, 0, 0], [255, 0, 0], [0, 255, 0], [255, 255, 0], [0, 0, 255], [255, 0, 255], [0, 255, 255]];

/// Line types 1 to 8: pen-down and pen-up lengths in percent of the pattern length.
const LINE_TYPES: [&[f64]; 8] = [
    &[0.0, 100.0],
    &[50.0, 50.0],
    &[70.0, 30.0],
    &[80.0, 10.0, 0.0, 10.0],
    &[70.0, 10.0, 10.0, 10.0],
    &[50.0, 10.0, 10.0, 10.0, 10.0, 10.0],
    &[70.0, 10.0, 0.0, 10.0, 0.0, 10.0],
    &[50.0, 10.0, 0.0, 10.0, 10.0, 10.0, 0.0, 10.0],
];

/// Fill rule for polygon mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    EvenOdd,
    NonZero,
}

/// Line cap style (`LA` kind 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Square,
    Triangular,
    Round,
}

/// Line join style (`LA` kind 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    MiterBevel,
    Triangular,
    Round,
    Bevel,
    None,
}

/// A stroke as the pen draws it (lengths in plotter units).
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub color: [u8; 3],
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter: f64,
    pub dash: Option<LinePattern>,
}

/// A line type as dashes (lengths in plotter units).
#[derive(Clone, Debug, PartialEq)]
pub struct LinePattern {
    /// Dash, gap, dash, gap…: an even number of lengths.
    pub pattern: Vec<f64>,
    /// How far into the pattern each line starts.
    pub offset: f64,
    /// The pattern stretches to fit each line, half its first dash at each end (adaptive line
    /// types).
    pub adaptive: bool,
}

impl LinePattern {
    /// The pattern of `percent` (pen-down and pen-up stretches, in turn, starting pen down) of
    /// `length`. A pen-down stretch of zero is a dot, a vector one plotter unit long that the gap
    /// after it gives up. An odd count repeats with its last pen-down stretch running into the
    /// first one, which the pattern then starts within.
    fn new(percent: &[f64], length: f64, adaptive: bool) -> Self {
        let mut pattern: Vec<f64> = percent.iter().map(|p| p / 100.0 * length).collect();
        for pair in pattern.chunks_mut(2) {
            if let [dash, rest @ ..] = pair
                && *dash == 0.0
            {
                *dash = 1.0;
                if let Some(gap) = rest.first_mut() {
                    *gap = (*gap - 1.0).max(0.0);
                }
            }
        }
        let mut offset = 0.0;
        if pattern.len() % 2 == 1 {
            match pattern.len() {
                // One pen-down stretch: a solid line.
                1 => pattern.push(0.0),
                _ => {
                    let last = pattern.pop().unwrap_or(0.0);
                    if let Some(first) = pattern.first_mut() {
                        *first += last;
                    }
                    offset = last;
                }
            }
        }
        Self { pattern, offset, adaptive }
    }
}

/// What the plot draws: an outline with a fill, a stroke or both.
#[derive(Clone, Debug)]
pub struct Shape {
    pub path: BezPath,
    pub fill: Option<([u8; 3], FillRule)>,
    pub stroke: Option<Stroke>,
    /// The pen SP selected when it was drawn (1 before any SP; 0 or less: no pen).
    pub pen: i64,
    /// The RO angle in force when it was drawn (0, 90, 180 or 270): the path is already turned
    /// by it, so a viewer that shows the art in the sender's own orientation turns it back.
    pub ro: u16,
}

/// A piece of a line the pen moves along, by its end.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Line(Point),
    Curve(Point, Point, Point),
}

impl Piece {
    fn end(self) -> Point {
        match self {
            Self::Line(p) | Self::Curve(_, _, p) => p,
        }
    }

    fn push(self, bp: &mut BezPath) {
        match self {
            Self::Line(p) => bp.line_to(p),
            Self::Curve(a, b, c) => bp.curve_to(a, b, c),
        }
    }
}

/// User units: an SC instruction in force.
#[derive(Clone, Copy, Debug)]
enum Scaling {
    /// The user ranges mapped onto P1 and P2; isotropic with the left and bottom percentages.
    Fit { x0: f64, x1: f64, y0: f64, y1: f64, iso: Option<(f64, f64)> },
    /// P1's user coordinates and the plotter units per user unit.
    Factor { x0: f64, fx: f64, y0: f64, fy: f64 },
}

/// How areas are filled.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FillKind {
    Solid,
    /// The pen's color at this percentage, on white.
    Shade(f64),
    /// Hatching and patterns, filled solid.
    Pattern,
}

/// The polygon buffer: its outline for filling (every edge) and for edging (pen-down edges only).
#[derive(Clone, Debug, Default)]
struct Poly {
    fill: BezPath,
    edge: BezPath,
    /// A subpolygon is open, from `start`; its last point.
    open: bool,
    start: Point,
    last: Point,
    /// The open subpolygon's points, and where its elements start in `fill` and `edge`.
    points: usize,
    marks: (usize, usize),
    /// Where the edge outline ends (`None`: it has to move before drawing).
    edge_at: Option<Point>,
    /// After PM1 or a circle: the next move starts a subpolygon.
    fresh: bool,
}

impl Poly {
    fn add(&mut self, a: Point, pieces: &[Piece], down: bool) {
        let Some(end) = pieces.last().map(|p| p.end()) else { return };
        // The move after PM1 goes to the next subpolygon's first point with the pen up, whatever
        // the pen's state (an arc's first chord aside): it is neither filled nor edged.
        if std::mem::take(&mut self.fresh) && matches!(pieces, [Piece::Line(_)]) {
            (self.start, self.last) = (end, end);
            return;
        }
        if !self.open {
            self.marks = (self.fill.elements().len(), self.edge.elements().len());
            self.fill.move_to(a);
            (self.start, self.open, self.points) = (a, true, 1);
        }
        pieces.iter().for_each(|p| p.push(&mut self.fill));
        self.points += pieces.len();
        if down {
            if self.edge_at != Some(a) {
                self.edge.move_to(a);
            }
            pieces.iter().for_each(|p| p.push(&mut self.edge));
            self.edge_at = Some(end);
        } else {
            self.edge_at = None;
        }
        self.last = end;
    }

    /// Close the open subpolygon, adding a point at its start when it ends elsewhere; the edge to
    /// that point is drawn when `down`. A subpolygon of fewer than three points (that one
    /// included) is neither filled nor edged. → The point added.
    fn close(&mut self, down: bool) -> Option<Point> {
        if !self.open {
            return None;
        }
        let closure = self.last != self.start;
        if self.points + usize::from(closure) < 3 {
            self.fill.truncate(self.marks.0);
            self.edge.truncate(self.marks.1);
        } else {
            if closure && down {
                if self.edge_at != Some(self.last) {
                    self.edge.move_to(self.last);
                }
                self.edge.line_to(self.start);
            }
            self.fill.close_path();
        }
        (self.open, self.edge_at) = (false, None);
        closure.then_some(self.start)
    }

    /// A closed outline of its own (a circle): the open subpolygon is closed with the pen up
    /// first, and the next move starts a subpolygon.
    fn closed(&mut self, path: &BezPath) {
        self.close(false);
        self.fill.extend(path.iter());
        self.edge.extend(path.iter());
        self.fresh = true;
    }
}

/// The line drawn so far with the pen down, not yet a shape.
struct Run {
    path: BezPath,
    stroke: Stroke,
    start: Point,
    end: Point,
}

pub(super) struct Plot {
    pub shapes: Vec<Shape>,
    pub warnings: Vec<String>,
    /// A budget ran out: nothing more is drawn.
    pub full: bool,
    elements: usize,
    // The coordinate system.
    p1: Point,
    p2: Point,
    scaling: Option<Scaling>,
    /// User units to plotter units (identity when scaling is off).
    user: Affine,
    /// The RO rotation (rotated plotter units to where the art lies) and its angle.
    rot: Affine,
    ro: u16,
    // The pen.
    relative: bool,
    down: bool,
    /// The pen's position, in plotter units before rotation.
    pos: Point,
    /// The pen SP selected (`None`: none yet, which draws with pen 1).
    pen: Option<i64>,
    pens: i64,
    colors: HashMap<i64, [u8; 3]>,
    ranges: [(f64, f64); 3],
    width: f64,
    widths: HashMap<i64, f64>,
    relative_widths: bool,
    // Line and fill attributes.
    line_type: Option<i32>,
    saved_line_type: Option<i32>,
    pattern: f64,
    pattern_mm: bool,
    user_types: [Option<Vec<f64>>; 8],
    cap: LineCap,
    join: LineJoin,
    miter: f64,
    fill: FillKind,
    // Polygons.
    poly: Option<Poly>,
    buffer: Option<Poly>,
    run: Option<Run>,
    terminator: u8,
    /// Instructions of kinds the reader doesn't know.
    pub unknown: usize,
    /// How many shapes there were at the first page advance.
    page_end: Option<usize>,
}

impl Plot {
    pub fn new() -> Self {
        let mut p = Self {
            shapes: vec![],
            warnings: vec![],
            full: false,
            elements: 0,
            p1: Point::ZERO,
            p2: HARD_CLIP,
            scaling: None,
            user: Affine::IDENTITY,
            rot: Affine::IDENTITY,
            ro: 0,
            relative: false,
            down: false,
            pos: Point::ZERO,
            pen: None,
            pens: 8,
            colors: HashMap::new(),
            ranges: [(0.0, 255.0); 3],
            width: DEFAULT_WIDTH_MM * PLU_PER_MM,
            widths: HashMap::new(),
            relative_widths: false,
            line_type: None,
            saved_line_type: None,
            pattern: DEFAULT_PATTERN,
            pattern_mm: false,
            user_types: Default::default(),
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 5.0,
            fill: FillKind::Solid,
            poly: None,
            buffer: None,
            run: None,
            terminator: ETX,
            unknown: 0,
            page_end: None,
        };
        p.init();
        p
    }

    pub fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    // ---------- defaults ----------

    /// DF: the line and fill attributes, scaling, plotting mode and polygon mode as they start.
    fn defaults(&mut self) {
        self.flush();
        self.relative = false;
        (self.line_type, self.saved_line_type, self.pattern, self.pattern_mm) = (None, None, DEFAULT_PATTERN, false);
        self.user_types = Default::default();
        (self.cap, self.join, self.miter) = (LineCap::Butt, LineJoin::Miter, 5.0);
        self.fill = FillKind::Solid;
        (self.poly, self.buffer) = (None, None);
        self.scaling = None;
        self.ranges = [(0.0, 255.0); 3];
        self.terminator = ETX;
        self.update_user();
    }

    /// IN: DF, and the pen up at the origin, default widths, palette and scaling points, no
    /// rotation. The selected pen stays.
    fn init(&mut self) {
        self.defaults();
        (self.pos, self.down) = (Point::ZERO, false);
        (self.width, self.relative_widths) = (DEFAULT_WIDTH_MM * PLU_PER_MM, false);
        self.widths.clear();
        (self.rot, self.ro) = (Affine::IDENTITY, 0);
        (self.p1, self.p2) = (Point::ZERO, HARD_CLIP);
        self.pens = 8;
        self.colors.clear();
        self.update_user();
    }

    // ---------- coordinates ----------

    /// User units to plotter units for the scaling in force; scaling that maps everything onto one
    /// point is ignored.
    fn update_user(&mut self) {
        let (p1, p2) = (self.p1, self.p2);
        let user = match self.scaling {
            None => Affine::IDENTITY,
            Some(Scaling::Factor { x0, fx, y0, fy }) => {
                Affine::translate(p1.to_vec2()) * Affine::scale_non_uniform(fx, fy) * Affine::translate((-x0, -y0))
            }
            Some(Scaling::Fit { x0, x1, y0, y1, iso }) => {
                let (w, h, ux, uy) = (p2.x - p1.x, p2.y - p1.y, x1 - x0, y1 - y0);
                let (mut sx, mut sy, mut off) = (w / ux, h / uy, Vec2::ZERO);
                if let Some((left, bottom)) = iso {
                    let s = sx.abs().min(sy.abs());
                    let (extra_x, extra_y) = (w.abs() - ux.abs() * s, h.abs() - uy.abs() * s);
                    off = Vec2::new(extra_x * left / 100.0 * w.signum(), extra_y * bottom / 100.0 * h.signum());
                    (sx, sy) = (s * sx.signum(), s * sy.signum());
                }
                Affine::translate(p1.to_vec2() + off) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-x0, -y0))
            }
        };
        let det = user.determinant();
        self.user = if det.is_finite() && det != 0.0 && user.as_coeffs().iter().all(|c| c.is_finite()) { user } else { Affine::IDENTITY };
    }

    /// A point in current units, in plotter units.
    fn to_plu(&self, x: f64, y: f64) -> Point {
        self.user * Point::new(x, y)
    }

    /// An increment in current units, in plotter units.
    fn delta(&self, x: f64, y: f64) -> Vec2 {
        self.to_plu(x, y) - self.to_plu(0.0, 0.0)
    }

    /// The pen's position in current units.
    fn pos_user(&self) -> Point {
        self.user.inverse() * self.pos
    }

    /// Plotter units (before rotation) to where the art lies.
    fn dev(&self, p: Point) -> Point {
        self.rot * p
    }

    /// User units to where the art lies.
    fn user_dev(&self) -> Affine {
        self.rot * self.user
    }

    /// The far corner of the hard-clip limits in the current orientation (their sides swap at 90
    /// and 270 degrees).
    fn hard_clip(&self) -> Point {
        if self.ro % 180 == 90 { Point::new(HARD_CLIP.y, HARD_CLIP.x) } else { HARD_CLIP }
    }

    /// RO: plotter units rotated `angle` degrees (0, 90, 180 or 270) to where the art lies, shifted
    /// so the hard-clip limits stay where they were.
    fn rotation(angle: u16) -> Affine {
        let (w, h) = (HARD_CLIP.x, HARD_CLIP.y);
        match angle {
            90 => Affine::new([0.0, 1.0, -1.0, 0.0, w, 0.0]),
            180 => Affine::new([-1.0, 0.0, 0.0, -1.0, w, h]),
            270 => Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, h]),
            _ => Affine::IDENTITY,
        }
    }

    fn diagonal(&self) -> f64 {
        (self.p2 - self.p1).hypot().max(1.0)
    }

    // ---------- pens and attributes ----------

    /// The palette entry the selected pen draws with (`None`: pen 0, no pen).
    fn pen_index(&self) -> Option<i64> {
        let n = self.pen.unwrap_or(1);
        if n <= 0 {
            return None;
        }
        Some((n - 1) % (self.pens - 1).max(1) + 1)
    }

    fn color(&self, i: i64) -> [u8; 3] {
        let default = usize::try_from(i).ok().and_then(|i| PALETTE.get(i)).copied().unwrap_or([0; 3]);
        self.colors.get(&i).copied().unwrap_or(default)
    }

    /// The stroke the selected pen draws (`None` for no pen).
    fn stroke(&self) -> Option<Stroke> {
        let i = self.pen_index()?;
        let width = self.widths.get(&i).copied().unwrap_or(self.width).clamp(0.0, MAX_WIDTH);
        let dash = self.line_type.filter(|k| *k != 0).map(|k| {
            let index = k.unsigned_abs().clamp(1, 8) as usize - 1;
            let length = if self.pattern_mm { self.pattern * PLU_PER_MM } else { self.pattern / 100.0 * self.diagonal() };
            match &self.user_types[index] {
                Some(pct) => LinePattern::new(pct, length, k < 0),
                None => LinePattern::new(LINE_TYPES[index], length, k < 0),
            }
        });
        Some(Stroke { color: self.color(i), width, cap: self.cap, join: self.join, miter: self.miter, dash })
    }

    /// The color the selected pen fills with (`None` for no pen).
    fn fill_color(&mut self) -> Option<[u8; 3]> {
        let c = self.color(self.pen_index()?);
        match self.fill {
            FillKind::Solid => Some(c),
            FillKind::Pattern => {
                self.warn(HATCHES);
                Some(c)
            }
            FillKind::Shade(level) => {
                let t = (level / 100.0).clamp(0.0, 1.0);
                Some(c.map(|v| (255.0 - (255.0 - f64::from(v)) * t).round() as u8))
            }
        }
    }

    // ---------- output ----------

    /// Count `n` path elements against the budget → whether they fit.
    fn spend(&mut self, n: usize) -> bool {
        self.elements += n;
        if self.elements > MAX_ELEMENTS {
            self.warn(TOO_MANY_POINTS);
            self.full = true;
        }
        !self.full
    }

    fn push(&mut self, mut shape: Shape) {
        (shape.pen, shape.ro) = (self.pen.unwrap_or(1), self.ro);
        if shape.path.elements().len() < 2 {
            return;
        }
        if self.shapes.len() >= MAX_SHAPES {
            self.warn(TOO_MANY);
            self.full = true;
            return;
        }
        let b = shape.path.bounding_box();
        if ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= LIMIT) {
            return self.warn(FAR_AWAY);
        }
        self.shapes.push(shape);
    }

    /// End the line being drawn.
    fn flush(&mut self) {
        let Some(run) = self.run.take() else { return };
        let mut els = run.path.into_elements();
        // Vectors of no length alone draw a dot: a vector one plotter unit long.
        if els.len() == 1 {
            els.push(PathEl::LineTo(run.start + Vec2::new(1.0, 0.0)));
        }
        // A line that ends where it started is closed, so its ends join.
        if els.len() > 3 && run.end == run.start {
            if matches!(els.last(), Some(PathEl::LineTo(p)) if *p == run.start) {
                els.pop();
            }
            els.push(PathEl::ClosePath);
        }
        self.push(Shape { path: BezPath::from_vec(els), fill: None, stroke: Some(run.stroke), pen: 0, ro: 0 });
    }

    /// A dot (a vector one plotter unit long) at `at`, drawn with the selected pen.
    fn dot(&mut self, at: Point) {
        // Its two path elements count against the budget like any other path's.
        if self.full || !self.spend(2) {
            return;
        }
        if let Some(stroke) = self.stroke() {
            let mut path = BezPath::new();
            path.move_to(at);
            path.line_to(at + Vec2::new(1.0, 0.0));
            self.push(Shape { path, fill: None, stroke: Some(stroke), pen: 0, ro: 0 });
        }
    }

    /// The pen moves from `a` along `pieces` (where the art lies), drawing when `down`; in polygon
    /// mode the move goes into the polygon buffer.
    fn trace(&mut self, a: Point, pieces: &[Piece], down: bool) {
        // Only what goes into a path counts against the budget: outside polygon mode, moves with the
        // pen up or with no pen (SP0) make nothing.
        let draws = down && self.pen_index().is_some();
        if pieces.is_empty() || self.full || (!draws && self.poly.is_none()) || !self.spend(pieces.len()) {
            return;
        }
        if let Some(poly) = &mut self.poly {
            return poly.add(a, pieces, down);
        }
        let Some(stroke) = self.stroke() else { return };
        if self.line_type == Some(0) {
            // Line type 0 draws a dot at each point given (an arc's or a curve's end).
            self.flush();
            if let Some(p) = pieces.last() {
                self.dot(p.end());
            }
            return;
        }
        if self.run.as_ref().is_some_and(|r| r.stroke != stroke || r.end != a) {
            self.flush();
        }
        let run = self.run.get_or_insert_with(|| {
            let mut path = BezPath::new();
            path.move_to(a);
            Run { path, stroke, start: a, end: a }
        });
        for p in pieces {
            // A vector of no length adds nothing to a line (a line of nothing else is a dot).
            if matches!(p, Piece::Line(b) if *b == run.end) {
                continue;
            }
            p.push(&mut run.path);
            run.end = p.end();
        }
    }

    /// Move the pen to `target` (plotter units), drawing when it is down.
    fn line_to(&mut self, target: Point) {
        if !(target.x.is_finite() && target.y.is_finite()) {
            return;
        }
        let (a, b) = (self.dev(self.pos), self.dev(target));
        let down = self.down;
        self.trace(a, &[Piece::Line(b)], down);
        self.pos = target;
    }

    /// The curves of an arc around `center` (user units) from `start` through `sweep` degrees,
    /// where the art lies, and its end (user units). `None` when it has no radius or sweep.
    fn arc(&self, center: Point, start: Point, sweep: f64) -> Option<(Vec<Piece>, Point)> {
        let r = (start - center).hypot();
        let sweep = sweep.clamp(-32768.0, 32767.0).to_radians();
        if !(r > 0.0 && r.is_finite() && sweep != 0.0 && sweep.is_finite()) {
            return None;
        }
        let a0 = (start - center).atan2();
        let arc = kurbo::Arc { center, radii: Vec2::new(r, r), start_angle: a0, sweep_angle: sweep, x_rotation: 0.0 };
        let m = self.user_dev();
        let pieces = arc
            .append_iter(r * 1e-6)
            .filter_map(|el| match el {
                PathEl::CurveTo(a, b, c) => Some(Piece::Curve(m * a, m * b, m * c)),
                PathEl::LineTo(p) => Some(Piece::Line(m * p)),
                _ => None,
            })
            .collect();
        Some((pieces, center + Vec2::from_angle(a0 + sweep) * r))
    }

    /// An arc from the pen around `center` (user units), drawn with the pen as it is.
    fn arc_from_pen(&mut self, center: Point, sweep: f64) {
        let start = self.pos_user();
        match self.arc(center, start, sweep) {
            Some((pieces, end)) => {
                let (a, down) = (self.dev(self.pos), self.down);
                self.trace(a, &pieces, down);
                self.pos = self.user * end;
            }
            None => {
                let at = self.dev(self.pos);
                let down = self.down;
                self.trace(at, &[Piece::Line(at)], down);
            }
        }
    }

    /// A closed outline through `pieces` from `a` (where the art lies).
    fn outline(a: Point, pieces: &[Piece]) -> BezPath {
        let mut path = BezPath::new();
        path.move_to(a);
        pieces.iter().for_each(|p| p.push(&mut path));
        path.close_path();
        path
    }

    /// A circle of `radius` (current units) around the pen, starting at 0 degrees (180 for a
    /// negative radius), where the art lies.
    fn circle(&self, radius: f64) -> Option<BezPath> {
        let c = self.pos_user();
        let start = c + Vec2::new(radius, 0.0);
        let (pieces, _) = self.arc(c, start, 360.0)?;
        Some(Self::outline(self.user_dev() * start, &pieces))
    }

    /// A wedge of `radius` (current units) around the pen, from `start` through `sweep` degrees.
    fn wedge(&self, radius: f64, start: f64, sweep: f64) -> Option<BezPath> {
        let c = self.pos_user();
        let reference = if radius < 0.0 { 180.0 } else { 0.0 };
        let sweep = sweep.clamp(-360.0, 360.0);
        let first = c + Vec2::from_angle((reference + start.clamp(-32768.0, 32767.0) % 360.0).to_radians()) * radius.abs();
        let m = self.user_dev();
        if sweep.abs() >= 360.0 {
            let (pieces, _) = self.arc(c, first, sweep)?;
            return Some(Self::outline(m * first, &pieces));
        }
        let mut pieces = vec![Piece::Line(m * first)];
        if let Some((arc, _)) = self.arc(c, first, sweep) {
            pieces.extend(arc);
        }
        (radius != 0.0).then(|| Self::outline(m * c, &pieces))
    }

    /// The rectangle from the pen to `corner` (plotter units), where the art lies.
    fn rect(&self, corner: Point) -> BezPath {
        let (a, b) = (self.pos_user(), self.user.inverse() * corner);
        let m = self.user_dev();
        let pieces = [Point::new(b.x, a.y), b, Point::new(a.x, b.y)].map(|p| Piece::Line(m * p));
        Self::outline(m * a, &pieces)
    }

    /// `path` as the polygon buffer, filled (`fill`) or edged.
    fn buffer_and_draw(&mut self, path: Option<BezPath>, fill: bool) {
        self.flush();
        if self.poly.is_some() {
            return;
        }
        let Some(path) = path else {
            return self.dot(self.dev(self.pos));
        };
        if !self.spend(path.elements().len()) {
            return;
        }
        let mut poly = Poly::default();
        poly.closed(&path);
        self.buffer = Some(poly);
        if fill { self.fill_polygon(FillRule::EvenOdd) } else { self.edge_polygon() }
    }

    /// FP: the polygon buffer filled (each fill is a copy of the buffer, counted against the
    /// budget).
    fn fill_polygon(&mut self, rule: FillRule) {
        self.flush();
        let n = self.buffer.as_ref().map_or(0, |b| b.fill.elements().len());
        if n == 0 || !self.spend(n) {
            return;
        }
        let Some(c) = self.fill_color() else { return };
        let Some(path) = self.buffer.as_ref().map(|b| b.fill.clone()) else { return };
        self.push(Shape { path, fill: Some((c, rule)), stroke: None, pen: 0, ro: 0 });
    }

    /// EP: the pen-down edges of the polygon buffer stroked (counted as FP's are).
    fn edge_polygon(&mut self) {
        self.flush();
        let n = self.buffer.as_ref().map_or(0, |b| b.edge.elements().len());
        if n == 0 || !self.spend(n) {
            return;
        }
        let Some(stroke) = self.stroke() else { return };
        let Some(path) = self.buffer.as_ref().map(|b| b.edge.clone()) else { return };
        self.push(Shape { path, fill: None, stroke: Some(stroke), pen: 0, ro: 0 });
    }

    // ---------- instructions ----------

    /// Read and play the instruction `op`.
    pub fn op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        if self.poly.is_some() && !POLYGON_MODE.contains(&op) {
            return self.skip_op(op, lex);
        }
        match &op {
            b"IN" | b"BP" | b"DF" | b"PU" | b"PD" | b"PA" | b"PR" | b"SP" => self.setup_op(op, lex),
            b"PW" | b"WU" | b"NP" | b"PC" | b"CR" => self.pen_op(op, lex),
            b"LT" | b"UL" => self.line_type_op(op, lex),
            b"LA" | b"FT" | b"SC" => self.attribute_op(op, lex),
            b"IP" | b"IR" | b"RO" | b"CI" => self.frame_op(op, lex),
            b"AA" | b"AR" | b"AT" | b"RT" | b"BZ" | b"BR" => self.curve_op(op, lex),
            b"EA" | b"ER" | b"RA" | b"RR" | b"EW" | b"WG" | b"PM" | b"FP" | b"EP" | b"PE" => self.area_op(op, lex),
            _ => self.other_op(op, lex),
        }
    }

    /// An instruction polygon mode ignores: its parameters (or text) are read.
    fn skip_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        match &op {
            b"LB" | b"BL" | b"WD" => lex.label(self.terminator),
            b"DT" => {
                lex.char();
                lex.params();
            }
            b"SM" => {
                lex.char();
            }
            _ => lex.params(),
        }
    }

    /// Initialization, pen moves and pen selection.
    fn setup_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"IN" | b"BP" => {
                lex.params();
                self.init();
            }
            b"DF" => {
                lex.params();
                self.defaults();
            }
            b"PU" | b"PD" => {
                self.down = op[1] == b'D';
                self.points(lex);
            }
            b"PA" | b"PR" => {
                self.relative = op[1] == b'R';
                self.points(lex);
            }
            b"SP" => {
                let n = lex.numbers(&mut v);
                self.flush();
                self.pen = Some(if n == 0 { 0 } else { v[0].round().clamp(-1.0, 1e9) as i64 });
            }
            _ => {}
        }
    }

    /// Pen widths, the palette and its colors.
    fn pen_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"PW" => {
                let n = lex.numbers(&mut v);
                self.flush();
                let width = |s: &Self, w: f64| if s.relative_widths { w / 100.0 * s.diagonal() } else { w * PLU_PER_MM };
                let default = if self.relative_widths { 0.1 } else { DEFAULT_WIDTH_MM };
                let w = width(self, if n == 0 { default } else { v[0].clamp(0.0, 32767.0) });
                if n >= 2 {
                    let pen = v[1].round().clamp(0.0, 1e9) as i64;
                    if pen > 0 && pen < self.pens.max(8) {
                        self.widths.insert(pen, w);
                    }
                } else {
                    self.width = w;
                    self.widths.clear();
                }
            }
            b"WU" => {
                let n = lex.numbers(&mut v);
                self.flush();
                self.relative_widths = n > 0 && v[0].round() == 1.0;
                self.width = if self.relative_widths { 0.1 / 100.0 * self.diagonal() } else { DEFAULT_WIDTH_MM * PLU_PER_MM };
                self.widths.clear();
            }
            b"NP" => {
                let n = lex.numbers(&mut v);
                self.flush();
                if n == 0 {
                    self.pens = 8;
                } else if v[0] >= 2.0 {
                    self.pens = i64::from((v[0].min(MAX_PENS as f64).ceil() as u32).next_power_of_two()).min(MAX_PENS);
                }
            }
            b"PC" => {
                let n = lex.numbers(&mut v);
                self.flush();
                let pen = v[0].round().clamp(0.0, 1e9) as i64;
                match n {
                    0 => self.colors.clear(),
                    1 => {
                        self.colors.remove(&pen);
                    }
                    // Two or three parameters: the instruction is ignored.
                    2 | 3 => {}
                    _ if pen < MAX_PENS => {
                        let channel = |i: usize| {
                            let (b, w) = self.ranges[i];
                            let t = if w == b { 0.0 } else { ((v[i + 1] - b) / (w - b)).clamp(0.0, 1.0) };
                            (t * 255.0).round() as u8
                        };
                        let c = [channel(0), channel(1), channel(2)];
                        self.colors.insert(pen, c);
                    }
                    _ => {}
                }
            }
            b"CR" => {
                let n = lex.numbers(&mut v);
                if n == 0 {
                    self.ranges = [(0.0, 255.0); 3];
                } else if n >= 6 {
                    self.ranges = [(v[0], v[1]), (v[2], v[3]), (v[4], v[5])];
                }
            }
            _ => {}
        }
    }

    /// Line types and their patterns.
    fn line_type_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"LT" => {
                let n = lex.numbers(&mut v);
                self.flush();
                if n == 0 {
                    if self.line_type.is_some() {
                        self.saved_line_type = self.line_type;
                    }
                    self.line_type = None;
                    return;
                }
                let k = v[0].round().clamp(-100.0, 100.0) as i32;
                if k == 99 {
                    if self.line_type.is_none() {
                        self.line_type = self.saved_line_type;
                    }
                    return;
                }
                if !(-8..=8).contains(&k) {
                    return;
                }
                // A pattern length of 0 or less, or a mode other than 0 and 1, makes the instruction ignored.
                if (n >= 2 && !(v[1] > 0.0 && v[1] <= 32767.0)) || (n >= 3 && !matches!(v[2].round() as i32, 0 | 1)) {
                    return;
                }
                self.line_type = Some(k);
                if n >= 2 {
                    self.pattern = v[1];
                }
                if n >= 3 {
                    self.pattern_mm = v[2].round() == 1.0;
                }
            }
            b"UL" => {
                let mut gaps = [0.0f64; 21];
                let n = lex.numbers(&mut gaps);
                self.flush();
                let index = gaps[0].round().abs();
                if n == 0 {
                    self.user_types = Default::default();
                } else if (1.0..=8.0).contains(&index) {
                    let slot = index as usize - 1;
                    let values = &gaps[1..n];
                    let sum: f64 = values.iter().sum();
                    self.user_types[slot] = if n == 1 {
                        None
                    } else if values.iter().all(|g| *g >= 0.0) && sum > 0.0 {
                        Some(values.iter().map(|g| g / sum * 100.0).collect())
                    } else {
                        return;
                    };
                }
            }
            _ => {}
        }
    }

    /// Line ends and joins, the fill type and scaling.
    fn attribute_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"LA" => {
                let n = lex.numbers(&mut v[..6]);
                self.flush();
                if n == 0 {
                    (self.cap, self.join, self.miter) = (LineCap::Butt, LineJoin::Miter, 5.0);
                }
                for pair in v[..n - n % 2].chunks(2) {
                    let value = pair[1].round() as i32;
                    match (pair[0].round() as i32, value) {
                        (1, 1) => self.cap = LineCap::Butt,
                        (1, 2) => self.cap = LineCap::Square,
                        (1, 3) => self.cap = LineCap::Triangular,
                        (1, 4) => self.cap = LineCap::Round,
                        (2, 1) => self.join = LineJoin::Miter,
                        (2, 2) => self.join = LineJoin::MiterBevel,
                        (2, 3) => self.join = LineJoin::Triangular,
                        (2, 4) => self.join = LineJoin::Round,
                        (2, 5) => self.join = LineJoin::Bevel,
                        (2, 6) => self.join = LineJoin::None,
                        (3, _) => self.miter = pair[1].clamp(1.0, 32767.0),
                        _ => {}
                    }
                }
            }
            b"FT" => {
                let n = lex.numbers(&mut v[..3]);
                let kind = if n == 0 { 1 } else { v[0].round() as i32 };
                self.fill = match kind {
                    1 | 2 => FillKind::Solid,
                    10 => FillKind::Shade(if n >= 2 { v[1] } else { 100.0 }),
                    3 | 4 | 11 | 21 | 22 => FillKind::Pattern,
                    _ => self.fill,
                };
            }
            b"SC" => {
                let n = lex.numbers(&mut v[..7]);
                self.flush();
                let kind = if n >= 5 { v[4].round() as i32 } else { 0 };
                self.scaling = match (n, kind) {
                    (0, _) => None,
                    (5, 2) if v[1] != 0.0 && v[3] != 0.0 => Some(Scaling::Factor { x0: v[0], fx: v[1], y0: v[2], fy: v[3] }),
                    (4..=7, 0 | 1) if v[0] != v[1] && v[2] != v[3] => {
                        let iso = (kind == 1).then(|| if n >= 7 { (v[5].clamp(0.0, 100.0), v[6].clamp(0.0, 100.0)) } else { (50.0, 50.0) });
                        Some(Scaling::Fit { x0: v[0], x1: v[1], y0: v[2], y1: v[3], iso })
                    }
                    _ => return,
                };
                self.update_user();
            }
            _ => {}
        }
    }

    /// The scaling points, the rotation and circles.
    fn frame_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"IP" | b"IR" => {
                let n = lex.numbers(&mut v[..4]);
                self.flush();
                // IR's percentages and the defaults are of the hard-clip limits in the current
                // orientation. Scaling points past the art's range are held at its edge, so lengths
                // taken from them stay finite.
                let clip = self.hard_clip();
                let at = |x: f64, y: f64| {
                    let p = if op[1] == b'R' { Point::new(x / 100.0 * clip.x, y / 100.0 * clip.y) } else { Point::new(x, y) };
                    Point::new(p.x.clamp(-LIMIT, LIMIT), p.y.clamp(-LIMIT, LIMIT))
                };
                match n {
                    0 => (self.p1, self.p2) = (Point::ZERO, clip),
                    2 | 3 => {
                        let p1 = at(v[0], v[1]);
                        (self.p1, self.p2) = (p1, p1 + (self.p2 - self.p1));
                    }
                    4 => {
                        let (p1, mut p2) = (at(v[0], v[1]), at(v[2], v[3]));
                        // A coordinate of P2 equal to P1's is one plotter unit more.
                        if p2.x == p1.x {
                            p2.x += 1.0;
                        }
                        if p2.y == p1.y {
                            p2.y += 1.0;
                        }
                        (self.p1, self.p2) = (p1, p2);
                    }
                    _ => {}
                }
                self.update_user();
            }
            b"RO" => {
                let n = lex.numbers(&mut v[..1]);
                self.flush();
                let angle = if n == 0 { 0.0 } else { v[0].round().rem_euclid(360.0) };
                if [0.0, 90.0, 180.0, 270.0].contains(&angle) {
                    let ro = angle as u16;
                    let rot = Self::rotation(ro);
                    // The pen keeps its place on the page, its coordinates changing with the axes;
                    // P1 and P2 keep their coordinates, and the polygon buffer turns with the axes.
                    let turn = rot * self.rot.inverse();
                    self.pos = rot.inverse() * (self.rot * self.pos);
                    if let Some(b) = &mut self.buffer {
                        b.fill.apply_affine(turn);
                        b.edge.apply_affine(turn);
                    }
                    (self.rot, self.ro) = (rot, ro);
                }
            }
            b"CI" => {
                let n = lex.numbers(&mut v[..2]);
                if n == 0 {
                    return;
                }
                let path = self.circle(v[0]);
                // In polygon mode the circle is a subpolygon of its own; the pen stays at its
                // center.
                if self.poly.is_some() {
                    if let Some(path) = path
                        && self.spend(path.elements().len())
                        && let Some(poly) = &mut self.poly
                    {
                        poly.closed(&path);
                    }
                    return;
                }
                self.flush();
                // Line type 0 draws a dot at the center.
                if self.line_type == Some(0) {
                    return self.dot(self.dev(self.pos));
                }
                let Some(path) = path else { return self.dot(self.dev(self.pos)) };
                if self.spend(path.elements().len())
                    && let Some(stroke) = self.stroke()
                {
                    self.push(Shape { path, fill: None, stroke: Some(stroke), pen: 0, ro: 0 });
                }
            }
            _ => {}
        }
    }

    /// Arcs and Bézier curves from the pen.
    fn curve_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"AA" | b"AR" => {
                if lex.numbers(&mut v[..4]) < 3 {
                    return;
                }
                let center = if op[1] == b'R' { self.pos_user() + Vec2::new(v[0], v[1]) } else { Point::new(v[0], v[1]) };
                self.arc_from_pen(center, v[2]);
            }
            b"AT" | b"RT" => {
                if lex.numbers(&mut v[..5]) < 4 {
                    return;
                }
                let a = self.pos_user();
                let off = if op[0] == b'R' { a.to_vec2() } else { Vec2::ZERO };
                let (b, c) = (Point::new(v[0], v[1]) + off, Point::new(v[2], v[3]) + off);
                self.three_point_arc(a, b, c);
            }
            b"BZ" | b"BR" => {
                while !self.full
                    && let Some(x) = lex.number()
                {
                    let mut p = [x, 0.0, 0.0, 0.0, 0.0, 0.0];
                    for slot in &mut p[1..] {
                        let Some(n) = lex.number() else { return };
                        *slot = n;
                    }
                    let a = self.pos_user();
                    let off = if op[1] == b'R' { a.to_vec2() } else { Vec2::ZERO };
                    let [c1, c2, end] = [0, 2, 4].map(|i| Point::new(p[i], p[i + 1]) + off);
                    let m = self.user_dev();
                    let (from, down) = (self.dev(self.pos), self.down);
                    self.trace(from, &[Piece::Curve(m * c1, m * c2, m * end)], down);
                    self.pos = self.user * end;
                }
            }
            _ => {}
        }
    }

    /// Rectangles, wedges, polygon mode and encoded polylines.
    fn area_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 8];
        match &op {
            b"EA" | b"ER" | b"RA" | b"RR" => {
                if lex.numbers(&mut v[..2]) < 2 {
                    return;
                }
                let corner = if op[1] == b'R' { self.pos + self.delta(v[0], v[1]) } else { self.to_plu(v[0], v[1]) };
                let path = self.rect(corner);
                self.buffer_and_draw(Some(path), op[0] == b'R');
            }
            b"EW" | b"WG" => {
                if lex.numbers(&mut v[..4]) < 3 {
                    return;
                }
                let path = self.wedge(v[0], v[1], v[2]);
                self.buffer_and_draw(path, op[0] == b'W');
            }
            b"PM" => {
                let n = lex.numbers(&mut v[..1]);
                self.flush();
                // The point that closes a subpolygon moves the pen there: PM1 closes it with the
                // pen up, PM2 with the pen as it is.
                let mut closure = None;
                match if n == 0 { 0 } else { v[0].round() as i32 } {
                    0 if self.poly.is_none() => {
                        let at = self.dev(self.pos);
                        self.poly = Some(Poly { start: at, last: at, ..Poly::default() });
                        self.buffer = None;
                    }
                    1 => {
                        if let Some(p) = &mut self.poly {
                            closure = p.close(false);
                            p.fresh = true;
                        }
                    }
                    2 => {
                        if let Some(mut p) = self.poly.take() {
                            closure = p.close(self.down);
                            self.buffer = Some(p);
                        }
                    }
                    _ => {}
                }
                if let Some(at) = closure {
                    self.pos = self.rot.inverse() * at;
                }
            }
            b"FP" => {
                let n = lex.numbers(&mut v[..1]);
                self.fill_polygon(if n > 0 && v[0].round() == 1.0 { FillRule::NonZero } else { FillRule::EvenOdd });
            }
            b"EP" => {
                lex.params();
                self.edge_polygon();
            }
            b"PE" => self.polyline_encoded(lex),
            _ => {}
        }
    }

    /// Labels, comments, pages, the input window, and instructions that draw nothing.
    fn other_op(&mut self, op: [u8; 2], lex: &mut Lexer) {
        let mut v = [0.0f64; 4];
        match &op {
            b"LB" | b"BL" | b"WD" => {
                lex.label(self.terminator);
                self.warn(LABELS);
            }
            b"DT" => {
                self.terminator = lex.char().unwrap_or(ETX);
                lex.params();
            }
            b"SM" => {
                // The symbol character is read so it isn't taken for an instruction; symbols
                // aren't drawn.
                if lex.char().is_some() {
                    self.warn(SYMBOLS);
                }
            }
            b"CO" => {
                lex.quoted();
            }
            b"PG" | b"FR" => {
                lex.params();
                self.flush();
                self.page_end.get_or_insert(self.shapes.len());
                // The next page starts with the pen up at the origin (FR also empties the
                // polygon buffer).
                (self.pos, self.down) = (Point::ZERO, false);
                if op[0] == b'F' {
                    self.buffer = None;
                }
            }
            b"IW" => {
                if lex.numbers(&mut v) == 4 {
                    self.warn(WINDOW);
                }
            }
            _ if super::IGNORED.contains(&op) => lex.params(),
            _ => {
                lex.params();
                self.unknown += 1;
            }
        }
    }

    /// The coordinate pairs of PU, PD, PA and PR, plotted in the current mode.
    fn points(&mut self, lex: &mut Lexer) {
        while let Some(x) = lex.number() {
            let Some(y) = lex.number() else { return };
            let target = if self.relative { self.pos + self.delta(x, y) } else { self.to_plu(x, y) };
            self.line_to(target);
            if self.full {
                return;
            }
        }
    }

    /// AT and RT: the arc from `a` through `b` to `c` (user units).
    fn three_point_arc(&mut self, a: Point, b: Point, c: Point) {
        let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
        let center = if a == c {
            // Back to the start: the circle whose diameter ends at the start and the middle point.
            Some(a.midpoint(b))
        } else if d.abs() > 1e-12 * (b - a).hypot2().max((c - a).hypot2()).max(1.0) {
            let (a2, b2, c2) = (a.to_vec2().hypot2(), b.to_vec2().hypot2(), c.to_vec2().hypot2());
            Some(Point::new(
                (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
                (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
            ))
        } else {
            None
        };
        let Some(center) = center.filter(|p| p.x.is_finite() && p.y.is_finite()) else {
            // Points in a line: a line to the end.
            return self.line_to(self.user * c);
        };
        let tau = std::f64::consts::TAU;
        let angle = |p: Point| (p - center).atan2();
        let sweep = if a == c {
            tau
        } else {
            let (s, t) = ((angle(c) - angle(a)).rem_euclid(tau), (angle(b) - angle(a)).rem_euclid(tau));
            if t < s { s } else { s - tau }
        };
        self.arc_from_pen(center, sweep.to_degrees());
        if a != c {
            self.pos = self.user * c;
        }
    }

    /// PE: encoded pen moves, pen selections and rectangles.
    fn polyline_encoded(&mut self, lex: &mut Lexer) {
        let mut st = Pe::default();
        let (mut rect, mut corner) = (false, false);
        while let Some(item) = lex.pe(&mut st) {
            // Reading stops once the budget is spent, so the rest of PE needn't be decoded.
            if self.full {
                break;
            }
            match item {
                PeItem::Pen(n) => {
                    if self.poly.is_none() {
                        self.flush();
                        self.pen = Some(n.clamp(-1, 1_000_000_000));
                    }
                }
                PeItem::Rect => (rect, corner) = (true, false),
                PeItem::Point { x, y, up, absolute } => {
                    if up {
                        rect = false;
                    }
                    let target = if absolute { self.to_plu(x, y) } else { self.pos + self.delta(x, y) };
                    if rect && corner {
                        // The second pair of a rectangle: its opposite corner, relative. It is filled
                        // as RR fills one, so polygon mode, which doesn't allow RR, ignores it.
                        let path = self.rect(self.pos + self.delta(x, y));
                        self.buffer_and_draw(Some(path), true);
                        corner = false;
                    } else if rect {
                        self.down = false;
                        self.line_to(target);
                        corner = true;
                    } else {
                        self.down = !up;
                        self.line_to(target);
                    }
                }
            }
        }
        if rect {
            self.down = false;
        }
    }

    /// The plot's end: the last line drawn, and notes on what wasn't read.
    pub fn finish(mut self, raster: bool) -> Self {
        self.flush();
        if let Some(n) = self.page_end
            && self.shapes.len() > n
        {
            self.warn(PAGES);
        }
        if raster {
            self.warn(RASTER);
        }
        if self.unknown > 0 {
            let n = self.unknown;
            self.warnings.push(format!(
                "{n} instruction{} of kinds the HP-GL reader doesn't support {} skipped",
                if n == 1 { "" } else { "s" },
                if n == 1 { "was" } else { "were" }
            ));
        }
        self
    }
}

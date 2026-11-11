//! [`SnapTargets`]: snapping (View › Snap / Snap To) and smart-guide geometry.
//!
//! The app builds a target set once per gesture from its document (guides, grid, document bounds
//! and centre, layer edges and centres, selection edges); every pointer move asks it to snap a
//! point (marquee corners, crop handles, guide positions, pen anchors, transform handles) or a
//! moving rectangle (the Move tool, Free Transform drags). Distances are document units; the
//! caller converts [`SNAP_PX`] screen points with the zoom. Results carry [`SnapLine`]s the
//! caller draws.
//!
//! Snapping is on when View › Snap is; holding Ctrl while dragging turns it off, except when Ctrl
//! is the tool's own modifier (the caller reads the keys). Smart guides follow View › Show ›
//! Smart Guides and also snap the Move tool to other layers' edges and centres.

use crate::flow::Axis;
use kurbo::{Point, Rect, Vec2};

/// Screen points within which a position snaps (Photoshop).
pub const SNAP_PX: f64 = 8.0;

/// What a target is, so an app can filter (Snap To › Guides only) and the lines can be drawn in
/// the smart-guide colour or the plain one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SnapKind {
    /// Guides.
    Guide,
    /// The grid.
    Grid,
    /// The document's edges.
    Bounds,
    /// The document's centre lines.
    Center,
    /// Another layer's (or page object's) edge: a smart guide.
    LayerEdge,
    /// Another layer's centre: a smart guide.
    LayerCenter,
    /// The selection's edges.
    Selection,
}

impl SnapGesture {
    /// A moving rectangle that was `at_press` when the gesture began.
    pub const fn rect(at_press: Rect) -> Self {
        SnapGesture::Rect { at_press }
    }

    /// Only the coordinate across the lines that run `along` snaps: x for `Axis::Y`.
    pub const fn axis(along: Axis) -> Self {
        SnapGesture::Axis { along }
    }
}

impl SnapTarget {
    /// A full-height line of `kind` running `along` an axis, at `position` across it: a line along `Axis::Y`
    /// snaps x, a line along `Axis::X` snaps y.
    pub const fn new(along: Axis, position: f64, kind: SnapKind) -> Self {
        SnapTarget { along, position, kind, span: None }
    }
}

/// One snap position on one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SnapTarget {
    /// The axis the line runs along: `Axis::Y` for a vertical line (snaps x), `Axis::X` for a horizontal one
    /// (snaps y).
    pub along: Axis,
    /// The position along the snapped axis, document units.
    pub position: f64,
    /// What the target is.
    pub kind: SnapKind,
    /// The extent of the target along its line, for drawing a smart guide between the two
    /// objects; `None` for a full-height line (guides, grid).
    pub span: Option<(f64, f64)>,
}

/// The line the UI draws for a snap that happened.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SnapLine {
    /// The axis the line runs along: `Axis::Y` for a vertical line (snaps x), `Axis::X` for a horizontal one
    /// (snaps y).
    pub along: Axis,
    /// The position along the snapped axis, in document units.
    pub position: f64,
    /// What the target was.
    pub kind: SnapKind,
    /// From the snapped object to the target, along the line.
    pub span: Option<(f64, f64)>,
}

/// View › Snap To and View › Show › Smart Guides, as the app stores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SnapOptions {
    /// View › Snap (the master switch).
    pub on: bool,
    /// Snap To › Guides.
    pub guides: bool,
    /// Snap To › Grid.
    pub grid: bool,
    /// Snap To › Layers.
    pub layers: bool,
    /// Snap To › Document Bounds.
    pub document_bounds: bool,
    /// Snap To › Selection.
    pub selection: bool,
    /// View › Show › Smart Guides.
    pub smart_guides: bool,
}

impl Default for SnapOptions {
    /// Photoshop's defaults: everything on.
    fn default() -> Self {
        Self { on: true, guides: true, grid: true, layers: true, document_bounds: true, selection: true, smart_guides: true }
    }
}

/// How the host snaps a tool's pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum SnapGesture {
    /// The pointer itself snaps (marquee corner, pen anchor, crop handle).
    Point,
    /// A rectangle moves with the pointer and its edges and centre snap: `at_press` is the
    /// rectangle when the gesture began; the host snaps `at_press + (pointer - press)`.
    #[non_exhaustive]
    Rect {
        /// The rectangle when the gesture began.
        at_press: Rect,
    },
    /// Only one axis snaps (a guide being dragged).
    #[non_exhaustive]
    Axis {
        /// The axis the dragged guide runs along; the coordinate across it snaps.
        along: Axis,
    },
}

/// The snap targets of one gesture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapTargets {
    targets: Vec<SnapTarget>,
}

/// Distances closer than this are equal.
const TIE: f64 = 1e-9;

/// The line through `target` reaching the snapped object at `a..b` along the line; a target
/// without a span stays full height.
fn line(target: &SnapTarget, a: f64, b: f64) -> SnapLine {
    let span = target.span.map(|(lo, hi)| (lo.min(a).min(b), hi.max(a).max(b)));
    SnapLine { along: target.along, position: target.position, kind: target.kind, span }
}

impl SnapTargets {
    /// No targets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one target.
    pub fn add(&mut self, target: SnapTarget) {
        self.targets.push(target);
    }

    /// Add a rectangle's four edges as `edge` targets and its two centre lines as `center`
    /// targets, each spanning the rectangle.
    fn add_rect(&mut self, rect: Rect, edge: SnapKind, center: SnapKind) {
        let r = rect.abs();
        let (xs, ys) = ((r.y0, r.y1), (r.x0, r.x1));
        for (along, position, kind, span) in [
            (Axis::Y, r.x0, edge, xs),
            (Axis::Y, r.x1, edge, xs),
            (Axis::Y, (r.x0 + r.x1) / 2.0, center, xs),
            (Axis::X, r.y0, edge, ys),
            (Axis::X, r.y1, edge, ys),
            (Axis::X, (r.y0 + r.y1) / 2.0, center, ys),
        ] {
            self.add(SnapTarget { along, position, kind, span: Some(span) });
        }
    }

    /// Guides as full-height targets, grid lines of `spacing` from `origin` over `bounds`, the
    /// document bounds and centre: what every app has. Layers and the selection are added by
    /// the app with [`add_rect`](Self::add_rect).
    pub fn with_document(
        self,
        options: &SnapOptions,
        bounds: Rect,
        guides: impl IntoIterator<Item = (Axis, f64)>,
        grid: Option<(Point, Vec2)>,
    ) -> Self {
        if !options.on {
            return self;
        }
        let mut this = self;
        if options.guides {
            for (along, position) in guides {
                this.add(SnapTarget { along, position, kind: SnapKind::Guide, span: None });
            }
        }
        if options.grid
            && let Some((origin, step)) = grid
        {
            let lines = |origin: f64, step: f64, lo: f64, hi: f64| {
                let first = ((lo - origin) / step).ceil() as i64;
                let last = ((hi - origin) / step).floor() as i64;
                (first..=last).map(move |k| origin + k as f64 * step)
            };
            let r = bounds.abs();
            if step.x.is_finite() && step.x > 0.0 {
                for position in lines(origin.x, step.x, r.x0, r.x1) {
                    this.add(SnapTarget { along: Axis::Y, position, kind: SnapKind::Grid, span: None });
                }
            }
            if step.y.is_finite() && step.y > 0.0 {
                for position in lines(origin.y, step.y, r.y0, r.y1) {
                    this.add(SnapTarget { along: Axis::X, position, kind: SnapKind::Grid, span: None });
                }
            }
        }
        if options.document_bounds {
            this.add_rect(bounds, SnapKind::Bounds, SnapKind::Center);
        }
        this
    }

    /// Only the targets `keep` accepts (Snap To with one kind off).
    pub fn filtered(&self, keep: impl Fn(SnapKind) -> bool) -> SnapTargets {
        SnapTargets { targets: self.targets.iter().copied().filter(|t| keep(t.kind)).collect() }
    }

    /// Whether there is no target.
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// The nearest target among the lines that run `along` an axis, within `tolerance` of `value` across them.
    pub fn nearest(&self, along: Axis, value: f64, tolerance: f64) -> Option<SnapTarget> {
        let mut best: Option<(f64, SnapTarget)> = None;
        for target in self.targets.iter().filter(|t| t.along == along) {
            let distance = (target.position - value).abs();
            if distance > tolerance {
                continue;
            }
            // A guide wins a tie, as in Photoshop.
            let better = best.is_none_or(|(d, t)| {
                distance < d - TIE || ((distance - d).abs() <= TIE && target.kind == SnapKind::Guide && t.kind != SnapKind::Guide)
            });
            if better {
                best = Some((distance, *target));
            }
        }
        best.map(|(_, target)| target)
    }

    /// `point` snapped on each axis independently, with the lines to draw.
    pub fn snap_point(&self, point: Point, tolerance: f64) -> (Point, Vec<SnapLine>) {
        let mut out = point;
        let mut lines = Vec::new();
        if let Some(t) = self.nearest(Axis::Y, point.x, tolerance) {
            out.x = t.position;
            lines.push(line(&t, point.y, point.y));
        }
        if let Some(t) = self.nearest(Axis::X, point.y, tolerance) {
            out.y = t.position;
            lines.push(line(&t, out.x, out.x));
        }
        (out, lines)
    }

    /// The offset that moves `rect` so its nearest edge or centre meets a target on each axis,
    /// with the lines to draw. Edges win over centres at equal distance.
    pub fn snap_rect(&self, rect: Rect, tolerance: f64) -> (Vec2, Vec<SnapLine>) {
        let mut delta = Vec2::ZERO;
        let mut lines = Vec::new();
        let rect = rect.abs();
        for along in [Axis::Y, Axis::X] {
            let (lo, hi, other) = match along {
                Axis::Y => (rect.x0, rect.x1, (rect.y0, rect.y1)),
                Axis::X => (rect.y0, rect.y1, (rect.x0, rect.x1)),
            };
            // Edges before the centre, so an edge wins a tie.
            let anchors = [lo, hi, (lo + hi) / 2.0];
            let mut best: Option<f64> = None;
            for anchor in anchors {
                if let Some(t) = self.nearest(along, anchor, tolerance) {
                    let d = t.position - anchor;
                    if best.is_none_or(|b| d.abs() < b.abs() - TIE) {
                        best = Some(d);
                    }
                }
            }
            let Some(d) = best else { continue };
            match along {
                Axis::Y => delta.x = d,
                Axis::X => delta.y = d,
            }
            for anchor in anchors {
                let moved = anchor + d;
                let hit = self
                    .targets
                    .iter()
                    .filter(|t| t.along == along && (t.position - moved).abs() < 1e-6)
                    .min_by_key(|t| u8::from(t.kind != SnapKind::Guide));
                if let Some(t) = hit {
                    lines.push(line(t, other.0, other.1));
                }
            }
        }
        (delta, lines)
    }
}

/// The axis across `along`: a line running along `Axis::Y` (vertical) is placed on `Axis::X`.
pub const fn across_axis(along: Axis) -> Axis {
    match along {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

/// The document direction a line running `along` an axis moves in: x for a line along `Axis::Y`. A screen caller
/// maps it through `CanvasView::map_vec`; `resize_cursor` takes it as is.
pub fn motion_across(along: Axis) -> Vec2 {
    let x = match along {
        Axis::X => 0.0,
        Axis::Y => 1.0,
    };
    Vec2::new(x, 1.0 - x)
}

/// The coordinate of `p` a line running `along` an axis is placed by: x for a line along `Axis::Y`.
pub const fn coordinate_across(along: Axis, p: Point) -> f64 {
    match along {
        Axis::X => p.y,
        Axis::Y => p.x,
    }
}

/// The bounding rectangle of `rects`, or `None` for none.
// pc: crates/engine/src/snap.rs:278 `union`.
pub fn union(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().reduce(|a, b| a.union(b))
}

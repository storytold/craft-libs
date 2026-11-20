//! Annotations on a page: Acrobat's kinds and tools, their geometry and handles, and the edits a
//! drawing or editing gesture reports (N5.03). Coordinates are page units (points, y down), as
//! the canvas tool host gives them.

use craft_behavior::geometry::turn_to_45;
use kurbo::{Point, Rect, Vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
/// Identifies an annotation within a document.
pub struct AnnotationId(pub u64);

/// What an annotation is (PDF annotation subtypes as Acrobat names them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AnnotationKind {
    /// A sticky note.
    #[default]
    Note,
    #[allow(missing_docs)]
    TextBox,
    /// A text box with a leader line.
    Callout,
    /// A file attachment.
    Attach,
    #[allow(missing_docs)]
    Highlight,
    /// A highlight drawn freehand.
    FreehandHighlight,
    #[allow(missing_docs)]
    Underline,
    #[allow(missing_docs)]
    StrikeOut,
    #[allow(missing_docs)]
    Squiggly,
    /// Insert Text at a caret.
    Caret,
    /// Strike-through with replacement text.
    ReplaceText,
    /// Freehand drawing.
    Ink,
    #[allow(missing_docs)]
    Line,
    #[allow(missing_docs)]
    Arrow,
    #[allow(missing_docs)]
    Rectangle,
    #[allow(missing_docs)]
    Oval,
    #[allow(missing_docs)]
    PolyLine,
    #[allow(missing_docs)]
    Polygon,
    #[allow(missing_docs)]
    Cloud,
}

/// A tool of the annotation toolbar: a kind to draw, the Eraser (removes Ink strokes it
/// crosses), or Select (edit existing annotations).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnnotationTool {
    #[default]
    /// Select and edit existing annotations.
    Select,
    /// Draw a new annotation of this kind.
    Draw(AnnotationKind),
    /// Remove the Ink strokes the pointer crosses.
    Eraser,
}

/// The toolbar's flyout groups, in Acrobat's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnotationGroup {
    /// Notes, text boxes, callouts and attachments.
    Comment,
    /// Text markup tools.
    Highlight,
    /// Drawing and shape tools, with the Eraser.
    Draw,
}

impl AnnotationGroup {
    /// Every group, in toolbar order.
    pub const ALL: [AnnotationGroup; 3] = [AnnotationGroup::Comment, AnnotationGroup::Highlight, AnnotationGroup::Draw];

    /// The group's tools in Acrobat's order (PdfCraft `tools/comments.rs:81` `GROUPS`).
    pub fn tools(self) -> &'static [AnnotationTool] {
        use AnnotationKind as K;
        use AnnotationTool::Draw as D;
        match self {
            AnnotationGroup::Comment => &[D(K::Note), D(K::TextBox), D(K::Callout), D(K::Attach)],
            AnnotationGroup::Highlight => {
                &[D(K::Highlight), D(K::Underline), D(K::StrikeOut), D(K::Squiggly), D(K::FreehandHighlight), D(K::Caret), D(K::ReplaceText)]
            }
            AnnotationGroup::Draw => {
                &[D(K::Ink), D(K::Line), D(K::Arrow), D(K::Rectangle), D(K::Oval), D(K::PolyLine), D(K::Polygon), D(K::Cloud), AnnotationTool::Eraser]
            }
        }
    }
}

impl AnnotationKind {
    /// Text markup: drawn over page text by selecting it (Highlight … Replace Text).
    pub fn is_markup(self) -> bool {
        matches!(
            self,
            AnnotationKind::Highlight
                | AnnotationKind::Underline
                | AnnotationKind::StrikeOut
                | AnnotationKind::Squiggly
                | AnnotationKind::ReplaceText
        )
    }

    /// Kinds drawn by points or strokes (Line, Arrow, PolyLine, Polygon, Cloud, Ink, Freehand
    /// highlight): reshaped by moving a point; the others resize by their box.
    pub fn is_pointwise(self) -> bool {
        matches!(
            self,
            AnnotationKind::Line
                | AnnotationKind::Arrow
                | AnnotationKind::PolyLine
                | AnnotationKind::Polygon
                | AnnotationKind::Cloud
                | AnnotationKind::Ink
                | AnnotationKind::FreehandHighlight
        )
    }

    /// Kinds that keep their size: icons placed at a point.
    pub fn is_fixed_size(self) -> bool {
        matches!(self, AnnotationKind::Note | AnnotationKind::Attach | AnnotationKind::Caret)
    }
}

/// An annotation as the overlay draws and edits it; the app maps its document's annotations to
/// these and back.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    #[allow(missing_docs)]
    pub id: AnnotationId,
    #[allow(missing_docs)]
    pub page: usize,
    #[allow(missing_docs)]
    pub kind: AnnotationKind,
    /// The bounding box on the page.
    pub rect: Rect,
    /// The defining points of pointwise kinds, and the quads of markups (four points each).
    pub points: Vec<Point>,
    /// Ink strokes.
    pub strokes: Vec<Vec<Point>>,
    /// Straight RGB.
    pub color: [u8; 3],
    /// Opacity from 0.0 (clear) to 1.0 (solid).
    pub opacity: f32,
    /// Line width in page units.
    pub stroke_width: f64,
    /// The note or text-box text.
    pub text: String,
    /// A locked annotation shows no handles.
    pub locked: bool,
    /// The width over height a corner resize keeps; `None` for a flat box.
    #[serde(default)]
    pub aspect: Option<f64>,
    /// Whether the host lets the annotation be reshaped.
    #[serde(default)]
    pub handle_rule: HandleRule,
}

/// A named option for an annotation the host cannot reshape: under `FixedPoints` and `NoResize` the overlay shows no
/// handle that would do nothing; the body still moves and selects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum HandleRule {
    /// The kind's own rule decides.
    #[default]
    Free,
    /// The host keeps the points (dimension lines and polygons): no point handles and no box handles.
    FixedPoints,
    /// The host cannot resize the box (a callout with no readable box): no handles.
    NoResize,
}

/// The knee of a callout stays at least this far from its box, in canvas document units.
pub const CALLOUT_KNEE_GAP: f64 = 12.0 * 96.0 / 72.0;

/// A move of at most this many page units is a slip, not a move.
pub const SLIP_MOVE: f64 = 2.0;

/// A resize of at most this many page units is a slip, not a resize.
pub const SLIP_RESIZE: f64 = 4.0;

/// The smallest side of a shape the tool keeps, in page units.
pub const MIN_SHAPE: f64 = 2.0;

/// The Shift rule of a point drag or drawn line, held from its anchor point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShiftLock {
    /// The segment snaps to 45 degree steps.
    #[default]
    Angle45,
    /// The segment stays level or plumb.
    LevelOrPlumb,
}

impl ShiftLock {
    /// `to` held to this rule from `anchor`.
    pub(crate) fn apply(self, anchor: Point, to: Point) -> Point {
        match self {
            ShiftLock::Angle45 => turn_to_45(anchor, to),
            // The nearer axis from the anchor: the segment goes level or plumb.
            ShiftLock::LevelOrPlumb => {
                if (to.x - anchor.x).abs() >= (to.y - anchor.y).abs() {
                    Point::new(to.x, anchor.y)
                } else {
                    Point::new(anchor.x, to.y)
                }
            }
        }
    }
}

/// `points` with the callout's point `i` moved to `to`: the knee (`i == 1`) moves along its knee line only (level
/// beside the box, upright above or below it) and never nearer the box than [`CALLOUT_KNEE_GAP`]; every other point
/// moves through [`reshaped`], held to `lock` from its neighbour.
pub fn callout_reshaped(rect: Rect, points: &[Point], i: usize, to: Point, lock: Option<ShiftLock>) -> Vec<Point> {
    let Some(&knee) = points.get(1).filter(|_| i == 1) else { return reshaped(points, i, to, lock) };
    let (r, gap) = (rect.abs(), CALLOUT_KNEE_GAP);
    let to = match KneeSide::of(r, knee) {
        KneeSide::Left => Point::new(to.x.min(r.x0 - gap), knee.y),
        KneeSide::Right => Point::new(to.x.max(r.x1 + gap), knee.y),
        KneeSide::Above => Point::new(knee.x, to.y.min(r.y0 - gap)),
        KneeSide::Below => Point::new(knee.x, to.y.max(r.y1 + gap)),
    };
    let mut out = points.to_vec();
    if let Some(k) = out.get_mut(1) {
        *k = to;
    }
    out
}

/// The side of a callout's box its knee lies on, which decides the knee line.
#[derive(Clone, Copy)]
enum KneeSide {
    Left,
    Right,
    Above,
    Below,
}

impl KneeSide {
    /// The side `knee` lies farthest outside of the normalised `rect`; inside it, the nearest edge's side.
    fn of(rect: Rect, knee: Point) -> KneeSide {
        [
            (KneeSide::Left, rect.x0 - knee.x),
            (KneeSide::Right, knee.x - rect.x1),
            (KneeSide::Above, rect.y0 - knee.y),
            (KneeSide::Below, knee.y - rect.y1),
        ]
        .into_iter()
        .fold((KneeSide::Left, f64::NEG_INFINITY), |best, side| if side.1 > best.1 { side } else { best })
        .0
    }
}

/// The width over height a corner resize of `annotation` keeps from `from` (with Shift); `None` for a flat box.
pub fn corner_aspect(annotation: &Annotation, from: Rect, shift: bool) -> Option<f64> {
    annotation.aspect.or_else(|| (shift && from.height() > 0.0).then(|| from.width() / from.height()))
}

/// The gap between a marker and its markup's last quad, in icon sides.
const MARKER_GAP: f64 = 0.25;

/// The box of `annotation`'s marker on `page`, clamped to the page; `None` when it has none. A commented markup has a
/// marker an `icon_side` square above and right of its last quad; a page smaller than the marker, or a markup off the
/// page, gets none rather than one covering the page or floating beside it.
pub fn marker_rect(annotation: &Annotation, page: Rect, icon_side: f64) -> Option<Rect> {
    if !annotation.kind.is_markup() || annotation.text.trim().is_empty() {
        return None;
    }
    let target = annotation.points.chunks(4).last().and_then(bounds)?;
    let page = page.abs();
    let gap = icon_side * MARKER_GAP;
    let wanted = Point::new(target.x1 + gap, target.y0 - gap - icon_side);
    let fits =
        page.is_finite() && target.is_finite() && wanted.is_finite() && icon_side > 0.0 && page.width() >= icon_side && page.height() >= icon_side;
    let on_page = target.x0 <= page.x1 && target.x1 >= page.x0 && target.y0 <= page.y1 && target.y1 >= page.y0;
    if !fits || !on_page {
        return None;
    }
    // `min` then `max`, not `clamp`: rounding can put `x1 - icon_side` an ulp below `x0`.
    let at = Point::new(wanted.x.min(page.x1 - icon_side).max(page.x0), wanted.y.min(page.y1 - icon_side).max(page.y0));
    Some(Rect::from_origin_size(at, (icon_side, icon_side)))
}

/// The annotation the comment part, the hover outline and the marker hit at `at`, within `tolerance` page units: a
/// marker of `markers` first, the last listed winning, then the topmost body (see [`hit`]).
pub fn pick_at(annotations: &[Annotation], at: Point, tolerance: f64, icon_side: f64, markers: &[(AnnotationId, Rect)]) -> Option<AnnotationId> {
    if let Some((id, _)) = markers.iter().rev().find(|(_, r)| r.inflate(tolerance, tolerance).contains(at)) {
        return Some(*id);
    }
    annotations.iter().rev().find(|a| body_hit(a, at, tolerance, icon_side)).map(|a| a.id)
}

impl Annotation {
    /// The annotation as the in-flight `edit` leaves it, for the overlay's live paint; `None` when the edit does not
    /// move, resize or reshape it. A resized callout keeps its tip and knee: only its box goes.
    #[cfg(feature = "ui")]
    pub fn edited(&self, edit: &AnnotationEdit) -> Option<Annotation> {
        let mut out = self.clone();
        match edit {
            AnnotationEdit::Move { ids, delta } if ids.contains(&self.id) => {
                out.rect = self.rect + *delta;
                for p in out.points.iter_mut().chain(out.strokes.iter_mut().flatten()) {
                    *p += *delta;
                }
            }
            AnnotationEdit::Resize { id, rect } if *id == self.id => out.rect = *rect,
            AnnotationEdit::Reshape { id, points } if *id == self.id => {
                out.points = points.clone();
                if self.kind.is_pointwise()
                    && let Some(r) = bounds(points)
                {
                    out.rect = r;
                }
            }
            _ => return None,
        }
        Some(out)
    }
}

impl Annotation {
    /// The box the annotation takes on the page: its `rect`, which the icon kinds (Note,
    /// Attachment, Caret) unite with the `icon_side` square drawn at the rect's corner, so the
    /// whole drawn icon and the whole rect both count. `icon_side` is the icon's side in page
    /// units at the current zoom.
    pub fn extent(&self, icon_side: f64) -> Rect {
        if self.kind.is_fixed_size() { self.rect.abs().union(Rect::from_origin_size(self.rect.origin(), (icon_side, icon_side))) } else { self.rect }
    }
}

/// A handle of a selected annotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnotationHandle {
    /// One of the eight box handles, clockwise from the top-left: `(-1, -1)`, `(0, -1)`, `(1, -1)`,
    /// `(1, 0)`, `(1, 1)`, `(0, 1)`, `(-1, 1)`, `(-1, 0)` (PdfCraft `geometry.rs:50`).
    Box(i8, i8),
    /// The defining point `i` of a pointwise kind.
    Point(usize),
}

/// What a point on the page hits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnnotationHit {
    /// A handle of a selected annotation.
    Handle(AnnotationId, AnnotationHandle),
    /// The body of an annotation.
    Body(AnnotationId),
}

/// The handles `annotation` shows when selected: its points for pointwise kinds, else the eight
/// box handles, with a callout's tip and knee besides. None under a [`HandleRule`] that fixes the
/// shape, nor for markups, icon kinds and locked annotations: a handle that would do nothing is
/// never offered.
pub fn handles(annotation: &Annotation) -> Vec<(AnnotationHandle, Point)> {
    if matches!(annotation.handle_rule, HandleRule::FixedPoints | HandleRule::NoResize) {
        return Vec::new();
    }
    if annotation.locked || annotation.kind.is_markup() {
        return Vec::new();
    }
    if annotation.kind.is_pointwise() && !annotation.points.is_empty() {
        return annotation.points.iter().enumerate().map(|(i, &p)| (AnnotationHandle::Point(i), p)).collect();
    }
    if annotation.kind.is_fixed_size() {
        return Vec::new();
    }
    let boxes = BOX_HANDLES.iter().map(|&(hx, hy)| (AnnotationHandle::Box(hx, hy), box_handle_point(annotation.rect, (hx, hy))));
    let leader = annotation.points.iter().take(2).enumerate().filter(|_| annotation.kind == AnnotationKind::Callout);
    boxes.chain(leader.map(|(i, &p)| (AnnotationHandle::Point(i), p))).collect()
}

/// The eight box handles, clockwise from the top-left.
pub const BOX_HANDLES: [(i8, i8); 8] = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];

/// Where handle `(hx, hy)` sits on `rect`.
pub fn box_handle_point(rect: Rect, (hx, hy): (i8, i8)) -> Point {
    let x = match hx {
        -1 => rect.x0,
        0 => rect.center().x,
        _ => rect.x1,
    };
    let y = match hy {
        -1 => rect.y0,
        0 => rect.center().y,
        _ => rect.y1,
    };
    Point::new(x, y)
}

/// The smallest width of an aspect-keeping corner resize (PdfCraft `geometry.rs:72`).
const MIN_ASPECT_WIDTH: f64 = 4.0;

fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 <= f64::EPSILON {
        return (p - a).hypot();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).hypot()
}

fn distance_to_path(p: Point, path: &[Point], closed: bool) -> f64 {
    let open = path.windows(2).map(|w| distance_to_segment(p, w[0], w[1]));
    let closing = (closed && path.len() > 2).then(|| distance_to_segment(p, path[path.len() - 1], path[0]));
    open.chain(closing).fold(f64::INFINITY, f64::min).min(path.first().map_or(f64::INFINITY, |&a| (p - a).hypot()))
}

/// The box that holds `points`; `None` for none.
fn bounds(points: &[Point]) -> Option<Rect> {
    let (&first, rest) = points.split_first()?;
    Some(rest.iter().fold(Rect::from_points(first, first), |r, &p| r.union_pt(p)))
}

/// A callout's leader in page units: from its tip through its knee to the nearest point of its box. The overlay paints
/// it and [`body_hit`] tests it, so what is drawn is what is picked.
pub(crate) fn callout_leader(a: &Annotation) -> Vec<Point> {
    let r = a.rect.abs();
    let mut path: Vec<Point> = a.points.iter().take(2).copied().collect();
    if let Some(&knee) = path.last() {
        path.push(Point::new(knee.x.max(r.x0).min(r.x1), knee.y.max(r.y0).min(r.y1)));
    }
    path
}

fn body_hit(a: &Annotation, at: Point, tolerance: f64, icon_side: f64) -> bool {
    let reach = tolerance + a.stroke_width / 2.0;
    match a.kind {
        AnnotationKind::Line | AnnotationKind::Arrow | AnnotationKind::PolyLine => distance_to_path(at, &a.points, false) <= reach,
        AnnotationKind::Polygon | AnnotationKind::Cloud => distance_to_path(at, &a.points, true) <= reach,
        AnnotationKind::Ink | AnnotationKind::FreehandHighlight => a.strokes.iter().any(|s| distance_to_path(at, s, false) <= reach),
        AnnotationKind::Callout => {
            a.rect.inflate(tolerance, tolerance).contains(at) || (a.points.len() > 1 && distance_to_path(at, &callout_leader(a), false) <= reach)
        }
        AnnotationKind::Oval => {
            let r = a.rect.inflate(tolerance, tolerance);
            let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
            let c = r.center();
            rx > 0.0 && ry > 0.0 && ((at.x - c.x) / rx).powi(2) + ((at.y - c.y) / ry).powi(2) <= 1.0
        }
        kind if kind.is_markup() && !a.points.is_empty() => {
            a.points.chunks(4).filter_map(bounds).any(|r| r.inflate(tolerance, tolerance).contains(at))
        }
        _ => a.extent(icon_side).inflate(tolerance, tolerance).contains(at),
    }
}

/// The topmost hit at `at` within `tolerance` page units: a handle of a selected annotation
/// first, then any annotation's body (strokes and lines by distance, boxes by containment),
/// last drawn first. `icon_side` is the side, in page units, of the icon a Note, Attachment or
/// Caret shows: those kinds keep one size on screen whatever the zoom (Acrobat's NoZoom), so the
/// caller works it out each pass from the view.
pub fn hit(annotations: &[Annotation], selected: &[AnnotationId], at: Point, tolerance: f64, icon_side: f64) -> Option<AnnotationHit> {
    for id in selected {
        let Some(annotation) = annotations.iter().find(|a| a.id == *id) else { continue };
        if let Some((handle, _)) = handles(annotation).into_iter().find(|(_, p)| (p.x - at.x).abs() <= tolerance && (p.y - at.y).abs() <= tolerance) {
            return Some(AnnotationHit::Handle(*id, handle));
        }
    }
    annotations.iter().rev().find(|a| body_hit(a, at, tolerance, icon_side)).map(|a| AnnotationHit::Body(a.id))
}

/// `rect` with handle `(hx, hy)` moved by `delta`; with `aspect` (Shift on a corner, or a kind
/// that keeps its aspect) the ratio is kept. A plain resize adds the delta and normalises the
/// rect, so dragging a handle past the opposite edge flips the box (PdfCraft `geometry.rs:89-101`).
// pdfcraft: crates/ui-egui/src/tools/comments/geometry.rs:72 `resized`, `:68` `corner_ratio`.
pub fn resized(rect: Rect, handle: (i8, i8), delta: Vec2, aspect: Option<f64>) -> Rect {
    let (hx, hy) = handle;
    if hx != 0
        && hy != 0
        && let Some(ratio) = aspect.filter(|r| r.is_finite() && *r > 0.0)
    {
        // The larger requested dimension drives the corner; the opposite corner stays fixed.
        let width = (rect.width() + f64::from(hx) * delta.x)
            .max((rect.height() + f64::from(hy) * delta.y) * ratio)
            .max(MIN_ASPECT_WIDTH)
            .max(MIN_ASPECT_WIDTH * ratio);
        let anchor = box_handle_point(rect, (-hx, -hy));
        let to = Point::new(anchor.x + f64::from(hx) * width, anchor.y + f64::from(hy) * width / ratio);
        return Rect::from_points(anchor, to);
    }
    let mut r = rect;
    match hx {
        -1 => r.x0 += delta.x,
        1 => r.x1 += delta.x,
        _ => {}
    }
    match hy {
        -1 => r.y0 += delta.y,
        1 => r.y1 += delta.y,
        _ => {}
    }
    r.abs()
}

/// `points` with point `i` moved to `to`; with `lock` the segment from its neighbour is held to the rule.
pub fn reshaped(points: &[Point], i: usize, to: Point, lock: Option<ShiftLock>) -> Vec<Point> {
    let mut out = points.to_vec();
    let anchor = if i == 0 { points.get(1) } else { points.get(i - 1) };
    let to = match (anchor, lock) {
        (Some(&a), Some(lock)) => lock.apply(a, to),
        _ => to,
    };
    if let Some(p) = out.get_mut(i) {
        *p = to;
    }
    out
}

/// What an annotation gesture asks of the app. Moves and reshapes are reported as the value of
/// an `EditEvent` gesture (one undo step per drag).
#[cfg(feature = "ui")]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AnnotationEdit {
    /// A new annotation drawn with a tool; for a Note or Text box the app opens the composer
    /// ([`CommentComposer`](crate::ui::CommentComposer)) on it.
    Create(Annotation),
    /// Move annotations by `delta`.
    Move {
        #[allow(missing_docs)]
        ids: Vec<AnnotationId>,
        /// How far the annotations move, in page units.
        delta: Vec2,
    },
    /// Resize an annotation's box.
    Resize {
        #[allow(missing_docs)]
        id: AnnotationId,
        /// The new bounding box.
        rect: Rect,
    },
    /// Move the defining points of an annotation.
    Reshape {
        #[allow(missing_docs)]
        id: AnnotationId,
        /// The new defining points.
        points: Vec<Point>,
    },
    /// Remove these annotations.
    Delete(Vec<AnnotationId>),
    /// The Eraser crossed these strokes of an Ink annotation. One event per annotation per
    /// gesture, with the strokes in ascending order.
    Erase {
        #[allow(missing_docs)]
        id: AnnotationId,
        /// Indices of the crossed strokes in [`Annotation::strokes`].
        strokes: Vec<usize>,
    },
    /// The selection changed; the tool has applied it to [`AnnotateTool::selected`](crate::ui::AnnotateTool).
    Selection(craft_behavior::SelectionChange<AnnotationId>),
    /// A double-click on a note or text box: the app opens its text for editing.
    Open(AnnotationId),
    /// A click or drag placement of a new annotation.
    Place(Placement),
    /// The Eraser erased along `path` with `radius` through these annotations.
    EraseAlong {
        /// The annotations the eraser reached.
        ids: Vec<AnnotationId>,
        /// The eraser's path, in page units.
        path: Vec<Point>,
        /// The eraser's radius, in page units.
        radius: f64,
    },
}

/// Where a click or drag placed a new annotation: a Note, Attach or Caret at `at`; a TextBox with `size`; a Callout
/// with its tip at `from` and its box at `at`.
#[cfg(feature = "ui")]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Placement {
    /// The kind placed.
    pub kind: AnnotationKind,
    /// The page index.
    pub page: usize,
    /// The click point, or the box's corner.
    pub at: Point,
    /// The box's size, when the placement dragged one.
    pub size: Option<Vec2>,
    /// A callout's tip.
    pub from: Option<Point>,
}

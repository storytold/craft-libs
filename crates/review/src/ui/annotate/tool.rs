//! The canvas tool: presses, drags, the "New Annotation..." form and the [`CanvasTool`] implementation.

use craft_behavior::{CancelReason, CanvasView, Cursor, CursorIcon, EditEvent, GestureId, Mixed, PointerButton, SelectionChange};
use craft_keymap::Mods;
use craft_theme::Srgba;
use craft_ui::canvas::{CanvasTool, NUDGE, NUDGE_FAR, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft_ui::primitives::{DataWidth, Draw, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::DataColor;
use egui::{Color32, Pos2};
use kurbo::{Point, Rect, Vec2};
use std::sync::Arc;

use super::AnnotateWords;
use super::overlay::ellipse_points;
use crate::annotate::{
    Annotation, AnnotationEdit, AnnotationHandle, AnnotationHit, AnnotationId, AnnotationKind, AnnotationTool, HandleRule, MIN_SHAPE, Placement,
    SLIP_MOVE, SLIP_RESIZE, ShiftLock, callout_reshaped, corner_aspect, hit, reshaped, resized,
};

/// The page text between two page points as quads (four corners each).
pub type TextQuads = Box<dyn Fn(Point, Point) -> Vec<[Point; 4]> + Send>;

/// Acrobat's default border width (Preferences > Commenting, "Line thickness" 1), in page units.
pub const DEFAULT_STROKE_WIDTH: f64 = 1.0;
/// Acrobat's default colours (Preferences > Commenting): red for shapes and notes, yellow for
/// highlights (straight RGB).
pub const DEFAULT_COLOR: [u8; 3] = [0xE0, 0x30, 0x30];
/// The default highlight colour, straight RGB.
pub const HIGHLIGHT_COLOR: [u8; 3] = [0xFF, 0xE0, 0x30];
/// A highlight lets the text show through.
const HIGHLIGHT_OPACITY: f32 = 0.4;
/// The side of a note the "New Annotation..." form makes, in page units: the fixed square Acrobat gives a note
/// annotation, whatever the icon size on screen.
const NOTE_SIDE: f64 = 16.0;
/// The key of the "New Annotation..." command form.
pub const NEW_ANNOTATION_FORM: &str = "newAnnotation";
/// A text box drag shorter than this on either side, in screen points, is a click: the host fits the box to its text.
const PLACE_DRAG_MIN: f32 = 6.0;
/// The shortest leader a callout drag places, in page units; a shorter drag places nothing.
const CALLOUT_MIN_LEADER: f64 = 8.0;
/// A polygon click this close to the last point, in screen points, adds none: a second press of the same spot.
const REPEAT_POINT: f32 = 2.0;

/// The app's caret snap: the snapped tip and the caret's side for a click, or `None` away from text.
type CaretSnap = Box<dyn Fn(Point) -> Option<(Point, f64)> + Send>;

/// What a press began and the drag or release continues.
#[derive(Clone, Debug)]
enum Drag {
    /// Moving, resizing or reshaping selected annotations; `begun` once the pointer has moved. `mods` are the
    /// latest the gesture saw, so the value in flight can be read between its events.
    Edit { id: GestureId, begun: bool, origin: Point, change: Change, mods: Mods },
    /// A selection rectangle on empty page.
    Marquee { origin: Point, current: Point, additive: bool },
    /// A box, oval or straight line drawn by dragging.
    Shape { kind: AnnotationKind, origin: Point, current: Point },
    /// A text box or callout placed by dragging (a callout from its tip to its box).
    Place { kind: AnnotationKind, origin: Point, current: Point },
    /// Page text selected for a markup kind.
    Markup { kind: AnnotationKind, origin: Point, current: Point },
    /// One Ink or freehand highlight stroke in progress.
    Stroke(Vec<Point>),
    /// The Eraser's path and the strokes it crossed.
    Erase { path: Vec<Point>, crossed: Vec<(AnnotationId, usize)> },
}

#[derive(Clone, Debug)]
enum Change {
    Move {
        ids: Vec<AnnotationId>,
    },
    /// Moving a callout's text box leaves its arrow point in place.
    MoveBox {
        target: AnnotationId,
        from: Rect,
    },
    /// `from` is the annotation as the press found it: its box and its own aspect.
    Resize {
        target: AnnotationId,
        handle: (i8, i8),
        from: Annotation,
    },
    /// `callout` is the box of a callout, whose points move by the callout's rule.
    Reshape {
        target: AnnotationId,
        index: usize,
        from: Vec<Point>,
        callout: Option<Rect>,
    },
}

/// What a call knows when it reads a pointer: the pointer in page units, the page-unit sizes of the screen
/// distances the tool tests with, the host's gesture id, the tool's words, the view and the page box.
struct Reading {
    at: Point,
    tolerance: f64,
    icon_side: f64,
    gesture: Option<GestureId>,
    words: Arc<AnnotateWords>,
    view: CanvasView,
    /// The page box, when the canvas knows the document's size.
    page: Option<Rect>,
}

fn read(cx: &ToolContext<'_>, pos: Pos2) -> Option<Reading> {
    let space = cx.space()?;
    Some(Reading {
        at: space.view.to_doc(pos),
        tolerance: space.view.doc_len(cx.slop),
        icon_side: space.view.doc_len(cx.icon_side()),
        gesture: cx.gesture_id(),
        words: cx.words::<AnnotateWords>(),
        view: space.view,
        page: space.document().map(|size| Rect::from_origin_size(Point::ORIGIN, size)),
    })
}

/// `p` held inside the page box, when there is one.
fn clamp_to(page: Option<Rect>, p: Point) -> Point {
    page.map_or(p, |page| Point::new(p.x.clamp(page.x0, page.x1), p.y.clamp(page.y0, page.y1)))
}

/// Whether two boxes touch; a box of no width (a straight stroke) still counts.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Whether the Eraser reaches `a`, in either mode: an unlocked Ink or freehand highlight.
fn erasable(a: &Annotation) -> bool {
    matches!(a.kind, AnnotationKind::Ink | AnnotationKind::FreehandHighlight) && !a.locked
}

/// The cursor over a handle: the resize arrow of a box handle, a crosshair over a point.
fn handle_icon(handle: AnnotationHandle) -> CursorIcon {
    match handle {
        AnnotationHandle::Box(0, _) => CursorIcon::ResizeVertical,
        AnnotationHandle::Box(_, 0) => CursorIcon::ResizeHorizontal,
        AnnotationHandle::Box(-1, -1) | AnnotationHandle::Box(1, 1) => CursorIcon::ResizeNwSe,
        AnnotationHandle::Box(..) => CursorIcon::ResizeNeSw,
        AnnotationHandle::Point(_) => CursorIcon::Crosshair,
    }
}

fn bounds(points: impl IntoIterator<Item = Point>) -> Rect {
    let mut it = points.into_iter();
    let Some(first) = it.next() else { return Rect::ZERO };
    it.fold(Rect::from_points(first, first), |r, p| r.union_pt(p))
}

pub(super) fn color32(annotation: &Annotation) -> Color32 {
    let [r, g, b] = annotation.color;
    Color32::from_rgba_unmultiplied(r, g, b, (annotation.opacity.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub(super) fn srgba([r, g, b]: [u8; 3]) -> Srgba {
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// `to` made square about `origin` (Shift on a box or oval).
fn squared(origin: Point, to: Point) -> Point {
    let d = to - origin;
    let side = d.x.abs().max(d.y.abs());
    Point::new(origin.x + side * d.x.signum(), origin.y + side * d.y.signum())
}

pub(super) fn corners(rect: Rect) -> [Point; 4] {
    [Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)]
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point, reach: f64) -> bool {
    let near = |p: Point, s: Point, e: Point| {
        let se = e - s;
        let len2 = se.hypot2();
        let t = if len2 <= f64::EPSILON { 0.0 } else { ((p - s).dot(se) / len2).clamp(0.0, 1.0) };
        (p - (s + se * t)).hypot() <= reach
    };
    let orient = |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let proper = orient(a, b, c) * orient(a, b, d) < 0.0 && orient(c, d, a) * orient(c, d, b) < 0.0;
    proper || near(a, c, d) || near(b, c, d) || near(c, a, b) || near(d, a, b)
}

/// Acrobat's annotation tools as one canvas tool on craft-ui's tool host.
///
/// - Select: a press on an annotation selects it (Shift adds), a drag moves the selection, a
///   drag on a handle of the primary (last) selected annotation resizes (box kinds; an annotation's own
///   aspect, or Shift on a corner, keeps the ratio) or reshapes (pointwise kinds; Shift holds the segment by
///   [`SelectOptions::shift_lock`]); a press on empty page starts a marquee. Moves, resizes and reshapes are
///   `EditEvent<AnnotationEdit>` gestures; a move within [`SLIP_MOVE`] or a resize within [`SLIP_RESIZE`] ends
///   as a cancel, Esc cancels and the release after it is ignored. [`SelectOptions`] names the variations.
/// - A drawing tool: a drag draws the kind (Shift: squares, circles, and lines held by
///   [`SelectOptions::shift_lock`]), and a shape smaller
///   than [`MIN_SHAPE`] makes none; PolyLine, Polygon and Cloud add a point per click, close on a click at
///   their first point (not PolyLine), take back their last point with Backspace and end with a double click
///   or Enter; Ink and freehand highlight record strokes until the tool changes, or make one annotation per
///   stroke ([`DrawOptions::ink`]). Note, Attach and Caret report `Place` at the click; Text box and Callout
///   report `Place` for their drag; the app opens the composer. Markup kinds select page text through the
///   app's text hit test ([`text_quads`](Self::text_quads)) and report the quads.
/// - Eraser: a drag reports one `Erase` per Ink or freehand highlight it crosses, with the strokes in
///   ascending order, or one `EraseAlong` with its path ([`DrawOptions::erase`]).
/// - Keys: Delete and Backspace remove the selection, arrows nudge it (one point, Shift ten) as `Move`s.
/// - A double click on a note or text box reports `Open`; the app's page text and selection reach the tool
///   in [`CanvasTool::each_pass`] through the public fields. A click on a comment part the host shows reaches
///   the tool through [`click`](Self::click), by the same rule.
/// - New annotations take the tool's [`color`](Self::color) and [`width`](Self::width) (a
///   highlight its [`highlight_color`](Self::highlight_color)); the toolbar's controls set them.
///   Notes, attachments and carets are created with an icon-sized rect and keep that size on
///   screen at any zoom, so hits test them against an icon side worked out each pass.
/// - Locked annotations are selected but never moved.
/// - Without a pointer, the "New Annotation..." form places the shape kinds
///   ([`new_annotation_params`](Self::new_annotation_params), [`create_from_form`](Self::create_from_form)).
#[non_exhaustive]
pub struct AnnotateTool {
    /// The tool in use.
    pub tool: AnnotationTool,
    /// The page's annotations, refreshed by the app each pass.
    pub annotations: Vec<Annotation>,
    /// The ids of the selected annotations.
    pub selected: Vec<AnnotationId>,
    /// The page the tool works on.
    pub page: usize,
    /// The colour of new annotations other than highlights (straight RGB).
    pub color: [u8; 3],
    /// The colour of new highlights (straight RGB).
    pub highlight_color: [u8; 3],
    /// The stroke width of new annotations, in page units.
    pub width: f64,
    /// The edits since the app last took them ([`take_edits`](Self::take_edits)).
    edits: Vec<EditEvent<AnnotationEdit>>,
    text_quads: Option<TextQuads>,
    gesture: Option<Drag>,
    /// The points of the PolyLine, Polygon or Cloud being drawn.
    poly: Vec<Point>,
    /// The Ink strokes drawn since the tool was chosen.
    strokes: Vec<Vec<Point>>,
    hover: Option<Point>,
    last_tool: AnnotationTool,
    select_options: SelectOptions,
    draw_options: DrawOptions,
    caret_snap: Option<CaretSnap>,
}

impl AnnotateTool {
    /// A tool of kind `tool` working on `page`, with the default colours and width.
    pub fn new(tool: AnnotationTool, page: usize) -> Self {
        Self {
            tool,
            annotations: Vec::new(),
            selected: Vec::new(),
            page,
            color: DEFAULT_COLOR,
            highlight_color: HIGHLIGHT_COLOR,
            width: DEFAULT_STROKE_WIDTH,
            edits: Vec::new(),
            text_quads: None,
            gesture: None,
            poly: Vec::new(),
            strokes: Vec::new(),
            hover: None,
            last_tool: tool,
            select_options: SelectOptions::default(),
            draw_options: DrawOptions::default(),
            caret_snap: None,
        }
    }

    /// The page text between two points as quads (the app's text selection), for markups.
    #[must_use]
    pub fn text_quads(mut self, quads: impl Fn(Point, Point) -> Vec<[Point; 4]> + Send + 'static) -> Self {
        self.text_quads = Some(Box::new(quads));
        self
    }

    /// The edits the tool reported since the last call, in order.
    pub fn take_edits(&mut self) -> Vec<EditEvent<AnnotationEdit>> {
        std::mem::take(&mut self.edits)
    }

    /// The rows of the "New Annotation..." form (`ParamDialog::command(NEW_ANNOTATION_FORM, title, params)`):
    /// kind, page, position and size in points.
    pub fn new_annotation_params(&self, words: &AnnotateWords) -> Vec<Param> {
        let options: Vec<(String, Label)> =
            FORM_KINDS.iter().map(|&kind| (form_id(kind).to_owned(), Label::from(words.kind(kind).to_owned()))).collect();
        let number = |default: f64| ParamKind::number(Some(0.0), None, default, 1.0, 1);
        vec![
            Param::new("kind", words.form_kind.clone(), ParamKind::choice(options, Some(form_id(AnnotationKind::Rectangle).to_owned()))),
            Param::new("page", words.form_page.clone(), ParamKind::number(Some(1.0), None, (self.page + 1) as f64, 1.0, 0)),
            Param::new("x", words.form_x.clone(), number(0.0)),
            Param::new("y", words.form_y.clone(), number(0.0)),
            Param::new("width", words.form_width.clone(), number(0.0)),
            Param::new("height", words.form_height.clone(), number(0.0)),
        ]
    }

    /// Reports `Create` for the annotation the confirmed "New Annotation..." form describes, in the tool's colour
    /// and width. Returns whether it made one: a form that describes no valid shape makes none.
    pub fn create_from_form(&mut self, fields: &DialogFields, words: &AnnotateWords) -> bool {
        let Some(FieldValue::Text(id)) = fields.get("kind") else { return false };
        let Some(kind) = FORM_KINDS.iter().copied().find(|&kind| form_id(kind) == id.as_str()) else { return false };
        let number = |key: &str| fields.number(key).filter(|value| value.is_finite());
        let (Some(x), Some(y), Some(width), Some(height)) = (number("x"), number("y"), number("width"), number("height")) else { return false };
        let page = number("page").map_or(self.page, |page| (page.round().max(1.0) as usize) - 1);
        let from = Point::new(x, y);
        let to = Point::new(x + width, y + height);
        let area = Rect::from_points(from, to);
        let mut annotation = match kind {
            AnnotationKind::Note => {
                let mut note = self.fresh(kind, Rect::from_origin_size(from, (NOTE_SIDE, NOTE_SIDE)));
                note.points = vec![from];
                note
            }
            AnnotationKind::Line | AnnotationKind::Arrow => {
                if from == to {
                    return false;
                }
                let mut line = self.fresh(kind, bounds([from, to]));
                line.points = vec![from, to];
                line
            }
            AnnotationKind::Polygon | AnnotationKind::Cloud => {
                if area.width() <= 0.0 || area.height() <= 0.0 {
                    return false;
                }
                let mut shape = self.fresh(kind, area);
                shape.points = corners(area).to_vec();
                shape
            }
            _ => {
                if area.width() <= 0.0 || area.height() <= 0.0 {
                    return false;
                }
                self.fresh(kind, area)
            }
        };
        annotation.page = page;
        self.create(annotation, words);
        true
    }
}

/// What a press on empty page does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmptyPress {
    /// A selection rectangle.
    #[default]
    Marquee,
    /// The press is ignored (`ToolAction::Ignored`).
    PassThrough,
}

/// The modifiers that toggle a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToggleMods {
    /// Shift.
    #[default]
    Shift,
    /// Shift or Command.
    ShiftOrCommand,
}

/// What opens a note or text box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenOn {
    /// A double click on a note or text box.
    #[default]
    NotesAndTextBoxes,
    /// A double click on any annotation.
    Every,
}

/// The modifiers that nudge a selection by the far step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FarNudge {
    /// Shift.
    #[default]
    Shift,
    /// Shift or Command.
    ShiftOrCommand,
}

/// What a press on a handle does while a shape is being drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandlesWhileDrawing {
    /// The press continues the drawing.
    #[default]
    Draw,
    /// A press on a resize handle edits through it; one on an end or corner point continues the drawing.
    Edit,
}

/// What a press on a text markup does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkupPress {
    /// The press selects the markup.
    #[default]
    Select,
    /// Only a click reports (`ToolAction::Ignored` otherwise).
    ClickOnly,
}

/// The tool's named selection options; `Default` is today's behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SelectOptions {
    /// What a press on empty page does.
    pub empty_press: EmptyPress,
    /// The modifiers that toggle a selection.
    pub toggle: ToggleMods,
    /// What opens a note or text box.
    pub open: OpenOn,
    /// The Shift rule of a point drag and of drawn lines and polygon sides.
    pub shift_lock: ShiftLock,
    /// The modifiers that nudge by the far step.
    pub far_nudge: FarNudge,
    /// What a press on a handle does while drawing.
    pub handles_while_drawing: HandlesWhileDrawing,
    /// What a press on a text markup does.
    pub markup_press: MarkupPress,
}

/// Whether ink strokes gather into one annotation per session or one per stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum InkStrokes {
    /// One annotation per session.
    #[default]
    Session,
    /// One annotation per stroke.
    PerStroke,
}

/// What the eraser removes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum EraseMode {
    /// Whole strokes.
    #[default]
    Strokes,
    /// Along its path (`EraseAlong`).
    Path,
}

/// The tool's named drawing options.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DrawOptions {
    /// How ink strokes gather.
    pub ink: InkStrokes,
    /// What the eraser removes.
    pub erase: EraseMode,
    /// The eraser's radius, in screen points.
    pub eraser_radius: f32,
}

impl Default for AnnotateTool {
    fn default() -> Self {
        Self::new(AnnotationTool::Select, 0)
    }
}

impl AnnotateTool {
    /// A click on a comment part handed from the page: selects (with the toggle modifiers of [`SelectOptions`]) and, on
    /// the open count, opens. The host calls it once from the part's click, never again on a later pass; the rule is
    /// the one a press on the annotation's body runs.
    pub fn click(&mut self, id: AnnotationId, mods: Mods, clicks: u32, words: &AnnotateWords) {
        self.select_part(id, mods, clicks, words);
    }

    /// The cursor over page point `at`, with the page-unit `tolerance` and `icon_side` the tool's reading holds.
    pub fn cursor_at(&self, at: Point, tolerance: f64, icon_side: f64) -> Cursor {
        let icon = match self.tool {
            AnnotationTool::Select => match hit(&self.annotations, self.primary(), at, tolerance, icon_side) {
                Some(AnnotationHit::Handle(_, handle)) => handle_icon(handle),
                // A locked annotation or a markup does not move: the press only selects it.
                Some(AnnotationHit::Body(id)) if self.annotations.iter().any(|a| a.id == id && (a.locked || a.kind.is_markup())) => {
                    CursorIcon::PointingHand
                }
                Some(AnnotationHit::Body(_)) => CursorIcon::Move,
                None => CursorIcon::Default,
            },
            AnnotationTool::Draw(kind) => match self.handle_while_drawing(kind, at, tolerance, icon_side) {
                Some((_, handle)) => handle_icon(handle),
                None if kind == AnnotationKind::Caret => CursorIcon::Text,
                None => CursorIcon::Crosshair,
            },
            AnnotationTool::Eraser => CursorIcon::Crosshair,
        };
        Cursor::System(icon)
    }

    /// The edit the gesture in flight would report, for the host's live paint: a move, resize or reshape once the
    /// pointer has moved; `None` otherwise.
    pub fn in_flight(&self) -> Option<AnnotationEdit> {
        match &self.gesture {
            Some(Drag::Edit { begun: true, origin, change, mods, .. }) => {
                Some(edit_value(change, *origin, self.hover?, *mods, self.select_options.shift_lock))
            }
            _ => None,
        }
    }

    /// The points of the PolyLine, Polygon or Cloud being clicked out; empty when none is.
    pub fn drawing(&self) -> &[Point] {
        &self.poly
    }

    /// Ends the shape being drawn as Enter does: the clicked-out points, or the pending strokes, become an annotation,
    /// reported at once.
    pub fn finish_drawing(&mut self, words: &AnnotateWords) {
        if let AnnotationTool::Draw(kind) = self.tool
            && self.is_poly()
        {
            self.finish_poly(kind, words);
        }
        self.flush_ink(words);
    }

    /// Takes back the last point of the shape being drawn; whether there was one. Taking back the last one ends the
    /// drawing.
    pub fn undo_point(&mut self) -> bool {
        self.poly.pop().is_some()
    }

    /// Drops the shape being drawn, the pending strokes and the gesture in flight; a move, resize or reshape that
    /// had begun reports its cancel.
    pub fn cancel_drawing(&mut self) {
        self.poly.clear();
        self.strokes.clear();
        if let Some(Drag::Edit { id, begun: true, .. }) = self.gesture.take() {
            // The host ended the gesture for the user (a view change, a dialog), not the user's Escape.
            self.edits.push(EditEvent::cancel(id, CancelReason::Dismissed, None));
        }
    }

    /// Snaps a caret to the page text through `snap`: the snapped tip and the caret's side, or `None` away from text,
    /// where the caret goes at the click.
    #[must_use]
    pub fn caret_snap(mut self, snap: impl Fn(Point) -> Option<(Point, f64)> + Send + 'static) -> Self {
        self.caret_snap = Some(Box::new(snap));
        self
    }

    /// The tool's selection options.
    #[must_use]
    pub fn select_options(mut self, options: SelectOptions) -> Self {
        self.select_options = options;
        self
    }

    /// The tool's drawing options.
    #[must_use]
    pub fn draw_options(mut self, options: DrawOptions) -> Self {
        self.draw_options = options;
        self
    }
}

/// The kinds the "New Annotation..." form places.
const FORM_KINDS: [AnnotationKind; 8] = [
    AnnotationKind::Rectangle,
    AnnotationKind::Oval,
    AnnotationKind::Line,
    AnnotationKind::Arrow,
    AnnotationKind::Polygon,
    AnnotationKind::Cloud,
    AnnotationKind::Note,
    AnnotationKind::TextBox,
];

fn form_id(kind: AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Rectangle => "rectangle",
        AnnotationKind::Oval => "oval",
        AnnotationKind::Line => "line",
        AnnotationKind::Arrow => "arrow",
        AnnotationKind::Polygon => "polygon",
        AnnotationKind::Cloud => "cloud",
        AnnotationKind::Note => "note",
        _ => "textBox",
    }
}

impl AnnotateTool {
    /// Reports `edit` as a step of its own, named `label`.
    fn push(&mut self, edit: AnnotationEdit, label: &str) {
        self.edits.push(EditEvent::set(Mixed::Mixed, edit, Arc::from(label), None));
    }

    /// Applies `change` to the selection and reports it.
    fn select(&mut self, change: SelectionChange<AnnotationId>, words: &AnnotateWords) {
        match &change {
            SelectionChange::Set(ids) => self.selected.clone_from(ids),
            SelectionChange::Toggle(id) => match self.selected.iter().position(|selected| selected == id) {
                Some(at) => {
                    self.selected.remove(at);
                }
                None => self.selected.push(*id),
            },
            SelectionChange::Clear => self.selected.clear(),
            _ => {}
        }
        self.push(AnnotationEdit::Selection(change), words.edit_select.as_str());
    }

    /// Replaces the selection with `ids` (a marquee or a plain click), or clears it.
    fn replace_selection(&mut self, ids: Vec<AnnotationId>, words: &AnnotateWords) {
        let change = if ids.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(ids) };
        self.select(change, words);
    }

    /// The primary selection, the last selected: the annotation whose handles the overlay draws and a press can take.
    fn primary(&self) -> &[AnnotationId] {
        let n = self.selected.len();
        &self.selected[n.saturating_sub(1)..]
    }

    fn toggles(&self, mods: Mods) -> bool {
        match self.select_options.toggle {
            ToggleMods::Shift => mods.shift,
            ToggleMods::ShiftOrCommand => mods.shift || mods.cmd,
        }
    }

    fn opens(&self, target: AnnotationId) -> bool {
        self.annotations.iter().any(|a| {
            a.id == target
                && match self.select_options.open {
                    OpenOn::NotesAndTextBoxes => matches!(a.kind, AnnotationKind::Note | AnnotationKind::TextBox),
                    OpenOn::Every => true,
                }
        })
    }

    /// The selection rule of a press on an annotation's body and of a click on its part: a second click opens it (by
    /// [`OpenOn`]), the toggle modifiers toggle it, a plain press on an unselected one selects only it and on a
    /// selected one makes it the primary, keeping the others. Returns whether it opened.
    fn select_part(&mut self, target: AnnotationId, mods: Mods, clicks: u32, words: &AnnotateWords) -> bool {
        let selected = self.selected.contains(&target);
        if clicks >= 2 && self.opens(target) {
            if !selected {
                self.select(SelectionChange::Set(vec![target]), words);
            }
            self.push(AnnotationEdit::Open(target), words.edit_open.as_str());
            return true;
        }
        if self.toggles(mods) {
            self.select(SelectionChange::Toggle(target), words);
        } else if !selected {
            self.select(SelectionChange::Set(vec![target]), words);
        } else if self.primary() != [target] {
            // A set's last key is its lead: the primary, whose handles show.
            let mut ids: Vec<AnnotationId> = self.selected.iter().copied().filter(|id| *id != target).collect();
            ids.push(target);
            self.select(SelectionChange::Set(ids), words);
        }
        false
    }

    /// The handle of the primary selection under `at` that a press of the drawing tool of `kind` edits: only under
    /// [`HandlesWhileDrawing::Edit`], for a shape or point kind, never while a polygon is being clicked out, which
    /// keeps its clicks, and only a resize handle: a drawing tool draws from where it is pressed, so a new line starts
    /// at the end of the one just drawn rather than dragging it.
    fn handle_while_drawing(&self, kind: AnnotationKind, at: Point, tolerance: f64, icon_side: f64) -> Option<(AnnotationId, AnnotationHandle)> {
        use AnnotationKind as K;
        let edits = self.select_options.handles_while_drawing == HandlesWhileDrawing::Edit
            && self.poly.is_empty()
            && !matches!(kind, K::Ink | K::FreehandHighlight | K::Note | K::Attach | K::Caret)
            && !kind.is_markup();
        if !edits {
            return None;
        }
        match hit(&self.annotations, self.primary(), at, tolerance, icon_side) {
            Some(AnnotationHit::Handle(target, handle @ AnnotationHandle::Box(..))) => Some((target, handle)),
            _ => None,
        }
    }

    /// Starts the resize or reshape of `target` through `handle`; whether the annotation was there to start it.
    fn grab_handle(&mut self, id: GestureId, target: AnnotationId, handle: AnnotationHandle, origin: Point, mods: Mods) -> bool {
        let Some(annotation) = self.annotations.iter().find(|a| a.id == target) else { return false };
        let change = match handle {
            AnnotationHandle::Box(hx, hy) => Change::Resize { target, handle: (hx, hy), from: annotation.clone() },
            AnnotationHandle::Point(index) => Change::Reshape {
                target,
                index,
                from: annotation.points.clone(),
                callout: (annotation.kind == AnnotationKind::Callout).then_some(annotation.rect),
            },
        };
        self.gesture = Some(Drag::Edit { id, begun: false, origin, change, mods });
        true
    }

    /// Reports the placement of a new `kind`, after clearing the selection.
    fn place(&mut self, kind: AnnotationKind, at: Point, size: Option<Vec2>, from: Option<Point>, words: &AnnotateWords) {
        if !self.selected.is_empty() {
            self.replace_selection(Vec::new(), words);
        }
        self.push(AnnotationEdit::Place(Placement { kind, page: self.page, at, size, from }), words.edit_create.as_str());
    }

    /// The end of a text box or callout drag from `origin` to `current`, both on the page.
    fn release_place(&mut self, kind: AnnotationKind, origin: Point, current: Point, reading: &Reading) {
        if kind == AnnotationKind::Callout {
            // The tip is where the drag began and the box where it ended; a click or a short drag places nothing.
            if (current - origin).hypot() >= CALLOUT_MIN_LEADER {
                self.place(kind, current, None, Some(origin), &reading.words);
            }
            return;
        }
        let on_screen = egui::Rect::from_two_pos(reading.view.to_screen(origin), reading.view.to_screen(current));
        if on_screen.width() >= PLACE_DRAG_MIN && on_screen.height() >= PLACE_DRAG_MIN {
            let rect = Rect::from_points(origin, current);
            self.place(kind, rect.origin(), Some(rect.size().to_vec2()), None, &reading.words);
        } else {
            // A click, or a drag of a few points: the host fits the box to its text.
            self.place(kind, origin, None, None, &reading.words);
        }
    }

    /// The kind the pending strokes make: that of the tool they were drawn with. [`CanvasTool::each_pass`] moves
    /// `last_tool` on only after it has flushed them.
    fn ink_kind(&self) -> AnnotationKind {
        if self.last_tool == AnnotationTool::Draw(AnnotationKind::FreehandHighlight) {
            AnnotationKind::FreehandHighlight
        } else {
            AnnotationKind::Ink
        }
    }

    fn fresh(&self, kind: AnnotationKind, rect: Rect) -> Annotation {
        let markup = matches!(kind, AnnotationKind::Highlight | AnnotationKind::FreehandHighlight);
        Annotation {
            id: AnnotationId::default(),
            page: self.page,
            kind,
            rect,
            points: Vec::new(),
            strokes: Vec::new(),
            color: if markup { self.highlight_color } else { self.color },
            opacity: if markup { HIGHLIGHT_OPACITY } else { 1.0 },
            stroke_width: self.width,
            text: String::new(),
            locked: false,
            aspect: None,
            handle_rule: HandleRule::default(),
        }
    }

    fn create(&mut self, annotation: Annotation, words: &AnnotateWords) {
        self.push(AnnotationEdit::Create(annotation), words.edit_create.as_str());
    }

    /// Ends the pending Ink or freehand highlight strokes as one annotation.
    fn flush_ink(&mut self, words: &AnnotateWords) {
        if self.strokes.is_empty() {
            return;
        }
        let strokes = std::mem::take(&mut self.strokes);
        let mut annotation = self.fresh(self.ink_kind(), bounds(strokes.iter().flatten().copied()));
        annotation.strokes = strokes;
        self.create(annotation, words);
    }

    /// Ends the pending PolyLine, Polygon or Cloud once it has enough points.
    fn finish_poly(&mut self, kind: AnnotationKind, words: &AnnotateWords) {
        let points = std::mem::take(&mut self.poly);
        let least = if kind == AnnotationKind::PolyLine { 2 } else { 3 };
        if points.len() >= least {
            let mut annotation = self.fresh(kind, bounds(points.iter().copied()));
            annotation.points = points;
            self.create(annotation, words);
        }
    }

    fn is_poly(&self) -> bool {
        matches!(self.tool, AnnotationTool::Draw(AnnotationKind::PolyLine | AnnotationKind::Polygon | AnnotationKind::Cloud))
    }

    fn press_select(&mut self, pointer: &ToolPointer, reading: &Reading) -> ToolAction {
        let Some(id) = reading.gesture else { return ToolAction::Ignored };
        match hit(&self.annotations, self.primary(), reading.at, reading.tolerance, reading.icon_side) {
            Some(AnnotationHit::Handle(target, handle)) => {
                if !self.grab_handle(id, target, handle, reading.at, pointer.mods) {
                    return ToolAction::Ignored;
                }
            }
            Some(AnnotationHit::Body(target)) => {
                let markup = self.annotations.iter().any(|a| a.id == target && a.kind.is_markup());
                if markup && self.select_options.markup_press == MarkupPress::ClickOnly {
                    // The host's text selection takes the drag; the click reaches the tool through `click`.
                    return ToolAction::Ignored;
                }
                if self.select_part(target, pointer.mods, pointer.clicks, &reading.words) {
                    return ToolAction::Done;
                }
                let ids = self.editable();
                if !ids.is_empty() {
                    let box_move = (ids.len() == 1)
                        .then(|| self.annotations.iter().find(|a| a.id == target && a.kind == AnnotationKind::Callout && a.rect.contains(reading.at)))
                        .flatten()
                        .map(|a| Change::MoveBox { target, from: a.rect });
                    let change = box_move.unwrap_or(Change::Move { ids });
                    self.gesture = Some(Drag::Edit { id, begun: false, origin: reading.at, change, mods: pointer.mods });
                }
            }
            None => match self.select_options.empty_press {
                EmptyPress::Marquee => {
                    self.gesture = Some(Drag::Marquee { origin: reading.at, current: reading.at, additive: self.toggles(pointer.mods) });
                }
                EmptyPress::PassThrough => return ToolAction::Ignored,
            },
        }
        ToolAction::Claimed
    }

    fn press_draw(&mut self, kind: AnnotationKind, pointer: &ToolPointer, reading: &Reading) -> ToolAction {
        use AnnotationKind as K;
        let at = reading.at;
        if let Some((target, handle)) = self.handle_while_drawing(kind, at, reading.tolerance, reading.icon_side)
            && let Some(id) = reading.gesture
            && self.grab_handle(id, target, handle, at, pointer.mods)
        {
            return ToolAction::Claimed;
        }
        // A placement off the page places nothing; the annotations under it do not matter.
        let on_page = reading.page.is_none_or(|page| page.contains(at));
        match kind {
            K::Note | K::Attach | K::Caret => {
                if !on_page {
                    return ToolAction::Ignored;
                }
                let snapped = if kind == K::Caret { self.caret_snap.as_ref().and_then(|snap| snap(at)) } else { None };
                let (at, size) = snapped.map_or((at, None), |(tip, side)| (tip, Some(Vec2::new(side, side))));
                self.place(kind, at, size, None, &reading.words);
                ToolAction::Done
            }
            K::TextBox | K::Callout => {
                if !on_page {
                    return ToolAction::Ignored;
                }
                self.gesture = Some(Drag::Place { kind, origin: at, current: at });
                ToolAction::Claimed
            }
            K::PolyLine | K::Polygon | K::Cloud => {
                let closes =
                    kind != K::PolyLine && self.poly.len() >= 3 && self.poly.first().is_some_and(|&first| (first - at).hypot() <= reading.tolerance);
                if pointer.clicks >= 2 || closes {
                    // The double click's first press already added its point; a click on the first point closes the
                    // shape without adding one.
                    self.finish_poly(kind, &reading.words);
                } else {
                    let next =
                        if pointer.mods.shift { self.poly.last().map_or(at, |&last| self.select_options.shift_lock.apply(last, at)) } else { at };
                    let repeat = reading.view.doc_len(REPEAT_POINT);
                    if !self.poly.last().is_some_and(|&last| (last - next).hypot() < repeat) {
                        self.poly.push(next);
                    }
                }
                ToolAction::Done
            }
            K::Ink | K::FreehandHighlight => {
                self.gesture = Some(Drag::Stroke(vec![at]));
                ToolAction::Claimed
            }
            kind if kind.is_markup() => {
                self.gesture = Some(Drag::Markup { kind, origin: at, current: at });
                ToolAction::Claimed
            }
            kind => {
                self.gesture = Some(Drag::Shape { kind, origin: at, current: at });
                ToolAction::Claimed
            }
        }
    }

    fn shape_annotation(&self, kind: AnnotationKind, origin: Point, current: Point, shift: bool) -> Option<Annotation> {
        use AnnotationKind as K;
        match kind {
            K::Line | K::Arrow => {
                let end = if shift { self.select_options.shift_lock.apply(origin, current) } else { current };
                if (end - origin).hypot() < MIN_SHAPE {
                    return None;
                }
                let mut annotation = self.fresh(kind, bounds([origin, end]));
                annotation.points = vec![origin, end];
                Some(annotation)
            }
            _ => {
                let end = if shift { squared(origin, current) } else { current };
                let rect = Rect::from_points(origin, end);
                (rect.width() >= MIN_SHAPE && rect.height() >= MIN_SHAPE).then(|| self.fresh(kind, rect))
            }
        }
    }

    fn markup_annotation(&self, kind: AnnotationKind, origin: Point, current: Point) -> Option<Annotation> {
        let quads = (self.text_quads.as_ref()?)(origin, current);
        if quads.is_empty() {
            return None;
        }
        let points: Vec<Point> = quads.iter().flatten().copied().collect();
        let mut annotation = self.fresh(kind, bounds(points.iter().copied()));
        annotation.points = points;
        Some(annotation)
    }

    fn editable(&self) -> Vec<AnnotationId> {
        self.selected.iter().copied().filter(|id| self.annotations.iter().any(|a| a.id == *id && !a.locked)).collect()
    }
}

/// The value a move, resize or reshape gesture reports with the pointer at `at`; `lock` is the Shift rule of a point
/// drag ([`SelectOptions::shift_lock`]).
fn edit_value(change: &Change, origin: Point, at: Point, mods: Mods, lock: ShiftLock) -> AnnotationEdit {
    match change {
        Change::Move { ids } => AnnotationEdit::Move { ids: ids.clone(), delta: at - origin },
        Change::MoveBox { target, from } => AnnotationEdit::Resize { id: *target, rect: *from + (at - origin) },
        Change::Resize { target, handle, from } => {
            let aspect = corner_aspect(from, from.rect, mods.shift);
            AnnotationEdit::Resize { id: *target, rect: resized(from.rect, *handle, at - origin, aspect) }
        }
        Change::Reshape { target, index, from, callout } => {
            let lock = mods.shift.then_some(lock);
            let points = match callout {
                Some(rect) => callout_reshaped(*rect, from, *index, at, lock),
                None => reshaped(from, *index, at, lock),
            };
            AnnotationEdit::Reshape { id: *target, points }
        }
    }
}

/// Whether a released move or resize came to rest within its slip of where it began: a move of at most
/// [`SLIP_MOVE`], a resize whose corners each moved at most [`SLIP_RESIZE`]. A reshape always counts.
fn slipped(change: &Change, value: &AnnotationEdit) -> bool {
    match (change, value) {
        (Change::Move { .. }, AnnotationEdit::Move { delta, .. }) => delta.hypot() <= SLIP_MOVE,
        (Change::MoveBox { from, .. }, AnnotationEdit::Resize { rect, .. }) => (rect.origin() - from.origin()).hypot() <= SLIP_MOVE,
        (Change::Resize { from, .. }, AnnotationEdit::Resize { rect, .. }) => {
            corners(from.rect.abs()).into_iter().zip(corners(rect.abs())).all(|(was, now)| (now - was).hypot() <= SLIP_RESIZE)
        }
        _ => false,
    }
}

fn edit_label<'a>(change: &Change, words: &'a AnnotateWords) -> &'a str {
    match change {
        Change::Move { .. } | Change::MoveBox { .. } => words.edit_move.as_str(),
        Change::Resize { .. } => words.edit_resize.as_str(),
        Change::Reshape { .. } => words.edit_reshape.as_str(),
    }
}

impl CanvasTool for AnnotateTool {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        if self.tool != self.last_tool {
            self.flush_ink(&cx.words::<AnnotateWords>());
            self.poly.clear();
            self.last_tool = self.tool;
        }
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        match cx.pointer.as_ref().and_then(|pointer| read(cx, pointer.pos)) {
            Some(reading) => self.cursor_at(reading.at, reading.tolerance, reading.icon_side),
            None if self.tool == AnnotationTool::Select => Cursor::System(CursorIcon::Default),
            None => Cursor::System(CursorIcon::Crosshair),
        }
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.gesture = None;
        let Some(reading) = read(cx, pointer.pos) else { return ToolAction::Ignored };
        match self.tool {
            AnnotationTool::Select => self.press_select(pointer, &reading),
            AnnotationTool::Draw(kind) => self.press_draw(kind, pointer, &reading),
            AnnotationTool::Eraser => {
                self.gesture = Some(Drag::Erase { path: vec![reading.at], crossed: Vec::new() });
                ToolAction::Claimed
            }
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(reading) = read(cx, pointer.pos) else { return };
        let lock = self.select_options.shift_lock;
        let by_strokes = self.draw_options.erase == EraseMode::Strokes;
        let Some(gesture) = self.gesture.as_mut() else { return };
        let mut events = Vec::new();
        match gesture {
            Drag::Edit { id, begun, origin, change, mods } => {
                *mods = pointer.mods;
                if !*begun {
                    *begun = true;
                    events.push(EditEvent::begin(*id, Mixed::Mixed, Arc::from(edit_label(change, &reading.words)), None));
                }
                events.push(EditEvent::update(*id, edit_value(change, *origin, reading.at, *mods, lock)));
            }
            Drag::Marquee { current, .. } | Drag::Shape { current, .. } | Drag::Markup { current, .. } => *current = reading.at,
            Drag::Place { current, .. } => *current = clamp_to(reading.page, reading.at),
            Drag::Stroke(points) => points.push(reading.at),
            Drag::Erase { path, .. } if !by_strokes => path.push(reading.at),
            Drag::Erase { path, crossed } => {
                let last = path.last().copied().unwrap_or(reading.at);
                for a in self.annotations.iter().filter(|a| erasable(a)) {
                    for (i, stroke) in a.strokes.iter().enumerate() {
                        let reach = reading.tolerance + a.stroke_width / 2.0;
                        let hits = stroke.windows(2).any(|w| segments_cross(last, reading.at, w[0], w[1], reach))
                            || stroke.len() == 1 && (stroke[0] - reading.at).hypot() <= reading.tolerance;
                        if hits && !crossed.contains(&(a.id, i)) {
                            crossed.push((a.id, i));
                        }
                    }
                }
                path.push(reading.at);
            }
        }
        self.hover = Some(reading.at);
        self.edits.extend(events);
        cx.request_repaint();
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(gesture) = self.gesture.take() else { return };
        let Some(reading) = read(cx, pointer.pos) else { return };
        let words = Arc::clone(&reading.words);
        let mods = pointer.mods;
        match gesture {
            Drag::Edit { id, begun, origin, change, .. } => {
                // A press that never moved (a click) began no gesture and reports nothing. One that moved and came to
                // rest within its slip ends as a cancel: the annotation stays where it was.
                if begun {
                    let value = edit_value(&change, origin, reading.at, mods, self.select_options.shift_lock);
                    if slipped(&change, &value) {
                        self.edits.push(EditEvent::cancel(id, CancelReason::Unchanged, None));
                    } else {
                        self.edits.push(EditEvent::commit(id, Mixed::Mixed, value));
                    }
                }
            }
            Drag::Place { kind, origin, .. } => {
                // The release point rather than the last drag sample, so a click (no drag) places at the press.
                let current = clamp_to(reading.page, reading.at);
                self.release_place(kind, origin, current, &reading);
            }
            Drag::Marquee { origin, additive, .. } => {
                let area = Rect::from_points(origin, reading.at);
                let inside: Vec<AnnotationId> = if area.width() == 0.0 && area.height() == 0.0 {
                    Vec::new()
                } else {
                    self.annotations
                        .iter()
                        .filter(|a| {
                            let extent = a.extent(reading.icon_side);
                            !area.intersect(extent).is_zero_area() || area.contains(extent.center())
                        })
                        .map(|a| a.id)
                        .collect()
                };
                let mut ids = if additive { self.selected.clone() } else { Vec::new() };
                for id in inside {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                self.replace_selection(ids, &words);
            }
            Drag::Shape { kind, origin, .. } => {
                if let Some(annotation) = self.shape_annotation(kind, origin, reading.at, mods.shift) {
                    self.create(annotation, &words);
                }
            }
            Drag::Markup { kind, origin, .. } => {
                if let Some(annotation) = self.markup_annotation(kind, origin, reading.at) {
                    self.create(annotation, &words);
                }
            }
            Drag::Stroke(points) => match self.draw_options.ink {
                InkStrokes::Session => self.strokes.push(points),
                InkStrokes::PerStroke => {
                    // A click leaves a one-point stroke: a dot, which MIN_SHAPE does not drop.
                    let mut annotation = self.fresh(self.ink_kind(), bounds(points.iter().copied()));
                    annotation.strokes = vec![points];
                    self.create(annotation, &words);
                }
            },
            Drag::Erase { path, .. } if self.draw_options.erase == EraseMode::Path => {
                let radius = reading.view.doc_len(self.draw_options.eraser_radius);
                let reach = bounds(path.iter().copied()).inflate(radius, radius);
                let ids: Vec<AnnotationId> = self.annotations.iter().filter(|a| erasable(a) && overlaps(a.rect, reach)).map(|a| a.id).collect();
                if !ids.is_empty() {
                    self.push(AnnotationEdit::EraseAlong { ids, path, radius }, words.edit_erase.as_str());
                }
            }
            Drag::Erase { crossed, .. } => {
                // One event per annotation, in the order the Eraser met them: an app that removes
                // strokes by index must not see the same annotation twice.
                let mut ids: Vec<AnnotationId> = Vec::new();
                for (id, _) in &crossed {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
                for id in ids {
                    let mut strokes: Vec<usize> = crossed.iter().filter(|(a, _)| *a == id).map(|(_, i)| *i).collect();
                    strokes.sort_unstable();
                    self.push(AnnotationEdit::Erase { id, strokes }, words.edit_erase.as_str());
                }
            }
        }
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.hover = cx.space().map(|space| space.view.to_doc(pointer.pos));
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if let Some(Drag::Edit { id, begun: true, .. }) = self.gesture.take() {
            self.edits.push(EditEvent::cancel(id, reason, None));
        }
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let words = cx.words::<AnnotateWords>();
        match key {
            ToolKey::Cancel if !self.poly.is_empty() || !self.strokes.is_empty() => {
                self.poly.clear();
                self.strokes.clear();
                true
            }
            ToolKey::Submit if self.is_poly() && !self.poly.is_empty() => {
                if let AnnotationTool::Draw(kind) = self.tool {
                    self.finish_poly(kind, &words);
                }
                true
            }
            ToolKey::Submit if !self.strokes.is_empty() => {
                self.flush_ink(&words);
                true
            }
            // While a shape is being clicked out, Backspace and Delete take back its last point, not the selection.
            ToolKey::Delete { .. } if self.is_poly() && !self.poly.is_empty() => self.undo_point(),
            ToolKey::Delete { .. } if self.tool == AnnotationTool::Select => {
                let ids = self.editable();
                if ids.is_empty() {
                    return false;
                }
                self.push(AnnotationEdit::Delete(ids), words.edit_delete.as_str());
                true
            }
            ToolKey::Nudge { direction, mods, .. } if self.tool == AnnotationTool::Select => {
                let ids = self.editable();
                let Some(space) = cx.space() else { return false };
                // Shift, and Command under `FarNudge::ShiftOrCommand`, take the far step; any other modifier (Alt)
                // leaves the arrow to the app.
                let command = self.select_options.far_nudge == FarNudge::ShiftOrCommand;
                let others = Mods { shift: false, cmd: mods.cmd && !command, ..*mods };
                if ids.is_empty() || !others.is_none() {
                    return false;
                }
                let step = if mods.shift || (command && mods.cmd) { NUDGE_FAR } else { NUDGE };
                let along = space.view.unmap_vec(*direction);
                if along.hypot() == 0.0 {
                    return false;
                }
                self.push(AnnotationEdit::Move { ids, delta: along / along.hypot() * step }, words.edit_move.as_str());
                true
            }
            _ => false,
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.is_poly() && !self.poly.is_empty() {
            ToolClaims::NONE.deletes()
        } else if self.tool == AnnotationTool::Select && !self.editable().is_empty() {
            ToolClaims::NONE.deletes().nudges()
        } else {
            ToolClaims::NONE
        }
    }

    /// In Select, the annotations the arrows move: their kinds and the bounds round them.
    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        if self.tool != AnnotationTool::Select {
            return None;
        }
        let ids = self.editable();
        let moved: Vec<&Annotation> = self.annotations.iter().filter(|a| ids.contains(&a.id)).collect();
        let bounds = moved.iter().map(|a| a.rect).reduce(|bounds, rect| bounds.union(rect))?;
        let words = cx.words::<AnnotateWords>();
        let names: Vec<&str> = moved.iter().map(|a| words.kind(a.kind)).collect();
        Some(ToolAccess::new(names.join(", ")).bounds(bounds))
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        let view = space.view;
        let screen = |points: &[Point]| -> Vec<Pos2> { points.iter().map(|&p| view.to_screen(p)).collect() };
        let outline = |points: Vec<Pos2>, closed: bool, color: Color32| {
            let mut path = points;
            if closed && let Some(&first) = path.first() {
                path.push(first);
            }
            draw.stroke(&PaintShape::Path(path), StrokeWidth::Heavy, DataColor::new(color));
        };
        let drawn = |kind: AnnotationKind| color32(&self.fresh(kind, Rect::ZERO));
        let shift = cx.mods.shift;
        match &self.gesture {
            Some(Drag::Marquee { origin, current, .. }) => {
                cx.marching_ants(&screen(&corners(Rect::from_points(*origin, *current))), true);
            }
            Some(Drag::Shape { kind, origin, current }) => {
                let color = drawn(*kind);
                match kind {
                    AnnotationKind::Line | AnnotationKind::Arrow => {
                        let end = if shift { self.select_options.shift_lock.apply(*origin, *current) } else { *current };
                        outline(screen(&[*origin, end]), false, color);
                    }
                    AnnotationKind::Oval => {
                        let end = if shift { squared(*origin, *current) } else { *current };
                        outline(screen(&ellipse_points(Rect::from_points(*origin, end))), true, color);
                    }
                    _ => {
                        let end = if shift { squared(*origin, *current) } else { *current };
                        outline(screen(&corners(Rect::from_points(*origin, end))), true, color);
                    }
                }
            }
            Some(Drag::Place { kind: AnnotationKind::Callout, origin, current }) => {
                outline(screen(&[*origin, *current]), false, drawn(AnnotationKind::Callout));
            }
            Some(Drag::Place { kind, origin, current }) => outline(screen(&corners(Rect::from_points(*origin, *current))), true, drawn(*kind)),
            Some(Drag::Markup { kind, origin, current }) => {
                if let Some(quads) = self.text_quads.as_ref() {
                    for quad in quads(*origin, *current) {
                        outline(screen(&quad), true, drawn(*kind));
                    }
                }
            }
            Some(Drag::Stroke(points)) => outline(screen(points), false, drawn(self.ink_kind())),
            Some(Drag::Erase { path, .. }) if self.draw_options.erase == EraseMode::Path => {
                // The swath the eraser rubs, as wide on screen as its reach at any zoom: a flat grey band.
                draw.stroke(
                    &PaintShape::Path(screen(path)),
                    DataWidth::points(2.0 * self.draw_options.eraser_radius),
                    DataColor::new(Color32::from_rgba_unmultiplied(0x80, 0x80, 0x80, 0xA0)),
                );
            }
            Some(Drag::Erase { path, .. }) => cx.marching_ants(&screen(path), false),
            Some(Drag::Edit { .. }) | None => {}
        }
        for stroke in &self.strokes {
            outline(screen(stroke), false, drawn(self.ink_kind()));
        }
        if let AnnotationTool::Draw(kind) = self.tool
            && self.is_poly()
            && !self.poly.is_empty()
        {
            let mut path = self.poly.clone();
            if let Some(hover) = self.hover {
                let next = if let Some(&first) = self.poly.first()
                    && kind != AnnotationKind::PolyLine
                    && self.poly.len() >= 3
                    && (first - hover).hypot() <= view.doc_len(cx.slop)
                {
                    first
                } else if shift {
                    self.poly.last().map_or(hover, |&last| self.select_options.shift_lock.apply(last, hover))
                } else {
                    hover
                };
                path.push(next);
            }
            outline(screen(&path), false, drawn(kind));
        }
    }

    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    fn command_form(&self) -> ToolPath {
        match self.tool {
            AnnotationTool::Select => {
                ToolPath::Command("the arrow keys nudge and Delete removes the selection; the New Annotation... command places shapes")
            }
            AnnotationTool::Eraser => ToolPath::Freehand("none; Delete removes the selected annotation"),
            AnnotationTool::Draw(AnnotationKind::Ink | AnnotationKind::FreehandHighlight) => {
                ToolPath::Freehand("none; a pen stroke has no numeric form")
            }
            AnnotationTool::Draw(kind) if kind.is_markup() => ToolPath::Freehand("none; a markup stroke follows the pointer over the text"),
            AnnotationTool::Draw(_) => ToolPath::Command("the New Annotation... command"),
        }
    }
}

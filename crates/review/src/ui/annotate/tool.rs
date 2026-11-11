//! The canvas tool: presses, drags, the "New Annotation..." form and the [`CanvasTool`] implementation.

use craft_behavior::geometry::draft::turn_to_45;
use craft_behavior::{CancelReason, Cursor, CursorIcon, EditEvent, GestureId, Mixed, PointerButton, SelectionChange};
use craft_keymap::Mods;
use craft_theme::Srgba;
use craft_ui::canvas::{CanvasTool, NUDGE, NUDGE_FAR, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::DataColor;
use egui::{Color32, Pos2};
use kurbo::{Point, Rect};
use std::sync::Arc;

use super::AnnotateWords;
use super::overlay::ellipse_points;
use crate::annotate::{
    Annotation, AnnotationEdit, AnnotationHandle, AnnotationHit, AnnotationId, AnnotationKind, AnnotationTool, hit, reshaped, resized,
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

/// What a press began and the drag or release continues.
#[derive(Clone, Debug)]
enum Drag {
    /// Moving, resizing or reshaping selected annotations; `begun` once the pointer has moved.
    Edit { id: GestureId, begun: bool, origin: Point, change: Change },
    /// A selection rectangle on empty page.
    Marquee { origin: Point, current: Point, additive: bool },
    /// A box, oval or straight line drawn by dragging.
    Shape { kind: AnnotationKind, origin: Point, current: Point },
    /// Page text selected for a markup kind.
    Markup { kind: AnnotationKind, origin: Point, current: Point },
    /// One Ink stroke in progress.
    Stroke(Vec<Point>),
    /// The Eraser's path and the strokes it crossed.
    Erase { path: Vec<Point>, crossed: Vec<(AnnotationId, usize)> },
}

#[derive(Clone, Debug)]
enum Change {
    Move { ids: Vec<AnnotationId> },
    Resize { target: AnnotationId, handle: (i8, i8), from: Rect },
    Reshape { target: AnnotationId, index: usize, from: Vec<Point> },
}

/// What a call knows when it reads a pointer: the pointer in page units, the page-unit sizes of the screen
/// distances the tool tests with, the host's gesture id and the tool's words.
struct Reading {
    at: Point,
    tolerance: f64,
    icon_side: f64,
    gesture: Option<GestureId>,
    words: Arc<AnnotateWords>,
}

fn read(cx: &ToolContext<'_>, pos: Pos2) -> Option<Reading> {
    let space = cx.space()?;
    Some(Reading {
        at: space.view.to_doc(pos),
        tolerance: space.view.doc_len(cx.slop),
        icon_side: space.view.doc_len(cx.icon_side()),
        gesture: cx.gesture_id(),
        words: cx.words::<AnnotateWords>(),
    })
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
///   drag on a handle resizes (box kinds; Shift keeps the aspect on a corner) or reshapes
///   (pointwise kinds; Shift snaps to 45 degrees); a press on empty page starts a marquee. Moves,
///   resizes and reshapes are `EditEvent<AnnotationEdit>` gestures; Esc cancels and the release
///   after it is ignored.
/// - A drawing tool: a drag draws the kind (Shift: squares, circles, 45 degree lines); PolyLine and
///   Polygon add a point per click and end with a double click or Enter; Ink records strokes until
///   the tool changes; Note and Text box place at the click and report `Create`, the app opens
///   the composer. Markup kinds select page text through the app's text hit test
///   ([`text_quads`](Self::text_quads)) and report the quads.
/// - Eraser: a drag reports one `Erase` per Ink annotation it crosses, with the strokes in
///   ascending order.
/// - Keys: Delete and Backspace remove the selection, arrows nudge it (one point, Shift ten) as `Move`s.
/// - A double click on a note or text box reports `Open`; the app's page text and selection reach the tool
///   in [`CanvasTool::each_pass`] through the public fields.
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

    fn fresh(&self, kind: AnnotationKind, rect: Rect) -> Annotation {
        let markup = kind == AnnotationKind::Highlight;
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
        }
    }

    fn create(&mut self, annotation: Annotation, words: &AnnotateWords) {
        self.push(AnnotationEdit::Create(annotation), words.edit_create.as_str());
    }

    /// Ends the pending Ink strokes as one annotation.
    fn flush_ink(&mut self, words: &AnnotateWords) {
        if self.strokes.is_empty() {
            return;
        }
        let strokes = std::mem::take(&mut self.strokes);
        let mut annotation = self.fresh(AnnotationKind::Ink, bounds(strokes.iter().flatten().copied()));
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
        let shift = pointer.mods.shift;
        let found = hit(&self.annotations, &self.selected, reading.at, reading.tolerance, reading.icon_side);
        let locked = |id: AnnotationId, list: &[Annotation]| list.iter().any(|a| a.id == id && a.locked);
        let Some(id) = reading.gesture else { return ToolAction::Ignored };
        let edit = |change: Change| Drag::Edit { id, begun: false, origin: reading.at, change };
        match found {
            Some(AnnotationHit::Handle(target, AnnotationHandle::Box(hx, hy))) => {
                let from = self.annotations.iter().find(|a| a.id == target).map_or(Rect::ZERO, |a| a.rect);
                self.gesture = Some(edit(Change::Resize { target, handle: (hx, hy), from }));
            }
            Some(AnnotationHit::Handle(target, AnnotationHandle::Point(index))) => {
                let from = self.annotations.iter().find(|a| a.id == target).map(|a| a.points.clone()).unwrap_or_default();
                self.gesture = Some(edit(Change::Reshape { target, index, from }));
            }
            Some(AnnotationHit::Body(target)) => {
                let selected = self.selected.contains(&target);
                if pointer.clicks >= 2 {
                    // A second click on a note or text box asks the app to open its text.
                    let opens = self.annotations.iter().any(|a| a.id == target && matches!(a.kind, AnnotationKind::Note | AnnotationKind::TextBox));
                    if opens {
                        if !selected {
                            self.select(SelectionChange::Set(vec![target]), &reading.words);
                        }
                        self.push(AnnotationEdit::Open(target), reading.words.edit_open.as_str());
                        return ToolAction::Done;
                    }
                }
                if shift {
                    self.select(SelectionChange::Toggle(target), &reading.words);
                } else if !selected {
                    self.select(SelectionChange::Set(vec![target]), &reading.words);
                }
                let ids: Vec<AnnotationId> = self.selected.iter().copied().filter(|s| !locked(*s, &self.annotations)).collect();
                if !ids.is_empty() {
                    self.gesture = Some(edit(Change::Move { ids }));
                }
            }
            None => {
                self.gesture = Some(Drag::Marquee { origin: reading.at, current: reading.at, additive: shift });
            }
        }
        ToolAction::Claimed
    }

    fn press_draw(&mut self, kind: AnnotationKind, pointer: &ToolPointer, reading: &Reading) -> ToolAction {
        use AnnotationKind as K;
        let at = reading.at;
        match kind {
            K::Note | K::Attach | K::Caret => {
                let mut annotation = self.fresh(kind, Rect::from_origin_size(at, (reading.icon_side, reading.icon_side)));
                annotation.points = vec![at];
                self.create(annotation, &reading.words);
                ToolAction::Done
            }
            K::PolyLine | K::Polygon | K::Cloud => {
                if pointer.clicks >= 2 {
                    // The double click's first press already added its point.
                    self.finish_poly(kind, &reading.words);
                } else {
                    self.poly.push(at);
                }
                ToolAction::Done
            }
            K::Ink => {
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
                let end = if shift { turn_to_45(origin, current) } else { current };
                if end == origin {
                    return None;
                }
                let mut annotation = self.fresh(kind, bounds([origin, end]));
                annotation.points = vec![origin, end];
                Some(annotation)
            }
            _ => {
                let end = if shift { squared(origin, current) } else { current };
                let rect = Rect::from_points(origin, end);
                (rect.width() > 0.0 && rect.height() > 0.0).then(|| self.fresh(kind, rect))
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

/// The value a move, resize or reshape gesture reports with the pointer at `at`.
fn edit_value(change: &Change, origin: Point, at: Point, mods: Mods) -> AnnotationEdit {
    match change {
        Change::Move { ids } => AnnotationEdit::Move { ids: ids.clone(), delta: at - origin },
        Change::Resize { target, handle, from } => {
            let aspect = (mods.shift && from.height() > 0.0).then(|| from.width() / from.height());
            AnnotationEdit::Resize { id: *target, rect: resized(*from, *handle, at - origin, aspect) }
        }
        Change::Reshape { target, index, from } => AnnotationEdit::Reshape { id: *target, points: reshaped(from, *index, at, mods.shift) },
    }
}

fn edit_label<'a>(change: &Change, words: &'a AnnotateWords) -> &'a str {
    match change {
        Change::Move { .. } => words.edit_move.as_str(),
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

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        match self.tool {
            AnnotationTool::Select => Cursor::System(CursorIcon::Default),
            AnnotationTool::Draw(_) | AnnotationTool::Eraser => Cursor::System(CursorIcon::Crosshair),
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
        let Some(gesture) = self.gesture.as_mut() else { return };
        let mut events = Vec::new();
        match gesture {
            Drag::Edit { id, begun, origin, change } => {
                if !*begun {
                    *begun = true;
                    events.push(EditEvent::begin(*id, Mixed::Mixed, Arc::from(edit_label(change, &reading.words)), None));
                }
                events.push(EditEvent::update(*id, edit_value(change, *origin, reading.at, pointer.mods)));
            }
            Drag::Marquee { current, .. } | Drag::Shape { current, .. } | Drag::Markup { current, .. } => *current = reading.at,
            Drag::Stroke(points) => points.push(reading.at),
            Drag::Erase { path, crossed } => {
                let last = path.last().copied().unwrap_or(reading.at);
                for a in self.annotations.iter().filter(|a| a.kind == AnnotationKind::Ink && !a.locked) {
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
            Drag::Edit { id, begun, origin, change } => {
                if begun {
                    self.edits.push(EditEvent::commit(id, Mixed::Mixed, edit_value(&change, origin, reading.at, mods)));
                }
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
            Drag::Stroke(points) => self.strokes.push(points),
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
                if ids.is_empty() || !(mods.is_none() || *mods == Mods::SHIFT) {
                    return false;
                }
                let step = if mods.shift { NUDGE_FAR } else { NUDGE };
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
        if self.tool == AnnotationTool::Select && !self.editable().is_empty() { ToolClaims::NONE.deletes() } else { ToolClaims::NONE }
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
            draw.stroke(&PaintShape::Path(path), StrokeWidth::Medium, DataColor::new(color));
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
                        let end = if shift { turn_to_45(*origin, *current) } else { *current };
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
            Some(Drag::Markup { kind, origin, current }) => {
                if let Some(quads) = self.text_quads.as_ref() {
                    for quad in quads(*origin, *current) {
                        outline(screen(&quad), true, drawn(*kind));
                    }
                }
            }
            Some(Drag::Stroke(points)) => outline(screen(points), false, drawn(AnnotationKind::Ink)),
            Some(Drag::Erase { path, .. }) => cx.marching_ants(&screen(path), false),
            Some(Drag::Edit { .. }) | None => {}
        }
        for stroke in &self.strokes {
            outline(screen(stroke), false, drawn(AnnotationKind::Ink));
        }
        if let AnnotationTool::Draw(kind) = self.tool
            && self.is_poly()
            && !self.poly.is_empty()
        {
            let mut path = self.poly.clone();
            path.extend(self.hover);
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
            AnnotationTool::Draw(AnnotationKind::Ink) => ToolPath::Freehand("none; a pen stroke has no numeric form"),
            AnnotationTool::Draw(kind) if kind.is_markup() => ToolPath::Freehand("none; a markup stroke follows the pointer over the text"),
            AnnotationTool::Draw(_) => ToolPath::Command("the New Annotation... command"),
        }
    }
}

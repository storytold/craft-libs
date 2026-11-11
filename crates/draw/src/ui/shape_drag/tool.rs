//! [`ShapeDragTool`]: the Shape tools' press, drag and release on the tool host.

use std::sync::Arc;

use craft_behavior::geometry::draft::turn_to_45;
use craft_behavior::{CancelReason, Cursor, CursorIcon, EditEvent, GestureId, Mixed};
use craft_keymap::{Key, Mods};
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::canvas_controls::path_runs;
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use kurbo::Point;

use super::words::ShapeDragWords;
use super::{ShapeDragEvent, ShapeDragOptions, ShapeDragRules, ShapeFrame, ShapeKind};
use crate::geometry::shape::{RadialFrame, SIDES, ShapeGeometry, box_drag, line_drag};

/// The flattening tolerance of the outline being drawn, in screen points.
const OUTLINE_TOLERANCE: f32 = 0.25;

/// A Shape tool on the canvas: drag to draw, click to ask for a size. The app owns one value for
/// all Shape tools, sets [`options`](Self::options) (the kind from the selected tool) each pass,
/// shows it on the canvas's `ToolHost` and drains [`take_events`](Self::take_events) after. The tool
/// never changes the document.
///
/// Pointer (Box frame, Photoshop's rules):
/// - The press starts at the snapped point; past the press slop the drag begins and each pass reports
///   the shape it would draw; release commits it.
/// - A side under 1 px makes nothing: the gesture is cancelled as `Dismissed`.
/// - A press and release inside the slop is a [`Click`](ShapeDragEvent::Click) at the press.
/// - ⇧ makes a square or circle (a Line: 45° steps keeping its length); ⌥ draws from the centre.
///   Both may go down or up mid-drag; the shape follows the modifiers held now.
/// - Space held moves the shape being drawn without resizing it: the press point moves with the
///   pointer while Space is down, and the box snaps as a whole.
/// - Esc cancels (the host calls [`cancel`](CanvasTool::cancel)); so does a tool switch: `Cancel`
///   goes out when a drag had begun.
///
/// Radial frame ([`ShapeDragRules::ILLUSTRATOR`] for Polygon and Star): the press is the centre;
/// ↑ and ↓ change the sides or points (3 to 100) while the button is down; Ctrl/Cmd holds a
/// star's inner radius; ⌥ straightens a star's shoulders (five points or more). ⇧ keeps the base
/// orientation.
///
/// Keyboard: with the canvas focused, Enter reports a `Click` at the view centre (the keyboard
/// route to Create Shape). The tool has no parts and no Tab stop of its own.
///
/// Paint: the outline of the shape being drawn over a halo; none when [`preview`](Self::preview) is
/// off (the app previews the shape in the document from `Update` events).
#[derive(Debug)]
pub struct ShapeDragTool {
    options: ShapeDragOptions,
    rules: ShapeDragRules,
    preview: bool,
    events: Vec<ShapeDragEvent>,
    drag: Option<Drag>,
    /// The id the next gesture takes.
    next: GestureId,
}

impl Default for ShapeDragTool {
    fn default() -> Self {
        ShapeDragTool {
            options: ShapeDragOptions::default(),
            rules: ShapeDragRules::default(),
            preview: true,
            events: Vec::new(),
            drag: None,
            next: GestureId::default(),
        }
    }
}

/// A drag between passes.
#[derive(Clone, Debug)]
struct Drag {
    id: GestureId,
    label: Arc<str>,
    /// The options of the press: a drag keeps its kind and values.
    options: ShapeDragOptions,
    /// The press on screen, for the press slop.
    press: egui::Pos2,
    /// The press as the pointer made it, snapped.
    origin: Point,
    /// The press, moved by Space.
    start: Point,
    /// The last pointer.
    at: Point,
    /// Past the press slop: `Begin` went out.
    began: bool,
    /// Whether Space was held in the last call.
    space: bool,
    /// A radial star's inner radius held by Ctrl/Cmd.
    inner: Option<f64>,
    /// Sides or points as changed by ↑ and ↓ during this drag.
    sides: u32,
    /// The shape of the last `Update`, for the commit and the paint.
    shape: Option<ShapeGeometry>,
}

impl ShapeDragTool {
    /// A tool with Photoshop's rules and default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// The kind and options of this pass. A drag in progress keeps the kind it started with.
    pub fn options(&mut self, options: ShapeDragOptions) -> &mut Self {
        self.options = options;
        self
    }

    /// Which product's rules the drag follows; Photoshop's by default.
    pub fn rules(&mut self, rules: ShapeDragRules) -> &mut Self {
        self.rules = rules;
        self
    }

    /// Whether the tool paints the outline being drawn; on by default.
    pub fn preview(&mut self, on: bool) -> &mut Self {
        self.preview = on;
        self
    }

    /// The kind this pass draws (the options bar shows its fields).
    pub fn kind(&self) -> ShapeKind {
        self.options.kind
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<ShapeDragEvent> {
        std::mem::take(&mut self.events)
    }

    /// Whether the drag draws a Polygon or Star from its centre.
    fn radial(&self, kind: ShapeKind) -> bool {
        self.rules.frame(kind) == ShapeFrame::Radial
    }

    /// The shape a drag from `start` to `at` with `mods` draws under the options and rules, or `None`
    /// when a side is under 1 px: [`box_drag`] for the box kinds (Rectangle with four equal radii when
    /// `corner_radius` > 0, Rounded Rectangle always with them, Triangle a 3-sided polygon, Polygon and
    /// Star with the options' sides and ratio, Custom a `Custom` box keeping proportions with ⇧),
    /// [`line_drag`] for a Line (weight at least 1, ⇧ turning the end to 45° steps), and
    /// [`RadialFrame::drag`] in the radial frame.
    fn shape(&self, drag: &Drag, mods: Mods) -> Option<ShapeGeometry> {
        // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:50-82 `shape_geometry`.
        // inv-vectorcraft: crates/tools/src/shape.rs:131-145 (the radial kinds).
        let o = &drag.options;
        let radii = [o.corner_radius.max(0.0); 4];
        if o.kind == ShapeKind::Line {
            let end = if mods.shift { turn_to_45(drag.start, drag.at) } else { drag.at };
            let to = line_drag(drag.start, end)?;
            return Some(ShapeGeometry::Line { from: drag.start, to, weight: o.line_weight.max(1.0) });
        }
        if self.radial(o.kind) {
            let frame = RadialFrame::drag(drag.start, drag.at, mods.shift)?;
            let star_ratio = if o.kind == ShapeKind::Star {
                match drag.inner {
                    Some(inner) => frame.ratio_holding(inner),
                    None => (self.rules.straight_shoulders && mods.alt)
                        .then(|| RadialFrame::straight_shoulders(drag.sides))
                        .flatten()
                        .unwrap_or(o.star_ratio),
                }
            } else {
                1.0
            };
            return Some(frame.geometry(drag.sides, star_ratio));
        }
        let rect = box_drag(drag.start, drag.at, mods.shift, mods.alt)?;
        Some(match o.kind {
            ShapeKind::Rectangle if o.corner_radius > 0.0 => ShapeGeometry::Rect { rect, radii },
            ShapeKind::Rectangle => ShapeGeometry::Rect { rect, radii: [0.0; 4] },
            ShapeKind::RoundedRectangle => ShapeGeometry::Rect { rect, radii },
            ShapeKind::Ellipse => ShapeGeometry::Ellipse { rect },
            ShapeKind::Triangle => ShapeGeometry::Polygon { rect, sides: 3, star_ratio: 1.0 },
            ShapeKind::Polygon => ShapeGeometry::Polygon { rect, sides: drag.sides, star_ratio: 1.0 },
            ShapeKind::Star => ShapeGeometry::Polygon { rect, sides: drag.sides, star_ratio: o.star_ratio },
            ShapeKind::Custom | ShapeKind::Line => ShapeGeometry::Custom { rect, keep_proportions: mods.shift },
        })
    }

    /// Takes the pointer at screen `pos` with `mods`: begins the drag past the press slop, moves the
    /// press with Space, and reports the shape when it changed.
    fn update(&mut self, pos: egui::Pos2, mods: Mods, cx: &mut ToolContext<'_>) {
        let Some(mut drag) = self.drag.take() else { return };
        let Some(space) = cx.space() else {
            self.drag = Some(drag);
            return;
        };
        let at = cx.snap(space.view.to_doc(pos));
        if !at.is_finite() {
            self.drag = Some(drag);
            return;
        }
        let space_held = cx.held(Key::Space);
        if space_held && drag.space {
            drag.start += at - drag.at;
        }
        drag.space = space_held;
        drag.at = at;
        if !drag.began {
            if pos.distance(drag.press) < cx.slop {
                self.drag = Some(drag);
                return;
            }
            drag.began = true;
            self.events.push(ShapeDragEvent::Draw(EditEvent::begin(drag.id, Mixed::Mixed, Arc::clone(&drag.label), None)));
        }
        // Ctrl/Cmd holds the star's inner radius as it was when the key went down.
        let holds = self.rules.hold_inner && (mods.cmd || mods.ctrl) && drag.options.kind == ShapeKind::Star && self.radial(ShapeKind::Star);
        if !holds {
            drag.inner = None;
        } else if drag.inner.is_none() {
            drag.inner = match &drag.shape {
                Some(ShapeGeometry::RadialPolygon { radius, star_ratio, .. }) => Some(radius * star_ratio),
                _ => RadialFrame::drag(drag.start, at, mods.shift).map(|f| f.radius * drag.options.star_ratio),
            };
        }
        let shape = self.shape(&drag, mods);
        if let Some(shape) = &shape {
            if drag.shape.as_ref() != Some(shape) {
                self.events.push(ShapeDragEvent::Draw(EditEvent::update(drag.id, shape.clone())));
            }
            let size = shape.bounds();
            let words = cx.words::<ShapeDragWords>();
            let units = cx.units();
            cx.readout(vec![
                (Label::from(words.width.as_str().to_owned()), units.pixels(size.width())),
                (Label::from(words.height.as_str().to_owned()), units.pixels(size.height())),
            ]);
        }
        drag.shape = shape;
        self.drag = Some(drag);
    }
}

impl CanvasTool for ShapeDragTool {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // pc: crates/ui-egui/src/canvas/gesture.rs:84 (a shape drag that stays inside
        // `shape_dialog::is_click` opens the dialog).
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let options = self.options;
        let origin = cx.snap(space.view.to_doc(pointer.pos));
        self.next = self.next.next();
        self.drag = Some(Drag {
            id: self.next,
            label: Arc::from(cx.words::<ShapeDragWords>().draw.as_str()),
            options,
            press: pointer.pos,
            origin,
            start: origin,
            at: origin,
            began: false,
            space: cx.held(Key::Space),
            inner: None,
            sides: options.sides.clamp(*SIDES.start(), *SIDES.end()),
            shape: None,
        });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:85-96 (Space moves the box, which snaps).
        self.update(pointer.pos, pointer.mods, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:99-112 `finish_shape`;
        // crates/ui-egui/src/dialogs/shape_dialog.rs:50-53 `is_click`.
        self.update(pointer.pos, pointer.mods, cx);
        let Some(drag) = self.drag.take() else { return };
        let event = if !drag.began {
            ShapeDragEvent::Click { at: drag.origin, kind: drag.options.kind, mods: pointer.mods }
        } else if let Some(shape) = drag.shape {
            ShapeDragEvent::Draw(EditEvent::commit(drag.id, Mixed::Mixed, shape))
        } else {
            ShapeDragEvent::Draw(EditEvent::cancel(drag.id, CancelReason::Dismissed, None))
        };
        self.events.push(event);
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if let Some(drag) = self.drag.take()
            && drag.began
        {
            self.events.push(ShapeDragEvent::Draw(EditEvent::cancel(drag.id, reason, None)));
        }
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        // inv-vectorcraft: crates/tools/src/shape.rs:5-7 (↑/↓ change the count during a drag).
        match key {
            ToolKey::Nudge { direction, mods, .. } if self.rules.arrow_sides && direction.x == 0.0 => {
                let Some(drag) = self.drag.as_mut() else { return false };
                if !matches!(drag.options.kind, ShapeKind::Polygon | ShapeKind::Star) || !self.rules.radial_polygons {
                    return false;
                }
                let sides = if direction.y < 0.0 { drag.sides.saturating_add(1) } else { drag.sides.saturating_sub(1) };
                drag.sides = sides.clamp(*SIDES.start(), *SIDES.end());
                let (mods, pos) = (*mods, cx.space().map(|space| space.view.to_screen(drag.at)));
                if let Some(pos) = pos {
                    self.update(pos, mods, cx);
                }
                true
            }
            ToolKey::Submit if self.drag.is_none() => {
                let Some(space) = cx.space() else { return false };
                let at = space.view.to_doc(space.view.rect.center());
                self.events.push(ShapeDragEvent::Click { at, kind: self.options.kind, mods: cx.mods });
                true
            }
            _ => false,
        }
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        // pc: crates/ui-egui/src/tools/vector_ui/shapes.rs:148-166 `draw_shape_preview` (the path over
        // a halo, :164 `contrast_path`).
        if !self.preview {
            return;
        }
        let (Some(space), Some(style)) = (cx.space(), cx.style()) else { return };
        let Some(shape) = self.drag.as_ref().filter(|d| d.began).and_then(|d| d.shape.as_ref()) else { return };
        let overlay = &style.tokens.color.overlay;
        let (halo, line) = (style.color(overlay.halo), style.color(overlay.line));
        let tolerance = space.view.doc_len(OUTLINE_TOLERANCE);
        let runs = path_runs(&space.view, &shape.to_path(tolerance));
        for (width, color) in [(StrokeWidth::Thick, halo), (StrokeWidth::Medium, line)] {
            for (points, closed) in &runs {
                let shape = if *closed { PaintShape::Polygon(points.clone()) } else { PaintShape::Path(points.clone()) };
                draw.stroke(&shape, width, color);
            }
        }
    }

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Crosshair)
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        // "Rectangle 120 × 80 px" while a shape is drawn.
        let drag = self.drag.as_ref().filter(|d| d.began)?;
        let bounds = drag.shape.as_ref()?.bounds();
        let words = cx.words::<ShapeDragWords>();
        let name = match drag.options.kind {
            ShapeKind::Rectangle => &words.rectangle,
            ShapeKind::RoundedRectangle => &words.rounded_rectangle,
            ShapeKind::Ellipse => &words.ellipse,
            ShapeKind::Triangle => &words.triangle,
            ShapeKind::Polygon => &words.polygon,
            ShapeKind::Star => &words.star,
            ShapeKind::Line => &words.line,
            ShapeKind::Custom => &words.custom,
        };
        let units = cx.units();
        let size = words.size.fill(&[("width", &units.pixels(bounds.width())), ("height", &units.pixels(bounds.height()))]);
        Some(ToolAccess::new(name.as_str()).value(size).bounds(bounds))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Create Shape dialog (Enter or a click)")
    }
}

//! [`PerspectiveCropTool`]: the Perspective Crop tool's background presses. A box drag draws an upright quad, and four
//! clicks place its corners one by one. Once a quad exists, [`PerspectiveCropQuad`](super::PerspectiveCropQuad)
//! takes the presses on its corners and inside, and a press elsewhere does nothing. The tool is a canvas tool for
//! one pass through [`PerspectiveCropPass`].

use std::sync::Arc;

use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, CursorIcon, EditEvent, GestureId, IconId, Mixed, PaintedCursor, PointerGlyph};
use craft_theme::variant::Tone;
use craft_ui::canvas::{CanvasTool, CropShield, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::patterns::form_dialogs::{Param, ParamDialog, ParamKind};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Env;
use egui::Pos2;

use super::perspective_crop::{PerspectiveCropOutput, PerspectiveCropQuad, PerspectiveQuad, grid_color, grid_lines, grid_step, is_valid, ordered};
use super::session::{CropDocKey, DocOwner};
use super::words::CropWords;

/// The key of the Perspective Crop To... command form.
const PERSPECTIVE_TO_KEY: &str = "draw.crop.perspectiveTo";

/// The keys of the form's four corner rows, clockwise from the top left.
const CORNER_KEYS: [&str; 4] = ["topLeft", "topRight", "bottomRight", "bottomLeft"];

/// What the Perspective Crop tool reports, from [`PerspectiveCropTool::take_events`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PerspectiveCropEvent {
    /// A session began: the press that starts a quad, before its box drag or its first clicked corner. The app runs
    /// what a new quad does first here (Photoshop drops the selection).
    Started,
    /// The tool drew the quad: a box drag (`Begin`, `Commit`; `Cancel` on Esc) or the fourth click (`Set`).
    Edit(EditEvent<PerspectiveQuad>),
    /// Another document became active: the pending quad, clicks or press of this one were dropped.
    Discard(CropDocKey),
}

/// A press the tool owns, on the background.
#[derive(Clone, Copy, Debug)]
struct Press {
    id: GestureId,
    /// The press in document space.
    anchor: Point,
    /// The press on screen.
    origin: Pos2,
    /// The quad before the press.
    before: PerspectiveQuad,
    began: bool,
}

/// The Perspective Crop tool, kept by the app. It owns the quad: show it in the canvas body with
/// [`PerspectiveCropPass::show_quad`], and read the tool's own edits with [`take_events`](Self::take_events).
///
/// - With no quad, a drag draws an upright box (none while it has no area), and four clicks place corners one by one;
///   the fourth makes the quad, corners put in order. A drag that makes a box clears the clicks; one that makes none
///   keeps them.
/// - Shift makes the box square and Alt draws it from its centre.
/// - After two or three clicks a quad to the pointer previews the corners to come.
/// - Esc drops the pending clicks. Once a quad exists, the quad's control takes Enter, Esc and the arrows
///   ([`PerspectiveCropOutput::apply`](super::PerspectiveCropOutput::apply) and
///   [`discard`](super::PerspectiveCropOutput::discard)).
/// - The first press of a session reports [`PerspectiveCropEvent::Started`]. What is pending belongs to the document
///   it was made on ([`PerspectiveInput::document`]): another document becoming active drops it and reports
///   [`PerspectiveCropEvent::Discard`], the same rule as the Crop tool's frame.
/// - Its keyboard path is the Perspective Crop To... command ([`perspective_to_form`]).
#[derive(Clone, Debug, Default)]
pub struct PerspectiveCropTool {
    quad: PerspectiveQuad,
    clicks: Vec<Point>,
    press: Option<Press>,
    events: Vec<PerspectiveCropEvent>,
    /// The document the pending quad, clicks or press belong to.
    owner: DocOwner,
}

impl PerspectiveCropTool {
    /// A tool with no quad.
    pub fn new() -> Self {
        Self::default()
    }

    /// The quad.
    pub fn quad(&self) -> &PerspectiveQuad {
        &self.quad
    }

    /// The quad, for the control that edits it in place.
    pub fn quad_mut(&mut self) -> &mut PerspectiveQuad {
        &mut self.quad
    }

    /// Sets the quad (the form's answer, an undo step) and drops the pending clicks.
    pub fn set(&mut self, quad: PerspectiveQuad) {
        self.quad = quad;
        self.clicks.clear();
    }

    /// Drops the quad and the pending clicks: the crop was applied or cancelled. A change of the active document
    /// drops them by itself ([`PerspectiveInput::document`]).
    pub fn clear(&mut self) {
        self.set(None);
        self.press = None;
        self.owner = DocOwner::default();
    }

    /// Something is pending: a quad, clicked corners or a press that may start one. The options bar's Commit and
    /// Cancel end it.
    pub fn pending(&self) -> bool {
        self.quad.is_some() || !self.clicks.is_empty() || self.press.is_some()
    }

    /// The corners clicked so far, before the fourth makes the quad.
    pub fn placing(&self) -> usize {
        self.clicks.len()
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<PerspectiveCropEvent> {
        std::mem::take(&mut self.events)
    }
}

/// The upright quad of a box drawn from `anchor` to `to`; none while the box has no area.
fn boxed(anchor: Point, to: Point) -> PerspectiveQuad {
    let (x0, y0, x1, y1) = (anchor.x.min(to.x), anchor.y.min(to.y), anchor.x.max(to.x), anchor.y.max(to.y));
    (x1 > x0 && y1 > y0).then_some([Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)])
}

/// What the host hands the perspective crop pass each frame.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct PerspectiveInput<'a> {
    /// The active document: pending work of another document is dropped ([`PerspectiveCropEvent::Discard`]). `None`
    /// keys nothing.
    pub document: Option<CropDocKey>,
    /// Whether the quad's grid shows.
    pub show_grid: bool,
    /// The shield over the area outside the quad; `None` shows none.
    pub shield: Option<&'a CropShield>,
}

/// This pass's perspective crop tool for the host, borrowing its input; it is the canvas tool while the pass lives.
pub struct PerspectiveCropPass<'a> {
    tool: &'a mut PerspectiveCropTool,
    input: PerspectiveInput<'a>,
}

impl PerspectiveCropTool {
    /// This pass's tool for the host, borrowing `input`'s data.
    pub fn pass<'a>(&'a mut self, input: PerspectiveInput<'a>) -> PerspectiveCropPass<'a> {
        PerspectiveCropPass { tool: self, input }
    }
}

impl PerspectiveCropPass<'_> {
    /// Shows the quad over the canvas at `canvas_rect` and reports its edits.
    pub fn show_quad(&mut self, ui: &mut egui::Ui, canvas_rect: egui::Rect) -> PerspectiveCropOutput {
        let label = Env::of(ui).words::<CropWords>().perspective_group.clone();
        let mut quad = PerspectiveCropQuad::new(label, Bind::bound(&mut self.tool.quad)).show_grid(self.input.show_grid);
        if let Some(shield) = self.input.shield {
            quad = quad.shield(shield.clone());
        }
        quad.show_at(ui, canvas_rect)
    }
}

impl CanvasTool for PerspectiveCropPass<'_> {
    /// Drops the pending work of another document ([`PerspectiveCropEvent::Discard`]), and keys what is pending to
    /// the active one.
    fn each_pass(&mut self, _: &mut ToolContext<'_>) {
        let Some(document) = self.input.document else { return };
        if let Some(dropped) = self.tool.owner.stale_for(document) {
            let had = self.tool.pending();
            self.tool.clear();
            if had {
                self.tool.events.push(PerspectiveCropEvent::Discard(dropped));
            }
        }
        self.tool.owner = DocOwner::of(self.tool.pending().then_some(document));
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let anchor = space.view.to_doc(pointer.pos);
        if self.tool.quad.is_some() || !(anchor.x.is_finite() && anchor.y.is_finite()) {
            return ToolAction::Ignored;
        }
        let Some(id) = cx.gesture_id() else { return ToolAction::Ignored };
        if !self.tool.pending() {
            self.tool.events.push(PerspectiveCropEvent::Started);
        }
        self.tool.press = Some(Press { id, anchor, origin: pointer.pos, before: self.tool.quad, began: false });
        self.tool.owner = DocOwner::of(self.input.document.or(self.tool.owner.key()));
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some(mut press), Some(space)) = (self.tool.press, cx.space()) else { return };
        if !press.began && (pointer.pos - press.origin).abs().max_elem() > cx.slop {
            press.began = true;
            let label: Arc<str> = Arc::from(cx.words::<CropWords>().perspective_group.as_str());
            self.tool.events.push(PerspectiveCropEvent::Edit(EditEvent::begin(press.id, Mixed::Same(press.before), label, None)));
        }
        if press.began {
            let (a, b) = Constraint::Free.corners(press.anchor, space.view.to_doc(pointer.pos), pointer.mods.shift, pointer.mods.alt);
            self.tool.quad = boxed(a, b);
            if self.tool.quad.is_some() {
                // A drag that makes no box keeps the corners clicked so far.
                self.tool.clicks.clear();
            }
            self.tool.events.push(PerspectiveCropEvent::Edit(EditEvent::update(press.id, self.tool.quad)));
        }
        self.tool.press = Some(press);
    }

    fn release(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(press) = self.tool.press.take() else { return };
        if press.began {
            self.tool.events.push(PerspectiveCropEvent::Edit(EditEvent::commit(press.id, Mixed::Same(press.before), self.tool.quad)));
            return;
        }
        self.tool.clicks.push(press.anchor);
        if let [a, b, c, d] = self.tool.clicks[..] {
            self.tool.clicks.clear();
            let quad = Some(ordered([a, b, c, d]));
            let label: Arc<str> = Arc::from(cx.words::<CropWords>().perspective_group.as_str());
            self.tool.quad = quad;
            self.tool.events.push(PerspectiveCropEvent::Edit(EditEvent::set(Mixed::Same(press.before), quad, label, None)));
        }
    }

    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        let Some(press) = self.tool.press.take() else { return };
        if press.began {
            self.tool.quad = press.before;
            self.tool.events.push(PerspectiveCropEvent::Edit(EditEvent::cancel(press.id, reason, Some(Mixed::Same(press.before)))));
        }
    }

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        match key {
            ToolKey::Cancel if !self.tool.clicks.is_empty() => {
                self.tool.clicks.clear();
                true
            }
            _ => false,
        }
    }

    /// The corners clicked so far, and with two or three of them the quad to the pointer: the corners to come sit
    /// at the pointer.
    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(space), Some(style)) = (cx.space(), cx.style()) else { return };
        let view = &space.view;
        let clicks = &self.tool.clicks;
        let handle = style.handle();
        let line = style.color(style.tokens.color.overlay.line);
        let pointer = cx.pointer.as_ref().map(|p| view.to_doc(p.pos)).filter(|p| p.x.is_finite() && p.y.is_finite());
        let preview = match (clicks.as_slice(), pointer) {
            ([a, b], Some(p)) => Some([*a, *b, p, p]),
            ([a, b, c], Some(p)) => Some([*a, *b, *c, p]),
            _ => None,
        };
        if let Some(quad) = preview {
            let ok = clicks.len() == 2 || is_valid(&quad);
            if ok && self.input.show_grid {
                let grid = grid_color(style);
                for [a, b] in grid_lines(view, &quad, grid_step(style)) {
                    draw.stroke(&PaintShape::Path(vec![a, b]), StrokeWidth::Thin, grid);
                }
            }
            let outline = PaintShape::Polygon(quad.iter().map(|p| view.to_screen(*p)).collect());
            draw.stroke(&outline, StrokeWidth::Medium, if ok { line } else { style.tone(Tone::Bad) });
        } else if clicks.len() > 1 {
            draw.stroke(&PaintShape::Path(clicks.iter().map(|c| view.to_screen(*c)).collect()), StrokeWidth::Medium, line);
        }
        for click in clicks {
            let shape = PaintShape::Circle(view.to_screen(*click), handle.side / 2.0);
            draw.fill(&shape, handle.fill.off.rest);
            draw.stroke(&shape, StrokeWidth::Thin, line);
        }
    }

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        if self.tool.quad.is_none() {
            PaintedCursor::pointer_tool(PointerGlyph::Crosshair, IconId::new("perspective_crop")).into()
        } else {
            Cursor::System(CursorIcon::Default)
        }
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Perspective Crop To... command")
    }
}

/// The Perspective Crop To... command's form: the four corners of `quad`, clockwise from the top left, in document
/// units. Without a quad the corners start at the origin.
pub fn perspective_to_form(words: &CropWords, quad: &PerspectiveQuad) -> ParamDialog<'static> {
    let corners = quad.unwrap_or([Point::ZERO; 4]);
    let names = [&words.perspective_top_left, &words.perspective_top_right, &words.perspective_bottom_right, &words.perspective_bottom_left];
    let params: Vec<Param> = names
        .into_iter()
        .zip(corners)
        .zip(CORNER_KEYS)
        .map(|((name, corner), key)| Param::new(key, name.clone(), ParamKind::point([corner.x, corner.y])))
        .collect();
    ParamDialog::command(PERSPECTIVE_TO_KEY, words.perspective_crop_to.clone(), params)
}

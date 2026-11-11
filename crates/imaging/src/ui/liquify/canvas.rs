//! Liquify's canvas: the canvas tool, the backdrop layering, the freeze mask source, the lasso line, the mesh.

mod backdrop;
mod lasso;
mod mesh;

pub(super) use backdrop::MaskAtTexel;
pub use backdrop::{BackdropLayering, LayeredImage};
pub use lasso::{LassoLine, LassoLineStyle, LassoTone};
pub use mesh::LiquifyMesh;

use super::LiquifyEvent;
use super::session::{LiquifySettings, LiquifyStroke, StrokeEvent};
use super::tools::{LiquifyBrush, LiquifyTool, StrokeRecorder};
use craft_behavior::geometry::draft::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, HandTool, NavigationOptions, ToolAction, ToolContext, ToolPath, ToolPointer, ZoomTool};
use craft_ui::canvas_controls::CanvasControlWords;
use craft_ui::canvas_controls::brush::{BrushCursor, BrushCursorOptions, BrushDimensions, BrushLevel, BrushResize, BrushResizeOptions, BrushTip};
use craft_ui::primitives::Draw;

/// The seconds between the dabs of a tool that acts while the brush rests (PhotoCraft: 30 a
/// second).
const HOLD_INTERVAL: f64 = 1.0 / 30.0;

/// How long the latest brush stroke has run and how many dabs it has made, measured in input
/// time (the diagnostics PhotoCraft's stats line showed; the app adds its own field and preview
/// times). It stays after the stroke ends, until the next press starts a new one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LiquifyTiming {
    /// The dabs so far: the press, each new point and each rest-time dab.
    pub dabs: usize,
    /// Seconds from the press to the latest dab.
    pub seconds: f64,
}

/// The state of Liquify's canvas between passes: the stroke being recorded, the Freeze Lasso being drawn, the
/// brush resize, and the events not yet taken. It lives in the dialog body's session slot; the workspace hands
/// [`LiquifyCanvasTool`] to its `FullWindowCanvas` while a Liquify tool is current, and the library's `HandTool` or
/// `ZoomTool` for H and Z.
#[derive(Clone, Debug)]
pub struct LiquifyCanvas {
    recorder: StrokeRecorder,
    lasso: Option<(OutlineDraft, bool)>,
    resize: BrushResize,
    events: Vec<LiquifyEvent>,
    /// The brush when the resize began: Esc puts it back.
    resize_from: Option<LiquifyBrush>,
    /// The latest point of the stroke in progress, to end it without a pointer.
    at: [f64; 2],
    /// The input time of the stroke's latest dab, for the tools that act while the brush rests.
    last_dab: f64,
    /// The input time of the press of the latest stroke.
    began: f64,
    timing: Option<LiquifyTiming>,
    pub(super) hand: HandTool,
    pub(super) zoom: ZoomTool,
}

impl Default for LiquifyCanvas {
    fn default() -> Self {
        Self {
            recorder: StrokeRecorder::default(),
            lasso: None,
            resize: BrushResize::default(),
            events: Vec::new(),
            resize_from: None,
            at: [0.0; 2],
            last_dab: 0.0,
            began: 0.0,
            timing: None,
            hand: HandTool::new(NavigationOptions::default()),
            zoom: ZoomTool::new(NavigationOptions::default()),
        }
    }
}

impl LiquifyCanvas {
    /// The canvas tool for this pass over `settings` (the tool, the brush; the resize writes the
    /// brush's size and density).
    pub fn tool<'a>(&'a mut self, settings: &'a mut LiquifySettings, cursor: &'a BrushCursorOptions) -> LiquifyCanvasTool<'a> {
        LiquifyCanvasTool { canvas: self, settings, cursor }
    }

    /// The timing of the latest brush stroke; `None` before the first.
    pub fn timing(&self) -> Option<LiquifyTiming> {
        self.timing
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<LiquifyEvent> {
        std::mem::take(&mut self.events)
    }

    /// A stroke, a lasso or a resize is in progress.
    pub fn busy(&self) -> bool {
        self.recorder.is_active() || self.lasso.is_some() || self.resize.active()
    }

    /// Sets the user's navigation options the Hand and Zoom tools run with.
    pub(super) fn navigate(&mut self, options: NavigationOptions) {
        self.hand.options = options;
        self.zoom.options = options;
    }

    /// The lasso's points and tone while it is drawn.
    pub(super) fn lasso_line(&self) -> Option<(Vec<Point>, LassoTone)> {
        let (draft, thaw) = self.lasso.as_ref()?;
        let tone = if *thaw { LassoTone::Thaw } else { LassoTone::Freeze };
        Some((draft.points().to_vec(), tone))
    }

    /// The brush resize in progress: where it started and its values.
    pub(super) fn resize(&self) -> &BrushResize {
        &self.resize
    }

    /// Ends the stroke in progress where it was last drawn: its `Commit`, for the caller to
    /// record. `None` with no stroke.
    pub(super) fn end_stroke(&mut self) -> Option<LiquifyEvent> {
        self.recorder.end(self.at).map(LiquifyEvent::Stroke)
    }

    /// A rest-time dab of the tool that acts while the brush is held still.
    fn hold(&mut self, at: Point, time: f64) {
        if let Some(event) = self.recorder.hold([at.x, at.y, 1.0]) {
            self.events.push(LiquifyEvent::Stroke(event));
            self.dabbed([at.x, at.y], time);
        }
    }

    /// A dab at `at` at input time `time`.
    fn dabbed(&mut self, at: [f64; 2], time: f64) {
        self.at = at;
        self.last_dab = time;
        if let Some(timing) = self.timing.as_mut() {
            timing.dabs += 1;
            timing.seconds = (time - self.began).max(0.0);
        }
    }
}

/// Whether the tool keeps working while the brush is held still.
fn acts_at_rest(tool: LiquifyTool) -> bool {
    // pc: crates/algo/src/liquify.rs:92 `is_stationary`.
    !matches!(tool, LiquifyTool::ForwardWarp | LiquifyTool::PushLeft | LiquifyTool::FreezeLasso | LiquifyTool::Hand | LiquifyTool::Zoom)
}

/// The brush outline options Photoshop's defaults give.
pub(super) fn default_cursor_options() -> BrushCursorOptions {
    BrushCursorOptions::new(craft_ui::canvas_controls::brush::PaintingCursor::default())
}

/// [`LiquifyCanvas`] as the `CanvasTool` of one pass.
///
/// - Press, drag, release with a brush tool: one stroke through [`StrokeRecorder`] (Alt at the
///   press paints with [`LiquifyTool::alternate`]), reported as [`LiquifyEvent::Stroke`] Begin, Update with
///   each new point (the new points only), Commit with the whole stroke. Esc or a lost pointer
///   reports Cancel, then a Replay (the workspace adds it from the session).
/// - The Freeze Lasso: an [`OutlineDraft`] of `DraftKind::Freehand`; the release closes it into a stroke with
///   `amount` 1, or 0 when Alt was held at the press; the workspace draws it with [`LassoLine`].
/// - Alt + right-drag, or Control + Alt + drag ([`BrushResize::chord`]): a [`BrushResize`] with
///   `BrushLevel::Density`, `max_diameter` 15000; horizontal travel sets the size, vertical the density; the
///   outline stays at the anchor; no stroke. Each update writes `settings.brush`.
/// - The cursor: the brush outline's (`BrushCursor::cursor`), the crosshair for the lasso, none while resizing.
pub struct LiquifyCanvasTool<'a> {
    canvas: &'a mut LiquifyCanvas,
    settings: &'a mut LiquifySettings,
    cursor: &'a BrushCursorOptions,
}

impl LiquifyCanvasTool<'_> {
    /// Ends the stroke, the lasso and the resize in progress without recording them (the host's `cancel`, and the
    /// workspace when the host has no gesture to cancel). Esc, a lost pointer, a tool switch or an undo end a
    /// stroke this way (PhotoCraft has no cancel).
    pub(super) fn abandon(&mut self, reason: CancelReason) {
        if let Some(event) = self.canvas.recorder.cancel(reason) {
            self.canvas.events.push(LiquifyEvent::Stroke(event));
        }
        if let Some((mut draft, _)) = self.canvas.lasso.take() {
            draft.cancel();
        }
        if self.canvas.resize.cancel().is_some()
            && let Some(brush) = self.canvas.resize_from.take()
        {
            self.settings.brush = brush;
        }
    }

    /// Writes a resize's values to the brush.
    fn apply_resize(&mut self, dimensions: BrushDimensions) {
        self.settings.brush.size = dimensions.diameter;
        self.settings.brush.density = dimensions.level * 100.0;
    }

    fn show_readout(&self, cx: &mut ToolContext<'_>) {
        let rows = self.canvas.resize.readout_rows(&cx.units(), &cx.words::<CanvasControlWords>());
        if !rows.is_empty() {
            cx.readout(rows);
        }
    }

    fn begin_resize(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let brush = self.settings.brush;
        let options = BrushResizeOptions::new(LiquifyBrush::SIZE_MAX).level(BrushLevel::Density);
        let start = BrushDimensions::new(brush.size, brush.density / 100.0);
        if self.canvas.resize.begin(&space.view, pointer.pos, start, &options).is_none() {
            return ToolAction::Ignored;
        }
        self.canvas.resize_from = Some(brush);
        self.show_readout(cx);
        ToolAction::Claimed
    }

    /// The lasso's stroke for the closed polygon `points`.
    fn lasso_stroke(&self, points: &[Point], thaw: bool) -> LiquifyStroke {
        let brush = self.settings.brush;
        LiquifyStroke {
            tool: LiquifyTool::FreezeLasso,
            size: brush.size,
            density: brush.density,
            pressure: brush.pressure,
            rate: brush.rate,
            points: points.iter().map(|p| [p.x, p.y, 1.0]).collect(),
            amount: Some(if thaw { 0.0 } else { 1.0 }),
        }
    }
}

impl CanvasTool for LiquifyCanvasTool<'_> {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:1162-1216 (the preview's pointer input;
        // :1203-1207 the tools that act at rest keep dabbing at 30 dabs a second while the brush
        // is held still: the host calls `drag` every pass and `drag` dabs, so this asks for the
        // pass).
        if self.canvas.recorder.is_active() && acts_at_rest(self.settings.tool) {
            cx.request_repaint();
        }
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:671 `brush_circle_at` (the outline stays at
        // the anchor while resizing: the pointer hides and the preview draws it there).
        if self.canvas.resize.active() {
            return Cursor::Painted(PaintedCursor::Hidden);
        }
        if self.settings.tool == LiquifyTool::FreezeLasso {
            return Cursor::Painted(PaintedCursor::Crosshair);
        }
        let Some(space) = cx.space() else { return Cursor::System(CursorIcon::Default) };
        let size = self.settings.brush.size;
        BrushCursor::new(&BrushTip::round(size, size / 2.0)).painting(self.canvas.recorder.is_active()).cursor(cx, self.cursor, &space.view)
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:676 `brush_resize_gesture` (Alt with the
        // secondary button, or Ctrl and Alt). The primary button always paints (Alt reverses to
        // the alternate tool); the secondary starts the resize chord while Alt is held.
        BrushResize::takes(button, mods)
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:693 `pointer` (Down), :682 `resized_brush`.
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        if BrushResize::chord(pointer.button, pointer.mods) {
            return self.begin_resize(pointer, cx);
        }
        if pointer.button != PointerButton::Primary {
            return ToolAction::Ignored;
        }
        let doc = space.view.to_doc(pointer.pos);
        let tool = self.settings.tool;
        if tool == LiquifyTool::FreezeLasso {
            let reach = f64::from(cx.slop);
            let mut draft = OutlineDraft::new(DraftKind::Freehand, DraftReach::new(reach, reach));
            draft.press(DraftPoint::new(doc, pointer.mods, space.view.zoom), false);
            self.canvas.lasso = Some((draft, pointer.mods.alt));
            cx.request_repaint();
            return ToolAction::Claimed;
        }
        if !tool.is_brush() {
            return ToolAction::Ignored;
        }
        // A stroke whose release was lost ends first: the recorder would replace it silently.
        if let Some(event) = self.canvas.end_stroke() {
            self.canvas.events.push(event);
        }
        let at = [doc.x, doc.y, 1.0];
        let event = self.canvas.recorder.begin(tool, &self.settings.brush, at, pointer.mods.alt);
        self.canvas.events.push(LiquifyEvent::Stroke(event));
        self.canvas.at = [at[0], at[1]];
        self.canvas.last_dab = cx.time;
        self.canvas.began = cx.time;
        self.canvas.timing = Some(LiquifyTiming { dabs: 1, seconds: 0.0 });
        cx.request_repaint();
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:704 (Move), :1196-1209 (a point that moved,
        // or the held dab).
        let Some(space) = cx.space() else { return };
        if self.canvas.resize.active() {
            if let Some(step) = self.canvas.resize.track(&space.view, pointer.pos) {
                self.apply_resize(step.dimensions);
            }
            self.show_readout(cx);
            return;
        }
        let doc = space.view.to_doc(pointer.pos);
        if let Some((draft, _)) = self.canvas.lasso.as_mut() {
            draft.drag(DraftPoint::new(doc, pointer.mods, space.view.zoom));
            cx.request_repaint();
            return;
        }
        if !self.canvas.recorder.is_active() {
            return;
        }
        // PhotoCraft passes pressure 1 whatever the pen reports.
        let at = [doc.x, doc.y, 1.0];
        if let Some(event) = self.canvas.recorder.extend(at) {
            self.canvas.events.push(LiquifyEvent::Stroke(event));
            self.canvas.dabbed([at[0], at[1]], cx.time);
        } else if acts_at_rest(self.settings.tool) && cx.time - self.canvas.last_dab > HOLD_INTERVAL {
            self.canvas.hold(doc, cx.time);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:714 (Up), :299 `close_lasso`.
        let Some(space) = cx.space() else { return };
        if self.canvas.resize.active() {
            if let Some(step) = self.canvas.resize.finish(&space.view, pointer.pos) {
                self.apply_resize(step.dimensions);
            }
            self.canvas.resize_from = None;
            return;
        }
        let doc = space.view.to_doc(pointer.pos);
        if let Some((mut draft, thaw)) = self.canvas.lasso.take() {
            let point = DraftPoint::new(doc, pointer.mods, space.view.zoom);
            if let Some(DraftEvent::Closed { points, .. }) = draft.release(point) {
                let stroke = self.lasso_stroke(&points, thaw);
                self.canvas.events.push(LiquifyEvent::Stroke(StrokeEvent::Commit(stroke)));
            }
            return;
        }
        if let Some(event) = self.canvas.recorder.end([doc.x, doc.y]) {
            self.canvas.events.push(LiquifyEvent::Stroke(event));
        }
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.abandon(reason);
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {
        // The lasso line and the resize preview are canvas controls the workspace shows (a tool holds no tone
        // colours): `LassoLine` and `FalloffPreview`.
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("none; a liquify stroke follows the pointer")
    }
}

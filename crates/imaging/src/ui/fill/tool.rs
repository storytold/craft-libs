//! Content-Aware Fill's tools and the canvas tool.

use craft_behavior::geometry::draft::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::canvas_controls::CanvasControlWords;
use craft_ui::canvas_controls::brush::{BrushCursor, BrushCursorOptions, BrushDimensions, BrushResize, BrushResizeOptions, BrushTip, PaintingCursor};
use craft_ui::primitives::Draw;
use craft_ui::rails::{RailTool, ToolSlot};

use super::FillEvent;
use super::commands::FillCommands;
use super::lasso::FillAreaEdit;
use super::session::{FillLimit, FillLimits, FillSession, FillStrokeEvent, SamplingStroke};
use crate::ui::select_mask::EditMode;

/// The tools of Content-Aware Fill's strip: Sampling Brush (B), Lasso and Polygonal Lasso (L), Hand (H), Zoom (Z).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui::contract::MemoProps, craft_ui::contract::PlainData)]
#[non_exhaustive]
pub enum FillTool {
    /// The Sampling Brush.
    #[default]
    SamplingBrush,
    /// The Lasso.
    Lasso,
    /// The Polygonal Lasso.
    PolygonalLasso,
    /// The Hand.
    Hand,
    /// The Zoom.
    Zoom,
}

impl FillTool {
    /// The tools, in strip and flyout order.
    pub const ALL: [FillTool; 5] = [FillTool::SamplingBrush, FillTool::Lasso, FillTool::PolygonalLasso, FillTool::Hand, FillTool::Zoom];

    /// Four slots; the Lasso slot lists Lasso then Polygonal Lasso. Labels, icons and letters come from the commands.
    pub fn slots() -> Vec<ToolSlot<FillTool>> {
        let entry = |tool| RailTool::new(tool, FillCommands::of(tool));
        vec![
            ToolSlot::new(0, vec![entry(FillTool::SamplingBrush)]),
            ToolSlot::ending_group(1, vec![entry(FillTool::Lasso), entry(FillTool::PolygonalLasso)]),
            ToolSlot::new(2, vec![entry(FillTool::Hand)]),
            ToolSlot::new(3, vec![entry(FillTool::Zoom)]),
        ]
    }
}

/// The tools' options: the bar's values, outside the history and not persisted.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FillToolOptions {
    /// The bar's + (true) or −; the sampling option sets it (Custom adds) and Alt reverses it.
    pub brush_add: bool,
    /// The Sampling Brush's diameter, 1..=5000; a tenth of the sampling window's side when the
    /// workspace opens (20 px for a 50 px selection).
    pub brush_size: f64,
    /// The Lasso's operation in the bar (Shift adds, Alt subtracts, both intersect at the press).
    pub lasso: SelectionOperation,
    /// Expand and Contract's amount, 1..=100 px ([`GROW_BY`](Self::GROW_BY) when the workspace opens).
    pub grow_by: f64,
}

impl FillToolOptions {
    /// The largest brush diameter the keys and the resize gesture set.
    pub const SIZE_MAX: f64 = 5000.0;

    /// Expand and Contract's amount when the workspace opens, in pixels (PhotoCraft `caf_ui/lifecycle.rs:30`).
    pub const GROW_BY: f64 = 3.0;

    /// The mode of a stroke pressed with the bar at `brush_add`: Alt reverses it.
    ///
    /// The brush takes the primary button always and the secondary while Alt is held
    /// ([`CanvasTool::takes`]), so an Alt + primary press reaches it and reverses the mode, while
    /// Alt + right-drag resizes.
    pub fn stroke_mode(brush_add: bool, alt: bool) -> EditMode {
        if brush_add != alt { EditMode::Add } else { EditMode::Subtract }
    }

    /// `[`: × 0.9 rounded, at least 1. `]`: × 1.1 rounded, at least a pixel larger, at most 5000.
    pub fn step_size(&mut self, larger: bool) {
        // pc: crates/ui-egui/src/dialogs/caf_ui/controls.rs:167-172 (PhotoCraft's `]` adds the
        // pixel after the cap, so at 5000 it reaches 5001; the cap holds here).
        let size = self.brush_size;
        self.brush_size =
            if larger { (size * 1.1).min(Self::SIZE_MAX).round().max(size + 1.0).min(Self::SIZE_MAX) } else { (size * 0.9).max(1.0).round() };
    }
}

/// The brush outline options Photoshop's defaults give.
pub(super) fn default_cursor_options() -> BrushCursorOptions {
    BrushCursorOptions::new(PaintingCursor::default())
}

/// The state of Content-Aware Fill's canvas between passes: the stroke being painted, the lasso
/// being drawn, the brush resize, the events not yet taken.
#[derive(Clone, Debug, Default)]
pub struct FillCanvas {
    stroke: Option<SamplingStroke>,
    lasso: Option<(OutlineDraft, SelectionOperation)>,
    resize: BrushResize,
    events: Vec<FillEvent>,
    /// The points limit was reported for the stroke in progress.
    refused: bool,
}

impl FillCanvas {
    /// An idle canvas.
    pub fn new() -> Self {
        Self::default()
    }

    /// The canvas tool for this pass. `session` and `limits` decide whether a stroke may begin
    /// or take another point; `cursor` is the user's brush outline preference.
    pub fn tool<'a>(
        &'a mut self,
        tool: FillTool,
        options: &'a mut FillToolOptions,
        session: &'a FillSession,
        limits: &'a FillLimits,
        cursor: &'a BrushCursorOptions,
    ) -> FillCanvasTool<'a> {
        FillCanvasTool { canvas: self, tool, options, session, limits, cursor }
    }

    /// Takes the events collected since the last call.
    pub fn take_events(&mut self) -> Vec<FillEvent> {
        std::mem::take(&mut self.events)
    }

    /// Whether a stroke, a lasso or a brush resize is in progress.
    pub fn busy(&self) -> bool {
        self.stroke.is_some() || self.lasso.is_some() || self.resize.active()
    }

    /// The lasso outline being drawn, for the canvas control that shows it.
    pub fn lasso_points(&self) -> Option<&[Point]> {
        self.lasso.as_ref().map(|(draft, _)| draft.points()).filter(|points| points.len() >= 2)
    }

    /// The Polygonal Lasso's next segment while it waits for another click.
    pub fn lasso_rubber_band(&self) -> Option<[Point; 2]> {
        self.lasso.as_ref().and_then(|(draft, _)| draft.rubber_band())
    }

    /// Keeps a click-built outline between presses and reports only its completed polygon.
    fn lasso_event(&mut self, event: Option<DraftEvent>) {
        match event {
            Some(DraftEvent::Closed { points, .. }) => {
                if let Some((_, operation)) = self.lasso.take()
                    && let Some(edit) = FillAreaEdit::lasso(operation, points.into_iter().map(|p| [p.x, p.y]).collect())
                {
                    self.events.push(FillEvent::FillArea(edit));
                }
            }
            Some(DraftEvent::Cancelled) => self.lasso = None,
            _ => {}
        }
    }

    /// The brush resize in progress, for the outline preview.
    pub fn resize(&self) -> &BrushResize {
        &self.resize
    }

    /// Drops the stroke (a `Cancel` event), the lasso and the resize without committing.
    pub fn abandon(&mut self, reason: CancelReason) {
        if self.stroke.take().is_some() {
            self.events.push(FillEvent::Stroke(FillStrokeEvent::Cancel(reason)));
        }
        if let Some((mut draft, _)) = self.lasso.take() {
            draft.cancel();
        }
        self.resize.cancel();
        self.refused = false;
    }
}

/// [`FillCanvas`] as the `CanvasTool` of one pass.
///
/// - **Sampling Brush**: press, drag, release is one [`SamplingStroke`] (the bar's mode, reversed
///   by Alt at the press; points equal to the last are skipped), reported as
///   [`FillEvent::Stroke`] Begin, Update with each new point (the app paints its sampling
///   plane as the stroke goes; each `Update` carries only the points added since the last event, so
///   the cost of a stroke stays linear), Commit with the whole stroke. A stroke the limits refuse
///   ([`FillSession::may_begin_stroke`]) does not begin: `FillEvent::Refused` with the limit. A stroke that reaches the points
///   limit stops taking points.
/// - **Lasso**: an [`OutlineDraft`] of `DraftKind::Freehand`; the operation is the bar's, or
///   Shift (Add), Alt (Subtract), Shift+Alt (Intersect) at the press; the release reports
///   [`FillEvent::FillArea`] with [`FillAreaEdit::lasso`](super::FillAreaEdit::lasso) when it has
///   3 points or more.
/// - **Polygonal Lasso**: the same operation and edit, drawn by `DraftKind::Polygonal`. Clicks add vertices;
///   a click near the start or last point, a double-click or Enter closes. Backspace, Delete and secondary click
///   retract; Escape cancels. The draft supplies the main Polygonal Lasso's modifier and close rules.
/// - **Brush resize**: Alt + right-drag or Control + Alt + drag, size only, `max_diameter` 5000.
/// - The lasso path and the resize preview are canvas controls the workspace shows from the state
///   ([`FillCanvas::lasso_points`], [`FillCanvas::resize`]), since a tool paints no styled colours.
pub struct FillCanvasTool<'a> {
    canvas: &'a mut FillCanvas,
    tool: FillTool,
    options: &'a mut FillToolOptions,
    session: &'a FillSession,
    limits: &'a FillLimits,
    cursor: &'a BrushCursorOptions,
}

impl FillCanvasTool<'_> {
    /// Adds `at` to the stroke in progress and reports it. A point equal to the last is skipped;
    /// at the points limit the stroke takes no more points and the message is reported once.
    fn extend(&mut self, at: Point) {
        let canvas = &mut *self.canvas;
        let Some(stroke) = canvas.stroke.as_mut() else { return };
        let point = [at.x, at.y];
        if stroke.points.last() == Some(&point) {
            return;
        }
        if self.session.points_used(Some(stroke)) >= self.limits.points {
            if !canvas.refused {
                canvas.refused = true;
                canvas.events.push(FillEvent::Refused(FillLimit::Vertices(self.limits.points)));
            }
            return;
        }
        stroke.points.push(point);
        let added = SamplingStroke { mode: stroke.mode, size: stroke.size, points: vec![point] };
        canvas.events.push(FillEvent::Stroke(FillStrokeEvent::Update(added)));
    }

    /// The resize's readout beside the pointer: the diameter only (the brush has no hardness).
    fn show_readout(&self, cx: &mut ToolContext<'_>) {
        let Some(dimensions) = self.canvas.resize.dimensions() else { return };
        let words = cx.words::<CanvasControlWords>();
        let pixels = cx.units().pixels(dimensions.diameter.round().max(0.0));
        cx.hud(vec![(words.diameter.clone(), pixels)]);
    }

    fn begin_resize(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        // The size only: no vertical travel changes a level.
        let options = BrushResizeOptions::new(FillToolOptions::SIZE_MAX).travel(0.0);
        let start = BrushDimensions::new(self.options.brush_size, 0.0);
        if self.canvas.resize.begin(&space.view, pointer.pos, start, &options).is_none() {
            return ToolAction::Ignored;
        }
        self.show_readout(cx);
        ToolAction::Claimed
    }
}

impl CanvasTool for FillCanvasTool<'_> {
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        // pc: crates/ui-egui/src/dialogs/caf_ui/view.rs:405-409 (the brush circle).
        match self.tool {
            FillTool::SamplingBrush => {
                if self.canvas.resize.active() {
                    return Cursor::Painted(PaintedCursor::Hidden);
                }
                let Some(space) = cx.space() else { return Cursor::System(CursorIcon::Default) };
                // A hard round brush: the 50 % contour is the edge.
                let size = self.options.brush_size;
                let tip = BrushTip::round(size, size / 2.0);
                BrushCursor::new(&tip).painting(self.canvas.stroke.is_some()).cursor(cx, self.cursor, &space.view)
            }
            FillTool::Lasso | FillTool::PolygonalLasso => Cursor::Painted(PaintedCursor::Crosshair),
            FillTool::Hand | FillTool::Zoom => Cursor::System(CursorIcon::Default),
        }
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        // The brush takes the resize chord's buttons (`BrushResize::takes`); the lasso only the primary button.
        match self.tool {
            FillTool::SamplingBrush => BrushResize::takes(button, mods),
            FillTool::PolygonalLasso => button == PointerButton::Primary || (button == PointerButton::Secondary && self.canvas.lasso.is_some()),
            _ => button == PointerButton::Primary,
        }
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // pc: crates/ui-egui/src/dialogs/caf_ui/controls.rs:109 (brush Down), :118-127 (lasso Down).
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        match self.tool {
            FillTool::SamplingBrush => {
                if BrushResize::chord(pointer.button, pointer.mods) {
                    return self.begin_resize(pointer, cx);
                }
                if pointer.button != PointerButton::Primary {
                    return ToolAction::Ignored;
                }
                if let Err(limit) = self.session.may_begin_stroke(self.limits) {
                    self.canvas.events.push(FillEvent::Refused(limit));
                    return ToolAction::Done;
                }
                let doc = space.view.to_doc(pointer.pos);
                // Alt at the press reverses the bar's mode.
                let mode = FillToolOptions::stroke_mode(self.options.brush_add, pointer.mods.alt);
                let stroke = SamplingStroke { mode, size: self.options.brush_size, points: vec![[doc.x, doc.y]] };
                self.canvas.refused = false;
                self.canvas.events.push(FillEvent::Stroke(FillStrokeEvent::Begin(stroke.clone())));
                self.canvas.stroke = Some(stroke);
                cx.request_repaint();
                ToolAction::Claimed
            }
            FillTool::Lasso | FillTool::PolygonalLasso => {
                if pointer.button == PointerButton::Secondary {
                    if self.tool != FillTool::PolygonalLasso {
                        return ToolAction::Ignored;
                    }
                    let event = self.canvas.lasso.as_mut().and_then(|(draft, _)| draft.retract());
                    self.canvas.lasso_event(event);
                    cx.request_repaint();
                    return ToolAction::Done;
                }
                if pointer.button != PointerButton::Primary {
                    return ToolAction::Ignored;
                }
                // A proximity close may have consumed the first press of a double-click.
                // Its continuation must not start another polygon after that draft was removed.
                if self.tool == FillTool::PolygonalLasso && self.canvas.lasso.is_none() && pointer.clicks >= 2 {
                    return ToolAction::Done;
                }
                let reach = f64::from(cx.slop);
                let kind = if self.tool == FillTool::PolygonalLasso { DraftKind::Polygonal } else { DraftKind::Freehand };
                let (draft, operation) =
                    self.canvas.lasso.get_or_insert_with(|| (OutlineDraft::new(kind, DraftReach::new(reach, reach)), self.options.lasso));
                draft.set_reach(DraftReach::new(reach, reach));
                let doc = space.view.to_doc(pointer.pos);
                // The fill area always has something to combine with: Shift adds, Alt subtracts,
                // both intersect, else the bar's operation.
                let event = if kind == DraftKind::Polygonal && draft.is_open() && pointer.clicks >= 2 {
                    draft.close()
                } else {
                    draft.press(DraftPoint::new(doc, pointer.mods, space.view.zoom), true)
                };
                let handled = event.is_some() || draft.is_open();
                if let Some(DraftEvent::Started { intent, .. }) = &event {
                    *operation = SelectionOperation::resolve(self.options.lasso, *intent, true);
                }
                if !handled {
                    self.canvas.lasso = None;
                    return ToolAction::Ignored;
                }
                self.canvas.lasso_event(event);
                cx.request_repaint();
                ToolAction::Claimed
            }
            FillTool::Hand | FillTool::Zoom => ToolAction::Ignored,
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/caf_ui/controls.rs:110-113, :128-134.
        let Some(space) = cx.space() else { return };
        if self.canvas.resize.active() {
            self.canvas.resize.track(&space.view, pointer.pos);
            self.show_readout(cx);
        } else if self.canvas.stroke.is_some() {
            self.extend(space.view.to_doc(pointer.pos));
        } else if let Some((draft, _)) = self.canvas.lasso.as_mut() {
            draft.drag(DraftPoint::new(space.view.to_doc(pointer.pos), pointer.mods, space.view.zoom));
            cx.request_repaint();
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/caf_ui/controls.rs:114-117, :135-141.
        let Some(space) = cx.space() else { return };
        if self.canvas.resize.active() {
            if let Some(step) = self.canvas.resize.finish(&space.view, pointer.pos) {
                self.options.brush_size = step.dimensions.diameter;
            }
            return;
        }
        let doc = space.view.to_doc(pointer.pos);
        if self.canvas.stroke.is_some() {
            self.extend(doc);
            if let Some(stroke) = self.canvas.stroke.take() {
                self.canvas.events.push(FillEvent::Stroke(FillStrokeEvent::Commit(stroke)));
            }
            return;
        }
        let event = self.canvas.lasso.as_mut().and_then(|(draft, _)| draft.release(DraftPoint::new(doc, pointer.mods, space.view.zoom)));
        self.canvas.lasso_event(event);
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        if let Some((draft, _)) = self.canvas.lasso.as_mut()
            && draft.hover(DraftPoint::new(space.view.to_doc(pointer.pos), pointer.mods, space.view.zoom)).is_some()
        {
            cx.request_repaint();
        }
    }

    fn autoscrolls_on_hover(&self) -> bool {
        self.tool == FillTool::PolygonalLasso && self.canvas.lasso.is_some()
    }

    fn claims(&self) -> ToolClaims {
        if self.canvas.lasso.as_ref().is_some_and(|(draft, _)| draft.takes_retract()) {
            // Keep the workspace's tool letters and history chords available between clicks.
            ToolClaims::NONE.deletes()
        } else {
            ToolClaims::NONE
        }
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        // Esc, a lost pointer, a tool switch or an undo drops the stroke or lasso.
        self.canvas.abandon(reason);
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        // Esc cancels a stroke or lasso in progress (the host does it first during a gesture; this
        // covers a key that arrives with one still open).
        if *key == ToolKey::Cancel && self.canvas.busy() {
            self.canvas.abandon(CancelReason::Escape);
            return true;
        }
        let Some((draft, _)) = self.canvas.lasso.as_mut() else { return false };
        if !draft.takes_retract() || cx.mods.cmd || cx.mods.ctrl {
            return false;
        }
        let event = match key {
            ToolKey::Submit => draft.close(),
            ToolKey::Delete { .. } => draft.retract(),
            _ => return false,
        };
        self.canvas.lasso_event(event);
        cx.request_repaint();
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        match self.tool {
            FillTool::SamplingBrush => ToolPath::Freehand("none; a sampling brush stroke follows the pointer"),
            FillTool::Lasso => ToolPath::Freehand("none; a lasso stroke follows the pointer"),
            FillTool::PolygonalLasso => ToolPath::Command("the fill workspace's lasso action with polygon points"),
            FillTool::Hand => ToolPath::Command("the scroll keys and the scroll bars"),
            FillTool::Zoom => ToolPath::Command("the Zoom In, Zoom Out and Fit commands"),
        }
    }
}

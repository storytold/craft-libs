//! [`CropTool`]: the Crop tool on the canvas tool host. The app owns one `CropTool` and builds a
//! [`CropToolPass`] each pass from it and the pass's input ([`CropInput`]). The pass shows the frame
//! as one [`CropOverlay`] in the canvas body ([`CropToolPass::show_frame`]) and takes the keys the
//! overlay does not (the canvas has the keyboard, no part is focused).

use std::sync::Arc;

use craft_behavior::geometry::{CropBox, SnapLine, SnapTargets, crop_view, screen_step};
use craft_behavior::kurbo::{Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{CancelReason, CanvasView, EditEvent};
use craft_keymap::{Key, Mods};
use craft_ui::canvas::{
    CanvasTool, CropOverlay, CropOverlayOutput, NUDGE, NUDGE_FAR, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer,
};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::patterns::form_dialogs::{Param, ParamDialog, ParamKind};
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::{Env, Units};
use egui::{Rect, Ui, Vec2};

use super::options::{CropFill, CropSettings, CropTarget};
use super::session::{CropDocument, CropKept, CropSession, LeaveFrame};
use super::words::CropWords;

/// The key of the Crop To... command form.
const CROP_TO_KEY: &str = "draw.crop.cropTo";

/// What the app crops with: the frame (its rect before the turn, and `angle`, the turn in degrees
/// clockwise on screen), the size to resample to, Delete Cropped Pixels and Fill. The app maps it
/// to its crop command; a commit it refuses goes back with [`CropTool::applied`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CropCommit {
    /// The frame to crop to.
    pub frame: CropBox,
    /// [`CropSettings::target`]: W x H x Resolution's size, `None` for no resample.
    pub target: Option<CropTarget>,
    /// Whether pixels outside the frame are deleted (else hidden).
    pub delete_pixels: bool,
    /// What fills the area beyond the canvas.
    pub fill: CropFill,
}

impl CropCommit {
    /// The commit of `frame` with `settings` on `document`, with the environment's `units`.
    pub fn new(frame: CropBox, settings: &CropSettings, document: &CropDocument, units: &Units) -> Self {
        Self { frame, target: settings.target(document, units), delete_pixels: settings.delete_pixels, fill: settings.fill }
    }
}

/// What the Crop tool reports. One pass's events come in order and are handled in that pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CropEvent {
    /// The frame changed: a drag (`Begin` on the first move, `Commit` on release; `Cancel` on Esc or a
    /// frame under one unit) or a key, refit, undo step or Straighten (`Set`). Mirrored, not applied:
    /// the frame belongs to the tool's [`CropSession`]. Not a document step.
    Frame(EditEvent<CropBox>),
    /// Crop the document (Enter, a double click inside a drawn frame, the check mark, Image > Crop,
    /// leaving the tool with a drawn frame): one document step. The app then calls [`CropTool::applied`].
    Apply(CropCommit),
    /// Esc with no drag, the cancel button, or a frame of another document: the frame is gone.
    Discard,
    /// O, Shift+O, H, P, X or I changed the options: the app stores the new settings.
    Settings(CropSettings),
    /// A Straighten line set the frame's angle (degrees, clockwise on screen) and fitted the frame
    /// to the canvas, as one box step; Straighten is disarmed.
    Straightened(f64),
    /// The app's Crop To... command opens its form ([`crop_to_form`]) at this frame.
    CropTo(CropBox),
}

/// What a pass of the tool reports, from [`CropToolPass::finish`]. The state the app reads after
/// the pass ([`CropTool::frame`], [`shows_beyond_canvas`](CropTool::shows_beyond_canvas),
/// [`dragging`](CropTool::dragging)) is on the tool.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CropOutput {
    /// What the pass reported, in order.
    pub events: Vec<CropEvent>,
    /// The snap lines of the part being dragged, for the app's smart guides.
    pub snapped: Vec<SnapLine>,
}

/// One pass's input: the options and the document, and how the frame snaps.
pub struct CropInput<'a> {
    ctx: &'a egui::Context,
    settings: &'a CropSettings,
    document: &'a CropDocument,
    snap: Option<&'a SnapTargets>,
    nudge_pause: Option<f64>,
}

impl<'a> CropInput<'a> {
    /// The pass on `document` with `settings`; `ctx` mints the ids of the arrow-key nudge gestures the tool opens.
    pub fn new(ctx: &'a egui::Context, settings: &'a CropSettings, document: &'a CropDocument) -> Self {
        Self { ctx, settings, document, snap: None, nudge_pause: None }
    }

    /// The targets a dragged part and a Straighten line's ends snap to (the app's snap data).
    #[must_use]
    pub fn snap(mut self, snap: &'a SnapTargets) -> Self {
        self.snap = Some(snap);
        self
    }

    /// The app's Nudge Bundle Pause in seconds: arrow nudges each within this of the one before are
    /// one box step. `None` (the default) makes every nudge its own step.
    #[must_use]
    pub fn nudge_pause(mut self, seconds: Option<f64>) -> Self {
        self.nudge_pause = seconds;
        self
    }
}

/// The Crop tool, kept by the app while the tool is picked and between picks.
///
/// ```ignore
/// let document = CropDocument::new(CropDocKey::new(1), Size::new(1200.0, 800.0), 300.0);
/// let mut pass = crop.pass(CropInput::new(&ctx, &settings, &document));
/// let shown = Canvas::new("Document", pass.command_form()).show(ui, |ui, body| pass.show_frame(ui, body.rect));
/// let host = ToolHost::new().run(ui, &shown.output, &mut pass);
/// for event in pass.finish().events {
///     match event {
///         CropEvent::Apply(commit) => { /* run the crop command, then */ crop.applied(true); }
///         CropEvent::Settings(next) => settings = next,
///         _ => {}
///     }
/// }
/// ```
///
/// - Pointer (Photoshop's rules, as [`CropOverlay`] has them): with no frame or inside the untouched
///   default frame a drag draws a frame; inside a frame a drag moves it; on an edge or corner it
///   resizes along the frame's own axes; outside it turns the frame about its centre. A double click
///   inside a drawn frame applies.
/// - Image mode (Classic Mode off, the default): the frame stays upright and the gestures move and
///   turn the image; the tool sets the turned view and the app adopts
///   [`ToolHostResponse::view`](craft_ui::canvas::ToolHostResponse::view). Classic Mode: the frame
///   moves and turns over a fixed image.
/// - Keys while the canvas has the keyboard and no drag runs: Enter applies; Esc discards; arrows
///   nudge the whole frame 1 unit, Shift 10 (in image mode they move the image); X swaps portrait and
///   landscape (and W with H in W x H x Resolution); O cycles the overlay, Shift+O its orientation, H
///   toggles Show Cropped Area (taken but inert in Classic Mode), P toggles Classic Mode; I is Front
///   Image. The tool keeps the plain keys only while a frame exists and no drag runs.
/// - Straighten: [`toggle_straighten`](Self::toggle_straighten) (the bar's button) arms one line;
///   Cmd (Ctrl) held is a temporary Straighten.
/// - Box Undo and Redo: the app routes Edit > Undo and Redo to [`undo`](Self::undo) and
///   [`redo`](Self::redo) while [`CropSession::owns_undo`].
/// - Leaving: [`leave`](Self::leave) answers `None` while a drag runs; the app calls it again on
///   later passes until it returns something else.
#[derive(Clone, Debug, Default)]
pub struct CropTool {
    session: CropSession,
    /// When the latest arrow nudge ran, while its box step is still open.
    last_nudge: Option<f64>,
    /// The overlay reported a drag of the frame this pass.
    dragging: bool,
    /// The frame the image-mode view was last set for: a frame changed outside a gesture (undo,
    /// refit, keys) turns the view on the next pass.
    synced: Option<CropBox>,
    /// The mode the last pass saw (Classic Mode on or off): a change brings the view to the new mode's.
    classic_seen: Option<bool>,
    /// The frame, box steps and view of the last Apply, until [`applied`](Self::applied); true when
    /// the Apply came from [`leave`](Self::leave).
    kept: Option<(CropKept, bool)>,
    /// The view a gesture of the overlay moved, for the next [`CanvasTool::each_pass`] to hand to the host.
    view: Option<CanvasView>,
    /// The view the last pass saw, kept for a refused image-mode commit.
    seen_view: Option<CanvasView>,
    /// The name of a box step made by a key, from the words of the last pass.
    step: Arc<str>,
}

impl CropTool {
    /// A tool with no frame.
    pub fn new() -> Self {
        Self::default()
    }

    /// The pending frame and its history.
    pub fn session(&self) -> &CropSession {
        &self.session
    }

    /// The frame after the last pass: the default one or the drawn one; `None` without a document.
    pub fn frame(&self) -> Option<CropBox> {
        self.session.frame()
    }

    /// The frame has been pressed since it was made: the app shows the layers' pixels past the
    /// canvas edges under the shield (Photoshop's crop preview).
    pub fn shows_beyond_canvas(&self) -> bool {
        self.session.editing()
    }

    /// A drag of the frame runs: Auto Adjust Opacity lightens the shield.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// This pass's tool for the host, borrowing `input`'s data.
    pub fn pass<'a>(&'a mut self, input: CropInput<'a>) -> CropToolPass<'a> {
        CropToolPass { tool: self, input, output: CropOutput::default() }
    }

    /// The bar's Straighten button is pressed (armed for one line).
    pub fn straighten_armed(&self) -> bool {
        self.session.straighten().armed()
    }

    /// The bar's Straighten button: arms one line, or disarms it.
    pub fn toggle_straighten(&mut self) {
        self.session.straighten_mut().toggle();
    }

    /// Edit > Undo while the box owns Undo ([`CropSession::owns_undo`]): one box step back. False when
    /// there was nothing to undo. In image mode the next pass turns the view with the frame. The app
    /// cancels a running drag first.
    pub fn undo(&mut self) -> bool {
        self.last_nudge = None;
        let stepped = self.session.undo();
        if stepped {
            self.synced = None;
        }
        stepped
    }

    /// Edit > Redo while the box owns Undo: as [`undo`](Self::undo), forward.
    pub fn redo(&mut self) -> bool {
        self.last_nudge = None;
        let stepped = self.session.redo();
        if stepped {
            self.synced = None;
        }
        stepped
    }

    /// Ends a run of arrow nudges: records them as one box step.
    fn end_nudges(&mut self) {
        self.last_nudge = None;
        self.session.settle();
    }

    /// The bar changed the ratio or W x H x Resolution ([`CropBarOutput::ratio`](super::CropBarOutput::ratio)):
    /// the frame refits to [`CropSettings::ratio_value`] about its centre, from the frame the field edit
    /// began with while `editing`, as one box step. Returns the frame's `Set` for the host's mirror, or
    /// `None` when nothing changed. In image mode the next pass turns the view with the frame.
    pub fn refit(&mut self, settings: &CropSettings, document: &CropDocument, units: &Units, editing: bool) -> Option<CropEvent> {
        let base = self.session.ratio_base(editing);
        self.end_nudges();
        if self.dragging {
            return None;
        }
        let (current, base) = (self.session.frame()?, base?);
        let rect = fitted_ratio(base.rect, settings.ratio_value(document, units)?)?;
        let next = CropBox::new(rect, base.angle);
        // W x H x Resolution can change the output size even when the frame already fits, so it
        // makes the frame a pending crop.
        let sized = settings.target(document, units).is_some();
        if next == current && (editing || !sized) {
            return None;
        }
        // Fitting the untouched default frame to a ratio alone is not an edit.
        let keep_default = !editing && !sized && self.session.is_default();
        let event = self.session.set(next, keep_default, &self.step);
        self.synced = None;
        Some(CropEvent::Frame(event))
    }

    /// The Crop To... command's answer: the frame is set as one box step. Returns the frame's `Set`,
    /// `None` for a frame that is not finite or has no area.
    pub fn crop_to(&mut self, frame: CropBox) -> Option<CropEvent> {
        if !frame.is_finite() || frame.rect.width() <= 0.0 || frame.rect.height() <= 0.0 {
            return None;
        }
        self.end_nudges();
        let event = self.session.set(frame, false, &self.step);
        self.synced = None;
        Some(CropEvent::Frame(event))
    }

    /// The check mark in the bar or Image > Crop: [`CropEvent::Apply`] for a drawn frame, `None` for
    /// the default frame or no frame.
    pub fn apply(&mut self, settings: &CropSettings, document: &CropDocument, units: &Units) -> Option<CropEvent> {
        if !self.session.pending() {
            return None;
        }
        let frame = self.session.frame()?;
        self.end_nudges();
        let view = self.seen_view.filter(|_| !settings.shield.classic_mode);
        self.kept = Some((self.session.keep(view), false));
        Some(CropEvent::Apply(CropCommit::new(frame, settings, document, units)))
    }

    /// The app's Crop To... command fired: [`CropEvent::CropTo`] at the frame, for the app to open
    /// [`crop_to_form`]; `None` without a frame.
    pub fn crop_to_command(&self) -> Option<CropEvent> {
        self.session.frame().map(CropEvent::CropTo)
    }

    /// The cancel button: [`CropEvent::Discard`] and the session cleared (the next pass makes a new
    /// default frame); `None` without a frame.
    pub fn discard(&mut self) -> Option<CropEvent> {
        self.session.frame()?;
        self.reset();
        Some(CropEvent::Discard)
    }

    /// The app picked another tool (not a held temporary one): drops an untouched default frame and
    /// applies a drawn one (`keeps_pending`: the pen's eraser end stands in for the tool; a running
    /// drag also keeps the frame). Disarms Straighten. While a drag runs the frame stays pending and
    /// this returns `None`: the app calls it again on later passes until it returns something else,
    /// which applies the frame once the drag ends.
    pub fn leave(&mut self, settings: &CropSettings, document: &CropDocument, units: &Units, keeps_pending: bool) -> Option<CropEvent> {
        self.end_nudges();
        match self.session.leave(keeps_pending || self.dragging) {
            LeaveFrame::Kept | LeaveFrame::Nothing => None,
            LeaveFrame::Dropped => {
                self.reset();
                None
            }
            LeaveFrame::Apply(frame) => {
                self.kept = Some((self.session.keep(None), true));
                Some(CropEvent::Apply(CropCommit::new(frame, settings, document, units)))
            }
        }
    }

    /// The app ran its crop command for the last [`CropEvent::Apply`]: `true` clears the session,
    /// `false` (refused) gives the frame and its box steps back; the image-mode view kept with them goes
    /// to the host on the next pass. An Apply that came from [`leave`](Self::leave) and is refused clears
    /// the session instead, so the next tool does not retry it.
    pub fn applied(&mut self, accepted: bool) {
        let kept = self.kept.take();
        if accepted || kept.as_ref().is_some_and(|(_, on_leave)| *on_leave) {
            self.reset();
            return;
        }
        let Some((kept, _)) = kept else { return };
        self.view = self.session.restore(kept);
        // The restored view comes with the frame: nothing to turn.
        self.synced = self.session.frame();
    }

    /// The session, the kept frame and the view's sync are dropped; the next pass makes a new default frame.
    fn reset(&mut self) {
        self.session.clear();
        self.dragging = false;
        self.synced = None;
        self.kept = None;
        self.last_nudge = None;
    }
}

/// The Crop tool for one pass of the host ([`CropTool::pass`]).
pub struct CropToolPass<'a> {
    tool: &'a mut CropTool,
    input: CropInput<'a>,
    output: CropOutput,
}

impl CropToolPass<'_> {
    /// What the pass reported: its events in order.
    pub fn finish(self) -> CropOutput {
        self.output
    }

    /// Shows the session's frame as one [`CropOverlay`] at `canvas_rect`, in the canvas body, with this
    /// pass's options, and turns what it reports into events. Shows nothing without a frame.
    pub fn show_frame(&mut self, ui: &mut Ui, canvas_rect: Rect) {
        let words = Env::of(ui).words::<CropWords>();
        self.tool.step = Arc::from(words.frame.as_str());
        let (settings, document) = (self.input.settings, self.input.document);
        let units = Env::of(ui).units();
        let (ratio, default, straighten) =
            (settings.ratio_value(document, &units), self.tool.session.is_default(), self.tool.session.straighten().on());
        let Some(frame) = self.tool.session.frame_mut() else { return };
        let mut overlay = CropOverlay::new(words.frame.clone(), Bind::bound(frame))
            .guide((settings.guide, settings.guide_orientation))
            .guide_show(settings.guide_show)
            .shield(settings.shield.clone());
        if let Some(ratio) = ratio {
            overlay = overlay.ratio(ratio);
        }
        if default {
            overlay = overlay.default_frame();
        }
        if straighten {
            overlay = overlay.straighten();
        }
        if settings.delete_pixels {
            overlay = overlay.bounds(document.canvas());
        }
        if !settings.shield.classic_mode {
            overlay = overlay.image_mode();
        }
        if settings.shield.auto_center_preview {
            overlay = overlay.auto_center();
        }
        if let Some(snap) = self.input.snap {
            overlay = overlay.snap(snap);
        }
        let shown = overlay.show_at(ui, canvas_rect);
        self.take(shown, &units);
    }

    /// Turns the overlay's output into events and session steps.
    fn take(&mut self, shown: CropOverlayOutput, units: &Units) {
        let CropOverlayOutput { edit, apply, discard, straightened, view, snapped, dragging, .. } = shown;
        self.tool.dragging = dragging;
        self.output.snapped = snapped;
        if let Some(view) = view {
            self.tool.view = Some(view);
        }
        for event in edit.events {
            self.tool.session.mirror(&event);
            self.tool.synced = self.tool.session.frame();
            self.push(CropEvent::Frame(event));
        }
        if let Some(angle) = straightened {
            self.straightened(angle);
        }
        if apply && let Some(event) = self.tool.apply(self.input.settings, self.input.document, units) {
            self.push(event);
        }
        if discard && let Some(event) = self.tool.discard() {
            self.push(event);
        }
    }

    /// A Straighten line levelled the frame by `angle`: the frame is fitted to the canvas as one box
    /// step, and the one line is spent.
    fn straightened(&mut self, angle: f64) {
        self.tool.session.straighten_mut().disarm();
        let Some(frame) = self.tool.session.frame() else { return };
        if let Some(next) = frame.fitted(self.input.document.size, angle) {
            let event = self.tool.session.set(next, false, &self.tool.step);
            self.tool.synced = None;
            self.push(CropEvent::Frame(event));
            self.push(CropEvent::Straightened(angle));
        }
    }

    fn classic(&self) -> bool {
        self.input.settings.shield.classic_mode
    }

    fn push(&mut self, event: CropEvent) {
        self.output.events.push(event);
    }

    /// Sets the frame from a key: one box step, or a bundled nudge; the view follows in image mode.
    fn key_frame(&mut self, next: CropBox, nudge: bool, view: Option<CanvasView>, cx: &mut ToolContext<'_>) {
        let bundled = nudge && self.input.nudge_pause.is_some();
        let event = if bundled {
            self.tool.last_nudge = Some(cx.time);
            self.tool.session.nudge(self.input.ctx, next, &self.tool.step)
        } else {
            self.tool.end_nudges();
            self.tool.session.set(next, false, &self.tool.step)
        };
        self.push(CropEvent::Frame(event));
        match view {
            Some(view) => {
                cx.set_view(view);
                self.tool.synced = Some(next);
            }
            None => self.tool.synced = None,
        }
    }

    /// An arrow moves the whole frame by one step on screen (the image under the frame in image mode).
    fn nudge(&mut self, frame: CropBox, direction: Vec2, mods: Mods, cx: &mut ToolContext<'_>) {
        let distance = if mods.shift { NUDGE_FAR } else { NUDGE };
        let by = DocVec::new(f64::from(direction.x), f64::from(direction.y)) * distance;
        let space = cx.space();
        let by = space.map_or(by, |space| screen_step(&space.view, by));
        let image = !self.classic();
        let moved = frame.moved(if image { -by } else { by });
        let next = self.within(moved, frame, false, &cx.units());
        let view = space.filter(|_| image).map(|space| {
            let mut view = space.view;
            view.center += next.center() - frame.center();
            view
        });
        if next != frame {
            self.key_frame(next, true, view, cx);
        }
    }

    /// `next` kept inside the canvas while Delete Cropped Pixels is on, else `frame` as it was.
    fn within(&self, next: CropBox, frame: CropBox, resizing: bool, units: &Units) -> CropBox {
        let settings = self.input.settings;
        if !next.is_finite() {
            return frame;
        }
        if !settings.delete_pixels {
            return next;
        }
        let held = settings.ratio_value(self.input.document, units).is_some();
        next.within(self.input.document.canvas(), resizing, held).unwrap_or(frame)
    }

    /// X: swaps portrait and landscape, and W with H in W x H x Resolution.
    fn swap(&mut self, frame: CropBox, cx: &mut ToolContext<'_>) {
        let (settings, document) = (self.input.settings, self.input.document);
        let units = cx.units();
        let mut next = frame.swapped();
        let mut swapped = settings.clone();
        if swapped.swap(document) && swapped != *settings {
            if let Some(rect) = swapped.ratio_value(document, &units).and_then(|k| fitted_ratio(next.rect, k)) {
                next = CropBox::new(rect, next.angle);
            }
            self.push(CropEvent::Settings(swapped));
        }
        let next = self.within(next, frame, true, &units);
        let view = cx.space().filter(|_| !self.classic() && settings.shield.auto_center_preview).map(|space| {
            let mut view = crop_view(&next, &space.view);
            view.center = next.center();
            view
        });
        self.key_frame(next, false, view, cx);
    }
}

impl CanvasTool for CropToolPass<'_> {
    /// Drops a frame of another document ([`CropEvent::Discard`]), makes the default frame, reads Cmd
    /// for a temporary Straighten, hands the view a gesture moved to the host, and in image mode sets
    /// the view for a frame changed outside a gesture.
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        let document = self.input.document;
        let nudging = matches!((self.tool.last_nudge, self.input.nudge_pause), (Some(at), Some(pause)) if cx.time - at < pause);
        if !nudging {
            self.tool.end_nudges();
        }
        if self.tool.session.for_document(document.key) {
            self.tool.reset();
            self.push(CropEvent::Discard);
        }
        self.tool.session.straighten_mut().set_held(cx.mods.cmd);
        let dragging = self.tool.dragging;
        if self.tool.session.ensure_frame(document, dragging) {
            self.tool.synced = None;
        }
        if let Some(view) = self.tool.view.take() {
            cx.set_view(view);
        }
        self.tool.seen_view = cx.space().map(|space| space.view);
        if dragging {
            return;
        }
        let Some(frame) = self.tool.session.frame() else { return };
        let classic = self.classic();
        if self.tool.classic_seen != Some(classic) {
            self.tool.synced = None;
            self.tool.classic_seen = Some(classic);
        }
        let Some(space) = cx.space() else { return };
        if classic {
            // The camera is upright and the frame turns on it (never back to a Rotate View turn).
            if self.tool.synced.is_none() && space.view.rotation != 0.0 {
                cx.set_view(space.view.rotation(0.0));
            }
            self.tool.synced = Some(frame);
        } else if self.tool.synced != Some(frame) {
            cx.set_view(crop_view(&frame, &space.view));
            self.tool.synced = Some(frame);
        }
    }

    /// The keyboard, while a finite frame stands and no gesture runs.
    fn claims(&self) -> ToolClaims {
        let open = !self.tool.dragging && self.tool.session.frame().is_some_and(|frame| frame.is_finite());
        if open { ToolClaims::NONE.keyboard() } else { ToolClaims::NONE }
    }

    /// A press that reaches the tool is on no part of the frame: the overlay owns the whole canvas.
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    /// Enter applies, Esc discards, the arrows nudge the whole frame; O, Shift+O, H, P, X and I as the
    /// type's doc says. Returns `false` for a key the tool leaves to the app.
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        if self.tool.dragging {
            return false;
        }
        let (settings, document) = (self.input.settings, self.input.document);
        let frame = self.tool.session.frame().filter(CropBox::is_finite);
        let mut next = settings.clone();
        match key {
            ToolKey::Submit if frame.is_some() => {
                if let Some(event) = self.tool.apply(settings, document, &cx.units()) {
                    self.push(event);
                }
                return true;
            }
            ToolKey::Cancel if frame.is_some() => {
                if let Some(event) = self.tool.discard() {
                    self.push(event);
                }
                return true;
            }
            ToolKey::Nudge { direction, mods, .. } => {
                let Some(frame) = frame else { return false };
                self.nudge(frame, *direction, *mods, cx);
                return true;
            }
            ToolKey::Kept(chord) => {
                let plain = chord.mods.is_none();
                match chord.key {
                    Key::I if plain => next.front_image(document),
                    Key::O if frame.is_some() && plain => next.cycle_guide(),
                    Key::O if frame.is_some() && chord.mods == Mods::SHIFT => next.cycle_orientation(),
                    // Show Cropped Area is taken but inert in Classic Mode, where it acts as on.
                    Key::H if frame.is_some() && plain => next.shield.show_cropped_area ^= !next.shield.classic_mode,
                    Key::P if frame.is_some() && plain => next.shield.classic_mode = !next.shield.classic_mode,
                    Key::X if plain => {
                        let Some(frame) = frame else { return false };
                        self.swap(frame, cx);
                        return true;
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
        if next != *settings {
            self.push(CropEvent::Settings(next));
        }
        true
    }

    /// The overlay paints the frame, its guide and the shield.
    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Crop To... command")
    }
}

/// The Crop To... command's form: X, Y, W, H and Angle of `frame`, in document units. The answer goes
/// to [`CropTool::crop_to`].
pub fn crop_to_form(words: &CropWords, frame: &CropBox) -> ParamDialog<'static> {
    let rect = frame.rect.abs();
    let params = vec![
        Param::number("x", words.crop_to_x.clone(), rect.x0, 1.0, 2),
        Param::number("y", words.crop_to_y.clone(), rect.y0, 1.0, 2),
        Param::number("width", words.crop_to_width.clone(), rect.width(), 1.0, 2).bounds(Some(1.0), None),
        Param::number("height", words.crop_to_height.clone(), rect.height(), 1.0, 2).bounds(Some(1.0), None),
        Param::new("angle", words.crop_to_angle.clone(), ParamKind::angle(frame.angle)),
    ];
    ParamDialog::command(CROP_TO_KEY, words.crop_to.clone(), params)
}

/// Largest centred rectangle of `ratio` (width over height) contained in `rect`; never adds
/// area. `None` for a rect or a ratio that is not a positive finite number.
fn fitted_ratio(rect: DocRect, ratio: f64) -> Option<DocRect> {
    let r = rect.abs();
    let (w, h) = (r.width(), r.height());
    let finite = [r.x0, r.y0, r.x1, r.y1, ratio].iter().all(|v| v.is_finite());
    if !finite || w <= 0.0 || h <= 0.0 || ratio <= 0.0 {
        return None;
    }
    if ((w / h) / ratio - 1.0).abs() < 1e-12 {
        return Some(r);
    }
    let c = r.center();
    let (w, h) = if w / h > ratio { (h * ratio, h) } else { (w, w / ratio) };
    let fitted = DocRect::new(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0);
    (fitted.x1 > fitted.x0 && fitted.y1 > fitted.y0 && [fitted.x0, fitted.y0, fitted.x1, fitted.y1].iter().all(|v| v.is_finite())).then_some(fitted)
}

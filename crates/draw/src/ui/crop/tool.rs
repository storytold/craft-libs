//! [`CropTool`]: the Crop tool on the canvas tool host. The app owns one `CropTool`. Each pass it shows
//! the frame as one [`CropOverlay`] in the canvas body ([`CropTool::show_frame`]), then builds a
//! [`CropToolPass`] from the tool, the pass's input ([`CropInput`]) and what the body reported
//! ([`CropShown`]). The pass takes the keys the overlay does not (the canvas has the keyboard, no part
//! is focused).

use std::sync::Arc;

use craft_behavior::geometry::crop::normalized_angle;
use craft_behavior::geometry::{CropBox, SnapLine, crop_view};
use craft_behavior::kurbo::Size;
use craft_behavior::{CancelReason, CanvasView, EditEvent};
use craft_keymap::{Chord, Key, Mods};
use craft_ui::canvas::{CanvasTool, CropApply, CropOverlay, CropOverlayOutput, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::patterns::form_dialogs::{Param, ParamDialog, ParamKind};
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::runtime::pointer;
use egui::{Pos2, Rect, Ui};

use super::options::{CropFill, CropRatio, CropSettings, CropTarget};
use super::session::{CropDocKey, CropDocument, CropKept, CropSession, LeaveFrame, default_box, fitted_ratio};
use super::words::CropWords;

/// The key of the Crop To... command form.
const CROP_TO_KEY: &str = "draw.crop.cropTo";

/// The chords the tool takes while a crop is pending and no drag runs: Front Image, Swap, the overlay and its
/// orientation, Show Cropped Area and Classic Mode. Over the untouched default frame (also a selection's) every key
/// stays the app's, as in Photoshop: O and I pick their tools, X swaps the colours.
const PENDING_KEYS: &[Chord] =
    &[Chord::plain(Key::I), Chord::plain(Key::X), Chord::plain(Key::O), Chord::new(Mods::SHIFT, Key::O), Chord::plain(Key::H), Chord::plain(Key::P)];

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
    /// What the overlay applied for (Enter, a double click, a click away), so the app can let the selection go on
    /// a click away; `None` for the check mark, Image > Crop and leaving the tool.
    pub cause: Option<CropApply>,
}

impl CropCommit {
    /// The commit of `frame` with `settings` on `document`, with the environment's `units`; it has no `cause`.
    pub fn new(frame: CropBox, settings: &CropSettings, document: &CropDocument, units: &Units) -> Self {
        Self { frame, target: settings.target(document, units), delete_pixels: settings.delete_pixels, fill: settings.fill, cause: None }
    }

    /// The size the document has after the crop: the target's, else the frame's.
    fn size(&self) -> Size {
        match self.target {
            Some(target) => Size::new(target.width, target.height),
            None => Size::new(self.frame.rect.width(), self.frame.rect.height()),
        }
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
    /// Esc with no drag, the cancel button, or a frame of another document: the frame is gone. Names
    /// the document the frame was made for.
    Discard(CropDocKey),
    /// O, Shift+O, H, P, X or I changed the options: the app stores the new settings.
    Settings(CropSettings),
    /// A Straighten line set the frame's angle (degrees, clockwise on screen), as one box step:
    /// fitted to the canvas when armed, else turned in place. Straighten is disarmed.
    Straightened(f64),
    /// The app's Crop To... command opens its form ([`crop_to_form`]) at this frame.
    CropTo(CropBox),
}

/// What the frame's overlay reported in the canvas body ([`CropTool::show_frame`]): its events, the snap lines of the
/// part being dragged and the pointer it asked to scroll at. The pass of the same frame takes it
/// ([`CropTool::pass`]), so the body's report reaches the host in that pass.
#[derive(Debug, Default)]
#[must_use = "hand it to this pass's `CropTool::pass`"]
pub struct CropShown {
    events: Vec<CropEvent>,
    snapped: Vec<SnapLine>,
    scrolls_at: Option<Pos2>,
    context: Option<(Pos2, Mods)>,
}

impl CropShown {
    /// Where a secondary press on the frame's overlay was released, with the modifiers held, taken once: the host opens
    /// its canvas menu there, as it does for a secondary press the canvas's own tool refuses.
    pub fn take_context(&mut self) -> Option<(Pos2, Mods)> {
        self.context.take()
    }
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

/// One pass's input: the options and the document.
pub struct CropInput<'a> {
    ctx: &'a egui::Context,
    settings: &'a CropSettings,
    document: &'a CropDocument,
    nudge_pause: Option<f64>,
}

impl<'a> CropInput<'a> {
    /// The pass on `document` with `settings`; `ctx` mints the ids of the arrow-key nudge gestures the tool opens.
    pub fn new(ctx: &'a egui::Context, settings: &'a CropSettings, document: &'a CropDocument) -> Self {
        Self { ctx, settings, document, nudge_pause: None }
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
/// let input = CropInput::new(&ctx, &settings, &document);
/// let mut frame = None;
/// let mut pass = None;
/// let mut host = None;
/// let _ = Canvas::new("Document").show(ui, |ui, body| {
///     // The canvas controls first, then the host on the canvas's pass.
///     frame = Some(crop.show_frame(ui, body.rect, &input));
///     if let Some(canvas) = tool_pass(&body.env) {
///         let mut tool = crop.pass(input, frame.take());
///         host = Some(ToolHost::new().run(ui, &canvas, &mut tool));
///         pass = Some(tool);
///     }
/// });
/// for event in pass.map(|pass| pass.finish().events).unwrap_or_default() {
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
/// - Keys while no drag runs. The frame's overlay holds the session: with the canvas or a frame part
///   focused, or nothing focused, Enter applies, Esc discards and the arrows nudge the whole frame (the
///   picked part when one has focus) 1 unit, Shift 10 (in image mode they move the image). While the
///   canvas has the keyboard: X swaps portrait and landscape (and W with H in W x H x Resolution); O
///   cycles the overlay, Shift+O its orientation, H toggles Show Cropped Area (taken but inert in
///   Classic Mode), P toggles Classic Mode; I is Front Image. The tool keeps these keys only while a crop
///   is pending (a frame drawn or changed) and no drag runs: over the untouched default frame they stay
///   the app's, as in Photoshop.
/// - Straighten: [`toggle_straighten`](Self::toggle_straighten) (the bar's button) arms one line;
///   Cmd (Ctrl) held is a temporary Straighten.
/// - Box Undo and Redo: the app routes Edit > Undo and Redo to [`undo`](Self::undo) and
///   [`redo`](Self::redo) while [`CropSession::owns_undo`].
/// - Leaving: [`leave`](Self::leave) ends a drag that still runs at its last frame and applies a
///   drawn frame.
#[derive(Clone, Debug, Default)]
pub struct CropTool {
    session: CropSession,
    /// When the latest arrow nudge ran, while its box step is still open.
    last_nudge: Option<f64>,
    /// The overlay reported a drag of the frame this pass.
    dragging: bool,
    /// The frame the image-mode view was last set for. A box step (Undo, Redo) keeps it, so the next
    /// pass moves the view with the frame's centre and the frame stays put on screen; a frame set anew
    /// (refit, Reset, Crop To, Straighten) clears it, and the next pass turns the view about the frame.
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

    /// This pass's tool for the host, borrowing `input`'s data. `shown` is what [`show_frame`](Self::show_frame)
    /// reported in this pass's canvas body; `None` when the body showed no frame. Its events come first in the
    /// pass's output, and the host takes its snap lines and scroll request.
    pub fn pass<'a>(&'a mut self, input: CropInput<'a>, shown: Option<CropShown>) -> CropToolPass<'a> {
        let CropShown { events, snapped, scrolls_at, .. } = shown.unwrap_or_default();
        CropToolPass { tool: self, input, output: CropOutput { events, snapped }, scrolls_at }
    }

    /// Shows the session's frame as one [`CropOverlay`] at `canvas_rect`, in the canvas body, with the pass's
    /// options, and turns what it reports into events for this pass's [`pass`](Self::pass). Shows nothing without a
    /// frame.
    pub fn show_frame(&mut self, ui: &mut Ui, canvas_rect: Rect, input: &CropInput<'_>) -> CropShown {
        let words = Env::of(ui).words::<CropWords>();
        self.step = Arc::from(words.frame.as_str());
        let (settings, document) = (input.settings, input.document);
        let units = Env::of(ui).units();
        // Cmd (Ctrl) held is a temporary Straighten. It is read here, before the overlay declares its parts, so the
        // Straighten surface is there for a press on the pass after the key came down; a line in progress keeps the
        // state it started with, so letting go of the key mid-line does not turn it into a frame drag.
        if !self.dragging {
            self.session.straighten_mut().set_held(pointer::mods(ui.ctx()).cmd);
        }
        let (ratio, default, straighten, dismissed) =
            (settings.ratio_value(document, &units), self.session.is_default(), self.session.straighten().on(), self.session.dismissed());
        let Some(frame) = self.session.frame_mut() else { return CropShown::default() };
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
        if settings.fill == CropFill::ContentAware {
            overlay = overlay.fills_uncovered();
        }
        if settings.image_mode() {
            overlay = overlay.image_mode();
        }
        if settings.shield.auto_center_preview {
            overlay = overlay.auto_center();
        }
        if dismissed {
            overlay = overlay.hidden();
        }
        let shown = overlay.show_at(ui, canvas_rect);
        self.take(shown, input, &units, ui.input(|i| i.time))
    }

    /// Turns the overlay's output into events and session steps. An arrow nudge joins the open run of nudges while
    /// the Nudge Bundle Pause is set; any other edit is its own box step. It applies only this pass's output: a
    /// second pass of the frame reads its own overlay output and never repeats an edit, a Straighten line or a
    /// scroll request.
    fn take(&mut self, shown: CropOverlayOutput, input: &CropInput<'_>, units: &Units, time: f64) -> CropShown {
        let CropOverlayOutput { edit, apply, discard, straightened, straighten_ended, view, snapped, dragging, nudged, scrolls_at, context, .. } =
            shown;
        let mut out = CropShown { events: Vec::new(), snapped, scrolls_at, context };
        self.dragging = dragging;
        if let Some(view) = view {
            self.view = Some(view);
        }
        if !edit.events.is_empty() {
            // The first edit of a hidden default frame draws a new frame.
            self.session.show_frame();
        }
        for event in edit.events {
            match &event {
                EditEvent::Set { after, .. } if nudged && input.nudge_pause.is_some() => {
                    self.last_nudge = Some(time);
                    self.session.nudge(input.ctx, *after, &self.step);
                }
                _ => {
                    self.end_nudges();
                    self.session.mirror(&event);
                }
            }
            self.synced = self.session.frame();
            out.events.push(CropEvent::Frame(event));
        }
        if straightened.is_some() || straighten_ended {
            // The button was armed when the line began: a line made with Cmd alone turns the frame in place.
            let fit = self.session.straighten().armed();
            self.session.show_frame();
            // A click with no line spends the armed button too.
            self.session.straighten_mut().disarm();
            if let Some(angle) = straightened {
                self.straightened(angle, fit, input.document, &mut out.events);
            }
        }
        if let Some(cause) = apply
            && let Some(event) = self.apply_for(Some(cause), input.settings, input.document, units)
        {
            out.events.push(event);
        }
        if discard && let Some(event) = self.discard() {
            out.events.push(event);
        }
        out
    }

    /// A Straighten line levelled the frame by `angle`, as one box step: with `fit` (the button was armed)
    /// the frame is fitted to `document`'s canvas, else (Cmd held) it keeps its size and centre and turns in place.
    fn straightened(&mut self, angle: f64, fit: bool, document: &CropDocument, events: &mut Vec<CropEvent>) {
        let Some(frame) = self.session.frame() else { return };
        let next = if fit { frame.fitted(document.size, angle) } else { Some(CropBox::new(frame.rect, normalized_angle(angle))) };
        if let Some(next) = next {
            let event = self.session.set(next, false, &self.step);
            self.synced = None;
            events.push(CropEvent::Frame(event));
            events.push(CropEvent::Straightened(angle));
        }
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
    /// there was nothing to undo. In image mode the next pass moves and turns the view with the frame,
    /// which stays put on screen. The app cancels a running drag first.
    pub fn undo(&mut self) -> bool {
        self.last_nudge = None;
        self.session.undo()
    }

    /// Edit > Redo while the box owns Undo: as [`undo`](Self::undo), forward.
    pub fn redo(&mut self) -> bool {
        self.last_nudge = None;
        self.session.redo()
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
        self.session.show_frame();
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
        // The untouched selection frame leaves its state with the first refit, even when it already fits.
        let selection = self.session.from_selection();
        if next == current && !selection && (editing || !sized) {
            return None;
        }
        // Fitting the untouched default frame to a ratio alone is not an edit.
        let keep_default = !editing && !sized && self.session.is_default();
        let event = self.session.set(next, keep_default, &self.step);
        self.synced = None;
        Some(CropEvent::Frame(event))
    }

    /// Reset: the frame returns to the default frame of the document (the selection's bounds, else the
    /// canvas) fitted to the ratio, upright, as one box step, and Straighten is disarmed. `None` when the
    /// frame is not [`modified`](Self::modified).
    pub fn reset_frame(&mut self, settings: &CropSettings, document: &CropDocument, units: &Units) -> Option<CropEvent> {
        if !self.modified() {
            return None;
        }
        self.end_nudges();
        let (next, _) = default_box(document, settings.ratio_value(document, units))?;
        let event = self.session.reset(next, &self.step);
        self.synced = None;
        Some(CropEvent::Frame(event))
    }

    /// The frame differs from what Reset makes: it is turned, or edited since it was made or reset.
    /// False without a frame, while a drag runs and while the default frame is dismissed.
    pub fn modified(&self) -> bool {
        let Some(frame) = self.session.frame() else { return false };
        if self.dragging || self.session.dismissed() {
            return false;
        }
        frame.angle != 0.0 || (!self.session.is_default() && self.session.reset_to() != Some(frame))
    }

    /// The preset list's Selection entry is the chosen one: the untouched default frame is the
    /// selection's bounds and the ratio is free.
    pub fn shows_selection_entry(&self, settings: &CropSettings) -> bool {
        self.session.is_default() && self.session.from_selection() && settings.ratio == CropRatio::Free
    }

    /// The Selection entry was picked: the selection's frame is set as a drawn frame, one box step.
    /// `None` unless the untouched default frame is the selection's, or while a drag runs.
    pub fn take_selection_frame(&mut self) -> Option<CropEvent> {
        if self.dragging || !self.session.is_default() || !self.session.from_selection() {
            return None;
        }
        let frame = self.session.frame()?;
        self.end_nudges();
        self.session.show_frame();
        let event = self.session.set(frame, false, &self.step);
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

    /// The check mark in the bar, Enter or Image > Crop: [`CropEvent::Apply`] for a drawn frame or the
    /// untouched selection frame, `None` for another default frame or no frame.
    pub fn apply(&mut self, settings: &CropSettings, document: &CropDocument, units: &Units) -> Option<CropEvent> {
        self.apply_for(None, settings, document, units)
    }

    /// [`CropTool::apply`] for what the overlay reported: the commit carries `cause`.
    fn apply_for(&mut self, cause: Option<CropApply>, settings: &CropSettings, document: &CropDocument, units: &Units) -> Option<CropEvent> {
        if !self.session.applies() {
            return None;
        }
        let frame = self.session.frame()?;
        self.end_nudges();
        let view = self.seen_view.filter(|_| settings.image_mode());
        let commit = CropCommit { cause, ..CropCommit::new(frame, settings, document, units) };
        self.kept = Some((self.session.keep(view, commit.size()), false));
        Some(CropEvent::Apply(commit))
    }

    /// The app's Crop To... command fired: [`CropEvent::CropTo`] at the frame, for the app to open
    /// [`crop_to_form`]; `None` without a frame.
    pub fn crop_to_command(&self) -> Option<CropEvent> {
        self.session.frame().map(CropEvent::CropTo)
    }

    /// The cancel button: [`CropEvent::Discard`], the session cleared and Straighten disarmed; the next
    /// pass makes a default frame that stays hidden ([`CropSession::dismiss`]). `None` without a frame
    /// and while the default frame is already hidden.
    pub fn discard(&mut self) -> Option<CropEvent> {
        self.session.frame()?;
        if self.session.dismissed() {
            return None;
        }
        let document = self.session.frame_document()?;
        self.dismiss();
        Some(CropEvent::Discard(document))
    }

    /// The app picked another tool (not a held temporary one): drops an untouched default frame and
    /// applies a drawn one (`keeps_pending`: the pen's eraser end stands in for the tool). Disarms
    /// Straighten. A drag that still runs ends at the frame of its last update, as one box step; the
    /// overlay is not shown after the tool changes, so its press ends with the tool.
    pub fn leave(&mut self, settings: &CropSettings, document: &CropDocument, units: &Units, keeps_pending: bool) -> Option<CropEvent> {
        self.end_nudges();
        if self.dragging {
            self.session.finish_drag();
            self.dragging = false;
        }
        match self.session.leave(keeps_pending) {
            LeaveFrame::Kept | LeaveFrame::Nothing => None,
            LeaveFrame::Dropped => {
                self.reset();
                None
            }
            LeaveFrame::Apply(frame) => {
                let commit = CropCommit::new(frame, settings, document, units);
                self.kept = Some((self.session.keep(None, commit.size()), true));
                Some(CropEvent::Apply(commit))
            }
        }
    }

    /// The app ran its crop command for the last [`CropEvent::Apply`]: `true` clears the session,
    /// `false` (refused) gives the frame and its box steps back; the image-mode view kept with them goes
    /// to the host on the next pass. An accepted crop hides the default frame ([`CropSession::dismiss`]).
    /// An Apply that came from [`leave`](Self::leave) clears the session, accepted or refused, so the next
    /// tool does not retry it.
    pub fn applied(&mut self, accepted: bool) {
        let kept = self.kept.take();
        if kept.as_ref().is_some_and(|(_, on_leave)| *on_leave) {
            self.reset();
            return;
        }
        if accepted {
            match kept {
                Some((kept, _)) => self.session.dismiss_cropped(kept.crops_to()),
                None => self.session.dismiss(),
            }
            self.forget();
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
        self.forget();
    }

    /// As [`reset`](Self::reset), and the new default frame stays hidden until the next press.
    fn dismiss(&mut self) {
        self.session.dismiss();
        self.forget();
    }

    /// The tool's own state that follows a cleared session.
    fn forget(&mut self) {
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
    /// The pointer the body's overlay asked to scroll at ([`CropShown`]), for [`CanvasTool::each_pass`] to hand to
    /// the host.
    scrolls_at: Option<Pos2>,
}

impl CropToolPass<'_> {
    /// What the pass reported: its events in order.
    pub fn finish(self) -> CropOutput {
        self.output
    }

    /// Image mode: the gestures move and turn the view; the frame stays upright.
    fn image_mode(&self) -> bool {
        self.input.settings.image_mode()
    }

    fn push(&mut self, event: CropEvent) {
        self.output.events.push(event);
    }

    /// Sets the frame from a key: one box step; the view follows in image mode.
    fn key_frame(&mut self, next: CropBox, view: Option<CanvasView>, cx: &mut ToolContext<'_>) {
        self.tool.end_nudges();
        let event = self.tool.session.set(next, false, &self.tool.step);
        self.push(CropEvent::Frame(event));
        match view {
            Some(view) => {
                cx.set_view(view);
                self.tool.synced = Some(next);
            }
            None => self.tool.synced = None,
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
        if next == frame {
            // A square frame has nothing to swap: no box step.
            return;
        }
        let view = cx.space().filter(|_| self.image_mode() && settings.shield.auto_center_preview).map(|space| {
            let mut view = crop_view(&next, &space.view);
            view.center = next.center();
            view
        });
        self.key_frame(next, view, cx);
    }
}

impl CanvasTool for CropToolPass<'_> {
    /// Drops a frame of another document ([`CropEvent::Discard`]), makes the default frame, hands the
    /// view a gesture moved to the host, and in image mode sets the view for a frame changed outside a gesture.
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        if let Some(pointer) = self.scrolls_at.take() {
            cx.autoscroll_at(pointer);
        }
        cx.add_snapped(self.output.snapped.iter().cloned());
        let document = self.input.document;
        let nudging = matches!((self.tool.last_nudge, self.input.nudge_pause), (Some(at), Some(pause)) if cx.time - at < pause);
        if !nudging {
            self.tool.end_nudges();
        }
        if let Some(dropped) = self.tool.session.for_document(document.key) {
            self.tool.reset();
            self.push(CropEvent::Discard(dropped));
        }
        let dragging = self.tool.dragging;
        let ratio = self.input.settings.ratio_value(document, &cx.units());
        if self.tool.session.ensure_frame(document, dragging, ratio) {
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
        let classic = !self.image_mode();
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
            // From the frame the view shows (a box step), the view follows the frame's centre so the frame stays put.
            let mut view = space.view;
            if let Some(shown) = self.tool.synced {
                view.center += frame.center() - shown.center();
            }
            cx.set_view(crop_view(&frame, &view));
            self.tool.synced = Some(frame);
        }
    }

    /// The chords the tool takes while a crop is pending with a finite frame and no gesture runs: I, X, O,
    /// Shift+O, H and P. Every other key, and every key over the untouched default frame, stays the app's.
    fn claims(&self) -> ToolClaims {
        let open = !self.tool.dragging && self.tool.session.pending() && self.tool.session.frame().is_some_and(|frame| frame.is_finite());
        if !open {
            return ToolClaims::NONE;
        }
        ToolClaims::NONE.keys(PENDING_KEYS)
    }

    /// A press that reaches the tool is on no part of the frame: the overlay owns the whole canvas.
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    /// O, Shift+O, H, P, X and I as the type's doc says, only while a crop is pending; the frame's overlay takes
    /// Enter, Esc and the arrows before the tool. Returns `false` for a key the tool leaves to the app.
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        if self.tool.dragging || !self.tool.session.pending() {
            return false;
        }
        let Some(frame) = self.tool.session.frame().filter(CropBox::is_finite) else { return false };
        let (settings, document) = (self.input.settings, self.input.document);
        let mut next = settings.clone();
        match key {
            ToolKey::Kept(chord) => {
                let plain = chord.mods.is_none();
                match chord.key {
                    Key::I if plain => next.front_image(document),
                    Key::O if plain => next.cycle_guide(),
                    Key::O if chord.mods == Mods::SHIFT => next.cycle_orientation(),
                    // Show Cropped Area is taken but inert in Classic Mode, where it acts as on.
                    Key::H if plain => next.shield.show_cropped_area ^= next.image_mode(),
                    Key::P if plain => next.shield.classic_mode = !next.shield.classic_mode,
                    Key::X if plain => {
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

#[cfg(feature = "suite")]
pub(crate) mod stories;

//! [`CropSession`]: the Crop tool's pending frame and everything around it. The tool always shows
//! a frame (the selection's bounds or the canvas, marked default), keeps the box's own Undo and
//! Redo while a crop is pending, decides what leaving the tool does, and gives the frame back when
//! the app refuses a commit. Plain data: no egui state, no document access.

use std::sync::Arc;

use craft_behavior::edit_history::{EditHistory, Step};
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Rect, Size};
use craft_behavior::{CanvasView, EditEvent, GestureId, Mixed};
use craft_ui::runtime::edit::next_gesture_id;

/// The app's identity of a document (its id, never reused while the app runs). A frame belongs
/// to the document it was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct CropDocKey(pub u64);

impl CropDocKey {
    /// The key of document `id`.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// The document a crop tool's pending work belongs to: the Crop tool's frame, the Perspective Crop tool's quad.
/// Switching documents drops that work (as Photoshop discards a pending crop), and this one rule says when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct DocOwner(Option<CropDocKey>);

impl DocOwner {
    /// Work held for `held`, `None` for no document.
    pub(super) const fn of(held: Option<CropDocKey>) -> Self {
        Self(held)
    }

    /// The document the work belongs to.
    pub(super) const fn key(self) -> Option<CropDocKey> {
        self.0
    }

    /// The document the work belongs to when that is another one than `key`: the work is stale and goes.
    pub(super) fn stale_for(self, key: CropDocKey) -> Option<CropDocKey> {
        self.0.filter(|held| *held != key)
    }
}

/// What the Crop tool needs to know of the active document each pass.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CropDocument {
    /// Identifies the document, so the tool keeps a frame per document.
    pub key: CropDocKey,
    /// The canvas size in document units (pixels).
    pub size: Size,
    /// The document's resolution in px/in.
    pub resolution: f64,
    /// The selection's bounds clipped to the canvas, when there is a selection.
    pub selection: Option<Rect>,
}

impl CropDocument {
    /// Document `key` of `size` at `resolution` px/in, no selection.
    pub fn new(key: CropDocKey, size: Size, resolution: f64) -> Self {
        Self { key, size, resolution, selection: None }
    }

    /// The resolution in px/in.
    pub fn resolution(mut self, ppi: f64) -> Self {
        self.resolution = ppi;
        self
    }

    /// The selection's bounds, clipped to the canvas by the tool.
    pub fn selection(mut self, bounds: Option<Rect>) -> Self {
        self.selection = bounds;
        self
    }

    /// The canvas: `(0, 0)` to [`size`](Self::size).
    pub fn canvas(&self) -> Rect {
        self.size.to_rect()
    }
}

/// One state of the box's own history: the frame and whether it is the untouched default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CropStep {
    /// The frame, or `None` when there is none.
    pub frame: Option<CropBox>,
    /// Whether the frame is the untouched default.
    pub default_frame: bool,
    /// Whether the untouched default frame is the document's selection (the Selection entry of the
    /// preset list is then the chosen one). Always false once the frame is not the default.
    pub from_selection: bool,
}

/// Straighten: armed by the bar's button for one line, or held with ⌘ (Ctrl) for as long as the
/// key is down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StraightenState {
    armed: bool,
    held: bool,
}

impl StraightenState {
    /// The bar's button: arms Straighten for one line, or disarms it.
    pub fn toggle(&mut self) {
        self.armed = !self.armed;
    }

    /// One line was drawn, Esc was pressed or another tool was picked.
    pub fn disarm(&mut self) {
        self.armed = false;
    }

    /// ⌘ (Ctrl) is held over the canvas this pass: a temporary Straighten.
    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// The bar's button is on (its pressed look).
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// The next press draws a Straighten line.
    pub fn on(&self) -> bool {
        self.armed || self.held
    }
}

/// What leaving the Crop tool does to the frame ([`CropSession::leave`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LeaveFrame {
    /// No frame, or the session was already cleared.
    Nothing,
    /// The untouched default frame was dropped.
    Dropped,
    /// A drawn frame: the app crops with it as Enter would, then calls
    /// [`CropSession::clear`] (or [`restore`](CropSession::restore) when the crop is refused, and
    /// then clear, so the next tool does not retry it).
    Apply(CropBox),
    /// The pen's eraser end stands in for the tool: the frame stays pending. A running gesture does
    /// not keep it: [`CropTool::leave`](super::CropTool::leave) ends the gesture first.
    Kept,
}

/// The pending frame, its box history and the view, kept while the app's crop command runs, so a
/// refused commit gives them back ([`CropSession::restore`]).
#[derive(Clone, Debug)]
pub struct CropKept {
    step: CropStep,
    history: Option<EditHistory<CropStep>>,
    settled: CropStep,
    view: Option<CanvasView>,
    frame_for: Option<(CropDocKey, Size)>,
    editing: bool,
    /// The size the document has once the crop is made.
    crops_to: Size,
}

impl CropKept {
    /// The size the document has once the crop is made.
    pub(super) fn crops_to(&self) -> Size {
        self.crops_to
    }
}

/// The pending frame of the Crop tool. The tool ([`CropTool`](super::CropTool)) owns one and drives it.
///
/// - The default frame: [`ensure_frame`](Self::ensure_frame) frames the selection's bounds, else
///   the canvas, marked default, whenever there is no frame. A drag inside the untouched default
///   frame draws a new one; applying it does nothing. Another document, or the same one resized,
///   gets its own default frame.
/// - Box Undo and Redo: an [`EditHistory`] of [`CropStep`]s fed with the overlay's edit events
///   ([`mirror`](Self::mirror)). Each committed gesture, key step, ratio refit and Straighten is one
///   step; the app routes Edit > Undo and Redo here while the box [owns Undo](Self::owns_undo).
///   The view is not in the steps: Undo never moves it.
/// - Leaving the tool ([`leave`](Self::leave)) drops an untouched default frame and applies a
///   drawn one.
/// - Dismissal ([`dismiss`](Self::dismiss)): after Discard or an accepted crop the default frame
///   is still made but stays hidden until the next press, ratio change or size change
///   ([`show_frame`](Self::show_frame)), for that document and size only.
/// - [`CropTool`](super::CropTool) is the one writer: the session has no edit type of its own.
#[derive(Clone, Debug, Default)]
pub struct CropSession {
    frame: Option<CropBox>,
    default_frame: bool,
    /// The document and size the frame was made for.
    frame_for: Option<(CropDocKey, Size)>,
    /// Pressed since it was made: the canvas shows what lies past its edges.
    editing: bool,
    history: Option<EditHistory<CropStep>>,
    /// The step as of the latest history entry: what a cancelled drag goes back to.
    settled: CropStep,
    /// The gesture a run of arrow nudges keeps open, until [`settle`](Self::settle).
    nudging: Option<GestureId>,
    straighten: StraightenState,
    /// The frame a ratio-field edit began with.
    ratio_base: Option<CropBox>,
    /// The default frame is the selection's bounds.
    from_selection: bool,
    /// The document and size whose default frame was dismissed (Discard, an accepted crop).
    dismissed: Option<(CropDocKey, Size)>,
    /// The frame the last Reset made, to tell a frame that is back at its default from one that
    /// was edited.
    reset_to: Option<CropBox>,
    /// The overlay gesture that runs and the frame of its last update, for
    /// [`finish_drag`](Self::finish_drag).
    drag: Option<(GestureId, CropBox)>,
}

/// The default frame of `document`: the selection's bounds clipped to the canvas, else the
/// canvas, fitted to `ratio` about its centre. The flag is true when the selection's bounds were
/// used. `None` for a canvas without area.
pub(super) fn default_box(document: &CropDocument, ratio: Option<f64>) -> Option<(CropBox, bool)> {
    let canvas = document.canvas();
    let usable = |r: &Rect| r.width() > 0.0 && r.height() > 0.0;
    if !usable(&canvas) {
        return None;
    }
    let selection = document.selection.map(|s| s.intersect(canvas)).filter(usable);
    let rect = selection.unwrap_or(canvas);
    let rect = ratio.and_then(|k| fitted_ratio(rect, k)).unwrap_or(rect);
    Some((CropBox::upright(rect), selection.is_some()))
}

/// Largest centred rectangle of `ratio` (width over height) contained in `rect`; never adds
/// area. `None` for a rect or a ratio that is not a positive finite number.
pub(super) fn fitted_ratio(rect: Rect, ratio: f64) -> Option<Rect> {
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
    let fitted = Rect::new(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0);
    (fitted.x1 > fitted.x0 && fitted.y1 > fitted.y0 && [fitted.x0, fitted.y0, fitted.x1, fitted.y1].iter().all(|v| v.is_finite())).then_some(fitted)
}

impl CropSession {
    fn current(&self) -> CropStep {
        CropStep { frame: self.frame, default_frame: self.default_frame, from_selection: self.from_selection }
    }

    fn adopt(&mut self, step: CropStep) {
        self.frame = step.frame;
        self.default_frame = step.default_frame;
        self.from_selection = step.from_selection;
    }

    /// The step a gesture or `Set` that leaves the frame at `after` settles: the last step when
    /// nothing changed (a click keeps the default mark), else a drawn frame.
    fn step_after(&self, after: &CropBox) -> CropStep {
        if self.settled.frame.as_ref() == Some(after) {
            self.settled
        } else {
            CropStep { frame: Some(*after), default_frame: false, from_selection: false }
        }
    }

    /// Records the end of gesture `id` at `after` as one box step.
    fn commit(&mut self, id: GestureId, after: &CropBox) {
        let step = self.step_after(after);
        self.adopt(step);
        self.record(EditEvent::commit(id, Mixed::Same(self.settled), step));
    }

    fn history(&mut self) -> &mut EditHistory<CropStep> {
        let now = self.current();
        self.history.get_or_insert_with(|| EditHistory::new(&now))
    }

    /// Feeds `event` to the history and notes the step it settles.
    fn record(&mut self, event: EditEvent<CropStep>) {
        if let EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } = &event {
            self.settled = *after;
        }
        self.history().record(&event);
    }

    /// A session with no frame.
    pub fn new() -> Self {
        Self::default()
    }

    /// The frame, default or drawn.
    pub fn frame(&self) -> Option<CropBox> {
        self.frame
    }

    /// The frame the overlay edits in place.
    pub(super) fn frame_mut(&mut self) -> Option<&mut CropBox> {
        self.frame.as_mut()
    }

    /// The frame is the untouched default one.
    pub fn is_default(&self) -> bool {
        self.default_frame
    }

    /// A crop is pending: a frame that was drawn, moved, resized, turned or keyed, not the
    /// untouched default. Photoshop greys most menu commands out then; Edit > Undo steps the box.
    pub fn pending(&self) -> bool {
        self.frame.is_some() && !self.default_frame
    }

    /// The frame was pressed since it was made: the app shows the layers' pixels past the canvas
    /// edges under the shield.
    pub fn editing(&self) -> bool {
        self.editing && self.frame.is_some()
    }

    /// Drops the frame, its history and Straighten when they belong to another document than
    /// `key` (as Photoshop: switching documents discards the pending crop). Returns true when a
    /// drawn frame was dropped, so the tool reports [`CropEvent::Discard`](super::CropEvent): the document whose
    /// drawn frame it dropped.
    pub fn for_document(&mut self, key: CropDocKey) -> Option<CropDocKey> {
        let dropped = DocOwner::of(self.frame_document()).stale_for(key)?;
        let drawn = self.pending();
        self.clear();
        drawn.then_some(dropped)
    }

    /// The document the frame was made for.
    pub(super) fn frame_document(&self) -> Option<CropDocKey> {
        self.frame_for.map(|(held, _)| held)
    }

    /// Makes the default frame when there is none and no gesture runs (`dragging` false): the
    /// selection's bounds clipped to the canvas when they are not empty, else the canvas; an empty
    /// canvas gets none. A default frame made for another size is replaced. A new default frame
    /// starts a new box history, upright. Returns true when it made one.
    pub fn ensure_frame(&mut self, document: &CropDocument, dragging: bool, ratio: Option<f64>) -> bool {
        let made_for = Some((document.key, document.size));
        if self.default_frame && self.frame_for != made_for && !dragging {
            self.frame = None;
            self.default_frame = false;
        }
        if self.frame.is_some() || dragging {
            return false;
        }
        if self.dismissed.is_some_and(|held| Some(held) != made_for) {
            self.dismissed = None;
        }
        let Some((frame, from_selection)) = default_box(document, ratio) else { return false };
        self.frame = Some(frame);
        self.from_selection = from_selection;
        self.default_frame = true;
        self.frame_for = made_for;
        self.editing = false;
        self.settled = self.current();
        self.nudging = None;
        self.history = Some(EditHistory::new(&self.settled));
        true
    }

    /// Takes in the overlay's edit of the frame, which the overlay has already written to the frame:
    /// a gesture ends the default mark and marks the frame as pressed; one that changes nothing, or is
    /// cancelled, leaves the last step's frame and mark. Each committed gesture and each `Set` is one
    /// box step.
    pub fn mirror(&mut self, event: &EditEvent<CropBox>) {
        self.settle();
        match event {
            EditEvent::Begin { id, label, .. } => {
                if let Some(frame) = self.frame {
                    self.editing = true;
                    self.default_frame = false;
                    self.from_selection = false;
                    self.drag = Some((*id, frame));
                }
                self.record(EditEvent::begin(*id, Mixed::Same(self.settled), Arc::clone(label), None));
            }
            EditEvent::Update { value, .. } => {
                if let Some((_, last)) = self.drag.as_mut() {
                    *last = *value;
                }
            }
            EditEvent::Commit { id, after, .. } => {
                self.drag = None;
                self.commit(*id, after);
            }
            EditEvent::Cancel { id, reason, .. } => {
                self.drag = None;
                self.adopt(self.settled);
                self.record(EditEvent::cancel(*id, *reason, None));
            }
            EditEvent::Set { after, label, .. } => {
                let step = self.step_after(after);
                self.adopt(step);
                self.editing |= !step.default_frame;
                self.record(EditEvent::set(Mixed::Same(self.settled), step, Arc::clone(label), None));
            }
            _ => {}
        }
    }

    /// A key, refit or Straighten set the frame: one box step named `label`, reported as the `Set` the
    /// tool mirrors. The frame is no longer the default unless `keep_default` (a refit of the untouched
    /// default frame to a ratio alone). Either way it is no longer the selection's bounds.
    pub fn set(&mut self, frame: CropBox, keep_default: bool, label: &str) -> EditEvent<CropBox> {
        self.settle();
        // The history starts from the frame as it is now: made after the change, it would take the new frame as its
        // first state and record no step.
        self.history();
        let before = self.frame.map_or(Mixed::Mixed, Mixed::Same);
        self.frame = Some(frame);
        self.from_selection = false;
        if !keep_default {
            self.default_frame = false;
        }
        self.editing |= !self.default_frame;
        let step = self.current();
        self.record(EditEvent::set(Mixed::Same(self.settled), step, label.into(), None));
        EditEvent::set(before, frame, label.into(), None)
    }

    /// An arrow nudge set the frame: it joins the run of nudges, one open box step that
    /// [`settle`](Self::settle) closes. Reported as a `Set`, as [`set`](Self::set).
    pub fn nudge(&mut self, ctx: &egui::Context, frame: CropBox, label: &str) -> EditEvent<CropBox> {
        let before = self.frame.map_or(Mixed::Mixed, Mixed::Same);
        self.frame = Some(frame);
        self.default_frame = false;
        self.from_selection = false;
        self.editing = true;
        if self.nudging.is_none() {
            let id = next_gesture_id(ctx);
            self.nudging = Some(id);
            self.record(EditEvent::begin(id, Mixed::Same(self.settled), label.into(), None));
        }
        EditEvent::set(before, frame, label.into(), None)
    }

    /// Ends the overlay gesture that still runs (the tool was left mid-drag) at the frame of its
    /// last update, as one box step: the step the release would have recorded. False when no
    /// gesture runs.
    pub(super) fn finish_drag(&mut self) -> bool {
        let Some((id, after)) = self.drag.take() else { return false };
        self.settle();
        self.commit(id, &after);
        true
    }

    /// The frame is the untouched default and is the selection's bounds.
    pub fn from_selection(&self) -> bool {
        self.from_selection
    }

    /// Enter, the check mark and Image > Crop have a crop to make: a pending frame, or the
    /// untouched default frame that is the selection's bounds (not dismissed).
    pub fn applies(&self) -> bool {
        self.pending() || (self.frame.is_some() && self.default_frame && self.from_selection && self.dismissed.is_none())
    }

    /// The default frame was dismissed (Discard, an accepted crop) and stays hidden.
    pub fn dismissed(&self) -> bool {
        self.dismissed.is_some()
    }

    /// Discard or an accepted crop: the session is cleared and the document's default frame stays
    /// hidden until [`show_frame`](Self::show_frame). The next [`ensure_frame`](Self::ensure_frame) still makes
    /// the frame, so the hidden overlay has one to show.
    pub fn dismiss(&mut self) {
        let made_for = self.frame_for;
        self.clear();
        self.dismissed = made_for;
    }

    /// An accepted crop: like [`dismiss`](Self::dismiss), for the document at the size the crop
    /// makes (`crops_to`), so the frame stays hidden on the cropped document and a later size change
    /// shows it.
    pub fn dismiss_cropped(&mut self, crops_to: Size) {
        let made_for = self.frame_for;
        self.clear();
        self.dismissed = made_for.map(|(key, _)| (key, crops_to));
    }

    /// The first frame edit from the hidden overlay, a ratio or size change or a Straighten press:
    /// the default frame shows again.
    pub fn show_frame(&mut self) {
        self.dismissed = None;
    }

    /// The frame the last `reset` made, while no other step replaced it.
    pub fn reset_to(&self) -> Option<CropBox> {
        self.reset_to
    }

    /// Reset: `frame` (the default frame at angle 0) as one box step, Straighten disarmed.
    pub(super) fn reset(&mut self, frame: CropBox, label: &str) -> EditEvent<CropBox> {
        self.straighten.disarm();
        let event = self.set(frame, false, label);
        self.reset_to = Some(frame);
        event
    }

    /// Ends a run of arrow nudges: records them as one box step.
    pub fn settle(&mut self) {
        if let Some(id) = self.nudging.take() {
            let step = self.current();
            self.record(EditEvent::commit(id, Mixed::Same(self.settled), step));
        }
    }

    /// A run of nudges is open.
    pub fn nudging(&self) -> bool {
        self.nudging.is_some()
    }

    /// Edit > Undo while the box owns Undo ([`owns_undo`](Self::owns_undo)): the frame one box
    /// step back, read through [`frame`](Self::frame). False, and nothing changed, when there is
    /// nothing to undo. The app cancels a running drag first.
    pub fn undo(&mut self) -> bool {
        self.step(Step::Undo)
    }

    /// Edit > Redo while the box owns Undo: as [`undo`](Self::undo), forward.
    pub fn redo(&mut self) -> bool {
        self.step(Step::Redo)
    }

    fn step(&mut self, step: Step) -> bool {
        self.settle();
        let mut now = self.current();
        let Some(history) = self.history.as_mut() else { return false };
        if !history.step(&mut now, step) {
            return false;
        }
        self.adopt(now);
        self.settled = now;
        true
    }

    /// Whether Edit > Undo has a box step to go to: a step on the stack, or a run of nudges not yet
    /// recorded.
    pub fn can_undo(&self) -> bool {
        self.nudging.is_some() || self.history.as_ref().is_some_and(EditHistory::can_undo)
    }

    /// Whether Edit > Redo has a box step to go to.
    pub fn can_redo(&self) -> bool {
        self.history.as_ref().is_some_and(EditHistory::can_redo)
    }

    /// The app routes Edit > Undo and Redo to the box, not the document, while either has a step.
    /// It is not [`pending`](Self::pending): after Undo back to the default frame Redo still steps
    /// the box, and a stack with nothing left greys the item out instead of offering the
    /// document's step.
    pub fn owns_undo(&self) -> bool {
        self.can_undo() || self.can_redo()
    }

    /// Another tool was picked. `keeps_pending`: a gesture on the frame still runs, or the pen's
    /// eraser end stands in for the Crop tool; the held temporary tools (Space, Cmd) never count as
    /// leaving and never call this. Disarms Straighten. An untouched default frame is dropped
    /// whatever `keeps_pending` says. For [`LeaveFrame::Kept`] the app calls `leave` again on
    /// later passes until it returns something else.
    pub fn leave(&mut self, keeps_pending: bool) -> LeaveFrame {
        self.straighten.disarm();
        let Some(frame) = self.frame else { return LeaveFrame::Nothing };
        if self.default_frame {
            self.clear();
            return LeaveFrame::Dropped;
        }
        if keeps_pending {
            return LeaveFrame::Kept;
        }
        self.editing = false;
        LeaveFrame::Apply(frame)
    }

    /// Drops the frame, its history, the default mark, the editing state and Straighten: Esc, the
    /// discard button, an applied crop. With the tool still picked, the next
    /// [`ensure_frame`](Self::ensure_frame) makes a new default frame.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Keeps the frame, the box history and `view` (an image-mode view) before the app runs its
    /// crop command, with the size `crops_to` the crop makes.
    pub fn keep(&self, view: Option<CanvasView>, crops_to: Size) -> CropKept {
        CropKept {
            step: self.current(),
            history: self.history.clone(),
            settled: self.settled,
            view,
            frame_for: self.frame_for,
            editing: self.editing,
            crops_to,
        }
    }

    /// The crop command refused the frame: the frame, its default mark and its box history come
    /// back, and the view kept with them for the host to adopt.
    pub fn restore(&mut self, kept: CropKept) -> Option<CanvasView> {
        self.adopt(kept.step);
        self.history = kept.history;
        self.settled = kept.settled;
        self.nudging = None;
        self.drag = None;
        self.frame_for = kept.frame_for;
        self.editing = kept.editing;
        kept.view
    }

    /// The frame a ratio-field edit fits from: the frame when the edit began (`editing` true on
    /// the first change of the run), kept until the run ends (`editing` false), so 1 : 3 on the way
    /// to 16 : 3 does not shrink the frame first. The untouched selection frame fits from the
    /// canvas, not from the selection. `None` without a frame.
    pub fn ratio_base(&mut self, editing: bool) -> Option<CropBox> {
        let start =
            if self.default_frame && self.from_selection { self.frame_for.map(|(_, size)| CropBox::upright(size.to_rect())) } else { self.frame };
        if editing {
            if self.ratio_base.is_none() {
                self.ratio_base = start;
            }
            self.ratio_base
        } else {
            self.ratio_base = None;
            start
        }
    }

    /// Straighten's state.
    pub fn straighten(&self) -> &StraightenState {
        &self.straighten
    }

    /// Straighten's state, mutably.
    pub fn straighten_mut(&mut self) -> &mut StraightenState {
        &mut self.straighten
    }
}

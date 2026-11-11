//! [`PenTool`]: the path being drawn with the Pen and the Curvature Pen, a canvas tool.

use std::mem;
use std::sync::Arc;

use super::auto::AutoEdit;
use super::bezier::{EditSubpath, HandleSide, Knot, KnotRef};
use super::{PathOptionsSettings, PenMode, PenWords};
use craft_behavior::geometry::{Directions, lock_direction};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{CancelReason, CanvasView, Cursor, CursorBadge, EditEvent, GestureId, Mixed, PaintedCursor, PointerButton};
use craft_keymap::Mods;
use craft_theme::Rgba;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolAnchor, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::canvas_controls::path_runs;
use craft_ui::primitives::{Dashes, Draw, PaintShape, StrokeWidth, ToolIcon};
use craft_ui::style::{HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Vec2};

/// The side of the box a tool icon is drawn on; an icon's hotspot is in these units.
const ICON_BOX: f32 = 24.0;

/// What the pen reports to the host, drained with [`PenTool::take_events`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[must_use = "apply the edits the pen reports and stop handing the draft in on Finished"]
pub enum PenEvent {
    /// The draft's edits, each value the whole draft: `Begin` at its first change (a draft the host hands in begins
    /// at once), `Update` on each change, `Commit` when the pen ends a changed draft. A draft that ends with fewer
    /// than two anchors (one left at Enter or Escape, the only one removed) is no path: `Cancel` while the host
    /// holds none of it, else a `Commit` of the empty draft, which removes the path the host holds (a resumed
    /// path).
    Edit(EditEvent<EditSubpath>),
    /// The pen ended the draft (Enter, Escape, closing it, removing its only anchor, an interruption): the host
    /// stops handing it in. It follows the draft's `Commit` or `Cancel`.
    Finished,
    /// A press on the end point of an open path while no draft is open: the host makes that path's subpath the draft
    /// with the knot of that end last (reversing it for its first knot).
    Resumed(u64, KnotRef),
}

/// The pen draft's look in a paint call, from the theme and the Path Options.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PenStyle {
    /// The path so far and the rubber band: the Path Options' colour, else the overlay line.
    pub line: StyleColor,
    /// An anchor and a direction handle's end.
    pub handle: HandleLook,
    /// The last anchor's fill, and the fill of the first anchor while a press there closes the path.
    pub selected: StyleColor,
    /// Dash length of the rubber band.
    pub dash: f32,
    /// Gap length of the rubber band.
    pub gap: f32,
}

impl PenStyle {
    /// The look in `cx` with the colour of `options`. The tool reads it in `paint` only, from
    /// [`ToolContext::style`].
    pub fn of(cx: &StyleCx<'_>, options: &PathOptionsSettings) -> PenStyle {
        let tokens = cx.tokens;
        let line = match options.color {
            Some([r, g, b]) => cx.color(Rgba::rgb(r, g, b)),
            None => cx.color(tokens.color.overlay.line),
        };
        PenStyle { line, handle: cx.handle(), selected: cx.color(tokens.color.accent.solid), dash: tokens.stroke.dash, gap: tokens.stroke.dash_gap }
    }
}

/// What a press drags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// A new last anchor: a drag pulls out its handles, symmetric.
    Pull,
    /// A press on the first anchor: the path closes on release; a drag pulls out that anchor's handles.
    Close,
    /// A press on the last anchor, whose out handle it retracts: a drag sets that handle alone.
    Retract,
    /// ⌥ on one of the last anchor's handles: a drag moves it alone.
    Grab(HandleSide),
    /// Curvature mode: a drag moves the anchor at this index.
    Move(usize),
}

/// A press the tool owns.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    pub(super) origin: Pos2,
    pub(super) kind: Kind,
    /// Past the click slop.
    pub(super) moved: bool,
    /// The in handle as the last pass left it: ⌥ keeps it there while the out handle follows.
    pub(super) in_ctrl: Point,
}

/// The gesture of the draft: its id and the draft when it began.
#[derive(Clone, Debug)]
struct Run {
    id: GestureId,
    before: EditSubpath,
}

/// The path being drawn with the Pen: the host's open draft, its gestures and its keys. The tool reports the whole
/// draft as an [`EditSubpath`] in [`PenEvent`]s and the host keeps it from them.
///
/// - Pointer, with the Pen ([`PenMode::Pen`]): a click adds a corner anchor (⇧ on the nearest 45° ray from the last
///   one); a drag pulls out symmetric handles (⇧ at 45°); ⌥ during the drag breaks the pair, so only the out handle
///   follows. A press on the last anchor retracts its out handle and a drag sets it alone; ⌥ on one of its handles
///   grabs that handle alone. A press on the first anchor closes the path on release, a drag there pulling out that
///   anchor's handles. With no draft, a press on an open path's end point ([`ends`](Self::ends)) reports
///   [`PenEvent::Resumed`].
/// - Keys while a draft is open: Enter and Escape end it open and keep it (a draft of one anchor is discarded, as no
///   path has fewer than two anchors); Backspace and Delete remove the last anchor, the only one discarding the
///   draft; the host's Undo does the same through [`remove_last`](Self::remove_last). Each ending is
///   [`PenEvent::Finished`].
/// - Look: the path so far, a dashed rubber band from the last anchor to where a click would land (with Path
///   Options' Rubber Band), the last anchor's handles, the anchors with the last one filled, and over the first
///   anchor the close cue.
/// - The draft gesture cannot be cancelled: Escape ends the path and keeps it. An interrupted draft (a cancel for
///   any reason, such as a tool switch) commits: anchors are the user's work. A path option or focus change leaves
///   the draft open.
/// - [`PenMode::Curvature`] ([`mode`](Self::mode)): a click adds a smooth anchor the curve flows through, which
///   follows the pointer while the button is down (past the click slop); a press on the first anchor of 3 or more
///   closes at once; a press on another anchor grabs and drags it; the second click of a double click on an anchor
///   toggles it between smooth and corner. Every change reflows the handles ([`EditSubpath::reflow`]). ⇧ and ⌥ do
///   nothing; the keys and the gesture are the Pen's; there is no resume. Hover: with two or more anchors a dashed
///   preview of the curve the next click makes, with one a dashed line; always drawn, whatever the rubber band
///   setting.
/// - [`sensing`](Self::sensing) false: drawn, keys kept, no press taken (the host's Direct Selection acts).
///   [`auto_add_delete`](Self::auto_add_delete) (Pen mode): a press on a segment of the draft inserts an anchor, on
///   an anchor that is not the first or the last deletes it, each an `Update`; off while ⇧, ⌥ or ⌘ is held. The hover
///   shows what a press would do.
#[derive(Debug)]
pub struct PenTool {
    pub(super) mode: PenMode,
    pub(super) sensing: bool,
    pub(super) auto_add_delete: bool,
    options: PathOptionsSettings,
    ends: Vec<(u64, KnotRef, Point)>,
    /// The draft being drawn; empty between drafts.
    pub(super) shown: EditSubpath,
    run: Option<Run>,
    /// A draft the host handed in waits for the pass that begins its gesture.
    resume: bool,
    next: GestureId,
    label: Arc<str>,
    pub(super) drag: Option<Drag>,
    events: Vec<PenEvent>,
}

impl Default for PenTool {
    fn default() -> Self {
        Self::new()
    }
}

impl PenTool {
    /// A Pen with no draft, the Pen's mode, taking presses.
    pub fn new() -> Self {
        Self {
            mode: PenMode::default(),
            sensing: true,
            auto_add_delete: false,
            options: PathOptionsSettings::default(),
            ends: Vec::new(),
            shown: EditSubpath::default(),
            run: None,
            resume: false,
            next: GestureId::default(),
            label: Arc::from(""),
            drag: None,
            events: Vec::new(),
        }
    }

    /// The host's open draft. The tool keeps the draft it is drawing and reads this only while it has none: a
    /// non-empty draft with none open (a resumed path) becomes the draft and begins its gesture at once. Hand in
    /// `None` once [`PenEvent::Finished`] came.
    pub fn draft(&mut self, draft: Option<&EditSubpath>) -> &mut Self {
        if self.run.is_none() {
            match draft.filter(|draft| !draft.knots.is_empty()) {
                Some(draft) => {
                    self.shown = draft.clone();
                    self.resume = true;
                }
                None => {
                    self.shown = EditSubpath::default();
                    self.resume = false;
                }
            }
        }
        self
    }

    /// The Pen or the Curvature Pen; [`PenMode::Pen`] by default.
    pub fn mode(&mut self, mode: PenMode) -> &mut Self {
        self.mode = mode;
        self
    }

    /// Whether the tool takes presses; true by default. False draws the draft and keeps its keys (Enter, Escape,
    /// Backspace, Delete, the host's Undo) while another tool takes the pointer ([`PathMode::draft_senses`](super::PathMode::draft_senses)).
    pub fn sensing(&mut self, sensing: bool) -> &mut Self {
        self.sensing = sensing;
        self
    }

    /// The Pen's Auto Add/Delete on the open draft; false by default. No effect in Curvature mode.
    pub fn auto_add_delete(&mut self, on: bool) -> &mut Self {
        self.auto_add_delete = on;
        self
    }

    /// The user's Path Options: the draft line's colour and whether the Pen's rubber band shows (off by default, as
    /// Photoshop).
    pub fn path_options(&mut self, options: PathOptionsSettings) -> &mut Self {
        self.options = options;
        self
    }

    /// The end points of the open paths a press continues: the path's key, the end's knot and its document point.
    pub fn ends(&mut self, ends: &[(u64, KnotRef, Point)]) -> &mut Self {
        self.ends = ends.to_vec();
        self
    }

    /// Removes the draft's last anchor: the host's Undo while a draft is open. The only anchor discards the draft.
    /// Returns whether it did anything; a press that is still down keeps the request from running.
    pub fn remove_last(&mut self) -> bool {
        if self.shown.knots.is_empty() || self.drag.is_some() {
            return false;
        }
        self.remove_last_anchor(CancelReason::UndoDuringGesture);
        true
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<PenEvent> {
        mem::take(&mut self.events)
    }

    /// Whether a press at the pointer would close the path: sensing, no button down and the pointer over the first
    /// anchor of a draft that can close (2 or more anchors with the Pen, 3 or more in Curvature mode).
    pub fn closes(&self, cx: &ToolContext<'_>) -> bool {
        let (Some(space), Some(pointer)) = (cx.space(), cx.pointer) else { return false };
        self.sensing && self.drag.is_none() && self.closes_at(&self.shown, pointer.pos, &space.view, reach(cx))
    }

    // ---- the draft gesture ----

    fn begin(&mut self) -> GestureId {
        let id = self.next;
        self.next = id.next();
        let before = self.shown.clone();
        self.events.push(PenEvent::Edit(EditEvent::begin(id, Mixed::Same(before.clone()), Arc::clone(&self.label), None)));
        self.run = Some(Run { id, before });
        id
    }

    /// The draft now is `draft`: begins the gesture when none runs and reports an `Update`.
    pub(super) fn change(&mut self, draft: EditSubpath) {
        let id = match &self.run {
            Some(run) => run.id,
            None => self.begin(),
        };
        self.shown = draft.clone();
        self.events.push(PenEvent::Edit(EditEvent::update(id, draft)));
    }

    /// Commits the gesture with the draft as it is.
    pub(super) fn commit(&mut self) {
        if let Some(run) = self.run.take() {
            self.events.push(PenEvent::Edit(EditEvent::commit(run.id, Mixed::Same(run.before), self.shown.clone())));
        }
    }

    /// Ends a draft too short to be a path. The gesture's start is what the host holds of the draft: when that is
    /// empty a cancel leaves nothing behind; when it holds anchors (a resumed path) the empty draft is committed, so
    /// the host removes them.
    fn discard(&mut self, reason: CancelReason) {
        let Some(run) = self.run.take() else { return };
        if run.before.knots.is_empty() {
            self.events.push(PenEvent::Edit(EditEvent::cancel(run.id, reason, Some(Mixed::Same(run.before)))));
        } else {
            let empty = EditSubpath::default();
            self.events.push(PenEvent::Edit(EditEvent::update(run.id, empty.clone())));
            self.events.push(PenEvent::Edit(EditEvent::commit(run.id, Mixed::Same(run.before), empty)));
        }
    }

    /// The draft is over: reports `Finished` and starts clean.
    pub(super) fn finish(&mut self) {
        self.events.push(PenEvent::Finished);
        self.shown = EditSubpath::default();
        self.run = None;
        self.drag = None;
        self.resume = false;
    }

    /// Ends the draft open: a path of two anchors or more is kept, a shorter one discarded.
    fn end(&mut self, reason: CancelReason) {
        if self.shown.knots.len() < 2 {
            self.discard(reason);
        } else {
            self.commit();
        }
        self.finish();
    }

    fn remove_last_anchor(&mut self, reason: CancelReason) {
        if self.shown.knots.len() <= 1 {
            self.discard(reason);
            self.finish();
            return;
        }
        let mut draft = self.shown.clone();
        draft.knots.pop();
        if self.mode == PenMode::Curvature {
            // Removing the last anchor changes the curve through the rest.
            draft.reflow();
        }
        self.change(draft);
    }

    // ---- presses ----

    /// The press at `pos` with the Pen: Auto Add/Delete, the draft's own parts, a resumed path or a new anchor.
    fn press_pen(&mut self, pos: Pos2, mods: Mods, view: &CanvasView, reach: f32) -> ToolAction {
        let mut draft = self.shown.clone();
        // Auto Add/Delete takes the press whole: one Update, no drag, nothing added.
        if let Some(edit) = self.auto_edit(&draft, pos, mods, view, reach) {
            let changed = match edit {
                AutoEdit::Insert { segment, t } => draft.insert_at(segment, t).is_some(),
                AutoEdit::Delete { knot } => draft.remove_knot(knot),
            };
            if changed {
                self.change(draft);
            }
            return ToolAction::Done;
        }
        let near = |p: Point| view.to_screen(p).distance(pos) <= reach;
        let n = draft.knots.len();
        let handle = (n > 0 && mods.alt)
            .then(|| {
                let last = draft.knots[n - 1];
                [HandleSide::Out, HandleSide::In].into_iter().find(|&side| !last.retracted(side) && near(last.handle(side)))
            })
            .flatten();
        let kind = if let Some(side) = handle {
            Kind::Grab(side)
        } else if n >= 2 && !draft.closed && near(draft.knots[0].anchor) {
            Kind::Close
        } else if n >= 1 && near(draft.knots[n - 1].anchor) {
            Kind::Retract
        } else if n == 0
            && let Some(&(path, knot, _)) = self.ends.iter().find(|(_, _, p)| near(*p))
        {
            self.events.push(PenEvent::Resumed(path, knot));
            return ToolAction::Done;
        } else {
            Kind::Pull
        };
        let at = if kind == Kind::Close { 0 } else { n.saturating_sub(1) };
        let in_ctrl = match kind {
            Kind::Pull => {
                let mut p = view.to_doc(pos);
                if mods.shift && n > 0 {
                    p = draft.knots[n - 1].anchor + constrain(p - draft.knots[n - 1].anchor);
                }
                draft.knots.push(Knot::corner(p));
                p
            }
            Kind::Retract => {
                let k = &mut draft.knots[at];
                k.out_ctrl = k.anchor;
                k.smooth = false;
                k.in_ctrl
            }
            Kind::Close | Kind::Grab(_) | Kind::Move(_) => draft.knots[at].in_ctrl,
        };
        self.change(draft);
        self.drag = Some(Drag { origin: pos, kind, moved: false, in_ctrl });
        ToolAction::Claimed
    }

    /// One sample of a press the tool owns: past the click slop the held part follows the pointer.
    fn track(&mut self, pos: Pos2, mods: Mods, view: &CanvasView, slop: f32) {
        let Some(mut drag) = self.drag.take() else { return };
        drag.moved |= pos.distance(drag.origin) > slop;
        if drag.moved {
            let mut draft = self.shown.clone();
            let to = view.to_doc(pos);
            match drag.kind {
                Kind::Move(index) => {
                    if let Some(k) = draft.knots.get_mut(index) {
                        let by = to - k.anchor;
                        k.anchor += by;
                        k.in_ctrl += by;
                        k.out_ctrl += by;
                        draft.reflow();
                        self.change(draft);
                    }
                }
                kind => {
                    let at = if kind == Kind::Close { 0 } else { draft.knots.len().saturating_sub(1) };
                    if let Some(k) = draft.knots.get_mut(at) {
                        pull(k, kind, to, mods, &mut drag.in_ctrl);
                        self.change(draft);
                    }
                }
            }
        }
        self.drag = Some(drag);
    }

    // ---- reading the draft ----

    fn rubber_band(&self) -> bool {
        self.options.rubber_band
    }

    fn knot_name(words: &PenWords, i: usize, n: usize) -> String {
        words.anchor.fill(&[("i", &i.to_string()), ("n", &n.to_string())])
    }
}

/// How far from an anchor, a handle or an end point a press still takes it, in screen points.
pub(super) fn reach(cx: &ToolContext<'_>) -> f32 {
    cx.slop
}

/// `offset` turned onto the nearest 45° ray.
fn constrain(offset: DocVec) -> DocVec {
    lock_direction(offset, None, Directions::Eight)
}

/// A pulled knot: its handles follow `to`. A pull and a close drag put the out handle there and the in handle
/// opposite it, unless ⌥ keeps the in handle where it was; a retracted anchor's handle and a grabbed handle move
/// alone. ⇧ turns the handle to the nearest 45° ray.
fn pull(knot: &mut Knot, kind: Kind, to: Point, mods: Mods, in_ctrl: &mut Point) {
    let mut v = to - knot.anchor;
    if mods.shift {
        v = constrain(v);
    }
    match kind {
        Kind::Pull | Kind::Close => {
            knot.out_ctrl = knot.anchor + v;
            knot.in_ctrl = if mods.alt { *in_ctrl } else { knot.anchor - v };
            *in_ctrl = knot.in_ctrl;
            knot.smooth = !mods.alt;
        }
        Kind::Retract | Kind::Grab(HandleSide::Out) => {
            knot.out_ctrl = knot.anchor + v;
            knot.smooth = false;
        }
        Kind::Grab(HandleSide::In) => {
            knot.in_ctrl = knot.anchor + v;
            knot.smooth = false;
        }
        Kind::Move(_) => {}
    }
}

impl CanvasTool for PenTool {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.label = Arc::from(cx.words::<PenWords>().path.as_str());
        if mem::take(&mut self.resume) && self.run.is_none() && !self.shown.knots.is_empty() {
            self.begin();
        }
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        if !self.takes(pointer.button, pointer.mods) {
            return ToolAction::Ignored;
        }
        let reach = reach(cx);
        match self.mode {
            PenMode::Pen => self.press_pen(pointer.pos, pointer.mods, &space.view, reach),
            PenMode::Curvature => self.press_curve(pointer, &space.view, reach),
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(space) = cx.space() {
            self.track(pointer.pos, pointer.mods, &space.view, cx.slop);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(space) = cx.space() {
            self.track(pointer.pos, pointer.mods, &space.view, cx.slop);
        }
        let Some(drag) = self.drag.take() else { return };
        if drag.kind == Kind::Close {
            let mut draft = self.shown.clone();
            draft.closed = true;
            self.change(draft);
            self.commit();
            self.finish();
        }
    }

    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        self.drag = None;
        if !self.shown.knots.is_empty() {
            self.end(reason);
        }
    }

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        if self.shown.knots.is_empty() || self.drag.is_some() {
            return false;
        }
        match key {
            ToolKey::Submit | ToolKey::Cancel => {
                self.end(CancelReason::Escape);
                true
            }
            ToolKey::Delete { .. } => {
                self.remove_last_anchor(CancelReason::UndoDuringGesture);
                true
            }
            _ => false,
        }
    }

    fn claims(&self) -> ToolClaims {
        if !self.shown.knots.is_empty() && self.drag.is_none() { ToolClaims::NONE.deletes() } else { ToolClaims::NONE }
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        button == PointerButton::Primary && self.sensing && !mods.cmd
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(space), Some(style)) = (cx.space(), cx.style()) else { return };
        if self.shown.knots.is_empty() {
            return;
        }
        let look = PenStyle::of(style, &self.options);
        match self.mode {
            PenMode::Pen => self.paint_pen(draw, cx, style, &look, &space.view),
            PenMode::Curvature => self.paint_curve(draw, cx, &look, &space.view),
        }
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let icon = match self.mode {
            PenMode::Pen => ToolIcon::Pen,
            PenMode::Curvature => ToolIcon::CurvaturePen,
        };
        // The pen's tip is the bottom left corner of the icon's box.
        let badge = self.closes(cx).then_some(CursorBadge::Close);
        Cursor::Painted(PaintedCursor::tool(icon.icon(), Vec2::Y * ICON_BOX, badge))
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        let space = cx.space()?;
        let n = self.shown.knots.len();
        if n == 0 {
            return None;
        }
        let words = cx.words::<PenWords>();
        let (units, rulers) = (cx.units(), space.rulers());
        let mut bounds = DocRect::from_points(self.shown.knots[0].anchor, self.shown.knots[0].anchor);
        let mut access = ToolAccess::new(words.group.as_str()).description(words.description.as_str());
        for (i, knot) in self.shown.knots.iter().enumerate() {
            bounds = bounds.union_pt(knot.anchor);
            let at = knot.anchor;
            let value = format!("{}, {}", rulers.x.format_value(&units, at.x, rulers.zero.x), rulers.y.format_value(&units, at.y, rulers.zero.y));
            access = access.anchor(ToolAnchor::new(Self::knot_name(&words, i + 1, n), at).description(value));
        }
        Some(access.bounds(bounds))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Direct Selection tool's anchor keys")
    }
}

impl PenTool {
    /// The Pen's look: the path, the rubber band, the last anchor's handles, the anchors and the close cue.
    fn paint_pen(&self, draw: &Draw<'_>, cx: &ToolContext<'_>, style: &StyleCx<'_>, look: &PenStyle, view: &CanvasView) {
        let draft = &self.shown;
        let n = draft.knots.len();
        paint_path(draw, look, view, draft);
        let last = draft.knots[n - 1];
        let pointer = cx.pointer.filter(|_| self.sensing).map(|pointer| pointer.pos);
        let pressing = self.drag.is_some();
        // Where a press edits the draft, the hover marks that edit and no rubber band runs.
        let auto = pointer.filter(|_| !pressing).and_then(|at| self.auto_edit(draft, at, cx.mods, view, reach(cx)));
        if let Some(edit) = auto {
            self.paint_auto_mark(draw, style, view, edit);
        }
        if let Some(pointer) = pointer
            && !pressing
            && auto.is_none()
            && self.rubber_band()
        {
            let mut end = view.to_doc(pointer);
            if cx.mods.shift {
                end = last.anchor + constrain(end - last.anchor);
            }
            draw.dashed(&[view.to_screen(last.anchor), view.to_screen(end)], Dashes::new(look.dash, look.gap), StrokeWidth::Thin, look.line);
        }
        for side in [HandleSide::In, HandleSide::Out] {
            if !last.retracted(side) {
                let line = PaintShape::Path(vec![view.to_screen(last.anchor), view.to_screen(last.handle(side))]);
                draw.stroke(&line, StrokeWidth::Thin, look.handle.fill.off.rest);
            }
        }
        paint_anchors(draw, look, view, draft);
        for side in [HandleSide::In, HandleSide::Out] {
            if !last.retracted(side) {
                let end = PaintShape::Circle(view.to_screen(last.handle(side)), look.handle.side / 2.0);
                draw.fill(&end, look.handle.fill.off.rest);
                draw.stroke(&end, StrokeWidth::Thin, look.handle.stroke);
            }
        }
        if let Some(pointer) = pointer
            && self.closes_at(draft, pointer, view, reach(cx))
        {
            paint_close_cue(draw, look, view, draft);
        }
    }
}

/// The path so far.
pub(super) fn paint_path(draw: &Draw<'_>, look: &PenStyle, view: &CanvasView, draft: &EditSubpath) {
    for (mut run, closed) in path_runs(view, &draft.bez_path()) {
        if closed {
            run.push(run[0]);
        }
        draw.stroke(&PaintShape::Path(run), StrokeWidth::Medium, look.line);
    }
}

/// The anchors as squares, the last one filled.
pub(super) fn paint_anchors(draw: &Draw<'_>, look: &PenStyle, view: &CanvasView, draft: &EditSubpath) {
    let n = draft.knots.len();
    for (i, knot) in draft.knots.iter().enumerate() {
        let fill = if i + 1 == n { look.selected } else { look.handle.fill.off.rest };
        square(draw, look, view.to_screen(knot.anchor), fill);
    }
}

fn square(draw: &Draw<'_>, look: &PenStyle, at: Pos2, fill: StyleColor) {
    let shape = PaintShape::Rect(Rect::from_center_size(at, Vec2::splat(look.handle.side)), CornerRadius::ZERO);
    draw.fill(&shape, fill);
    draw.stroke(&shape, StrokeWidth::Thin, look.handle.stroke);
}

/// The first anchor filled: a press here closes the path (the pointer carries the close badge).
pub(super) fn paint_close_cue(draw: &Draw<'_>, look: &PenStyle, view: &CanvasView, draft: &EditSubpath) {
    square(draw, look, view.to_screen(draft.knots[0].anchor), look.selected);
}

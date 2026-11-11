//! [`PolygonLassoTool`]: the Polygonal Lasso, click by click.

use craft_behavior::geometry::draft::turn_to_45;
use craft_behavior::geometry::{DraftKind, DraftPoint, DraftReach, OutlineDraft, SelectionOperation};
use craft_behavior::{CancelReason, Cursor, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, ToolIcon};

use super::drag::{InsideDrag, PressRule, drag_kind};
use super::lasso::{LassoCursor, apply, begin_inside, cursor, draft_key, outline_access, paint_outline, phase, reach, sample};
use super::{Place, Report, SelectEvent, SelectPhase, SelectToolPass, SelectionProbe};

/// The Polygonal Lasso: each click adds a vertex; the outline closes into a selection.
///
/// The drawing rules are [`OutlineDraft`]'s with [`DraftKind::Polygonal`]: Shift (without Alt) turns the new segment
/// to a multiple of 45 degrees from the last vertex (not the first vertex), an Alt drag draws freehand. The first
/// click latches the operation (Alt with nothing selected is New). The outline closes on a click near the first
/// vertex (3 or more), on a click at the last, on Enter or a double-click. Backspace, Delete or a secondary click
/// retract the last vertex (the last one cancels): the tool takes the secondary button while an outline is open, so
/// the app calls nothing and shows no context menu then. Escape cancels, between clicks and during a press alike; a
/// document change or [`reset`](Self::reset) drops it.
///
/// Click-driven: a plain press inside the selection places a vertex. Only Cmd drags the selection (pixels; module
/// docs), and not while an outline is open. A Cmd press outside the selection is `ToolAction::Ignored`.
///
/// While a New outline is open the selection it will replace is hidden: [`replaces_selection`](Self::replaces_selection)
/// is for the selection overlay's `visible`.
#[derive(Clone, Debug)]
pub struct PolygonLassoTool {
    draft: OutlineDraft,
    bar: SelectionOperation,
    operation: SelectionOperation,
    mods: Mods,
    inside: Option<InsideDrag>,
    report: Report,
}

impl Default for PolygonLassoTool {
    fn default() -> Self {
        Self::new()
    }
}

impl PolygonLassoTool {
    /// A Polygonal Lasso with the operation New and no outline.
    pub fn new() -> Self {
        Self {
            // The reach comes from the pass's slop in every pass (`each_pass`, `OutlineDraft::set_reach`).
            draft: OutlineDraft::new(DraftKind::Polygonal, DraftReach::new(0.0, 0.0)),
            bar: SelectionOperation::New,
            operation: SelectionOperation::New,
            mods: Mods::NONE,
            inside: None,
            report: Report::default(),
        }
    }

    /// The options bar's operation for outlines begun from now on.
    pub fn operation(&mut self, bar: SelectionOperation) -> &mut Self {
        self.bar = bar;
        self
    }

    /// The tool as the `CanvasTool` for one pass, asking `probe` about the document.
    pub fn pass<'a>(&'a mut self, probe: &'a dyn SelectionProbe) -> SelectToolPass<'a, Self> {
        SelectToolPass::new(self, probe)
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<SelectEvent> {
        self.report.take()
    }

    /// Drop the outline and any selection drag without an event (a tool switch).
    pub fn reset(&mut self) {
        self.draft.cancel();
        self.inside = None;
    }

    /// What the tool is doing.
    pub fn phase(&self) -> SelectPhase {
        phase(&self.draft, self.inside.as_ref())
    }

    /// An outline with the operation New is open: the app hides the selection's ants.
    pub fn replaces_selection(&self) -> bool {
        self.draft.is_open() && self.operation == SelectionOperation::New
    }
}

impl CanvasTool for SelectToolPass<'_, PolygonLassoTool> {
    /// The reach, the document check, modifier changes to the draft, and the kept keys while an outline is open.
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        tool.draft.set_reach(reach(cx));
        if tool.draft.for_document(probe.document()).is_some() {
            tool.inside = None;
        }
        if cx.mods != tool.mods {
            tool.mods = cx.mods;
            let event = tool.draft.mods_changed(cx.mods);
            apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.tool.draft.takes_retract() { ToolClaims::NONE.keyboard().deletes() } else { ToolClaims::NONE }
    }

    /// The primary button always, the secondary while an outline is open (it retracts the last vertex).
    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && self.tool.draft.is_open())
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let tool = &*self.tool;
        let rule = PressRule::Clicks { outline_open: tool.draft.is_open() };
        let shown = LassoCursor {
            icon: ToolIcon::PolygonalLasso,
            draft: &tool.draft,
            operation: tool.operation,
            bar: tool.bar,
            inside: tool.inside.as_ref(),
            rule,
        };
        cursor(&shown, self.probe, cx)
    }

    /// A secondary press retracts the last vertex (`ToolAction::Done`: no drag follows). Cmd inside the selection with
    /// no outline open: a pixel drag. Otherwise the click goes to the draft (the first latches the operation); a
    /// double-click closes.
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return ToolAction::Ignored };
        if pointer.button == PointerButton::Secondary {
            tool.draft.retract();
            return ToolAction::Done;
        }
        let open = tool.draft.is_open();
        if let Some(kind) = drag_kind(probe, place.doc(pointer.pos), pointer.mods, tool.bar, PressRule::Clicks { outline_open: open }) {
            begin_inside(kind, pointer, &place, cx, &mut tool.inside, &mut tool.report);
            return ToolAction::Claimed;
        }
        if pointer.mods.cmd && !open {
            return ToolAction::Ignored;
        }
        let event = if open && pointer.clicks >= 2 { tool.draft.close() } else { tool.draft.press(sample(pointer, &place), probe.has_selection()) };
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
        ToolAction::Claimed
    }

    /// An Alt drag adds freehand points; a selection drag moves.
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.as_mut() {
            if let Some(event) = inside.drag(place.doc(pointer.pos), pointer.mods, place.zoom()) {
                tool.report.push(event);
            }
            return;
        }
        let event = tool.draft.drag(sample(pointer, &place));
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
    }

    /// Ends an Alt-drag segment or a selection drag; the outline stays open.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.end(place.doc(pointer.pos), pointer.mods, place.zoom()));
            return;
        }
        let event = tool.draft.release(sample(pointer, &place));
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
    }

    /// The rubber band follows the pointer, constrained to 45 degrees with Shift.
    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        let Some(place) = Place::of(cx) else { return };
        let mut at = sample(pointer, &place);
        if pointer.mods.shift
            && !pointer.mods.alt
            && let Some(&last) = tool.draft.points().last()
        {
            at.at = turn_to_45(last, at.at);
        }
        if tool.draft.hover(at).is_some() {
            cx.request_repaint();
        }
    }

    /// Escape during a press drops the whole outline, as Escape between clicks does. Any other reason (a lost
    /// pointer, a tool switch) ends an Alt-drag segment where it is and keeps the outline (the app calls
    /// [`reset`](PolygonLassoTool::reset) to drop it). A selection drag reports `Cancel`.
    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.cancel(reason));
        }
        if reason == CancelReason::Escape {
            tool.draft.cancel();
            return;
        }
        if tool.draft.is_down()
            && let Some(&last) = tool.draft.points().last()
        {
            tool.draft.release(DraftPoint::new(last, Mods::NONE, 1.0));
        }
    }

    /// Enter closes, Escape cancels, Backspace and Delete retract.
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let (tool, probe) = (&mut *self.tool, self.probe);
        draft_key(&mut tool.draft, key, cx, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report)
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        paint_outline(&self.tool.draft, draw, cx);
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        outline_access(self.tool.draft.points(), cx)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

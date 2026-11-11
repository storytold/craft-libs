//! [`RulerTool`]: the Ruler tool on the canvas tool host. Its points are the [`RulerOverlay`]'s; the tool starts a new
//! line from a press on the background and moves the whole line with the arrow keys.

use super::model::{MIN_LINE, RulerLine, RulerPart};
use super::overlay::{RulerOverlay, moved};
use super::words::RulerWords;
use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, CursorIcon, EditEvent, GestureId, Mixed};
use craft_ui::canvas::{CanvasTool, NUDGE, NUDGE_FAR, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::canvas_controls::StraightLinePreview;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::Draw;
use egui::{Rect, Ui};
use std::sync::Arc;

/// A drag in progress: the part it moves and the line at the press.
#[derive(Clone, Copy, Debug)]
struct Drag {
    id: GestureId,
    before: Option<RulerLine>,
}

/// The Ruler tool, kept by the app while the tool is picked.
///
/// - **Pointer:** a press on the background starts a new line from the press; a drag moves its end and Shift puts the
///   end on the nearest 45° ray about the start, at the pointer's distance. A new line shorter than [`MIN_LINE`]
///   document units is cleared on release. Escape puts the line back. A press on a point of an existing line is the
///   [`RulerOverlay`]'s.
/// - **Keyboard:** with the canvas focused the arrows move the whole line 1 unit and Shift 10; Delete and Backspace
///   clear it. The overlay's points take the keys while they have the focus.
/// - **Events:** the line's changes, in order, from [`take_events`](Self::take_events) after the host has run: the
///   overlay's edits and the tool's. A drag is `Begin`, `Update`s and `Commit` (`Cancel` on Escape); a key press is a
///   `Set`. The app stores the value; it is not an undo step.
/// - **Cursor:** the crosshair.
#[derive(Debug, Default)]
pub struct RulerTool {
    drag: Option<Drag>,
    events: Vec<EditEvent<Option<RulerLine>>>,
}

impl RulerTool {
    /// A tool with no line.
    pub fn new() -> Self {
        Self::default()
    }

    /// The line's changes since the last call, in order.
    pub fn take_events(&mut self) -> Vec<EditEvent<Option<RulerLine>>> {
        std::mem::take(&mut self.events)
    }

    /// This pass's tool for the host, over `line`: the document's Ruler line, `None` before the first.
    pub fn pass(&mut self, line: Option<RulerLine>) -> RulerPass<'_> {
        RulerPass { tool: self, line: line.filter(RulerLine::is_finite) }
    }
}

/// The Ruler tool for one pass of the host ([`RulerTool::pass`]).
pub struct RulerPass<'a> {
    tool: &'a mut RulerTool,
    /// The line as this pass's events leave it: the input's, then every change the tool reported.
    line: Option<RulerLine>,
}

impl RulerPass<'_> {
    /// Shows the line and the protractor arm, and the overlay with the points, over the canvas `rect`; call it in the
    /// canvas body. The overlay's edits join the tool's events.
    pub fn show_line(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(line) = self.line else { return };
        let words = craft_ui::runtime::env::Env::of(ui).words::<RulerWords>();
        StraightLinePreview::new(words.group.clone(), line.start, line.end).show_at(ui, rect);
        if let Some(arm) = line.protractor {
            StraightLinePreview::new(words.protractor.clone(), line.start, arm).show_at(ui, rect);
        }
        let mut shown = Some(line);
        let edit = RulerOverlay::new(words.group.clone(), Bind::bound(&mut shown)).show_at(ui, rect);
        if !edit.events.is_empty() {
            self.line = shown;
        }
        self.tool.events.extend(edit.events);
    }

    fn label(cx: &ToolContext<'_>) -> Arc<str> {
        cx.words::<RulerWords>().group.as_str().into()
    }

    fn doc(pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<Point> {
        cx.space().map(|space| space.view.to_doc(pointer.pos)).filter(|at| at.is_finite())
    }

    fn report(&mut self, line: Option<RulerLine>, event: EditEvent<Option<RulerLine>>) {
        self.line = line;
        self.tool.events.push(event);
    }

    /// Moves the whole line by one key's nudge.
    fn nudge(&mut self, line: RulerLine, direction: egui::Vec2, shift: bool, cx: &ToolContext<'_>) -> bool {
        let Some(space) = cx.space() else { return false };
        let step = if shift { NUDGE_FAR } else { NUDGE };
        let origin = space.view.rect.center();
        let by = space.view.to_doc(origin + direction) - space.view.to_doc(origin);
        if by.hypot() == 0.0 {
            return false;
        }
        let by = by / by.hypot() * step;
        let mut next = line;
        next.start += by;
        next.end += by;
        next.protractor = line.protractor.map(|arm| arm + by);
        self.report(Some(next), EditEvent::set(Mixed::Same(Some(line)), Some(next), Self::label(cx), None));
        true
    }
}

impl CanvasTool for RulerPass<'_> {
    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Crosshair)
    }

    /// Starts a new line at the press (Begin).
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = Self::doc(pointer, cx) else { return ToolAction::Ignored };
        let before = self.line;
        let Some(id) = cx.gesture_id() else { return ToolAction::Ignored };
        self.tool.drag = Some(Drag { id, before });
        let line = RulerLine::new(at, at);
        self.report(Some(line), EditEvent::begin(id, Mixed::Same(before), Self::label(cx), None));
        self.tool.events.push(EditEvent::update(id, Some(line)));
        ToolAction::Claimed
    }

    /// Moves the end, Shift onto 45° rays about the start (Update).
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some(drag), Some(line), Some(at)) = (self.tool.drag, self.line, Self::doc(pointer, cx)) else { return };
        let next = moved(line, RulerPart::End, at, pointer.mods.shift);
        if next != line {
            self.report(Some(next), EditEvent::update(drag.id, Some(next)));
        }
    }

    /// Commit; a new line under [`MIN_LINE`] commits `None`.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(drag) = self.tool.drag.take() else { return };
        let Some(mut line) = self.line else { return };
        if let Some(at) = Self::doc(pointer, cx) {
            line = moved(line, RulerPart::End, at, pointer.mods.shift);
        }
        let value = (line.start.distance(line.end) >= MIN_LINE).then_some(line);
        self.report(value, EditEvent::commit(drag.id, Mixed::Same(drag.before), value));
    }

    /// Escape: Cancel; the app puts the line of the press back.
    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        let Some(drag) = self.tool.drag.take() else { return };
        self.report(drag.before, EditEvent::cancel(drag.id, reason, Some(Mixed::Same(drag.before))));
    }

    /// Arrows move the whole line; Delete and Backspace clear it.
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let Some(line) = self.line else { return false };
        if self.tool.drag.is_some() {
            return false;
        }
        match key {
            ToolKey::Nudge { direction, mods, .. } => self.nudge(line, *direction, mods.shift, cx),
            ToolKey::Delete { .. } => {
                self.report(None, EditEvent::set(Mixed::Same(Some(line)), None, Self::label(cx), None));
                true
            }
            _ => false,
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.line.is_some() && self.tool.drag.is_none() { ToolClaims::NONE.deletes() } else { ToolClaims::NONE }
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Ruler Line... command")
    }
}

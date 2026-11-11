//! [`ArtboardTool`]: the Artboard tool on the tool host. The boards' bodies and handles are the
//! [`ArtboardOverlay`](super::ArtboardOverlay)'s parts, so the tool gets only the presses on the pasteboard.

use super::{ArtboardEvent, ArtboardItem, ArtboardRules, ArtboardToolWords, Boards, KeyAct, key_events, label, overlaps};
use crate::geometry::artboard::drawn_rect;
use crate::ui::add_beside::AddSide;
use crate::ui::slice::Run;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, Cursor, CursorIcon, GestureId};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use egui::Pos2;

/// The Artboard tool. The app owns the value, sets the snapshot (boards, selection, rules) each pass, hands it to
/// `ToolHost::run` and drains [`take_events`](Self::take_events). The tool never changes the document.
///
/// - A drag on the pasteboard draws a board ([`drawn_rect`]); a drag under the click slop (3 screen points) draws
///   nothing. Esc cancels the drag; Cmd held is the app's momentary Move tool (`ToolHost::momentary`).
/// - Illustrator's rules add: Shift-drag on the pasteboard is a marquee that selects the boards it touches, a click on
///   an object outside the boards reports `FitObject`, Esc with no drag reports `Leave`.
/// - With the canvas focused and no board part focused, the arrows, Delete and Enter act on the selection by the same
///   rule as the control's focused board.
/// - [`add_beside`](Self::add_beside) turns an answer of the "+" buttons into `AddBeside`.
///
/// Paint: the drag preview, a medium accent line.
#[derive(Debug, Default)]
pub struct ArtboardTool {
    items: Vec<ArtboardItem>,
    selected: Vec<usize>,
    rules: ArtboardRules,
    events: Vec<ArtboardEvent>,
    drag: Option<Drag>,
    ids: GestureId,
}

/// A drag on the pasteboard.
#[derive(Clone, Debug)]
struct Drag {
    /// A new board, or a marquee.
    run: Option<Run<Rect>>,
    press: Pos2,
    start: Point,
    at: Point,
}

impl ArtboardTool {
    /// A tool with Photoshop's rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// This pass's boards, in the app's order.
    pub fn items(&mut self, items: &[ArtboardItem]) -> &mut Self {
        self.items.clear();
        self.items.extend_from_slice(items);
        self
    }

    /// The selected boards (the active board first).
    pub fn selected(&mut self, indices: &[usize]) -> &mut Self {
        self.selected.clear();
        self.selected.extend_from_slice(indices);
        self
    }

    /// Which product's rules the tool follows; Photoshop's by default.
    pub fn rules(&mut self, rules: ArtboardRules) -> &mut Self {
        self.rules = rules;
        self
    }

    /// A "+" button answered: reports `AddBeside` on the active board; `copy` (Alt) with its contents.
    pub fn add_beside(&mut self, side: AddSide, copy: bool) -> &mut Self {
        if let Some(index) = self.active() {
            self.events.push(ArtboardEvent::AddBeside { index, side, copy });
        }
        self
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<ArtboardEvent> {
        std::mem::take(&mut self.events)
    }

    fn boards(&self) -> Boards<'_> {
        Boards { items: &self.items, selected: &self.selected }
    }

    fn active(&self) -> Option<usize> {
        self.boards().active()
    }

    fn fresh(&mut self) -> GestureId {
        self.ids = self.ids.next();
        self.ids
    }

    fn point(pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<Point> {
        let space = cx.space()?;
        Some(cx.snap(space.view.to_doc(pointer.pos)))
    }

    fn marquee(&self) -> bool {
        self.rules.multi_select
    }
}

impl CanvasTool for ArtboardTool {
    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Default)
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = Self::point(pointer, cx) else { return ToolAction::Ignored };
        let marquee = self.marquee() && pointer.mods.shift;
        let run = if marquee {
            None
        } else {
            let label = label(&cx.words::<ArtboardToolWords>().new_board);
            Some(Run::new(self.fresh(), label, Rect::from_origin_size(at, (0.0, 0.0))))
        };
        self.drag = Some(Drag { run, press: pointer.pos, start: at, at });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(at) = Self::point(pointer, cx) else { return };
        let Some(mut drag) = self.drag.take() else { return };
        drag.at = at;
        if pointer.pos.distance(drag.press) > cx.slop
            && let Some(run) = drag.run.as_mut()
        {
            let rect = drawn_rect(drag.start, drag.at);
            if rect.width() >= 1.0 && rect.height() >= 1.0 {
                self.events.extend(run.follow(rect).into_iter().map(ArtboardEvent::Draw));
            }
        }
        self.drag = Some(drag);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(drag) = self.drag.take() else { return };
        let to = Self::point(pointer, cx).unwrap_or(drag.at);
        let moved = pointer.pos.distance(drag.press) > cx.slop;
        let mods: Mods = pointer.mods;
        match drag.run {
            None if moved => {
                let touched = Rect::from_points(drag.start, to);
                let indices = (0..self.items.len()).filter(|&i| overlaps(self.items[i].rect, touched)).collect();
                self.events.push(ArtboardEvent::Select { indices, mods });
            }
            Some(run) if moved => {
                let rect = drawn_rect(drag.start, to);
                if rect.width() >= 1.0 && rect.height() >= 1.0 {
                    self.events.extend(run.commit(rect).into_iter().map(ArtboardEvent::Draw));
                } else {
                    // A board dragged to nothing wide or tall is no board.
                    self.events.extend(run.cancel(CancelReason::Dismissed).map(ArtboardEvent::Draw));
                }
            }
            _ => {
                if self.rules.fit_object {
                    self.events.push(ArtboardEvent::FitObject { at: drag.start });
                }
            }
        }
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if let Some(Drag { run: Some(run), .. }) = self.drag.take() {
            self.events.extend(run.cancel(reason).map(ArtboardEvent::Draw));
        }
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let act = match key {
            ToolKey::Cancel if self.rules.escape_leaves => {
                self.events.push(ArtboardEvent::Leave);
                return true;
            }
            ToolKey::Nudge { direction, mods, .. } => {
                let Some(space) = cx.space() else { return false };
                let doc = space.view.unmap_vec(*direction);
                let unit = doc / doc.hypot().max(f64::EPSILON);
                KeyAct::Nudge { direction: craft_behavior::kurbo::Vec2::new(unit.x.round(), unit.y.round()), mods: *mods }
            }
            ToolKey::Delete { .. } => KeyAct::Delete,
            ToolKey::Submit => KeyAct::Submit,
            _ => return false,
        };
        let words = cx.words::<ArtboardToolWords>();
        let events = key_events(&Boards { items: &self.items, selected: &self.selected }, &self.rules, act, &words);
        let taken = !events.is_empty();
        self.events.extend(events);
        taken
    }

    fn claims(&self) -> ToolClaims {
        if self.selected.is_empty() { ToolClaims::NONE } else { ToolClaims::NONE.deletes() }
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(drag), Some(space), Some(style)) = (&self.drag, cx.space(), cx.style()) else { return };
        let rect = if drag.run.is_some() { drawn_rect(drag.start, drag.at) } else { Rect::from_points(drag.start, drag.at) };
        if pointer_moved(drag) {
            let quad = space.view.quad(rect.abs()).to_vec();
            draw.stroke(&PaintShape::Polygon(quad), StrokeWidth::Medium, style.color(style.tokens.color.accent.solid));
        }
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        let drag = self.drag.as_ref().filter(|drag| drag.run.is_some())?;
        let rect = drawn_rect(drag.start, drag.at);
        let units = cx.units();
        let name = cx.words::<ArtboardToolWords>().tool.as_str().to_owned();
        Some(ToolAccess::new(name).value(format!("{} x {}", units.pixels(rect.width()), units.pixels(rect.height()))).bounds(rect))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Width and Height fields of the options bar and Add New Artboard")
    }
}

/// Whether the drag has moved: the preview shows from the first change.
fn pointer_moved(drag: &Drag) -> bool {
    drag.start != drag.at
}

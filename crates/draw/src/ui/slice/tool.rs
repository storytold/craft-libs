//! [`SliceTool`]: the Slice and Slice Select tools on the tool host. The slices themselves are the
//! [`SliceOverlay`](super::SliceOverlay)'s parts, so the tool gets only the presses away from every slice: it draws a new
//! slice in Slice mode and clears the selection in Select mode.

use super::{CLICK_SLOP, Run, SliceCommand, SliceEvent, SliceItem, SliceMode, SliceRefusal, SliceRules, SliceToolSettings, SliceToolWords, slice_at};
use crate::geometry::slice::fixed_ratio_pixels;
use craft_behavior::geometry::{Constraint, marquee_pixels, square_pixels};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, Cursor, CursorIcon, GestureId};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::{Dashes, Draw, PaintShape, StrokeWidth};

/// The Slice and Slice Select tools. The app owns one value, sets its [`mode`](Self::mode) from the selected tool and
/// its snapshot (items, selection, lock, settings) each pass, hands it to `ToolHost::run` and drains
/// [`take_events`](Self::take_events). The tool never changes the document.
///
/// Slice mode:
/// - Normal: press and drag; a drag no wider and no taller than two document pixels is a click that selects the slice
///   under the point (a `Select` event, and `Command(Options)` on a double click). Shift makes the slice square and
///   Alt draws it from the centre under [`SliceRules::modifiers`]. Fixed Ratio keeps the ratio
///   ([`fixed_ratio_pixels`]). Fixed Size is exactly its pixels, made by a click and previewed from the press.
/// - The pointer snaps by the canvas's snapping, the press included.
///
/// Select mode: a press away from every slice clears the selection.
///
/// Locked ([`locked`](Self::locked)): a press draws and moves nothing, clears the selection (`Select { number: None }`,
/// as PhotoCraft) and reports `Refused(Locked)`.
///
/// Paint: the new slice's preview, a white line under the dashed slice-colour line.
#[derive(Debug, Default)]
pub struct SliceTool {
    mode: SliceMode,
    items: Vec<SliceItem>,
    selected: Option<usize>,
    locked: bool,
    settings: SliceToolSettings,
    rules: SliceRules,
    events: Vec<SliceEvent>,
    drag: Option<Drag>,
    ids: GestureId,
}

/// A new slice between presses.
#[derive(Clone, Debug)]
struct Drag {
    run: Run<Rect>,
    start: Point,
    at: Point,
    mods: Mods,
}

impl SliceTool {
    /// The Slice tool with Photoshop's rules and the Normal style.
    pub fn new() -> Self {
        Self::default()
    }

    /// Slice or Select.
    pub fn mode(&mut self, mode: SliceMode) -> &mut Self {
        self.mode = mode;
        self
    }

    /// This pass's slices, in the app's order (stored slices before auto slices).
    pub fn items(&mut self, items: &[SliceItem]) -> &mut Self {
        self.items.clear();
        self.items.extend_from_slice(items);
        self
    }

    /// The selected slice's number.
    pub fn selected(&mut self, number: Option<usize>) -> &mut Self {
        self.selected = number;
        self
    }

    /// View › Lock Slices.
    pub fn locked(&mut self, locked: bool) -> &mut Self {
        self.locked = locked;
        self
    }

    /// The Style, Width and Height a new slice is drawn with, and Hide Auto Slices.
    pub fn settings(&mut self, settings: SliceToolSettings) -> &mut Self {
        self.settings = settings;
        self
    }

    /// Which product's rules the tool follows; Photoshop's by default.
    pub fn rules(&mut self, rules: SliceRules) -> &mut Self {
        self.rules = rules;
        self
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<SliceEvent> {
        std::mem::take(&mut self.events)
    }

    /// Fixed Size draws by a click.
    fn fixed_size(&self) -> bool {
        matches!(self.settings.style, Constraint::Size(..))
    }

    /// The new slice from `start` to `at` under the settings and `mods`.
    fn new_rect(&self, start: Point, at: Point, mods: Mods) -> Rect {
        match self.settings.style {
            // The slice sits on the pixel the press is in and grows the way the drag points, so it holds that pixel; a
            // slice drawn left or up ends at the pixel's far edge.
            Constraint::Size(w, h) => {
                let (w, h) = (w.round().max(1.0), h.round().max(1.0));
                let sign = |d: f64| if d < 0.0 { -1.0 } else { 1.0 };
                let (dx, dy) = (sign(at.x - start.x), sign(at.y - start.y));
                let corner =
                    Point::new(if dx < 0.0 { start.x.ceil() } else { start.x.floor() }, if dy < 0.0 { start.y.ceil() } else { start.y.floor() });
                marquee_pixels(corner, Point::new(corner.x + dx * w, corner.y + dy * h))
            }
            Constraint::Ratio(w, h) => fixed_ratio_pixels(start, at, (w, h)),
            _ => {
                let (square, centre) = if self.rules.modifiers { (mods.shift, mods.alt) } else { (false, false) };
                let (a, b) = Constraint::Free.corners(start, at, square, centre);
                let rect = marquee_pixels(a, b);
                if square && !centre { square_pixels(rect, start, at) } else { rect }
            }
        }
    }

    /// Whether `rect` is a slice: it has an area and, past a click, a drag the slop does not swallow.
    fn drawn(&self, rect: Rect, began: bool) -> bool {
        rect.width() > 0.0 && rect.height() > 0.0 && (began || self.fixed_size() || rect.width() > CLICK_SLOP || rect.height() > CLICK_SLOP)
    }

    /// Reports the new slice's `Begin` (past the click slop, or at the press for Fixed Size) and an `Update` when its
    /// rectangle changed.
    fn update_new(&mut self) {
        let Some(mut drag) = self.drag.take() else { return };
        let rect = self.new_rect(drag.start, drag.at, drag.mods);
        if self.drawn(rect, drag.run.began()) {
            self.events.extend(drag.run.follow(rect).into_iter().map(SliceEvent::Draw));
        }
        self.drag = Some(drag);
    }

    fn fresh(&mut self) -> GestureId {
        self.ids = self.ids.next();
        self.ids
    }

    /// The document point under `pointer`, snapped.
    fn point(pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<Point> {
        let space = cx.space()?;
        Some(cx.snap(space.view.to_doc(pointer.pos)))
    }

    /// The slice the point selects, as a click does.
    fn select_at(&mut self, at: Point, mods: Mods) -> Option<usize> {
        let number = slice_at(&self.items, at, self.settings.hide_auto).map(|item| item.number);
        let add = self.rules.multi_select && mods.shift;
        if !add {
            self.selected = number;
        }
        self.events.push(SliceEvent::Select { number, add, mods });
        number
    }
}

impl CanvasTool for SliceTool {
    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        Cursor::System(if self.mode == SliceMode::Slice && !self.locked { CursorIcon::Crosshair } else { CursorIcon::Default })
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        if self.locked {
            // PhotoCraft clears the selection on a locked press.
            self.selected = None;
            self.events.push(SliceEvent::Select { number: None, add: false, mods: pointer.mods });
            self.events.push(SliceEvent::Refused(SliceRefusal::Locked));
            return ToolAction::Done;
        }
        if self.mode == SliceMode::Select {
            self.selected = None;
            self.events.push(SliceEvent::Select { number: None, add: false, mods: pointer.mods });
            return ToolAction::Done;
        }
        let Some(at) = Self::point(pointer, cx) else { return ToolAction::Ignored };
        let label = cx.words::<SliceToolWords>().new_slice.as_str().into();
        let id = self.fresh();
        self.drag = Some(Drag { run: Run::new(id, label, Rect::from_origin_size(at, (0.0, 0.0))), start: at, at, mods: pointer.mods });
        // Fixed Size is drawn from the press.
        self.update_new();
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(at) = Self::point(pointer, cx) else { return };
        if let Some(drag) = self.drag.as_mut() {
            drag.at = at;
            drag.mods = pointer.mods;
        }
        self.update_new();
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(drag) = self.drag.take() else { return };
        let to = Self::point(pointer, cx).unwrap_or(drag.at);
        let rect = self.new_rect(drag.start, to, pointer.mods);
        let began = drag.run.began();
        if self.drawn(rect, began) {
            self.events.extend(drag.run.commit(rect).into_iter().map(SliceEvent::Draw));
        } else if began {
            self.events.extend(drag.run.cancel(CancelReason::Dismissed).map(SliceEvent::Draw));
        } else if self.select_at(to, pointer.mods).is_some() && pointer.clicks == 2 {
            self.events.push(SliceEvent::Command(SliceCommand::Options));
        }
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if let Some(drag) = self.drag.take() {
            self.events.extend(drag.run.cancel(reason).map(SliceEvent::Draw));
        }
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(drag), Some(space), Some(style)) = (&self.drag, cx.space(), cx.style()) else { return };
        let rect = self.new_rect(drag.start, drag.at, drag.mods);
        let quad = space.view.quad(rect).to_vec();
        let colors = &style.tokens.color.overlay;
        draw.stroke(&PaintShape::Polygon(quad.clone()), StrokeWidth::Thin, style.color(colors.handle_fill));
        let mut path = quad.clone();
        path.push(quad[0]);
        draw.dashed(&path, Dashes::new(style.tokens.stroke.dash, style.tokens.stroke.dash_gap), StrokeWidth::Thin, style.color(colors.line));
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        let drag = self.drag.as_ref()?;
        let rect = self.new_rect(drag.start, drag.at, drag.mods);
        let units = cx.units();
        let name = cx.words::<SliceToolWords>().tool.as_str().to_owned();
        Some(ToolAccess::new(name).value(format!("{} x {}", units.pixels(rect.width()), units.pixels(rect.height()))).bounds(rect))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Divide Slice and Slice Options commands")
    }
}

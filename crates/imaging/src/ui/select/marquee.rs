//! [`MarqueeTool`]: the Rectangular, Elliptical, Single Row and Single Column Marquees on the tool host.

use craft_behavior::geometry::{
    ConstrainStyle, Constraint, MarqueeGesture, MarqueeRect, SelectionOperation, fixed_size_pixels, marquee_pixels, square_pixels,
};
use craft_behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor};
use craft_keymap::Key;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use egui::Pos2;
use kurbo::{Point, Rect};

use super::drag::{InsideDrag, PressRule, cursor_over, drag_kind};
use super::{Place, Report, SelectEvent, SelectPhase, SelectToolPass, SelectWords, SelectionProbe, SelectionShape};

/// The points that approximate an ellipse's ants.
const ELLIPSE_SEGMENTS: usize = 96;

/// Which marquee.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MarqueeKind {
    /// A rectangle on whole pixels.
    #[default]
    Rectangle,
    /// The ellipse inscribed in a rectangle on whole pixels.
    Ellipse,
    /// The one-pixel row under the pointer, across the whole document.
    SingleRow,
    /// The one-pixel column under the pointer, down the whole document.
    SingleColumn,
}

impl MarqueeKind {
    /// Whether the marquee is a line across the document, which never drags the selection.
    fn is_line(self) -> bool {
        matches!(self, MarqueeKind::SingleRow | MarqueeKind::SingleColumn)
    }
}

/// The press of a marquee drag, kept until the release.
#[derive(Clone, Copy, Debug)]
struct MarqueePress {
    /// The marquee rules: the fresh Shift and Alt, Space moving the marquee.
    gesture: MarqueeGesture,
    /// The operation the press latched.
    operation: SelectionOperation,
    /// The document's size at the press, for the Single Row and Single Column Marquees.
    size: [u32; 2],
    /// The pointer's document point as last followed.
    last: Point,
    /// The corners the gesture last gave, once it has begun.
    corners: Option<MarqueeRect>,
}

/// The marquee selection tools (the M group). The app owns one value per canvas, sets the options bar's operation and
/// style each pass, and wraps it in a [`SelectToolPass`] for `ToolHost::run`.
///
/// Pointer, in document units:
/// - A drag sizes the marquee from the press: [`Constraint`] (Normal, Fixed Ratio, Fixed Size) from
///   [`constraint`](Self::constraint), Shift square (circle), Alt from the centre, each only when pressed after the
///   press (or released and pressed again: Shift and Alt held at the press are the operation). Space held moves the
///   whole marquee. The rectangle is on whole pixels, exactly as the release commits it; the readout beside the
///   pointer shows W and H.
/// - A release without a drag, or with no area, is a click: [`SelectEvent::Deselect`]. With Fixed Size a click
///   places the box there.
/// - [`MarqueeKind::SingleRow`] and [`MarqueeKind::SingleColumn`] select the pixel row (column) under the pointer
///   across the whole document, following the pointer until the release; a click selects too.
/// - A press inside the selection drags it ([`SelectEvent::DragSelection`], module docs); a Cmd press outside it is
///   `ToolAction::Ignored`. The Single Row and Single Column Marquees never drag the selection: a press inside it
///   draws the line.
/// - Escape cancels the drag (the host calls `cancel`): no event.
///
/// The operation is latched at the press and the cursor shows its badge before the press, following the live
/// modifiers. The ants of the drag are built in document space and mapped point by point through the view, so a
/// rotated view draws them where the release selects.
#[derive(Clone, Debug, Default)]
pub struct MarqueeTool {
    kind: MarqueeKind,
    bar: SelectionOperation,
    constraint: Constraint,
    press: Option<MarqueePress>,
    inside: Option<InsideDrag>,
    report: Report,
}

impl MarqueeTool {
    /// A `kind` marquee with the operation New and the Normal style.
    pub fn new(kind: MarqueeKind) -> Self {
        Self { kind, ..Self::default() }
    }

    /// Which marquee is set.
    pub fn marquee_kind(&self) -> MarqueeKind {
        self.kind
    }

    /// Switch marquee (the tool flyout); a drag in progress is dropped without an event.
    pub fn kind(&mut self, kind: MarqueeKind) -> &mut Self {
        if kind != self.kind {
            self.reset();
        }
        self.kind = kind;
        self
    }

    /// The options bar's operation (New, Add, Subtract, Intersect) for presses from now on.
    pub fn operation(&mut self, bar: SelectionOperation) -> &mut Self {
        self.bar = bar;
        self
    }

    /// The options bar's Style for drags from now on.
    pub fn constraint(&mut self, constraint: Constraint) -> &mut Self {
        self.constraint = constraint;
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

    /// Drop a drag in progress without an event (a document or tool switch the host did not see).
    pub fn reset(&mut self) {
        self.press = None;
        self.inside = None;
    }

    /// What the tool is doing: the marquee's pixel rectangle, or a selection drag.
    pub fn phase(&self) -> SelectPhase {
        if let Some(inside) = &self.inside {
            return SelectPhase::Dragging(inside.kind());
        }
        match &self.press {
            Some(press) => SelectPhase::Marquee { rect: self.pixels(press) },
            None => SelectPhase::Idle,
        }
    }

    /// The pixel rectangle the release would select now, `None` while the drag is a click (not for Fixed Size) or has
    /// no area. A one-pixel row or column is a selection. Square is the gesture's Shift, or a Fixed Ratio of 1 : 1.
    fn pixels(&self, press: &MarqueePress) -> Option<Rect> {
        if self.kind.is_line() {
            return single_line(self.kind, press.size, press.last);
        }
        let corners = press.corners.filter(|_| press.gesture.has_begun())?;
        let (a, b) = (corners.a, corners.b);
        let mut rect = match self.constraint {
            Constraint::Size(w, h) => fixed_size_pixels(a, b, (w, h)),
            _ => marquee_pixels(a, b),
        };
        let square = match self.constraint {
            Constraint::Size(..) => false,
            Constraint::Ratio(w, h) if w > 0.0 && h > 0.0 => w == h,
            _ => press.gesture.square_and_centre().0,
        };
        if square {
            // The corners run from the press point towards the pointer, whichever way Alt centres them.
            rect = square_pixels(rect, a, b);
        }
        (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
    }

    /// Follows the pointer to document point `at`: the gesture records the keys, and once begun sizes the marquee to
    /// the pointer (Space moves it).
    fn follow(&mut self, pointer: &ToolPointer, at: Point, space: bool) {
        let (constraint, kind) = (self.constraint, self.kind);
        let Some(press) = self.press.as_mut() else { return };
        press.gesture.follow(Some(pointer.pos), pointer.mods);
        press.last = at;
        if press.gesture.has_begun() && !kind.is_line() {
            press.corners = Some(press.gesture.size_to(at, space, constraint));
        }
    }

    /// The W and H readout beside the pointer.
    fn show_readout(&self, cx: &mut ToolContext<'_>) {
        let Some(rect) = self.press.as_ref().and_then(|press| self.pixels(press)) else { return };
        let (words, units) = (cx.words::<SelectWords>(), cx.units());
        cx.readout(vec![(words.width.clone(), units.pixels(rect.width())), (words.height.clone(), units.pixels(rect.height()))]);
    }
}

/// The one-pixel row (column) under `at`, kept on the canvas, across the whole `size`.
fn single_line(kind: MarqueeKind, size: [u32; 2], at: Point) -> Option<Rect> {
    let [w, h] = size.map(f64::from);
    if w < 1.0 || h < 1.0 || !at.x.is_finite() || !at.y.is_finite() {
        return None;
    }
    match kind {
        MarqueeKind::SingleRow => {
            let y = at.y.floor().clamp(0.0, h - 1.0);
            Some(Rect::new(0.0, y, w, y + 1.0))
        }
        MarqueeKind::SingleColumn => {
            let x = at.x.floor().clamp(0.0, w - 1.0);
            Some(Rect::new(x, 0.0, x + 1.0, h))
        }
        MarqueeKind::Rectangle | MarqueeKind::Ellipse => None,
    }
}

/// The outline of `rect` as document points: its corners, or the ellipse inscribed in it.
fn outline(rect: Rect, ellipse: bool) -> Vec<Point> {
    if !ellipse {
        return vec![Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)];
    }
    let (centre, radius) = (rect.center(), (rect.width() / 2.0, rect.height() / 2.0));
    (0..ELLIPSE_SEGMENTS)
        .map(|step| {
            let angle = std::f64::consts::TAU * step as f64 / ELLIPSE_SEGMENTS as f64;
            Point::new(centre.x + radius.0 * angle.cos(), centre.y + radius.1 * angle.sin())
        })
        .collect()
}

impl CanvasTool for SelectToolPass<'_, MarqueeTool> {
    /// The crosshair with the operation's badge (live modifiers, or the latched one during the drag); over the
    /// selection the cursors of [`cursor_over`].
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let (tool, probe) = (&*self.tool, self.probe);
        let (Some(pointer), Some(place)) = (cx.pointer, Place::of(cx)) else { return CursorIcon::Crosshair.into() };
        let dragging = tool.inside.as_ref().map(InsideDrag::kind);
        if tool.press.is_none()
            && !tool.kind.is_line()
            && let Some(cursor) = cursor_over(probe, place.doc(pointer.pos), cx.mods, tool.bar, PressRule::Drags, dragging)
        {
            return cursor;
        }
        // Cmd is the Move tool's: no badge for the operation.
        let badge = match &tool.press {
            Some(press) => press.operation.badge(),
            None if cx.mods.cmd => None,
            None => SelectionOperation::resolve(tool.bar, cx.mods, probe.has_selection()).badge(),
        };
        badge.map_or(CursorIcon::Crosshair.into(), |badge| PaintedCursor::badged(CursorIcon::Crosshair, badge).into())
    }

    /// A press inside the selection begins a selection drag; a Cmd press outside is `Ignored`; otherwise the marquee
    /// begins at the pointer's document point with the operation latched.
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return ToolAction::Ignored };
        tool.reset();
        let raw = place.doc(pointer.pos);
        let at = cx.snap(raw);
        if !tool.kind.is_line()
            && let Some(kind) = drag_kind(probe, raw, pointer.mods, tool.bar, PressRule::Drags)
        {
            let id = tool.report.gesture();
            let label = cx.words::<SelectWords>().move_selection.as_str().into();
            let (drag, event) = InsideDrag::begin(&ConstrainStyle::new(f64::from(cx.slop), f64::from(cx.slop)), kind, at, pointer.mods, id, label);
            tool.inside = Some(drag);
            tool.report.push(event);
            return ToolAction::Claimed;
        }
        if pointer.mods.cmd {
            return ToolAction::Ignored;
        }
        let size = probe.size();
        tool.press = Some(MarqueePress {
            gesture: MarqueeGesture::press(pointer.pos, at, pointer.mods),
            operation: SelectionOperation::resolve(tool.bar, pointer.mods, probe.has_selection()),
            size: [size.width, size.height],
            last: at,
            corners: None,
        });
        // A Fixed Size box shows at the press, so a click places it.
        if matches!(tool.constraint, Constraint::Size(..)) && !tool.kind.is_line() {
            if let Some(press) = tool.press.as_mut() {
                press.gesture.begin();
            }
            tool.follow(pointer, at, false);
        }
        tool.show_readout(cx);
        ToolAction::Claimed
    }

    /// Sizes (or with Space moves) the marquee, or moves the selection drag; sets the W and H readout.
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.as_mut() {
            if let Some(event) = inside.drag(cx.snap(place.doc(pointer.pos)), pointer.mods, place.zoom()) {
                tool.report.push(event);
            }
            return;
        }
        if let Some(press) = tool.press.as_mut() {
            press.gesture.begin();
        }
        let at = cx.snap(place.doc(pointer.pos));
        tool.follow(pointer, at, cx.held(Key::Space));
        tool.show_readout(cx);
    }

    /// Commits: `Commit { shape, operation }` for a rectangle or ellipse with area, `Deselect` for a click, the
    /// drag's `Commit` for a selection drag.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        let Some(place) = Place::of(cx) else { return };
        let at = cx.snap(place.doc(pointer.pos));
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.end(at, pointer.mods, place.zoom()));
            return;
        }
        tool.follow(pointer, at, cx.held(Key::Space));
        let Some(press) = tool.press.take() else { return };
        let event = match tool.pixels(&press) {
            Some(rect) if tool.kind == MarqueeKind::Ellipse => {
                SelectEvent::Commit { shape: SelectionShape::Ellipse(rect), operation: press.operation }
            }
            Some(rect) => SelectEvent::Commit { shape: SelectionShape::Rectangle(rect), operation: press.operation },
            None => SelectEvent::Deselect,
        };
        tool.report.push(event);
    }

    /// Escape, a lost pointer or a tool switch: the marquee is dropped silently; a selection drag reports its
    /// `Cancel`.
    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.cancel(reason));
        }
        tool.press = None;
    }

    /// The ants of the marquee being dragged.
    fn paint(&self, _draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(place), Some(rect)) = (Place::of(cx), self.tool.press.as_ref().and_then(|press| self.tool.pixels(press))) else { return };
        let points: Vec<Pos2> = outline(rect, self.tool.kind == MarqueeKind::Ellipse).into_iter().map(|at| place.screen(at)).collect();
        cx.marching_ants(&points, true);
    }

    /// The marquee being dragged as one object: named `SelectWords::outline`, valued with its pixel size, over the
    /// pixel rectangle; `None` with no marquee.
    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        let rect = self.tool.press.as_ref().and_then(|press| self.tool.pixels(press))?;
        let (words, units) = (cx.words::<SelectWords>(), cx.units());
        let value = words.size.fill(&[("w", &units.pixels(rect.width())), ("h", &units.pixels(rect.height()))]);
        Some(ToolAccess::new(words.outline.as_str()).value(value).bounds(rect))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

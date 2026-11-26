//! The selection tools on the tool host: each test drives one tool through a `ToolFixture` with injected pointer input
//! and a fake `SelectionProbe`, then reads the events the tool reports.

use craft_behavior::geometry::{Constraint, SelectionOperation, SnapKind, SnapOptions, SnapTarget, SnapTargets};
use craft_behavior::{Axis, CancelReason, EditEvent, PixelSize, PointerButton};
use craft_imaging::ui::select::{
    DragKind, EdgeSearch, EdgeTracer, LassoTool, MagneticLassoTool, MagneticSettings, MarqueeKind, MarqueeTool, PolygonLassoTool, SelectEvent,
    SelectPhase, SelectToolPass, SelectionDrag, SelectionProbe, SelectionShape,
};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{CanvasTool, Snap, ToolAccess, ToolContext, ToolHost, ToolInput, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, pointer, trial};
use egui::{Key, Modifiers, Pos2, Rect as ScreenRect, pos2};
use kurbo::{Point, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const SIZE: PixelSize = PixelSize::new(400, 300);
const NONE: Mods = Mods::NONE;

/// A document with an optional rectangle selection and an optional floating piece where it is shown.
#[derive(Default)]
struct Fake {
    selection: Option<Rect>,
    floating: Option<Rect>,
    document: u64,
}

impl Fake {
    fn empty() -> Self {
        Fake { document: 1, ..Fake::default() }
    }

    fn selected(selection: Rect) -> Self {
        Fake { selection: Some(selection), ..Fake::empty() }
    }

    /// A selection whose pixels float, shown at `piece`.
    fn floating(selection: Rect, piece: Rect) -> Self {
        Fake { floating: Some(piece), ..Fake::selected(selection) }
    }
}

impl SelectionProbe for Fake {
    fn document(&self) -> u64 {
        self.document
    }

    fn size(&self) -> PixelSize {
        SIZE
    }

    fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    fn inside_selection(&self, at: Point) -> bool {
        self.selection.is_some_and(|rect| rect.contains(at))
    }

    fn bounds(&self) -> Option<Rect> {
        self.selection
    }

    fn floating(&self) -> bool {
        self.floating.is_some()
    }

    fn on_floating(&self, at: Point) -> bool {
        self.floating.is_some_and(|piece| piece.contains(at))
    }
}

/// What the tests read from every tool.
trait Reports {
    fn events(&mut self) -> Vec<SelectEvent>;
    fn phase(&self) -> SelectPhase;
    fn reset(&mut self);
}

macro_rules! reports {
    ($tool:ty $(, $param:ident: $bound:path)?) => {
        impl$(<$param: $bound>)? Reports for $tool {
            fn events(&mut self) -> Vec<SelectEvent> {
                self.take_events()
            }

            fn phase(&self) -> SelectPhase {
                self.phase()
            }

            fn reset(&mut self) {
                self.reset();
            }
        }
    };
}

reports!(MarqueeTool);
reports!(LassoTool);
reports!(PolygonLassoTool);
reports!(MagneticLassoTool<T>, T: EdgeTracer);

/// A tool with its probe, owning both so a fixture can hold them: every call builds the pass the app builds.
struct Owned<T> {
    tool: RefCell<T>,
    probe: Fake,
}

impl<T: Reports> Owned<T> {
    fn new(tool: T, probe: Fake) -> Self {
        Owned { tool: RefCell::new(tool), probe }
    }

    fn tool(&mut self) -> &mut T {
        self.tool.get_mut()
    }

    fn events(&mut self) -> Vec<SelectEvent> {
        self.tool().events()
    }

    fn phase(&self) -> SelectPhase {
        self.tool.borrow().phase()
    }
}

impl<T: Reports + 'static> CanvasTool for Owned<T>
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).each_pass(cx);
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> craft_ui::canvas::ToolAction {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).press(pointer, cx)
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).drag(pointer, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).release(pointer, cx);
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).hover(pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).cancel(reason, cx);
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        SelectToolPass::new(self.tool.get_mut(), &self.probe).key(key, cx)
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).paint(draw, cx);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> craft_behavior::Cursor {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).cursor(cx)
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).takes(button, mods)
    }

    fn claims(&self) -> craft_ui::canvas::ToolClaims {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).claims()
    }

    fn takes_every_move(&self) -> bool {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).takes_every_move()
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).access(cx)
    }

    fn command_form(&self) -> ToolPath {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).command_form()
    }
}

type Rig<T> = ToolFixture<Owned<T>>;

/// The view in which a canvas's document point and screen point coincide.
fn same_points(room: ScreenRect) -> craft_behavior::CanvasView {
    let centre = room.center();
    craft_behavior::CanvasView::new(room, Point::new(f64::from(centre.x), f64::from(centre.y)), 1.0)
}

/// A canvas whose document point and screen point coincide.
fn rig<T: Reports + 'static>(label: &'static str, tool: T, probe: Fake) -> Rig<T>
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    ToolFixture::with_view(label, Owned::new(tool, probe), same_points, |_, _| {})
}

/// [`rig`] on a canvas that snaps to a vertical guide at x = 100, on macOS, where Control alone suspends snapping.
fn snapped_rig<T: Reports + 'static>(label: &'static str, tool: T, probe: Fake) -> Rig<T>
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    let mut targets = SnapTargets::new();
    targets.add(SnapTarget::new(Axis::Y, 100.0, SnapKind::Guide));
    let snap = Snap::new(Arc::new(targets), SnapOptions::default());
    let mut rig = ToolFixture::with_snap(label, Owned::new(tool, probe), same_points, snap, |_, _| {});
    rig.fixture().os(Platform::Mac);
    rig.fixture().frame(Vec::new());
    rig
}

fn at(point: (f64, f64)) -> Pos2 {
    pos2(point.0 as f32, point.1 as f32)
}

fn inject<T: Reports + 'static>(rig: &mut Rig<T>, input: ToolInput)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    let (ctx, id) = (rig.fixture().ctx().clone(), rig.canvas_id());
    ToolHost::inject(&ctx, id, input);
    rig.fixture().run_injected();
}

fn down<T: Reports + 'static>(rig: &mut Rig<T>, point: (f64, f64), mods: Mods)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    inject(rig, ToolInput::down(PointerButton::Primary, at(point)).mods(mods));
}

fn drag_to<T: Reports + 'static>(rig: &mut Rig<T>, point: (f64, f64), mods: Mods)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    inject(rig, ToolInput::moved(at(point)).mods(mods));
}

fn up<T: Reports + 'static>(rig: &mut Rig<T>, point: (f64, f64), mods: Mods)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    inject(rig, ToolInput::up(at(point)).mods(mods));
}

fn click<T: Reports + 'static>(rig: &mut Rig<T>, point: (f64, f64), mods: Mods)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    down(rig, point, mods);
    up(rig, point, mods);
}

/// A press, a drag and a release; the events.
fn stroke(rig: &mut Rig<MarqueeTool>, from: (f64, f64), to: (f64, f64), at_press: Mods, later: Mods) -> Vec<SelectEvent> {
    down(rig, from, at_press);
    drag_to(rig, to, later);
    up(rig, to, later);
    rig.tool_mut().events()
}

/// Holds `mods` from now on; one pass shows them to the tool.
fn hold<T: Reports + 'static>(rig: &mut Rig<T>, mods: Modifiers)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    rig.fixture().hold(mods).frame(Vec::new());
}

/// Gives the canvas the keyboard with a click that the tool then forgets.
fn focus<T: Reports + 'static>(rig: &mut Rig<T>)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    let corner = rig.canvas().rect.left_top() + egui::vec2(2.0, 2.0);
    rig.press_at(corner).release_at(corner);
    let mut tool = rig.tool_mut();
    tool.tool().reset();
    tool.events();
    drop(tool);
    // One pass drops the node the click's outline left in the tree.
    rig.fixture().frame(Vec::new());
}

fn press_key<T: Reports + 'static>(rig: &mut Rig<T>, key: Key, mods: Modifiers)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    rig.fixture().key(key, mods);
}

fn commit(rect: (f64, f64, f64, f64), operation: SelectionOperation) -> SelectEvent {
    SelectEvent::commit(SelectionShape::Rectangle(Rect::new(rect.0, rect.1, rect.2, rect.3)), operation)
}

fn polygon(points: &[(f64, f64)], operation: SelectionOperation) -> SelectEvent {
    SelectEvent::commit(SelectionShape::Polygon(points.iter().map(|&(x, y)| Point::new(x, y)).collect()), operation)
}

/// "begin", "update 1,0", "commit 2,0", "cancel"; other events as `Debug`.
fn tags(events: &[SelectEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            SelectEvent::DragSelection(EditEvent::Begin { .. }) => "begin".to_owned(),
            SelectEvent::DragSelection(EditEvent::Update { value, .. }) => format!("update {},{}", value.delta[0], value.delta[1]),
            SelectEvent::DragSelection(EditEvent::Commit { after, .. }) => format!("commit {},{}", after.delta[0], after.delta[1]),
            SelectEvent::DragSelection(EditEvent::Cancel { .. }) => "cancel".to_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// Cancels the gesture the tool holds, as the app does on a tool switch.
fn cancel_with<T: Reports + 'static>(rig: &mut Rig<T>, reason: CancelReason)
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    let (ctx, id) = (rig.fixture().ctx().clone(), rig.canvas_id());
    let _ = ToolHost::new().cancel(&ctx, id, &mut *rig.tool_mut(), reason);
    rig.fixture().frame(Vec::new());
}

// ---- the shared tool row ----

// ---- the marquees ----

/// The rectangle of the one `Commit` in `events`.
fn committed_rect(events: &[SelectEvent]) -> Rect {
    match events {
        [SelectEvent::Commit { shape: SelectionShape::Rectangle(rect), .. }] => *rect,
        _ => panic!("one rectangle commit: {events:?}"),
    }
}

// Normal style: Shift pressed after the press squares, Alt pressed after it centres. With nothing selected Shift and
// Alt held at the press square and centre from the first move; with a selection they pick the operation only. Fixed
// Ratio and Fixed Size give whole-pixel rectangles, and a Shift square pressed between pixels has equal sides. The
// Single Row and Single Column Marquees keep a one-pixel drag. On a floating piece a plain press drags the outline and
// Cmd the piece; a Single Row press inside the selection drags the outline, and one released where it was pressed
// selects the row under it.
#[trial]
fn marquee_corners_follow_constraint_and_modifiers() {
    let mut marquee = rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::empty());
    let events = stroke(&mut marquee, (10.5, 10.5), (50.5, 30.5), NONE, NONE);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 31.0), SelectionOperation::New)]);

    let events = stroke(&mut marquee, (10.5, 10.5), (50.5, 30.5), NONE, Mods::SHIFT);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 51.0), SelectionOperation::New)], "Shift after the press squares");

    let events = stroke(&mut marquee, (100.0, 100.0), (110.0, 120.0), NONE, Mods::ALT);
    assert_eq!(events, [commit((90.0, 80.0, 110.0, 120.0), SelectionOperation::New)], "Alt after the press centres");

    // #11389: with nothing selected the press modifiers count from the first move.
    let events = stroke(&mut marquee, (10.5, 10.5), (50.5, 30.5), Mods::SHIFT, Mods::SHIFT);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 51.0), SelectionOperation::Add)], "Shift at the press squares with nothing selected");
    let events = stroke(&mut marquee, (100.0, 100.0), (110.0, 120.0), Mods::ALT, Mods::ALT);
    assert_eq!(events, [commit((90.0, 80.0, 110.0, 120.0), SelectionOperation::New)], "Alt at the press centres with nothing selected");

    // #5472: a square pressed between pixels.
    let square = committed_rect(&stroke(&mut marquee, (10.3, 10.7), (50.6, 30.2), NONE, Mods::SHIFT));
    assert!(square.width() > 0.0 && square.width() == square.height(), "{square:?}");

    marquee.tool_mut().tool().constraint(Constraint::Ratio(2.0, 1.0));
    let events = stroke(&mut marquee, (10.0, 10.0), (50.0, 12.0), NONE, NONE);
    assert_eq!(events, [commit((10.0, 10.0, 50.0, 30.0), SelectionOperation::New)]);

    marquee.tool_mut().tool().constraint(Constraint::Size(10.0, 10.0));
    let events = stroke(&mut marquee, (20.0, 20.0), (40.4, 60.4), NONE, NONE);
    assert_eq!(events, [commit((40.0, 60.0, 50.0, 70.0), SelectionOperation::New)], "the box follows the pointer");
    // #6015: Fixed Size between pixels is exactly its size.
    let fixed = committed_rect(&stroke(&mut marquee, (20.3, 20.6), (40.7, 60.2), NONE, NONE));
    assert_eq!((fixed.width(), fixed.height()), (10.0, 10.0), "{fixed:?}");

    // With a selection the press modifiers pick the operation and neither square nor centre.
    let mut marquee = rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::selected(Rect::new(300.0, 250.0, 350.0, 290.0)));
    let events = stroke(&mut marquee, (10.5, 10.5), (50.5, 30.5), Mods::SHIFT, Mods::SHIFT);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 31.0), SelectionOperation::Add)], "Shift at the press is Add, not square");
    let events = stroke(&mut marquee, (100.0, 100.0), (110.0, 120.0), Mods::ALT, Mods::ALT);
    assert_eq!(events, [commit((100.0, 100.0, 110.0, 120.0), SelectionOperation::Subtract)], "Alt at the press is Subtract, not centred");

    // #2391: a one-pixel line drag is a selection.
    let mut marquee = rig("Marquee", MarqueeTool::new(MarqueeKind::SingleRow), Fake::empty());
    let events = stroke(&mut marquee, (30.5, 20.5), (31.5, 20.5), NONE, NONE);
    assert_eq!(events, [commit((0.0, 20.0, 400.0, 21.0), SelectionOperation::New)]);
    marquee.tool_mut().tool().kind(MarqueeKind::SingleColumn);
    let events = stroke(&mut marquee, (30.5, 20.5), (30.5, 21.5), NONE, NONE);
    assert_eq!(events, [commit((30.0, 0.0, 31.0, 300.0), SelectionOperation::New)]);

    // #11368: on a floating piece (the selection shown 20 pixels to the right) a plain press drags the outline and Cmd
    // the piece; a press beside the piece, still inside the selection, starts a new marquee.
    let piece = Rect::new(120.0, 100.0, 220.0, 200.0);
    let mut marquee = rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::floating(Rect::new(100.0, 100.0, 200.0, 200.0), piece));
    down(&mut marquee, (210.0, 150.0), NONE);
    assert_eq!(marquee.tool().phase(), SelectPhase::Dragging(SelectionDrag::new(DragKind::Outline, [0, 0])), "a plain press on the piece");
    up(&mut marquee, (210.0, 150.0), NONE);
    assert_eq!(tags(&marquee.tool_mut().events()), ["begin", "commit 0,0"]);
    down(&mut marquee, (210.0, 150.0), Mods::CMD);
    assert_eq!(marquee.tool().phase(), SelectPhase::Dragging(SelectionDrag::new(DragKind::pixels(false), [0, 0])), "a Cmd press on the piece");
    up(&mut marquee, (210.0, 150.0), Mods::CMD);
    marquee.tool_mut().events();
    down(&mut marquee, (110.0, 150.0), NONE);
    assert!(matches!(marquee.tool().phase(), SelectPhase::Marquee { .. }), "beside the piece: {:?}", marquee.tool().phase());
    up(&mut marquee, (110.0, 150.0), NONE);
    marquee.tool_mut().events();

    // #11392: a Single Row press inside the selection drags the outline; released where it was pressed it selects the
    // row under the press (the drag's Cancel, then the Commit).
    let mut marquee = rig("Marquee", MarqueeTool::new(MarqueeKind::SingleRow), Fake::selected(Rect::new(100.0, 100.0, 200.0, 200.0)));
    down(&mut marquee, (150.0, 150.5), NONE);
    drag_to(&mut marquee, (170.0, 150.5), NONE);
    up(&mut marquee, (170.0, 150.5), NONE);
    assert_eq!(tags(&marquee.tool_mut().events()), ["begin", "update 20,0", "commit 20,0"]);
    click(&mut marquee, (150.0, 120.5), NONE);
    let events = marquee.tool_mut().events();
    assert_eq!(tags(&events[..2]), ["begin", "cancel"], "{events:?}");
    assert_eq!(events[2..], [commit((0.0, 120.0, 400.0, 121.0), SelectionOperation::New)]);
}

// Cmd inside the selection drags pixels, Cmd+Alt a copy; Cmd outside it is the Move tool's, so the host starts no
// gesture.

// A selection drag reports Begin, whole-pixel Updates (only when the rounded delta changes) and Commit; a cancel reports
// Cancel and nothing else; a release without travel commits a zero delta.
#[trial]
fn marquee_selection_drag_reports_whole_pixel_deltas_and_cancel() {
    let mut rig = rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::selected(Rect::new(100.0, 100.0, 200.0, 200.0)));
    // The first move goes past the press arena's slop, so the press is a drag; the later ones move by parts of a pixel.
    down(&mut rig, (150.0, 150.0), NONE);
    drag_to(&mut rig, (160.2, 150.2), NONE);
    drag_to(&mut rig, (160.6, 150.0), NONE);
    drag_to(&mut rig, (160.7, 150.0), NONE);
    up(&mut rig, (162.0, 150.0), NONE);
    // The release elsewhere is a move there, then the button coming up.
    assert_eq!(tags(&rig.tool_mut().events()), ["begin", "update 10,0", "update 11,0", "update 12,0", "commit 12,0"]);

    click(&mut rig, (150.0, 150.0), NONE);
    assert_eq!(tags(&rig.tool_mut().events()), ["begin", "commit 0,0"]);

    down(&mut rig, (150.0, 150.0), NONE);
    drag_to(&mut rig, (170.0, 150.0), NONE);
    cancel_with(&mut rig, CancelReason::Removed);
    let events = rig.tool_mut().events();
    assert_eq!(tags(&events), ["begin", "update 20,0", "cancel"]);
    assert!(matches!(events.last(), Some(SelectEvent::DragSelection(EditEvent::Cancel { reason: CancelReason::Removed, .. }))), "{events:?}");
    assert_eq!(rig.tool().phase(), SelectPhase::Idle);
}

// A marquee corner snaps to the guide at x = 100. A dragged selection snaps by its bounds, once and before the delta
// rounds: from (35, 45) to (93, 45) its centre lands on the guide (55 pixels), where a second snap of the pointer
// itself would have moved it 70. Control suspends both.
#[trial]
fn a_marquee_corner_and_a_dragged_selection_snap_to_a_guide() {
    let mut rig = snapped_rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::empty());
    let events = stroke(&mut rig, (20.0, 20.0), (96.0, 60.0), NONE, NONE);
    assert_eq!(events, [commit((20.0, 20.0, 100.0, 60.0), SelectionOperation::New)], "the corner snaps to the guide");
    down(&mut rig, (20.0, 20.0), NONE);
    hold(&mut rig, Modifiers::CTRL);
    drag_to(&mut rig, (96.0, 60.0), Mods::CTRL);
    up(&mut rig, (96.0, 60.0), Mods::CTRL);
    hold(&mut rig, Modifiers::NONE);
    assert_eq!(rig.tool_mut().events(), [commit((20.0, 20.0, 96.0, 60.0), SelectionOperation::New)], "Control suspends the corner snap");

    let mut rig = snapped_rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::selected(Rect::new(30.0, 30.0, 60.0, 60.0)));
    down(&mut rig, (35.0, 45.0), NONE);
    drag_to(&mut rig, (93.0, 45.0), NONE);
    up(&mut rig, (93.0, 45.0), NONE);
    assert_eq!(tags(&rig.tool_mut().events()), ["begin", "update 55,0", "commit 55,0"], "the bounds snap once");
    down(&mut rig, (35.0, 45.0), NONE);
    hold(&mut rig, Modifiers::CTRL);
    drag_to(&mut rig, (93.0, 45.0), Mods::CTRL);
    up(&mut rig, (93.0, 45.0), Mods::CTRL);
    hold(&mut rig, Modifiers::NONE);
    assert_eq!(tags(&rig.tool_mut().events()), ["begin", "update 58,0", "commit 58,0"], "Control suspends the box snap");

    // A click inside a selection whose edge lies within the tolerance of the guide moves nothing and calls no snap of
    // its bounds.
    let near = Rect::new(72.0, 30.0, 97.0, 60.0);
    let mut rig = snapped_rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::selected(near));
    click(&mut rig, (80.0, 45.0), NONE);
    assert_eq!(tags(&rig.tool_mut().events()), ["begin", "commit 0,0"], "a click commits a zero delta");
    let mut rig = snapped_rig("Marquee", MarqueeTool::new(MarqueeKind::SingleRow), Fake::selected(near));
    click(&mut rig, (80.0, 45.5), NONE);
    let events = rig.tool_mut().events();
    assert_eq!(tags(&events[..2]), ["begin", "cancel"], "{events:?}");
    assert_eq!(events[2..], [commit((0.0, 45.0, 400.0, 46.0), SelectionOperation::New)], "the click selects the row");
    // The clicked line snaps as a point: 4.5 pixels left of the guide, the column on it.
    let mut rig = snapped_rig("Marquee", MarqueeTool::new(MarqueeKind::SingleColumn), Fake::selected(near));
    click(&mut rig, (95.5, 45.0), NONE);
    let events = rig.tool_mut().events();
    assert_eq!(events[2..], [commit((100.0, 0.0, 101.0, 300.0), SelectionOperation::New)], "the click selects the guide's column");
}

// Inside the click distance a selection drag still moves: one that snapping pulls back to its start (its edge on the
// guide, moved 2 pixels off it) changed nothing and ends with a cancel, not the click's zero commit, so the Single Row
// Marquee selects no row. A press whose pointer stays within half a pixel of it is a click, snapped or not.
#[trial]
fn a_selection_drag_that_snaps_back_is_no_click_and_a_jittered_press_is() {
    let on_guide = Rect::new(75.0, 30.0, 100.0, 60.0);
    for kind in [MarqueeKind::Rectangle, MarqueeKind::SingleRow] {
        let mut rig = snapped_rig("Marquee", MarqueeTool::new(kind), Fake::selected(on_guide));
        down(&mut rig, (80.0, 45.5), NONE);
        drag_to(&mut rig, (82.0, 45.5), NONE);
        up(&mut rig, (82.0, 45.5), NONE);
        assert_eq!(tags(&rig.tool_mut().events()), ["begin", "cancel"], "{kind:?}: snapped back, no click");

        down(&mut rig, (80.0, 45.5), NONE);
        drag_to(&mut rig, (80.2, 45.2), NONE);
        up(&mut rig, (80.2, 45.2), NONE);
        let events = rig.tool_mut().events();
        match kind {
            MarqueeKind::SingleRow => {
                assert_eq!(tags(&events[..2]), ["begin", "cancel"], "{events:?}");
                assert_eq!(events[2..], [commit((0.0, 45.0, 400.0, 46.0), SelectionOperation::New)], "the jittered click selects the row");
            }
            _ => assert_eq!(tags(&events), ["begin", "commit 0,0"], "{kind:?}: a jittered press is a click"),
        }
    }
}

// ---- the Lasso ----

// An open New outline hides the selection it will replace: the lasso reports it from the Alt-click on, not before or after.
#[trial]
fn an_open_new_lasso_outline_replaces_the_selection() {
    let mut rig = rig("Lasso", LassoTool::new(), Fake::empty());
    focus(&mut rig);
    assert!(!rig.tool_mut().tool().replaces_selection());
    rig.press_at(at((100.0, 100.0))).move_to(at((140.0, 100.0)));
    hold(&mut rig, Modifiers::ALT);
    rig.release_at(at((140.0, 100.0)));
    assert!(rig.tool_mut().tool().replaces_selection(), "the outline is open with the operation New");
    hold(&mut rig, Modifiers::NONE);
    assert!(!rig.tool_mut().tool().replaces_selection(), "closing it ends the replacement");
}

// Alt pressed during a freehand drag turns on straight segments, Alt-clicks add vertices and releasing Alt closes. A
// first press with Alt and nothing selected starts straight segments and stays New; Backspace and a secondary press
// each retract one vertex, and a click on the last point closes. Alt stays down across presses and releases, as a
// hand holds it, so the pointer and the keyboard here are the hand's (an injected release lets go of its modifiers).
#[trial]
fn lasso_alt_clicks_make_straight_segments_that_retract_and_close() {
    let hand_click = |rig: &mut Rig<LassoTool>, point: (f64, f64)| {
        rig.press_at(at(point)).release_at(at(point));
    };
    let mut rig = rig("Lasso", LassoTool::new(), Fake::empty());
    focus(&mut rig);
    rig.press_at(at((100.0, 100.0))).move_to(at((140.0, 100.0)));
    hold(&mut rig, Modifiers::ALT);
    rig.release_at(at((140.0, 100.0)));
    assert_eq!(rig.tool().phase(), SelectPhase::Outline { points: 2 }, "with Alt down the release keeps the outline open");
    hand_click(&mut rig, (140.0, 140.0));
    hand_click(&mut rig, (100.0, 140.0));
    assert!(rig.tool_mut().events().is_empty(), "Alt-clicks add vertices");
    hold(&mut rig, Modifiers::NONE);
    let events = rig.tool_mut().events();
    assert_eq!(events, [polygon(&[(100.0, 100.0), (140.0, 100.0), (140.0, 140.0), (100.0, 140.0)], SelectionOperation::New)], "releasing Alt closes");

    hold(&mut rig, Modifiers::ALT);
    for point in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0), (100.0, 200.0), (150.0, 250.0)] {
        hand_click(&mut rig, point);
    }
    assert_eq!(rig.tool().phase(), SelectPhase::Outline { points: 5 }, "a first Alt press with nothing selected starts straight segments");
    press_key(&mut rig, Key::Backspace, Modifiers::ALT);
    assert_eq!(rig.tool().phase(), SelectPhase::Outline { points: 4 }, "Backspace retracts");
    let right = at((300.0, 300.0));
    rig.fixture().frame(vec![pointer::moved(right)]).frame(vec![pointer::secondary(right, true)]).frame(vec![pointer::secondary(right, false)]);
    assert_eq!(rig.tool().phase(), SelectPhase::Outline { points: 3 }, "a secondary press retracts");
    hand_click(&mut rig, (200.0, 200.0));
    let events = rig.tool_mut().events();
    assert_eq!(
        events,
        [polygon(&[(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)], SelectionOperation::New)],
        "a click on the last point closes, New"
    );
    hold(&mut rig, Modifiers::NONE);
    assert!(rig.tool_mut().events().is_empty(), "nothing is left to close");
}

// A plain press inside the selection drags the outline instead of drawing.

// A click outside the selection deselects only when its operation is New: a Shift, Alt or Shift+Alt click, or a plain
// click with Add on the bar, keeps the selection (#13612). A drag from the same point still selects.
#[trial]
fn a_lasso_click_deselects_only_in_new_mode() {
    let selected = Rect::new(20.0, 20.0, 100.0, 100.0);
    let shift_alt = Mods { shift: true, alt: true, ..NONE };
    for (mods, deselects) in [(NONE, true), (Mods::SHIFT, false), (Mods::ALT, false), (shift_alt, false)] {
        let mut clicked = rig("Lasso", LassoTool::new(), Fake::selected(selected));
        click(&mut clicked, (200.0, 150.0), mods);
        let expected = if deselects { vec![SelectEvent::Deselect] } else { Vec::new() };
        assert_eq!(clicked.tool_mut().events(), expected, "a click with {mods:?}");
    }

    let mut adding = rig("Lasso", LassoTool::new(), Fake::selected(selected));
    adding.tool_mut().tool().operation(SelectionOperation::Add);
    click(&mut adding, (200.0, 150.0), NONE);
    assert!(adding.tool_mut().events().is_empty(), "a plain click with Add on the bar keeps the selection");

    let mut drawing = rig("Lasso", LassoTool::new(), Fake::selected(selected));
    down(&mut drawing, (200.0, 150.0), NONE);
    for point in [(260.0, 150.0), (260.0, 200.0), (230.0, 220.0)] {
        drag_to(&mut drawing, point, NONE);
    }
    up(&mut drawing, (200.0, 200.0), NONE);
    assert!(
        matches!(drawing.tool_mut().events().as_slice(), [SelectEvent::Commit { operation: SelectionOperation::New, .. }]),
        "a drag from outside the selection still selects"
    );
}

// A freehand outline drawn inside the click distance still closes into a selection: the lasso follows the pointer from
// the press, and only an outline of fewer than three points is a click.
#[trial]
fn a_lasso_drawn_inside_the_click_distance_selects() {
    let mut rig = rig("Lasso", LassoTool::new(), Fake::empty());
    down(&mut rig, (100.0, 100.0), NONE);
    for point in [(101.0, 100.0), (100.0, 100.75)] {
        drag_to(&mut rig, point, NONE);
    }
    up(&mut rig, (100.0, 100.75), NONE);
    assert!(
        matches!(rig.tool_mut().events().as_slice(), [SelectEvent::Commit { operation: SelectionOperation::New, .. }]),
        "a lasso a pixel across selects"
    );
}

// The outline keeps the operation its first press latched: Alt at the press over a selection subtracts, and letting go
// of Alt during the drag changes neither the operation the tool reports nor the commit's. No outline, no operation.
#[trial]
fn the_lasso_reports_the_operation_its_outline_latched() {
    let mut rig = rig("Lasso", LassoTool::new(), Fake::selected(Rect::new(20.0, 20.0, 100.0, 100.0)));
    assert_eq!(rig.tool_mut().tool().outline_operation(), None);
    down(&mut rig, (200.0, 150.0), Mods::ALT);
    drag_to(&mut rig, (260.0, 150.0), Mods::ALT);
    assert_eq!(rig.tool_mut().tool().outline_operation(), Some(SelectionOperation::Subtract));
    for point in [(260.0, 200.0), (230.0, 220.0)] {
        drag_to(&mut rig, point, NONE);
    }
    assert_eq!(rig.tool_mut().tool().outline_operation(), Some(SelectionOperation::Subtract), "Alt let go keeps the latched operation");
    up(&mut rig, (200.0, 200.0), NONE);
    assert!(
        matches!(rig.tool_mut().events().as_slice(), [SelectEvent::Commit { operation: SelectionOperation::Subtract, .. }]),
        "the commit subtracts"
    );
    assert_eq!(rig.tool_mut().tool().outline_operation(), None, "the closed outline has no operation");
}

// ---- the Polygonal Lasso ----

// Shift turns a segment to 45 degree steps from the last vertex, not for the first vertex and not with Alt.
#[trial]
fn polygon_shift_constrains_to_45_degrees() {
    let mut rig = rig("Polygonal Lasso", PolygonLassoTool::new(), Fake::empty());
    focus(&mut rig);
    hold(&mut rig, Modifiers::SHIFT);
    click(&mut rig, (50.0, 50.0), Mods::SHIFT);
    // 11 degrees from the last vertex rounds to 0, keeping the distance.
    click(&mut rig, (150.0, 70.0), Mods::SHIFT);
    hold(&mut rig, Modifiers::SHIFT | Modifiers::ALT);
    click(&mut rig, (140.0, 170.0), Mods { shift: true, alt: true, ..NONE });
    hold(&mut rig, Modifiers::NONE);
    press_key(&mut rig, Key::Enter, Modifiers::NONE);
    let events = rig.tool_mut().events();
    let [SelectEvent::Commit { shape: SelectionShape::Polygon(points), operation, .. }] = events.as_slice() else { panic!("one commit: {events:?}") };
    assert_eq!(*operation, SelectionOperation::Add);
    let turned = Point::new(50.0 + 100.0_f64.hypot(20.0), 50.0);
    assert_eq!(points.len(), 3);
    assert_eq!(points[0], Point::new(50.0, 50.0), "the first vertex is never turned");
    assert!((points[1] - turned).hypot() < 1e-9, "{:?}", points[1]);
    assert_eq!(points[2], Point::new(140.0, 170.0), "Shift with Alt keeps the point");
}

// One outline: Alt at the first click with a selection latches Subtract for every later click; Backspace and a
// secondary press each retract a vertex; an Alt drag adds freehand points; a double click closes. A second outline
// closes on a click on its start, with the operation New.
#[trial]
fn a_polygonal_lasso_closes_by_double_click_or_its_start_and_retracts_by_backspace_or_secondary_press() {
    let mut rig = rig("Polygonal Lasso", PolygonLassoTool::new(), Fake::selected(Rect::new(300.0, 250.0, 350.0, 290.0)));
    focus(&mut rig);
    hold(&mut rig, Modifiers::ALT);
    click(&mut rig, (100.0, 100.0), Mods::ALT);
    hold(&mut rig, Modifiers::NONE);
    for point in [(200.0, 100.0), (250.0, 150.0), (260.0, 160.0)] {
        click(&mut rig, point, NONE);
    }
    press_key(&mut rig, Key::Backspace, Modifiers::NONE);
    assert_eq!(rig.tool().phase(), SelectPhase::Outline { points: 3 }, "Backspace retracts");
    inject(&mut rig, ToolInput::down(PointerButton::Secondary, at((300.0, 50.0))));
    inject(&mut rig, ToolInput::up(at((300.0, 50.0))));
    assert_eq!(rig.tool().phase(), SelectPhase::Outline { points: 2 }, "a secondary press retracts");
    hold(&mut rig, Modifiers::ALT);
    down(&mut rig, (200.0, 200.0), Mods::ALT);
    drag_to(&mut rig, (180.0, 220.0), Mods::ALT);
    drag_to(&mut rig, (160.0, 230.0), Mods::ALT);
    up(&mut rig, (160.0, 230.0), Mods::ALT);
    hold(&mut rig, Modifiers::NONE);
    assert!(rig.tool_mut().events().is_empty(), "the outline stays open");
    rig.double_press_at(at((120.0, 200.0)));
    let events = rig.tool_mut().events();
    let drawn = [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0), (180.0, 220.0), (160.0, 230.0), (120.0, 200.0)];
    assert_eq!(events, [polygon(&drawn, SelectionOperation::Subtract)], "a double click closes; the Alt drag is freehand");

    for point in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0), (100.0, 100.0)] {
        click(&mut rig, point, NONE);
    }
    let events = rig.tool_mut().events();
    assert_eq!(events, [polygon(&[(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)], SelectionOperation::New)], "a click on the start closes");
}

// A new outline replaces the selection while it is open: true only for the operation New.

// ---- the Magnetic Lasso ----

const CENTRE: Point = Point::new(200.0, 150.0);
const RADIUS: f64 = 80.0;

/// What the fake tracer was asked.
#[derive(Debug, Default)]
struct Log {
    snaps: Cell<usize>,
    /// The `from` of every `trace`: the fastening points, with repeats.
    froms: RefCell<Vec<Point>>,
    searches: RefCell<Vec<EdgeSearch>>,
}

impl Log {
    /// The distinct points traces started from.
    fn starts(&self) -> usize {
        let mut seen: Vec<Point> = Vec::new();
        for &p in self.froms.borrow().iter() {
            if !seen.iter().any(|q| q.distance(p) < 1e-6) {
                seen.push(p);
            }
        }
        seen.len()
    }

    fn clear(&self) {
        self.snaps.set(0);
        self.froms.borrow_mut().clear();
        self.searches.borrow_mut().clear();
    }
}

/// Edges lie on a circle of radius 80 round (200, 150): `snap` finds the nearest circle point within the width, `trace`
/// walks the shorter arc.
#[derive(Clone, Debug)]
struct Circle {
    log: Rc<Log>,
}

fn on_circle(p: Point) -> bool {
    ((p - CENTRE).hypot() - RADIUS).abs() < 0.5
}

impl EdgeTracer for Circle {
    fn snap(&mut self, at: Point, search: EdgeSearch) -> Option<Point> {
        self.log.snaps.set(self.log.snaps.get() + 1);
        self.log.searches.borrow_mut().push(search);
        let d = at - CENTRE;
        let len = d.hypot();
        ((len - RADIUS).abs() <= search.width && len > 0.0).then(|| CENTRE + d * (RADIUS / len))
    }

    fn trace(&mut self, from: Point, to: Point, _guide: &[Point], search: EdgeSearch) -> Vec<Point> {
        self.log.froms.borrow_mut().push(from);
        self.log.searches.borrow_mut().push(search);
        if !on_circle(from) || !on_circle(to) {
            return vec![from, to];
        }
        let angle = |p: Point| (p.y - CENTRE.y).atan2(p.x - CENTRE.x);
        let mut delta = angle(to) - angle(from);
        if delta > std::f64::consts::PI {
            delta -= std::f64::consts::TAU;
        } else if delta < -std::f64::consts::PI {
            delta += std::f64::consts::TAU;
        }
        let n = ((delta.abs() * RADIUS / 2.0).ceil() as usize).max(1);
        let mut points: Vec<Point> = (1..n).map(|k| CENTRE + circle_offset(angle(from) + delta * k as f64 / n as f64)).collect();
        points.insert(0, from);
        points.push(to);
        points
    }
}

fn circle_offset(angle: f64) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin()) * RADIUS
}

fn on(angle: f64) -> Point {
    CENTRE + circle_offset(angle)
}

fn magnetic() -> (Rig<MagneticLassoTool<Circle>>, Rc<Log>) {
    let log = Rc::new(Log::default());
    let mut rig = rig("Magnetic Lasso", MagneticLassoTool::new(Circle { log: Rc::clone(&log) }), Fake::empty());
    focus(&mut rig);
    log.clear();
    (rig, log)
}

fn pt(point: Point) -> (f64, f64) {
    (point.x, point.y)
}

/// A hover to `point`.
fn glide(rig: &mut Rig<MagneticLassoTool<Circle>>, point: Point, mods: Mods) {
    inject(rig, ToolInput::moved(at(pt(point))).mods(mods));
}

/// The pointer moves along the circle from angle `from` to angle `to` in small samples, as a hand follows an edge.
fn glide_along(rig: &mut Rig<MagneticLassoTool<Circle>>, from: f64, to: f64) {
    let samples = ((to - from).abs() / 0.05).ceil() as usize;
    for k in 1..=samples {
        glide(rig, on(from + (to - from) * k as f64 / samples as f64), NONE);
    }
}

// Points fasten every `spacing` along the edge at frequencies 0, 57 and 100, and a long jump is walked in at most 256
// steps of `spacing / 2`.
#[trial]
fn magnetic_fastens_every_spacing_and_walks_jumps() {
    let start = on(0.0);
    let fastened = |frequency: f32| {
        let (mut rig, log) = magnetic();
        rig.tool_mut().tool().settings_mut().frequency = frequency;
        click(&mut rig, pt(start), NONE);
        // The pointer follows 2.5 rad (200 px of arc) of the circle.
        glide_along(&mut rig, 0.0, 2.5);
        log.starts()
    };
    let (slow, normal, fast) = (fastened(0.0), fastened(57.0), fastened(100.0));
    assert!(slow < normal && normal < fast, "{slow} {normal} {fast}");
    // 200 px at 47 px: the start and four fastened points.
    assert!((4..=6).contains(&normal), "{normal}");
    // A jump of 1220 points at a step of 4 is longer than 256 steps; the canvas is wide enough that it ends over it.
    let (mut rig, log) = magnetic();
    rig.fixture().room(egui::vec2(1600.0, 400.0)).frame(Vec::new());
    rig.tool_mut().tool().settings_mut().frequency = 100.0;
    click(&mut rig, pt(start), NONE);
    glide(&mut rig, Point::new(1500.0, 150.0), NONE);
    assert_eq!(log.snaps.get(), 1 + 256, "the start, then 256 steps");
}

/// The points of the one `Commit` with the operation New in `events`.
fn committed_polygon(events: &[SelectEvent]) -> Vec<Point> {
    match events {
        [SelectEvent::Commit { shape: SelectionShape::Polygon(points), operation: SelectionOperation::New, .. }] => points.clone(),
        _ => panic!("one New polygon commit: {events:?}"),
    }
}

// An outline traced along the circle closes on a click on its start, on a double click and on Enter, each into a
// polygon on the circle's edge. Backspace removes the fastening points one by one, the first one last: one press per
// fastening point drops the outline, and nothing is committed.
#[trial]
fn magnetic_closes_by_start_click_double_click_or_enter_and_retracts_to_the_first() {
    let on_edge = |points: &[Point]| points.len() >= 3 && points.iter().all(|&p| on_circle(p));

    let (mut rig, _) = magnetic();
    click(&mut rig, pt(on(0.0)), NONE);
    // Round the circle back to the start, so the pointer reaches the start along the edge.
    glide_along(&mut rig, 0.0, std::f64::consts::TAU);
    // A second apart, so the click on the start is not a double click.
    rig.fixture().advance(1.0);
    click(&mut rig, pt(on(0.0)), NONE);
    let points = committed_polygon(&rig.tool_mut().events());
    assert!(on_edge(&points), "a click on the start closes: {points:?}");
    assert_eq!(rig.tool().phase(), SelectPhase::Idle);

    let (mut rig, _) = magnetic();
    click(&mut rig, pt(on(0.0)), NONE);
    glide_along(&mut rig, 0.0, 1.5);
    rig.double_press_at(at(pt(on(1.5))));
    let points = committed_polygon(&rig.tool_mut().events());
    assert!(on_edge(&points), "a double click closes: {points:?}");

    let (mut rig, _) = magnetic();
    click(&mut rig, pt(on(0.0)), NONE);
    glide_along(&mut rig, 0.0, 1.5);
    press_key(&mut rig, Key::Enter, Modifiers::NONE);
    let points = committed_polygon(&rig.tool_mut().events());
    assert!(on_edge(&points), "Enter closes: {points:?}");

    let (mut rig, log) = magnetic();
    click(&mut rig, pt(on(0.0)), NONE);
    glide_along(&mut rig, 0.0, 2.5);
    let fastened = log.starts();
    assert!(fastened >= 3, "{fastened}");
    let mut presses = 0;
    loop {
        let phase = rig.tool().phase();
        if phase == SelectPhase::Idle || presses > fastened {
            break;
        }
        assert!(matches!(phase, SelectPhase::Outline { .. }), "{phase:?}");
        press_key(&mut rig, Key::Backspace, Modifiers::NONE);
        presses += 1;
    }
    assert_eq!(presses, fastened, "one Backspace per fastening point, the first one last");
    assert!(rig.tool_mut().events().is_empty(), "retracting commits nothing");
}

// The first point snaps to the edge a few points from the press that placed it, farther than the slop: a click back
// where the outline began closes it, as a click on the start itself does.
#[trial]
fn a_click_where_the_magnetic_outline_began_closes_it() {
    let (mut rig, _) = magnetic();
    let began = CENTRE + circle_offset(0.0) * ((RADIUS + 5.0) / RADIUS);
    click(&mut rig, pt(began), NONE);
    glide_along(&mut rig, 0.0, std::f64::consts::TAU);
    rig.fixture().advance(1.0);
    click(&mut rig, pt(began), NONE);
    let points = committed_polygon(&rig.tool_mut().events());
    assert!(points.len() >= 3 && points.iter().all(|&p| on_circle(p)), "a click where the outline began closes: {points:?}");
}

// `[` and `]` change the width by 1 within 1..=256 while an outline is open.

// With `pressure_width` on, a pen sample at pressure 0.5 halves the searched width (minus one); a mouse sample never
// narrows it.

// While the outline is open the tool is one accessible object with its point count (fastening points and the live trace).

// The Magnetic Lasso's cursor follows the canvas's precise cursor (Other Cursors > Precise, swapped by Caps Lock):
// the width circle while it is on, the lasso icon otherwise (#11690).
#[trial]
fn the_magnetic_lasso_shows_the_width_circle_under_the_precise_cursor() {
    use craft_behavior::{Cursor, PaintedCursor};
    let (mut rig, _) = magnetic();
    let over = rig.canvas().center();
    let ctx = rig.fixture().ctx().clone();
    rig.fixture().frame(vec![pointer::moved(over)]);
    for caps_lock in [false, true, false] {
        craft_ui::runtime::windows::WindowHost::of(&ctx).set_caps_lock(Some(caps_lock));
        rig.fixture().frame(Vec::new());
        rig.fixture().frame(Vec::new());
        let cursor = craft_ui::runtime::cursor::current(&ctx);
        let circle = matches!(cursor, Some(Cursor::Painted(PaintedCursor::Circle { .. })));
        assert_eq!(circle, caps_lock, "Caps Lock {caps_lock}: {cursor:?}");
    }
}

/// The cursor `rig` shows over the middle of its canvas with `mods` held.
fn lasso_cursor<T: Reports + 'static>(rig: &mut Rig<T>, mods: Modifiers) -> Option<craft_behavior::Cursor>
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    let ctx = rig.fixture().ctx().clone();
    let over = rig.canvas().center();
    rig.fixture().frame(vec![pointer::moved(over)]);
    hold(rig, mods);
    rig.fixture().frame(Vec::new());
    craft_ui::runtime::cursor::current(&ctx)
}

// The lassos show an arrow with its tip where the outline starts and the tool's icon below right of it, as the reference
// application does; the operation's badge rides at the icon's lower right. With the icon alone on the pointer, its
// empty top-left corner was the hotspot, so the outline began beside the icon (photocraft#16671).
#[trial]
fn the_lassos_point_an_arrow_at_where_the_outline_starts() {
    use craft_behavior::{Cursor, CursorBadge, PaintedCursor, PointerGlyph};
    use craft_ui::primitives::ToolIcon;
    fn check(cursor: Option<Cursor>, icon: ToolIcon, badge: Option<CursorBadge>, what: &str) {
        match cursor {
            Some(Cursor::Painted(PaintedCursor::PointerTool { pointer, icon: shown, badge: b, .. })) => {
                assert_eq!(pointer, PointerGlyph::Arrow, "{what}: an arrow");
                assert_eq!(shown, icon.icon(), "{what}: the tool's icon");
                assert_eq!(b, badge, "{what}: the badge");
            }
            other => panic!("{what}: an arrow with the tool's icon, got {other:?}"),
        }
    }
    let selected = Rect::new(20.0, 20.0, 60.0, 60.0);
    let shift = Modifiers::SHIFT;
    let mut lasso = rig("Lasso", LassoTool::new(), Fake::selected(selected));
    check(lasso_cursor(&mut lasso, Modifiers::NONE), ToolIcon::Lasso, None, "Lasso");
    check(lasso_cursor(&mut lasso, shift), ToolIcon::Lasso, Some(CursorBadge::Add), "Lasso with Shift");
    let mut polygon = rig("Polygonal Lasso", PolygonLassoTool::new(), Fake::selected(selected));
    check(lasso_cursor(&mut polygon, Modifiers::NONE), ToolIcon::PolygonalLasso, None, "Polygonal Lasso");
    check(lasso_cursor(&mut polygon, Modifiers::ALT), ToolIcon::PolygonalLasso, Some(CursorBadge::Subtract), "Polygonal Lasso with Alt");
    let (mut magnetic, _) = magnetic();
    check(lasso_cursor(&mut magnetic, Modifiers::NONE), ToolIcon::MagneticLasso, None, "Magnetic Lasso");
}

#[trial]
fn width_steps_by_one_and_shift_jumps_to_the_limits() {
    let mut settings = MagneticSettings::default();
    settings.step_width(1, false);
    assert_eq!(settings.width, 11.0, "a step adds one to the default width");
    settings.width = 10.4;
    settings.step_width(1, false);
    assert_eq!(settings.width, 11.0, "a step adds one to the rounded width");
    settings.width = 1.0;
    settings.step_width(-1, false);
    assert_eq!(settings.width, 1.0, "the step stops at the minimum");
    settings.width = 256.0;
    settings.step_width(1, false);
    assert_eq!(settings.width, 256.0, "the step stops at the maximum");
    settings.step_width(-1, true);
    assert_eq!(settings.width, 1.0, "Shift+[ goes to the minimum");
    settings.step_width(1, true);
    assert_eq!(settings.width, 256.0, "Shift+] goes to the maximum");
    settings.width = f32::NAN;
    settings.step_width(1, false);
    assert_eq!(settings.width, MagneticSettings::default().width, "a width that is no number goes back to the default");
}

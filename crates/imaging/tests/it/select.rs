//! The selection tools on the tool host: each test drives one tool through a `ToolFixture` with injected pointer input
//! and a fake `SelectionProbe`, then reads the events the tool reports.

use craft_behavior::geometry::{Constraint, SelectionOperation};
use craft_behavior::{CancelReason, EditEvent, PixelSize, PointerButton};
use craft_imaging::ui::select::{
    EdgeSearch, EdgeTracer, MagneticLassoTool, MarqueeKind, MarqueeTool, PolygonLassoTool, SelectEvent, SelectPhase, SelectToolPass, SelectionProbe,
    SelectionShape,
};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolContext, ToolHost, ToolInput, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, pointer, trial};
use egui::{Key, Modifiers, Pos2, Rect as ScreenRect, pos2};
use kurbo::{Point, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SIZE: PixelSize = PixelSize::new(400, 300);
const NONE: Mods = Mods::NONE;

/// A document with an optional rectangle selection.
#[derive(Default)]
struct Fake {
    selection: Option<Rect>,
    document: u64,
}

impl Fake {
    fn empty() -> Self {
        Fake { document: 1, ..Fake::default() }
    }

    fn selected(selection: Rect) -> Self {
        Fake { selection: Some(selection), ..Fake::empty() }
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

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).access(cx)
    }

    fn command_form(&self) -> ToolPath {
        SelectToolPass::new(&mut *self.tool.borrow_mut(), &self.probe).command_form()
    }
}

type Rig<T> = ToolFixture<Owned<T>>;

/// A canvas whose document point and screen point coincide.
fn rig<T: Reports + 'static>(label: &'static str, tool: T, probe: Fake) -> Rig<T>
where
    for<'a> SelectToolPass<'a, T>: CanvasTool,
{
    let view = |room: ScreenRect| {
        let centre = room.center();
        craft_behavior::CanvasView::new(room, Point::new(f64::from(centre.x), f64::from(centre.y)), 1.0)
    };
    ToolFixture::with_view(label, Owned::new(tool, probe), view, |_, _| {})
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
    rig.fixture().frame(Vec::new());
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
    let _ = ToolHost::cancel(&ctx, id, &mut *rig.tool_mut(), reason);
    rig.fixture().frame(Vec::new());
}

// ---- the shared tool row ----

// ---- the marquees ----

// Normal style: Shift pressed after the press squares, Alt pressed after it centres, Shift held at the press does not
// (it is Add); Fixed Ratio and Fixed Size give whole-pixel rectangles.
#[trial]
fn marquee_corners_follow_constraint_and_modifiers() {
    let mut rig = rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::empty());
    let events = stroke(&mut rig, (10.5, 10.5), (50.5, 30.5), NONE, NONE);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 31.0), SelectionOperation::New)]);

    let events = stroke(&mut rig, (10.5, 10.5), (50.5, 30.5), NONE, Mods::SHIFT);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 51.0), SelectionOperation::New)], "Shift after the press squares");

    let events = stroke(&mut rig, (10.5, 10.5), (50.5, 30.5), Mods::SHIFT, Mods::SHIFT);
    assert_eq!(events, [commit((10.0, 10.0, 51.0, 31.0), SelectionOperation::Add)], "Shift at the press is Add, not square");

    let events = stroke(&mut rig, (100.0, 100.0), (110.0, 120.0), NONE, Mods::ALT);
    assert_eq!(events, [commit((90.0, 80.0, 110.0, 120.0), SelectionOperation::New)], "Alt after the press centres");

    rig.tool_mut().tool().constraint(Constraint::Ratio(2.0, 1.0));
    let events = stroke(&mut rig, (10.0, 10.0), (50.0, 12.0), NONE, NONE);
    assert_eq!(events, [commit((10.0, 10.0, 50.0, 30.0), SelectionOperation::New)]);

    rig.tool_mut().tool().constraint(Constraint::Size(10.0, 10.0));
    let events = stroke(&mut rig, (20.0, 20.0), (40.4, 60.4), NONE, NONE);
    assert_eq!(events, [commit((40.0, 60.0, 50.0, 70.0), SelectionOperation::New)], "the box follows the pointer");
}

// Cmd inside the selection drags pixels, Cmd+Alt a copy; Cmd outside it is the Move tool's, so the host starts no
// gesture.

// A selection drag reports Begin, whole-pixel Updates (only when the rounded delta changes) and Commit; a cancel reports
// Cancel and nothing else; a release without travel commits a zero delta.
#[trial]
fn marquee_selection_drag_reports_whole_pixel_deltas_and_cancel() {
    let mut rig = rig("Marquee", MarqueeTool::new(MarqueeKind::Rectangle), Fake::selected(Rect::new(100.0, 100.0, 200.0, 200.0)));
    down(&mut rig, (150.0, 150.0), NONE);
    drag_to(&mut rig, (150.2, 150.2), NONE);
    drag_to(&mut rig, (150.6, 150.0), NONE);
    drag_to(&mut rig, (150.7, 150.0), NONE);
    up(&mut rig, (152.0, 150.0), NONE);
    assert_eq!(tags(&rig.tool_mut().events()), ["begin", "update 1,0", "commit 2,0"]);

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

// ---- the Lasso ----

// A plain press inside the selection drags the outline instead of drawing.

// A press and release that draw fewer than 3 points (a click, or a two-point drag) deselect.

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
    // The real pointer goes with the injected one, so the hover each pass reports is this point and not where the pointer was left.
    let (ctx, id) = (rig.fixture().ctx().clone(), rig.canvas_id());
    ToolHost::inject(&ctx, id, ToolInput::moved(at(pt(point))).mods(mods));
    rig.fixture().frame(vec![pointer::moved(at(pt(point)))]);
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
    let (mut rig, log) = magnetic();
    rig.tool_mut().tool().settings_mut().frequency = 100.0;
    click(&mut rig, pt(start), NONE);
    glide(&mut rig, Point::new(5000.0, 150.0), NONE);
    assert_eq!(log.snaps.get(), 1 + 256, "the start, then 256 steps");
}

// `[` and `]` change the width by 1 within 1..=256 while an outline is open.

// With `pressure_width` on, a pen sample at pressure 0.5 halves the searched width (minus one); a mouse sample never
// narrows it.

// While the outline is open the tool is one accessible object with its point count (fastening points and the live trace).

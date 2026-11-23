//! The Ruler: its line model, the options bar, the handles and the tool, and the distances between boxes.

use craft_behavior::kurbo::{self, Point, Rect};
use craft_behavior::{Axis, CancelReason, CanvasView, EditEvent};
use craft_draw::ui::measure::{MIN_LINE, RulerLine, RulerOverlay, RulerTool, box_distances};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

/// Boxes apart on one axis give one gap, overlapping boxes the distances between like edges, diagonal boxes a line
/// through the first box's centre with dashed extensions of the second box's edges, a box inside another four
/// distances; zero and non-finite distances are dropped; labels come from the closure with the axis.
#[trial]
fn apart_overlapping_diagonal_and_inside() {
    let run = |a: Rect, b: Rect| {
        let (lines, measures) = box_distances(a, b, |len, axis| format!("{len}{}", if axis == Axis::X { "x" } else { "y" }));
        assert!(lines.iter().all(|l| l.extension));
        let measures: Vec<_> = measures.into_iter().map(|m| (m.a, m.b, m.label.unwrap_or_default())).collect();
        (measures, lines.into_iter().map(|l| (l.a, l.b)).collect::<Vec<_>>())
    };
    let p = Point::new;
    let (m, e) = run(Rect::new(0.0, 0.0, 40.0, 40.0), Rect::new(100.0, 20.0, 150.0, 80.0));
    assert_eq!(
        m,
        vec![
            (p(40.0, 30.0), p(100.0, 30.0), "60x".into()),
            (p(20.0, 0.0), p(20.0, 20.0), "20y".into()),
            (p(20.0, 40.0), p(20.0, 80.0), "40y".into())
        ]
    );
    assert_eq!(e, vec![(p(100.0, 20.0), p(20.0, 20.0)), (p(100.0, 80.0), p(20.0, 80.0))]);

    let (m, e) = run(Rect::new(100.0, 0.0, 140.0, 20.0), Rect::new(20.0, 70.0, 70.0, 90.0));
    assert_eq!(m, vec![(p(70.0, 10.0), p(100.0, 10.0), "30x".into()), (p(120.0, 20.0), p(120.0, 70.0), "50y".into())]);
    assert_eq!(e, vec![(p(70.0, 70.0), p(70.0, 10.0)), (p(70.0, 70.0), p(120.0, 70.0))]);

    let (m, e) = run(Rect::new(40.0, 40.0, 90.0, 80.0), Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!(m.len(), 4);
    assert_eq!((m[0].0, m[0].1), (p(0.0, 60.0), p(40.0, 60.0)));
    assert_eq!((m[3].0, m[3].1), (p(65.0, 80.0), p(65.0, 300.0)));
    assert!(e.is_empty());

    let canvas = Rect::new(0.0, 0.0, 400.0, 300.0);
    assert!(run(canvas, canvas).0.is_empty());
    assert!(run(Rect::new(f64::NAN, 0.0, 1.0, 1.0), canvas).0.is_empty());
}

/// The tool for the fixture: it builds a pass over the shared line for every call and applies the events the pass
/// reported, as the app would.
struct Driven {
    tool: RulerTool,
    line: Rc<RefCell<Option<RulerLine>>>,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Driven {
    fn sync(&mut self) {
        for event in self.tool.take_events() {
            let (phase, value) = match &event {
                EditEvent::Begin { .. } => ("begin", None),
                EditEvent::Update { value, .. } => ("update", Some(*value)),
                EditEvent::Commit { after, .. } => ("commit", Some(*after)),
                EditEvent::Set { after, .. } => ("set", Some(*after)),
                EditEvent::Cancel { before, .. } => (
                    "cancel",
                    before.as_ref().and_then(|b| match b {
                        craft_behavior::Mixed::Same(v) => Some(*v),
                        _ => None,
                    }),
                ),
                _ => ("other", None),
            };
            if let Some(value) = value {
                *self.line.borrow_mut() = value;
            }
            self.log.borrow_mut().push(phase);
        }
    }

    fn with_pass<R>(&mut self, call: impl FnOnce(&mut dyn CanvasTool) -> R) -> R {
        let line = *self.line.borrow();
        let result = call(&mut self.tool.pass(line));
        self.sync();
        result
    }
}

impl CanvasTool for Driven {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.with_pass(|pass| pass.press(pointer, cx))
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with_pass(|pass| pass.drag(pointer, cx));
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with_pass(|pass| pass.release(pointer, cx));
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.with_pass(|pass| pass.cancel(reason, cx));
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        self.with_pass(|pass| pass.key(key, cx))
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    // The Ruler's cursor reads only the line and the canvas, so a pass of a new tool gives it.
    fn cursor(&self, cx: &ToolContext<'_>) -> craft_behavior::Cursor {
        RulerTool::new().pass(*self.line.borrow()).cursor(cx)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Ruler Line... command")
    }
}

struct Shown {
    fixture: ToolFixture<Driven>,
    /// The line the tool's events left, as the app would store it.
    line: Rc<RefCell<Option<RulerLine>>>,
    /// The phases of the tool's and the overlay's edits, in order.
    log: Rc<RefCell<Vec<&'static str>>>,
}

fn shown(start: Option<RulerLine>) -> Shown {
    let line = Rc::new(RefCell::new(start));
    let log = Rc::new(RefCell::new(Vec::new()));
    let driven = Driven { tool: RulerTool::new(), line: Rc::clone(&line), log: Rc::clone(&log) };
    let body_line = Rc::clone(&line);
    let body_log = Rc::clone(&log);
    let fixture = ToolFixture::with_view(
        "Ruler",
        driven,
        |room| CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0),
        move |ui, body| {
            let mut line = *body_line.borrow();
            let out = RulerOverlay::new("Ruler", Bind::bound(&mut line)).show_at(ui, body.rect);
            body_log.borrow_mut().extend(out.events.iter().map(|event| match event {
                EditEvent::Begin { .. } => "begin",
                EditEvent::Update { .. } => "update",
                EditEvent::Commit { .. } => "commit",
                EditEvent::Set { .. } => "set",
                _ => "other",
            }));
            *body_line.borrow_mut() = line;
        },
    );
    Shown { fixture, line, log }
}

/// A line near the canvas's top left corner (the view puts the document origin at the room's corner), away from the
/// canvas's middle where the trials press, so a press there is the tool's and not a point of the line.
fn corner_line() -> RulerLine {
    RulerLine::new(Point::new(20.0, 20.0), Point::new(80.0, 20.0))
}

/// The release rule: with a line, a click (a press and release under `MIN_LINE`) commits the line from before the
/// press; with none it commits none; a drag past `MIN_LINE` commits the new line.
#[trial]
fn a_click_keeps_the_line_and_a_drag_replaces_it() {
    let before = corner_line();

    let mut with_line = shown(Some(before));
    let at = with_line.fixture.canvas().center();
    with_line.fixture.press_at(at).release_at(at);
    assert_eq!(*with_line.line.borrow(), Some(before), "a click keeps the line");

    let mut without = shown(None);
    let at = without.fixture.canvas().center();
    without.fixture.press_at(at).release_at(at);
    assert_eq!(*without.line.borrow(), None, "a click with no line makes none");

    let mut dragged = shown(Some(before));
    dragged.fixture.drag(egui::vec2(60.0, 40.0));
    let after = (*dragged.line.borrow()).expect("a drag makes a line");
    assert_ne!(after, before, "a drag replaces the line");
    assert!(after.start.distance(after.end) >= MIN_LINE);
}

/// L-81 Q4: Escape during a drag that replaces the line leaves the line as it was before the press.
#[trial]
fn escape_during_a_ruler_drag_keeps_the_line_from_before_the_press() {
    let before = corner_line();
    let mut ruler = shown(Some(before));
    let from = ruler.fixture.canvas().center();
    ruler.fixture.press_at(from);
    ruler.fixture.move_to(from + egui::vec2(20.0, 15.0));
    ruler.fixture.move_to(from + egui::vec2(40.0, 30.0));
    assert_ne!(*ruler.line.borrow(), Some(before), "the drag replaced the line while it ran");
    ruler.fixture.escape();
    assert_eq!(*ruler.line.borrow(), Some(before), "Escape puts the line from before the press back");
    ruler.fixture.release_at(from + egui::vec2(40.0, 30.0));
    assert_eq!(*ruler.line.borrow(), Some(before), "the release after Escape changes nothing");
}

/// The tool owes the canvas a command form and a clean gesture: one begin, one commit, and Escape cancels.
#[trial]
fn the_ruler_tool_passes_the_tool_checks() {
    shown(None).fixture.check();
}

impl Shown {
    /// The screen point of document point `p`: the view puts the room's centre at the document's (w/2, h/2).
    fn screen(&self, p: Point) -> egui::Pos2 {
        let rect = self.fixture.canvas().rect;
        rect.center() + egui::vec2(p.x as f32 - rect.width() / 2.0, p.y as f32 - rect.height() / 2.0)
    }

    /// The document point at the canvas's centre.
    fn middle(&self) -> Point {
        let rect = self.fixture.canvas().rect;
        Point::new(f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0)
    }

    /// Presses at document point `from`, moves to `to` in two steps and releases, with `mods` held.
    fn drag(&mut self, from: Point, to: Point, mods: egui::Modifiers) {
        let (a, b) = (self.screen(from), self.screen(to));
        self.fixture.fixture().hold(mods);
        self.fixture.press_at(a);
        self.fixture.move_to(a.lerp(b, 0.5));
        self.fixture.move_to(b);
        self.fixture.release_at(b);
        self.fixture.fixture().hold(egui::Modifiers::NONE);
    }

    /// The cursor with the pointer resting at document point `at` and `mods` held.
    fn cursor_at(&mut self, at: Point, mods: egui::Modifiers) -> Option<craft_behavior::Cursor> {
        let to = self.screen(at);
        self.fixture.fixture().hold(mods);
        self.fixture.move_to(to);
        self.fixture.fixture().frame(Vec::new());
        let cursor = craft_ui::runtime::cursor::current(self.fixture.fixture().ctx());
        self.fixture.fixture().hold(egui::Modifiers::NONE);
        cursor
    }

    fn count(&self, phase: &str) -> usize {
        self.log.borrow().iter().filter(|seen| **seen == phase).count()
    }
}

/// The overlay's reach in screen points (the view is at scale 1, so also in document units).
fn reach() -> f64 {
    f64::from(craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.hit.target)
}

/// A plain ruler through the canvas's middle along (0.8, 0.6), 150 units long.
fn plain(middle: Point) -> RulerLine {
    RulerLine::new(middle - kurbo::Vec2::new(60.0, 45.0), middle + kurbo::Vec2::new(60.0, 45.0))
}

/// `plain` with a protractor arm 60 units down from its start.
fn protractor(middle: Point) -> RulerLine {
    let line = plain(middle);
    line.with_protractor(line.start + kurbo::Vec2::new(0.0, 60.0))
}

fn near(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-3
}

/// From upstream PhotoCraft's analysis_ruler_move_tests (#14978): a press on a plain ruler's line moves the whole
/// ruler, the ends are hit in squares and move alone, and a press past the reach starts a new ruler.
#[trial]
fn a_drag_on_the_line_moves_the_whole_ruler_and_the_ends_are_hit_in_squares() {
    let r = reach();
    let across = kurbo::Vec2::new(-0.6, 0.8);
    let by = kurbo::Vec2::new(20.0, 30.0);

    let mut ruler = shown(None);
    let middle = ruler.middle();
    let line = plain(middle);
    *ruler.line.borrow_mut() = Some(line);
    let from = middle + across * (0.75 * r);
    ruler.drag(from, from + by, egui::Modifiers::NONE);
    let moved = (*ruler.line.borrow()).expect("a line");
    assert!(near(moved.start, line.start + by) && near(moved.end, line.end + by), "the whole ruler moved by the drag: {moved:?}");
    assert_eq!((ruler.count("begin"), ruler.count("commit")), (1, 1), "one gesture: {:?}", ruler.log.borrow());

    let mut ruler = shown(Some(line));
    let from = middle + across * (2.0 * r);
    ruler.drag(from, from + by, egui::Modifiers::NONE);
    let new = (*ruler.line.borrow()).expect("a line");
    assert!(near(new.start, from), "twice the reach off the line a press starts a new ruler: {new:?}");

    // Inside the end's square, outside the circle of the same reach.
    let mut ruler = shown(Some(line));
    let to = line.end + kurbo::Vec2::new(30.0, -10.0);
    ruler.drag(line.end + kurbo::Vec2::new(0.8 * r, 0.8 * r), to, egui::Modifiers::NONE);
    let after = (*ruler.line.borrow()).expect("a line");
    assert!(near(after.end, to) && near(after.start, line.start), "the end goes to the pointer, alone: {after:?}");
}

#[trial]
fn a_protractors_vertex_moves_the_whole_protractor_and_its_arm_end_moves_alone() {
    let by = kurbo::Vec2::new(20.0, 30.0);
    for mods in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
        let mut ruler = shown(None);
        let line = protractor(ruler.middle());
        *ruler.line.borrow_mut() = Some(line);
        ruler.drag(line.start, line.start + by, mods);
        let moved = (*ruler.line.borrow()).expect("a line");
        let arm = moved.protractor.expect("the arm stays");
        assert!(
            near(moved.start, line.start + by) && near(moved.end, line.end + by) && near(arm, line.protractor.unwrap() + by),
            "{mods:?}: all three points move: {moved:?}"
        );
    }

    let mut ruler = shown(None);
    let line = protractor(ruler.middle());
    *ruler.line.borrow_mut() = Some(line);
    let to = line.protractor.unwrap() + kurbo::Vec2::new(-10.0, 10.0);
    ruler.drag(line.protractor.unwrap(), to, egui::Modifiers::NONE);
    let after = (*ruler.line.borrow()).expect("a line");
    assert!(after.protractor.is_some_and(|arm| near(arm, to)) && near(after.start, line.start) && near(after.end, line.end), "{after:?}");

    // The arm's line takes no press: a new ruler starts there.
    let mut ruler = shown(Some(line));
    let from = line.start + kurbo::Vec2::new(0.0, 30.0);
    ruler.drag(from, from + by, egui::Modifiers::NONE);
    let new = (*ruler.line.borrow()).expect("a line");
    assert!(near(new.start, from) && new.protractor.is_none(), "a press on the arm's line starts a new ruler: {new:?}");
}

/// From upstream PhotoCraft's analysis_ruler_shift_tests (#11985): Shift snaps a ruler end to 45 degree rays and keeps
/// the shorter leg, so a snap never makes the ruler longer; the same for either end moved about the other and for a
/// protractor's arm.
#[trial]
fn shift_snaps_the_ruler_to_45_degrees_without_making_it_longer() {
    let shift = egui::Modifiers::SHIFT;
    let v = kurbo::Vec2::new;
    // (pointer offset from the press, the end's offset after the snap).
    for (to, end) in
        [(v(82.0, 100.0), v(82.0, 82.0)), (v(122.0, 120.0), v(120.0, 120.0)), (v(300.0, 30.0), v(300.0, 0.0)), (v(-82.0, -100.0), v(-82.0, -82.0))]
    {
        let mut ruler = shown(None);
        let from = ruler.middle();
        ruler.drag(from, from + to, shift);
        let line = (*ruler.line.borrow()).expect("a line");
        assert!(near(line.start, from) && near(line.end, from + end), "to {to:?}: {line:?}");
        assert!(line.start.distance(line.end) <= to.hypot() + 1e-6, "never longer than the pointer's distance: {line:?}");
    }

    // Either end moved about the other.
    let mut ruler = shown(None);
    let m = ruler.middle();
    let line = RulerLine::new(m, m + v(200.0, 150.0));
    *ruler.line.borrow_mut() = Some(line);
    ruler.drag(line.end, line.start + v(282.0, -90.0), shift);
    let after = (*ruler.line.borrow()).expect("a line");
    assert!(near(after.start, line.start) && near(after.end, line.start + v(282.0, 0.0)), "the end about the start: {after:?}");
    let mut ruler = shown(None);
    *ruler.line.borrow_mut() = Some(line);
    ruler.drag(line.start, line.end + v(-282.0, -90.0), shift);
    let after = (*ruler.line.borrow()).expect("a line");
    assert!(near(after.end, line.end) && near(after.start, line.end + v(-282.0, 0.0)), "the start about the end: {after:?}");

    // A protractor's arm end keeps the shorter leg about the vertex.
    let mut ruler = shown(None);
    let line = protractor(ruler.middle());
    *ruler.line.borrow_mut() = Some(line);
    ruler.drag(line.protractor.unwrap(), line.start + v(122.0, -170.0), shift);
    let after = (*ruler.line.borrow()).expect("a line");
    assert!(after.protractor.is_some_and(|arm| near(arm, line.start + v(122.0, -122.0))), "{after:?}");

    // Without Shift the end follows the pointer.
    let mut ruler = shown(None);
    let from = ruler.middle();
    ruler.drag(from, from + v(82.0, 100.0), egui::Modifiers::NONE);
    assert!(near((*ruler.line.borrow()).expect("a line").end, from + v(82.0, 100.0)));
}

/// From upstream PhotoCraft's ruler_tests (#13438): the Ruler's arrow everywhere, the protractor with Alt exactly where
/// an Alt press starts the protractor arm, and the crosshair while the canvas shows precise cursors.
#[trial]
fn the_ruler_shows_its_pointer_and_the_protractor_where_alt_starts_the_arm() {
    use craft_behavior::{Cursor, IconId, PaintedCursor, PointerGlyph};
    let pointer = |icon: &'static str| Some(Cursor::Painted(PaintedCursor::pointer_tool(PointerGlyph::Arrow, IconId::new(icon))));
    let alt = egui::Modifiers::ALT;

    let mut empty = shown(None);
    let middle = empty.middle();
    assert_eq!(empty.cursor_at(middle, alt), pointer("ruler"), "no ruler: no end to mark");

    let mut ruler = shown(Some(plain(middle)));
    let line = plain(middle);
    assert_eq!(ruler.cursor_at(middle + kurbo::Vec2::new(0.0, 40.0), egui::Modifiers::NONE), pointer("ruler"), "away from the ruler");
    assert_eq!(ruler.cursor_at(middle, egui::Modifiers::NONE), pointer("ruler"), "on its line");
    assert_eq!(ruler.cursor_at(line.end, egui::Modifiers::NONE), pointer("ruler"), "on an end without Alt");
    assert_eq!(ruler.cursor_at(line.end, egui::Modifiers::SHIFT), pointer("ruler"), "on an end with Shift");
    assert_eq!(ruler.cursor_at(line.end, alt), pointer("protractor"), "on the end with Alt");
    assert_eq!(ruler.cursor_at(line.start, alt), pointer("protractor"), "on the start with Alt");
    assert_eq!(ruler.cursor_at(middle, alt), pointer("ruler"), "on the line with Alt");

    let mut with_arm = shown(Some(protractor(middle)));
    let line = protractor(middle);
    assert_eq!(with_arm.cursor_at(line.start, alt), pointer("ruler"), "a protractor's vertex moves the whole ruler, Alt or not");
    assert_eq!(with_arm.cursor_at(line.end, alt), pointer("protractor"), "its far end starts a new arm with Alt");

    let ctx = ruler.fixture.fixture().ctx().clone();
    craft_ui::runtime::windows::WindowHost::of(&ctx).set_caps_lock(Some(true));
    let line = plain(middle);
    for (at, mods) in [(middle + kurbo::Vec2::new(0.0, 40.0), egui::Modifiers::NONE), (line.end, alt)] {
        assert_eq!(ruler.cursor_at(at, mods), Some(Cursor::System(craft_behavior::CursorIcon::Crosshair)), "precise: the crosshair");
    }
}

/// The ruler drawn under another tool: the line and the crosses paint, and a press on an end reaches the tool.
#[trial]
fn a_read_only_ruler_paints_and_takes_no_press() {
    let line = Rc::new(RefCell::new(None::<RulerLine>));
    let drawn = Rc::new(std::cell::Cell::new(None::<RulerLine>));
    let (tool_line, body_drawn) = (Rc::clone(&line), Rc::clone(&drawn));
    let driven = Driven { tool: RulerTool::new(), line: tool_line, log: Rc::new(RefCell::new(Vec::new())) };
    let mut fixture = ToolFixture::with_view(
        "Ruler",
        driven,
        |room| CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0),
        move |ui, body| {
            if let Some(shown) = body_drawn.get() {
                craft_draw::ui::measure::show_ruler(ui, body.rect, shown);
            }
        },
    );
    let rect = fixture.canvas().rect;
    let middle = Point::new(f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0);
    let screen = |p: Point| rect.center() + egui::vec2(p.x as f32 - rect.width() / 2.0, p.y as f32 - rect.height() / 2.0);
    let ruler = protractor(middle);
    drawn.set(Some(ruler));
    fixture.fixture().frame(Vec::new());
    let end = screen(ruler.end);
    let shapes = fixture.fixture().frame_shapes();
    let paths =
        |test: &dyn Fn(&[egui::Pos2]) -> bool| shapes.iter().filter(|shape| matches!(shape, egui::Shape::Path(path) if test(&path.points))).count();
    let close = |a: egui::Pos2, b: egui::Pos2| (a - b).length() < 0.5;
    assert!(
        paths(&|points: &[egui::Pos2]| points.iter().any(|p| close(*p, screen(ruler.start))) && points.iter().any(|p| close(*p, end))) > 0,
        "the line paints"
    );
    assert!(paths(&|points: &[egui::Pos2]| points.len() == 2 && close(points[0].lerp(points[1], 0.5), end)) >= 2, "the cross at the end paints");

    fixture.press_at(end).release_at(end);
    let started = (*line.borrow()).is_none();
    assert!(started, "a click on the drawn end reaches the tool, whose click leaves no line: {:?}", line.borrow());
    let to = end + egui::vec2(40.0, 0.0);
    fixture.press_at(end).move_to(end + egui::vec2(20.0, 0.0)).move_to(to).release_at(to);
    let new = (*line.borrow()).expect("the tool drew a line");
    assert!(new.start.distance(ruler.end) < 1e-3, "the tool's new line starts at the press on the drawn end: {new:?}");
}

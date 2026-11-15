//! `CanvasTool`, `ToolHost` and `tool_pick` (8.3): a drag is one `Begin`...`Commit`, Escape cancels it once, a press
//! on a canvas control never reaches the tool, and a group's letter picks and cycles its tools.

use craft_behavior::geometry::{SnapKind, SnapLine, SnapOptions, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CancelReason, CanvasView, Cursor, CursorIcon, GesturePhase, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{
    Autoscroll, Canvas, CanvasScrollbars, CanvasTool, ContextRequest, CropOverlay, HandTool, Momentary, NavigationOptions, Snap,
    StandInKey, ToolAction, ToolClaims, ToolContext, ToolHost, ToolInput, ToolKey, ToolPath, ToolPointer,
};
use craft_ui::contract::{Bind, Sampler as _, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::frame;
use craft_ui::style::DataColor;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, ToolFixture, pointer, trial};
use egui::{Color32, Id, Key, Modifiers, Pos2, Rect, Shape, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const DOCUMENT: Size = Size::new(1200.0, 900.0);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Call {
    Press,
    Drag,
    Release,
    Cancel(CancelReason),
}

/// A tool that claims every press it takes and records what the host called.
#[derive(Default)]
struct Recorder {
    calls: Vec<Call>,
}

impl Recorder {
    fn count(&self, call: Call) -> usize {
        self.calls.iter().filter(|seen| **seen == call).count()
    }
}

impl CanvasTool for Recorder {
    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE.deletes()
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.calls.push(Call::Press);
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.calls.push(Call::Drag);
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.calls.push(Call::Release);
    }

    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        self.calls.push(Call::Cancel(reason));
    }

    fn key(&mut self, _: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    /// The primary button, and the secondary while Alt is held, as a brush takes them.
    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && mods.alt)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

const BY: Vec2 = Vec2::new(60.0, 40.0);

#[trial]
fn a_drag_reaches_the_tool_as_one_press_drags_and_one_release() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    fixture.drag(BY);
    let tool = fixture.tool();
    assert_eq!(tool.calls.first(), Some(&Call::Press), "a drag starts with the press: {:?}", tool.calls);
    assert_eq!(tool.calls.last(), Some(&Call::Release), "a drag ends with the release: {:?}", tool.calls);
    assert_eq!(tool.count(Call::Press), 1, "one press: {:?}", tool.calls);
    assert_eq!(tool.count(Call::Release), 1, "one release: {:?}", tool.calls);
    assert!(tool.count(Call::Drag) >= 1, "the moves reach the tool: {:?}", tool.calls);
    drop(tool);
    let phases = fixture.gestures();
    assert_eq!(phases.first(), Some(&GesturePhase::Begin), "{phases:?}");
    assert_eq!(phases.last(), Some(&GesturePhase::Commit), "{phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Begin).count(), 1, "one Begin: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 1, "one Commit: {phases:?}");
}

#[trial]
fn a_press_on_a_canvas_control_never_reaches_the_tool_and_one_away_from_it_does() {
    let crop = Rc::new(RefCell::new(craft_behavior::geometry::CropBox::upright(craft_behavior::kurbo::Rect::new(20.0, 20.0, 100.0, 80.0))));
    let shown = Rc::clone(&crop);
    let mut fixture = ToolFixture::with_body("Document", Recorder::default(), move |ui, body| {
        // The overlay takes the canvas's top left only: its whole-frame and turn parts cover nothing past it, where the
        // tool owns the press.
        let room = Rect::from_min_size(body.rect.min, Vec2::new(160.0, 120.0));
        let _ = CropOverlay::new("Crop", Bind::bound(&mut *shown.borrow_mut())).show_at(ui, room);
    });
    let corner = fixture.fixture().by_label("Crop corner, top left").expect("the crop shows its corners");
    fixture.fixture().click(&corner);
    assert!(fixture.tool().calls.is_empty(), "a click on a corner is the crop's: {:?}", fixture.tool().calls);

    let away = fixture.canvas().rect.right_bottom() - Vec2::splat(20.0);
    fixture.press_at(away).release_at(away);
    assert_eq!(fixture.tool().count(Call::Press), 1, "a press away from the corners reaches the tool: {:?}", fixture.tool().calls);
}

#[trial]
fn a_gesture_cancelled_by_the_app_starts_no_new_one_while_the_button_stays_down() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    let from = fixture.canvas().center();
    fixture.press_at(from);
    let (ctx, id) = (fixture.fixture().ctx().clone(), fixture.canvas_id());
    assert!(ToolHost::cancel(&ctx, id, &mut *fixture.tool_mut(), CancelReason::Dismissed), "a gesture was running");

    fixture.move_to(from + BY).move_to(from + BY * 2.0).release_at(from + BY * 2.0);
    assert_eq!(fixture.tool().calls, [Call::Press, Call::Cancel(CancelReason::Dismissed)], "the held button is not the tool's any more");

    fixture.press_at(from).release_at(from);
    assert_eq!(fixture.tool().count(Call::Press), 2, "the next press starts a gesture again: {:?}", fixture.tool().calls);
}

/// A tool with a rubber band that follows the hovering pointer, which asks for the hover autoscroll when `scrolls`.
struct Band {
    scrolls: bool,
    hovers: usize,
}

impl CanvasTool for Band {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn hover(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.hovers += 1;
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn autoscrolls_on_hover(&self) -> bool {
        self.scrolls
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the band only follows the pointer")
    }
}

type BandCanvas = (Fixture, Rc<RefCell<Band>>, Rc<Cell<Option<CanvasView>>>);

/// A canvas inset in the room, so the window has room outside it, shown as an app runs it: the document is zoomed in
/// past the canvas, and the app moves its view by the host's `scroll`. The view starts turned by `rotation` degrees.
/// Returns the fixture and the app's view.
fn scroll_canvas<T: CanvasTool + 'static>(tool: Rc<RefCell<T>>, rotation: f64) -> (Fixture, Rc<Cell<Option<CanvasView>>>) {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let shown_view = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| {
            let mut start = CanvasView::new(room, Point::new(600.0, 450.0), 4.0);
            start.rotation = rotation;
            start
        });
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room.shrink(40.0), |_, _| {});
        let reply = ToolHost::new().run(ui, &shown.output, &mut *tool.borrow_mut());
        let mut next = reply.view.unwrap_or(current);
        if let Some(step) = reply.scroll
            && Autoscroll::scroll(&mut next, step, DOCUMENT).is_some()
        {
            ui.ctx().request_repaint();
        }
        shown_view.set(Some(next));
    });
    fixture.settle();
    (fixture, view)
}

/// The hover autoscroll rig: a `Band` tool. Returns the fixture, the tool and the app's view.
fn band_canvas(scrolls: bool) -> BandCanvas {
    let tool = Rc::new(RefCell::new(Band { scrolls, hovers: 0 }));
    let (fixture, view) = scroll_canvas(Rc::clone(&tool), 0.0);
    (fixture, tool, view)
}

#[trial]
fn a_tool_hovering_near_the_canvas_edge_scrolls_the_view_while_the_pointer_rests() {
    let (mut fixture, tool, view) = band_canvas(true);
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let centre = |view: &Rc<Cell<Option<CanvasView>>>| view.get().expect("the canvas ran").center;
    let edge = |dx: f32| Pos2::new(canvas.right() + dx, canvas.center().y);

    let before = centre(&view);
    fixture.frame(vec![pointer::moved(canvas.center())]).idle(3);
    assert_eq!(centre(&view), before, "a hover away from the edge scrolls nothing");

    fixture.frame(vec![pointer::moved(edge(-4.0))]);
    let (entered, hovers) = (centre(&view), tool.borrow().hovers);
    fixture.idle(3);
    let rested = centre(&view);
    assert!(rested.x > entered.x, "resting at the right edge scrolls the view right: {entered:?} -> {rested:?}");
    assert_eq!(rested.y, entered.y, "a scroll toward the right edge keeps the height");
    assert!(tool.borrow().hovers >= hovers + 3, "the tool follows the view as it scrolls under the resting pointer");
    let in_band = rested.x - entered.x;

    // A pointer outside the canvas still hovers with its position and scrolls at full speed.
    let (mut fixture, tool, view) = band_canvas(true);
    fixture.frame(vec![pointer::moved(edge(30.0))]);
    let (entered, hovers) = (centre(&view), tool.borrow().hovers);
    fixture.idle(3);
    let outside = centre(&view).x - entered.x;
    assert!(outside > 0.0 && outside >= in_band, "outside the canvas the view scrolls at least as fast as in the band: {outside} vs {in_band}");
    assert!(tool.borrow().hovers >= hovers + 3, "the tool hovers while the pointer rests outside the canvas");

    // A button down scrolls nothing from a hover.
    let (mut fixture, _, view) = band_canvas(true);
    fixture.frame(vec![pointer::press(edge(-4.0))]);
    let entered = centre(&view);
    fixture.idle(3);
    assert_eq!(centre(&view), entered, "a pointer with a button down is no hover");

    let (mut fixture, _, view) = band_canvas(false);
    fixture.frame(vec![pointer::moved(edge(-4.0))]);
    let entered = centre(&view);
    fixture.idle(3);
    assert_eq!(centre(&view), entered, "a tool that does not autoscroll on hover leaves the view");

    a_hover_past_the_edge_scrolls_nothing_under_a_momentary_tool_a_modal_or_another_canvas();
    an_open_polygon_keeps_its_vertices_while_the_view_scrolls_under_a_resting_pointer();
}

/// A tool that keeps the points it was clicked at, as a polygon tool keeps its vertices, and autoscrolls on hover.
#[derive(Default)]
struct Lasso {
    vertices: Vec<Pos2>,
}

impl CanvasTool for Lasso {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) {
        self.vertices.push(pointer.pos);
    }

    fn hover(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn autoscrolls_on_hover(&self) -> bool {
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the lasso only keeps its clicks")
    }
}

/// What stands between a resting pointer and the hover autoscroll.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Crowd {
    Nothing,
    MomentaryHand,
    SecondCanvas,
    Modal,
}

/// A `Band` canvas inset in the room with one `Crowd` around it. Returns the fixture, the app's view and the canvas's
/// rect.
fn crowded_canvas(crowd: Crowd) -> (Fixture, Rc<Cell<Option<CanvasView>>>, Rc<Cell<Rect>>) {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let canvas = Rc::new(Cell::new(Rect::NOTHING));
    let (shown_view, shown_canvas) = (Rc::clone(&view), Rc::clone(&canvas));
    let mut tool = Band { scrolls: true, hovers: 0 };
    let mut momentary = Momentary::new(NavigationOptions::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        let area = room.shrink(40.0);
        let (first, second) = if crowd == Crowd::SecondCanvas {
            let middle = area.center().x;
            (
                Rect::from_min_max(area.min, Pos2::new(middle - 2.0, area.max.y)),
                Some(Rect::from_min_max(Pos2::new(middle + 2.0, area.min.y), area.max)),
            )
        } else {
            (area, None)
        };
        shown_canvas.set(first);
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, first, |_, _| {});
        if let Some(second) = second {
            Canvas::new("Other").document(DOCUMENT).view(CanvasView::new(second, Point::new(600.0, 450.0), 4.0)).show_at(ui, second, |_, _| {});
        }
        if crowd == Crowd::Modal {
            let mut open = true;
            Dialog::new("Over the canvas").opened(Bind::bound(&mut open)).show(ui, |ui, _| {
                Text::new("Modal").show(ui);
            });
        }
        let mut host = ToolHost::new();
        if crowd == Crowd::MomentaryHand {
            host = host.momentary(&mut momentary);
        }
        let reply = host.run(ui, &shown.output, &mut tool);
        let mut next = reply.view.unwrap_or(current);
        if let Some(step) = reply.scroll
            && Autoscroll::scroll(&mut next, step, DOCUMENT).is_some()
        {
            ui.ctx().request_repaint();
        }
        shown_view.set(Some(next));
    });
    fixture.settle();
    (fixture, view, canvas)
}

fn a_hover_past_the_edge_scrolls_nothing_under_a_momentary_tool_a_modal_or_another_canvas() {
    for crowd in [Crowd::Nothing, Crowd::MomentaryHand, Crowd::SecondCanvas, Crowd::Modal] {
        let (mut fixture, view, canvas) = crowded_canvas(crowd);
        let canvas = canvas.get();
        // Over the second canvas the pointer is past the first's edge and inside the other's background.
        let at = Pos2::new(canvas.right() + if crowd == Crowd::SecondCanvas { 10.0 } else { 30.0 }, canvas.center().y);
        let mut events = vec![pointer::moved(at)];
        if crowd == Crowd::MomentaryHand {
            events.push(key_press(Key::Space, Modifiers::NONE));
        }
        fixture.frame(events);
        let entered = view.get().expect("the canvas ran").center;
        fixture.idle(3);
        let rested = view.get().expect("the canvas ran").center;
        if crowd == Crowd::Nothing {
            assert!(rested.x > entered.x, "the rig scrolls when nothing is in the way: {entered:?} -> {rested:?}");
        } else {
            assert_eq!(rested, entered, "{crowd:?}: a pointer resting past the edge scrolls nothing");
        }
    }
}

fn an_open_polygon_keeps_its_vertices_while_the_view_scrolls_under_a_resting_pointer() {
    let tool = Rc::new(RefCell::new(Lasso::default()));
    let (mut fixture, view) = scroll_canvas(Rc::clone(&tool), 0.0);
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let inside = [canvas.center(), canvas.center() + Vec2::new(30.0, 10.0), canvas.center() + Vec2::new(10.0, 40.0)];
    for at in inside {
        fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    }
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::moved(Pos2::new(canvas.right() + 30.0, canvas.center().y))]).idle(4);
    let after = view.get().expect("the canvas ran").center;
    assert!(after.x > before.x, "the view scrolls while the polygon is open: {before:?} -> {after:?}");
    assert_eq!(tool.borrow().vertices, inside, "the vertices stay where they were clicked");
}

type Log = Rc<RefCell<Vec<String>>>;

/// A tool that logs its calls under its name. It refuses a press while its `refuses` key is held, answers it `Ignored`
/// while its `ignores` key is held, and takes the secondary button when `secondary`.
struct Probe {
    name: &'static str,
    refuses: Option<StandInKey>,
    ignores: Option<StandInKey>,
    secondary: bool,
    every_move: bool,
    snaps: bool,
    drags: Vec<(Pos2, f64)>,
    pass_time: f64,
    log: Log,
}

impl Probe {
    fn new(name: &'static str, log: &Log) -> Self {
        Probe {
            name,
            refuses: None,
            ignores: None,
            secondary: false,
            every_move: false,
            snaps: false,
            drags: Vec::new(),
            pass_time: 0.0,
            log: Rc::clone(log),
        }
    }

    fn note(&self, call: &str) {
        self.log.borrow_mut().push(format!("{} {call}", self.name));
    }
}

fn holds(key: Option<StandInKey>, mods: Mods) -> bool {
    match key {
        Some(StandInKey::Command) => mods.cmd,
        Some(StandInKey::Alt) => mods.alt,
        _ => false,
    }
}

impl CanvasTool for Probe {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.pass_time = cx.time;
        if self.snaps {
            let _ = cx.snap_rect(craft_behavior::kurbo::Rect::new(98.0, 40.0, 198.0, 90.0));
        }
    }

    fn press(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.note("press");
        if holds(self.ignores, pointer.mods) { ToolAction::Ignored } else { ToolAction::Claimed }
    }

    fn drag(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) {
        self.note("drag");
        self.drags.push((pointer.pos, pointer.time));
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.note("release");
    }

    fn hover(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.note("hover");
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.note("cancel");
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        (button == PointerButton::Primary || (self.secondary && button == PointerButton::Secondary)) && !holds(self.refuses, mods)
    }

    fn takes_every_move(&self) -> bool {
        self.every_move
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        Cursor::System(match self.name {
            "command" => CursorIcon::Crosshair,
            "alt" => CursorIcon::Wait,
            _ => CursorIcon::Default,
        })
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the probe only logs")
    }
}

/// What a `Rig` adds to the host: the momentary tools, a colour for smart guides.
#[derive(Default)]
struct Setup {
    momentary: Option<Momentary>,
    smart_color: Option<DataColor>,
    /// Set by a trial: the next pass is discarded after the host ran, so egui runs it again in the same frame.
    rerun: Rc<Cell<bool>>,
}

/// A canvas at 100 % with a `Probe` as the selected tool and two stand-in probes, `command` and `alt`.
struct Rig {
    fixture: Fixture,
    main: Rc<RefCell<Probe>>,
    canvas: Rc<Cell<Id>>,
    log: Log,
    contexts: Rc<RefCell<Vec<ContextRequest>>>,
    snapped: Rc<RefCell<Vec<SnapLine>>>,
    cursor: Rc<RefCell<Option<Cursor>>>,
}

impl Rig {
    fn new(main: Probe, log: &Log, snap: Option<Snap>) -> Self {
        Self::with(main, log, snap, Setup::default())
    }

    fn with(main: Probe, log: &Log, snap: Option<Snap>, mut setup: Setup) -> Self {
        let main = Rc::new(RefCell::new(main));
        let (canvas, contexts, snapped) = (Rc::new(Cell::new(Id::NULL)), Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(Vec::new())));
        let cursor = Rc::new(RefCell::new(None));
        let (shown_main, shown_canvas, shown_contexts, shown_snapped, shown_cursor) =
            (Rc::clone(&main), Rc::clone(&canvas), Rc::clone(&contexts), Rc::clone(&snapped), Rc::clone(&cursor));
        let (mut command, mut alt) = (Probe::new("command", log), Probe::new("alt", log));
        let mut fixture = Fixture::new(move |ui| {
            let room = ui.available_rect_before_wrap();
            let mut shown = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 1.0));
            if let Some(snap) = &snap {
                shown = shown.snap(snap.clone());
            }
            let shown = shown.show(ui, |_, _| {});
            shown_canvas.set(shown.output.response.id);
            let mut host = ToolHost::new().stand_in(StandInKey::Command, &mut command).stand_in(StandInKey::Alt, &mut alt).smart_guide_color(setup.smart_color);
            if let Some(momentary) = setup.momentary.as_mut() {
                host = host.momentary(momentary);
            }
            let reply = host.run(ui, &shown.output, &mut *shown_main.borrow_mut());
            shown_contexts.borrow_mut().extend(reply.context);
            shown_snapped.borrow_mut().extend(reply.snapped.iter().copied());
            if setup.rerun.replace(false) && frame::discardable(ui.ctx()) {
                frame::discard_pass(ui.ctx(), "a trial reruns the pass");
            }
            if reply.cursor.is_some() {
                *shown_cursor.borrow_mut() = reply.cursor.clone();
            }
        });
        fixture.settle();
        Rig { fixture, main, canvas, log: Rc::clone(log), contexts, snapped, cursor }
    }

    fn centre(&mut self) -> Pos2 {
        self.fixture.by_role(Role::CANVAS)[0].rect.center()
    }

    fn calls(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

#[trial]
fn a_held_command_or_alt_runs_its_stand_in_for_one_gesture() {
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();

    // Command at the press, up mid-drag: the stand-in keeps the gesture to the release.
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]);
    rig.fixture.hold(Modifiers::NONE).frame(vec![pointer::moved(at + BY * 2.0)]).frame(vec![pointer::release(at + BY * 2.0)]);
    let calls = rig.calls();
    assert_eq!(calls.first().map(String::as_str), Some("command press"), "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("command release"), "{calls:?}");
    assert!(calls.iter().all(|call| call.starts_with("command")), "the selected tool saw none of it: {calls:?}");

    // The next plain press is the selected tool's again.
    rig.log.borrow_mut().clear();
    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    assert_eq!(rig.calls().first().map(String::as_str), Some("main press"), "{:?}", rig.calls());

    // With the key held and no gesture, hover is the stand-in's.
    rig.log.borrow_mut().clear();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at + BY)]);
    assert!(rig.calls().iter().any(|call| call == "command hover"), "{:?}", rig.calls());
    rig.fixture.hold(Modifiers::NONE);

    // A press injected through the host is substituted from its own modifiers.
    rig.log.borrow_mut().clear();
    let (ctx, canvas) = (rig.fixture.ctx().clone(), rig.canvas.get());
    ToolHost::inject(&ctx, canvas, ToolInput::down(PointerButton::Primary, at).mods(Mods::CMD));
    rig.fixture.idle(1);
    ToolHost::inject(&ctx, canvas, ToolInput::up(at));
    rig.fixture.idle(1);
    let calls = rig.calls();
    assert_eq!(calls.first().map(String::as_str), Some("command press"), "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("command release"), "{calls:?}");

    // A selection tool's Ignored Command press reaches the stand-in.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.ignores = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::release(at + BY)]);
    let calls = rig.calls();
    assert_eq!(&calls[..2], ["main press", "command press"], "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("command release"), "{calls:?}");

    // Alt runs the Alt stand-in.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Alt);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::ALT).frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    assert_eq!(rig.calls().first().map(String::as_str), Some("alt press"), "{:?}", rig.calls());

    // The hover cursor is the stand-in's while its key is held, no gesture runs and the selected tool does not take
    // the press.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at + BY)]);
    assert_eq!(*rig.cursor.borrow(), Some(Cursor::System(CursorIcon::Crosshair)), "the Command stand-in's cursor");
    rig.fixture.hold(Modifiers::NONE).frame(vec![pointer::moved(at + BY * 2.0)]);
    assert_eq!(*rig.cursor.borrow(), Some(Cursor::System(CursorIcon::Default)), "the selected tool's cursor again");

    // Space's momentary Hand wins over both stand-ins: Command held with Space is the momentary tool's press.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::with(main, &log, None, Setup { momentary: Some(Momentary::new(NavigationOptions::default())), ..Setup::default() });
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::COMMAND)]);
    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::release(at + BY)]);
    assert!(rig.calls().is_empty(), "neither the selected tool nor a stand-in sees a press under a momentary tool: {:?}", rig.calls());

    // With the Hand selected, Cmd at a press is Zoom In, not the Command stand-in.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut momentary = Momentary::new(NavigationOptions::default());
    momentary.hand_selected(true);
    let mut rig = Rig::with(main, &log, None, Setup { momentary: Some(momentary), ..Setup::default() });
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    assert!(!rig.calls().iter().any(|call| call == "command press"), "Cmd over the selected Hand is Zoom In: {:?}", rig.calls());
}

/// A canvas under a dialog that keeps the canvas: returns the fixture, the app's view and the canvas's rect.
fn kept_canvas<T: CanvasTool + 'static>(tool: Rc<RefCell<T>>) -> (Fixture, Rc<Cell<Option<CanvasView>>>, Rc<Cell<Rect>>) {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let (shown_view, shown_rect) = (Rc::clone(&view), Rc::clone(&rect));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 1.0));
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room, |_, _| {});
        shown_rect.set(room);
        let id = shown.output.response.id;
        let mut open = true;
        Dialog::new("Keeps the canvas").opened(Bind::bound(&mut open)).show(ui, |ui, cx| {
            cx.keep_view_open(id);
            Text::new("Pick").show(ui);
        });
        let reply = ToolHost::new().run(ui, &shown.output, &mut *tool.borrow_mut());
        shown_view.set(Some(reply.view.unwrap_or(current)));
    });
    fixture.settle();
    (fixture, view, rect)
}

#[trial]
fn a_view_only_tool_drags_under_a_modal_that_keeps_the_canvas() {
    let hand = Rc::new(RefCell::new(HandTool::new(NavigationOptions::default())));
    let (mut fixture, view, rect) = kept_canvas(hand);
    let at = rect.get().center();
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::moved(at + BY * 2.0)]);
    fixture.frame(vec![pointer::release(at + BY * 2.0)]);
    assert_ne!(view.get().expect("the canvas ran").center, before, "a Hand drag pans under the modal");

    let tool = Rc::new(RefCell::new(Recorder::default()));
    let (mut fixture, _, rect) = kept_canvas(Rc::clone(&tool));
    let at = rect.get().center();
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::release(at + BY)]);
    assert_eq!(tool.borrow().count(Call::Press), 0, "a drawing tool's press stays blocked: {:?}", tool.borrow().calls);
}

#[trial]
fn matched_smart_lines_are_drawn_and_reported() {
    let shapes = |smart: bool, color: Option<DataColor>| {
        let log = Log::default();
        let mut targets = SnapTargets::new();
        targets.add(SnapTarget::new(Axis::Y, 100.0, SnapKind::LayerEdge));
        let mut options = SnapOptions::default();
        options.smart_guides = smart;
        let mut main = Probe::new("main", &log);
        main.snaps = true;
        let mut rig = Rig::with(main, &log, Some(Snap::new(Arc::new(targets), options)), Setup { smart_color: color, ..Setup::default() });
        rig.fixture.idle(2);
        let reported = rig.snapped.borrow().len();
        (rig.fixture.frame_shapes(), reported)
    };
    let (on, reported_on) = shapes(true, None);
    let (off, reported_off) = shapes(false, None);
    let (on, off) = (on.len(), off.len());
    assert!(reported_on > 0 && reported_off > 0, "the lines reach the response whatever the option: {reported_on}, {reported_off}");
    assert!(on > off, "smart kinds are painted only with Smart Guides on: {on} shapes vs {off}");

    let color = Color32::from_rgb(12, 200, 34);
    let (coloured, _) = shapes(true, Some(DataColor(color)));
    assert!(coloured.iter().any(|shape| strokes_in(shape, color)), "a smart line is drawn in `smart_guide_color` when it is set");
    let (plain, _) = shapes(true, None);
    assert!(!plain.iter().any(|shape| strokes_in(shape, color)), "and not otherwise");
}

/// Whether `shape` or a shape inside it is a line or path stroked in `color`.
fn strokes_in(shape: &Shape, color: Color32) -> bool {
    match shape {
        Shape::Vec(shapes) => shapes.iter().any(|shape| strokes_in(shape, color)),
        Shape::LineSegment { stroke, .. } => stroke.color == color,
        Shape::Path(path) => path.stroke.color == egui::epaint::ColorMode::Solid(color),
        _ => false,
    }
}

#[trial]
fn a_tool_that_takes_every_move_gets_each_move_of_a_pass_in_order() {
    for every in [true, false] {
        let log = Log::default();
        let mut main = Probe::new("main", &log);
        main.every_move = every;
        let rerun = Rc::new(Cell::new(false));
        let mut rig = Rig::with(main, &log, None, Setup { rerun: Rc::clone(&rerun), ..Setup::default() });
        let at = rig.centre();
        let path = [at + Vec2::new(20.0, 0.0), at + Vec2::new(40.0, 5.0), at + Vec2::new(60.0, 10.0)];
        rig.fixture.frame(vec![pointer::press(at)]);
        // The frame's pass runs twice: the moves reach the tool once.
        rerun.set(true);
        rig.fixture.frame(path.iter().map(|to| pointer::moved(*to)).collect());
        let main = rig.main.borrow();
        if every {
            let seen: Vec<Pos2> = main.drags.iter().map(|(pos, _)| *pos).collect();
            assert_eq!(seen, path, "each move of the pass reaches the tool, in order");
            assert!(main.drags.windows(2).all(|pair| pair[0].1 <= pair[1].1), "times rise: {:?}", main.drags);
            assert_eq!(main.drags.last().map(|(_, time)| *time), Some(main.pass_time), "the last move is at the pass time");
        } else {
            assert_eq!(main.drags.len(), 1, "a tool that does not ask gets one call: {:?}", main.drags);
            assert_eq!(main.drags[0].0, path[2], "at the last position");
        }
    }
}

#[trial]
fn a_secondary_click_the_tool_did_not_take_asks_for_the_menu() {
    let log = Log::default();
    let mut rig = Rig::new(Probe::new("main", &log), &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::SHIFT).frame(vec![pointer::secondary(at, true)]);
    rig.fixture.hold(Modifiers::NONE).frame(vec![pointer::secondary(at, false)]);
    let asked = rig.contexts.borrow().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].at, at);
    assert!(asked[0].mods.shift, "the modifiers are those held at the press: {:?}", asked[0]);
    assert_eq!(asked[0].momentary, None);

    // The Menu key asks at the view's centre.
    rig.contexts.borrow_mut().clear();
    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    rig.fixture.key(Key::F10, Modifiers::SHIFT).settle();
    let asked = rig.contexts.borrow().clone();
    assert_eq!(asked.first().map(|request| request.at), Some(at), "{asked:?}");

    // A press the tool took asks for nothing.
    let log = Log::default();
    let mut taker = Probe::new("main", &log);
    taker.secondary = true;
    let mut rig = Rig::new(taker, &log, None);
    let at = rig.centre();
    rig.fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::secondary(at, false)]);
    assert!(rig.contexts.borrow().is_empty(), "{:?}", rig.contexts.borrow());

    // A press on the scroll bars reports none.
    let (mut fixture, contexts, _, rect) = menu_canvas(false, true);
    let on_bar = Pos2::new(rect.get().center().x, rect.get().bottom() - 3.0);
    fixture.frame(vec![pointer::secondary(on_bar, true)]).frame(vec![pointer::secondary(on_bar, false)]);
    assert!(contexts.borrow().is_empty(), "{:?}", contexts.borrow());

    // With Space held a secondary drag pans with the momentary Hand and reports none.
    let (mut fixture, contexts, view, rect) = menu_canvas(true, false);
    let at = rect.get().center();
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::NONE)]);
    fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::moved(at + BY * 2.0)]);
    fixture.frame(vec![pointer::secondary(at + BY * 2.0, false)]);
    assert_ne!(view.get().expect("the canvas ran").center, before, "the secondary drag pans");
    assert!(contexts.borrow().is_empty(), "{:?}", contexts.borrow());

    // The selected Hand does not take the secondary button: its click asks for the menu.
    let (mut fixture, contexts, _, rect) = menu_canvas(false, false);
    let at = rect.get().center();
    fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::secondary(at, false)]);
    assert_eq!(contexts.borrow().len(), 1, "{:?}", contexts.borrow());
}

/// A canvas with the Hand selected, with the momentary set when `momentary` and scroll bars in its body when `bars`.
/// Returns the fixture, the menu requests, the app's view and the canvas's rect.
fn menu_canvas(
    momentary: bool,
    bars: bool,
) -> (Fixture, Rc<RefCell<Vec<ContextRequest>>>, Rc<Cell<Option<CanvasView>>>, Rc<Cell<Rect>>) {
    let (contexts, view, rect) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(None::<CanvasView>)), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown_contexts, shown_view, shown_rect) = (Rc::clone(&contexts), Rc::clone(&view), Rc::clone(&rect));
    let mut hand = HandTool::new(NavigationOptions::default());
    let mut set = Momentary::new(NavigationOptions::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        shown_rect.set(room);
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room, |ui, body| {
            if bars {
                let mut bar_view = current;
                CanvasScrollbars::new("Scroll bars", Bind::bound(&mut bar_view), craft_behavior::kurbo::Rect::from_origin_size(Point::ZERO, DOCUMENT)).show_at(ui, body.rect);
            }
        });
        let mut host = ToolHost::new();
        if momentary {
            host = host.momentary(&mut set);
        }
        let reply = host.run(ui, &shown.output, &mut hand);
        shown_contexts.borrow_mut().extend(reply.context);
        shown_view.set(Some(reply.view.unwrap_or(current)));
    });
    fixture.settle();
    (fixture, contexts, view, rect)
}

#[trial]
fn a_drag_past_the_canvas_edge_scrolls_the_view_and_a_turned_view_scrolls_on_the_clipped_axis() {
    for (rotation, upright) in [(0.0, true), (90.0, false)] {
        let tool = Rc::new(RefCell::new(Recorder::default()));
        let (mut fixture, view) = scroll_canvas(Rc::clone(&tool), rotation);
        let canvas = fixture.by_role(Role::CANVAS)[0].rect;
        let start = canvas.center();
        let before = view.get().expect("the canvas ran").center;
        fixture.frame(vec![pointer::press(start)]);
        fixture.frame(vec![pointer::moved(Pos2::new(canvas.right() + 25.0, start.y))]).idle(3);
        let after = view.get().expect("the canvas ran").center;
        if upright {
            assert!(after.x > before.x && (after.y - before.y).abs() < 1e-6, "scrolls along x: {before:?} -> {after:?}");
        } else {
            assert!(after.y != before.y && (after.x - before.x).abs() < 1e-6, "a view turned 90 degrees scrolls along y: {before:?} -> {after:?}");
        }
        fixture.frame(vec![pointer::release(Pos2::new(canvas.right() + 25.0, start.y))]);
        assert_eq!(tool.borrow().count(Call::Press), 1, "{:?}", tool.borrow().calls);
    }
}

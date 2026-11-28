//! `Guides` on a `Canvas`: which presses take a guide, and where a dragged guide lands or goes.

use craft_behavior::geometry::{SnapKind, SnapOptions, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{Axis, CancelReason, CanvasView};
use craft_ui::canvas::{CanvasTool, Guide, GuideKey, Snap, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::canvas_controls::Guides;
use craft_ui::canvas_controls::reading::{GuideReading, running};
use craft_ui::contract::{Bind, BodyCx, Props as _, Show as _, ShowContainer as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, pointer, tool_fixture, trial};
use egui::{Id, Key, Modifiers, Pos2, Rect, Vec2, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// A tool that counts the presses that reach it.
#[derive(Default)]
struct Recorder {
    presses: usize,
}

impl CanvasTool for Recorder {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.presses += 1;
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

type Shared<T> = Rc<RefCell<T>>;

/// The view at `zoom` that shows the room's centre at the same document point, so at zoom 1 document and screen
/// points coincide.
fn view_at(zoom: f64) -> impl Fn(Rect) -> CanvasView + 'static {
    move |room: Rect| {
        let centre = room.center();
        CanvasView::new(room, Point::new(f64::from(centre.x), f64::from(centre.y)), zoom)
    }
}

fn guide(key: u64, along: Axis, position: f64) -> Guide {
    Guide::new(GuideKey::new(key), along, position)
}

/// The id the fixture names its "Document" canvas with, which the guides are keyed by.
fn canvas_of(_: &BodyCx<'_>) -> Id {
    tool_fixture::canvas_id("Document")
}

/// The guide at `position` running `along`, if there is one.
fn at(guides: &Shared<Vec<Guide>>, along: Axis, position: f64) -> Option<Guide> {
    guides.borrow().iter().find(|guide| guide.along == along && (guide.position - position).abs() < 1e-9).cloned()
}

// With `interactive` off and `moves_with_command` on, a Cmd drag on an unlocked guide moves it and the tool gets
// nothing; a plain press on it, and a Cmd press on a locked guide, reach the tool.
#[trial]
fn a_command_press_takes_a_guide_when_the_guides_are_not_interactive() {
    let mut locked = guide(2, Axis::Y, 400.0);
    locked.locked = true;
    let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(vec![guide(1, Axis::Y, 200.0), locked]));
    let shown = Rc::clone(&guides);
    let mut fixture = ToolFixture::with_view("Document", Recorder::default(), view_at(1.0), move |ui, cx| {
        let _ = Guides::new("Guides", canvas_of(cx), Bind::bound(&mut *shown.borrow_mut())).moves_with_command(true).show_at(ui, cx.rect);
    });

    fixture.fixture().hold(Modifiers::COMMAND);
    fixture.drag_from(pos2(200.0, 300.0), vec2(40.0, 0.0));
    assert!(at(&guides, Axis::Y, 240.0).is_some(), "a Cmd drag moves the guide: {:?}", guides.borrow());
    assert_eq!(fixture.tool().presses, 0, "the tool gets nothing");

    fixture.fixture().hold(Modifiers::NONE);
    fixture.drag_from(pos2(240.0, 300.0), vec2(40.0, 0.0));
    assert!(at(&guides, Axis::Y, 240.0).is_some(), "a plain drag leaves the guide: {:?}", guides.borrow());
    assert_eq!(fixture.tool().presses, 1, "a plain press reaches the tool");

    fixture.fixture().hold(Modifiers::COMMAND);
    fixture.drag_from(pos2(400.0, 300.0), vec2(40.0, 0.0));
    fixture.fixture().hold(Modifiers::NONE);
    assert!(at(&guides, Axis::Y, 400.0).is_some_and(|guide| guide.locked), "the locked guide stays: {:?}", guides.borrow());
    assert_eq!(fixture.tool().presses, 2, "a Cmd press on a locked guide reaches the tool");

    // A tool drag begun off every guide with no key crosses the guide at 240: the guide declares no part, so the tool
    // keeps the drag and the guide stays.
    fixture.drag_from(pos2(100.0, 300.0), vec2(200.0, 0.0));
    assert!(at(&guides, Axis::Y, 240.0).is_some(), "a tool drag across a guide leaves it: {:?}", guides.borrow());
    assert_eq!(fixture.tool().presses, 3, "the press reaches the tool");
}

// A guide drag's reading comes from the note on the guides' running gesture, read by the canvas id outside the pass:
// the dragged guide's axis and position while the drag runs, nothing once the release commits it or Escape cancels it.
#[trial]
fn the_running_guide_reading_follows_the_drag_and_goes_when_it_ends() {
    let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(vec![guide(1, Axis::Y, 200.0)]));
    let shown = Rc::clone(&guides);
    let mut fixture = ToolFixture::with_view("Document", Recorder::default(), view_at(1.0), move |ui, cx| {
        let _ = Guides::new("Guides", canvas_of(cx), Bind::bound(&mut *shown.borrow_mut())).interactive(true).show_at(ui, cx.rect);
    });
    let canvas = fixture.canvas_id();
    let reading = |fixture: &mut ToolFixture<Recorder>| running(fixture.fixture().ctx(), canvas);
    let vertical = |position: f64| Some(GuideReading::new(Axis::Y, position));
    assert_eq!(reading(&mut fixture), None, "no drag, no reading");

    fixture.fixture().frame(vec![pointer::moved(pos2(200.0, 300.0))]);
    fixture.fixture().frame(vec![pointer::press(pos2(200.0, 300.0))]);
    fixture.fixture().frame(vec![pointer::moved(pos2(220.0, 300.0))]);
    fixture.fixture().frame(vec![pointer::moved(pos2(240.0, 300.0))]);
    assert_eq!(reading(&mut fixture), vertical(240.0), "the dragged guide's axis and position");
    fixture.fixture().frame(vec![pointer::release(pos2(240.0, 300.0))]);
    fixture.fixture().frame(Vec::new());
    assert!(at(&guides, Axis::Y, 240.0).is_some(), "the release commits the drag: {:?}", guides.borrow());
    assert_eq!(reading(&mut fixture), None, "a committed drag leaves no reading");

    fixture.fixture().frame(vec![pointer::press(pos2(240.0, 300.0))]);
    fixture.fixture().frame(vec![pointer::moved(pos2(260.0, 300.0))]);
    fixture.fixture().frame(vec![pointer::moved(pos2(280.0, 300.0))]);
    assert_eq!(reading(&mut fixture), vertical(280.0));
    fixture.fixture().key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(reading(&mut fixture), None, "a cancelled drag leaves no reading");
    assert!(at(&guides, Axis::Y, 240.0).is_some(), "Escape puts the guide back: {:?}", guides.borrow());
    fixture.fixture().frame(vec![pointer::release(pos2(280.0, 300.0))]);
}

/// Where guides may live, in document units.
const BOUNDS: DocRect = DocRect::new(100.0, 100.0, 600.0, 400.0);

/// A canvas at `zoom` that snaps to horizontal guide targets at y = 224.3 and y = 250.3, its rulers shown by the canvas and its body
/// showing interactive guides kept inside [`BOUNDS`], in a room that holds the bounds with a margin, so a release outside them
/// still lands on the canvas.
fn ruled(zoom: f64, guides: &Shared<Vec<Guide>>) -> ToolFixture<Recorder> {
    let mut targets = SnapTargets::new();
    targets.add(SnapTarget::new(Axis::X, 224.3, SnapKind::Guide));
    targets.add(SnapTarget::new(Axis::X, 250.3, SnapKind::Guide));
    let snap = Snap::new(Arc::new(targets), SnapOptions::default());
    let (shown, zero) = (Rc::clone(guides), Rc::new(RefCell::new(Point::ZERO)));
    // The view lies inside the strips: it keeps the document point under the screen point it was given, so at zoom 1
    // document and screen points coincide, from the second pass on (the strips' inset is the first pass's body).
    let inset = Rc::new(Cell::new(Vec2::ZERO));
    let moved = Rc::clone(&inset);
    let view = move |room: Rect| {
        let centre = room.center() + inset.get() / 2.0;
        CanvasView::new(room, Point::new(f64::from(centre.x), f64::from(centre.y)), zoom)
    };
    let mut fixture = ToolFixture::with_rulers("Document", Recorder::default(), view, snap, zero, move |ui, cx| {
        moved.set(cx.rect.min.to_vec2());
        let mut guides = shown.borrow_mut();
        let _ = Guides::new("Guides", canvas_of(cx), Bind::bound(&mut *guides)).interactive(true).bounds(BOUNDS).show_at(ui, cx.rect);
    });
    fixture.fixture().room(Vec2::new(700.0, 500.0)).settle();
    fixture
}

/// A drag with `mods` held from `from` to `to`.
fn drag_with(fixture: &mut ToolFixture<Recorder>, mods: Modifiers, from: Pos2, to: Pos2) {
    fixture.fixture().hold(mods);
    fixture.drag_from(from, to - from);
    fixture.fixture().hold(Modifiers::NONE);
}

// At zoom 1 (document and screen points coincide): a guide dragged out of the top ruler is horizontal and snaps to the
// target; with Alt it is vertical; with Shift it lands on a ruler tick, a whole pixel, and not on the target; with
// Shift just inside the document's edge it lands on the edge and stays. A drag out released back on a ruler or
// outside the bounds adds nothing, and a guide dragged off the bounds is removed. At zoom 8 a Shift drag still lands
// on a whole pixel.
#[trial]
fn a_dragged_guide_is_removed_off_the_bounds_or_on_a_ruler_and_alt_and_shift_change_where_it_lands() {
    let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(Vec::new()));
    let mut fixture = ruled(1.0, &guides);
    let rect = fixture.canvas().rect;
    // Inside the top strip, clear of the corner, and inside the left strip.
    let top = |x: f32| pos2(x, rect.min.y + 4.0);
    let left = |y: f32| pos2(rect.min.x + 4.0, y);

    drag_with(&mut fixture, Modifiers::NONE, top(300.0), pos2(300.0, 247.6));
    assert!(at(&guides, Axis::X, 250.3).is_some(), "out of the top ruler: horizontal, snapped: {:?}", guides.borrow());

    // 222.6 is 1.7 from the target at 224.3; the ruler's minor ticks at zoom 1 are 10 pixels apart.
    drag_with(&mut fixture, Modifiers::SHIFT, top(300.0), pos2(300.0, 222.6));
    let ticked: Vec<f64> =
        guides.borrow().iter().filter(|guide| guide.along == Axis::X && guide.position != 250.3).map(|guide| guide.position).collect();
    assert!(
        matches!(ticked.as_slice(), [tick] if tick.fract() == 0.0 && (tick - 222.6).abs() <= 10.0),
        "Shift lands on a tick, not on the snap target: {:?}",
        guides.borrow()
    );

    drag_with(&mut fixture, Modifiers::ALT, top(350.0), pos2(350.0, 300.0));
    assert!(at(&guides, Axis::Y, 350.0).is_some(), "Alt turns a guide out of the top ruler vertical: {:?}", guides.borrow());

    drag_with(&mut fixture, Modifiers::SHIFT, top(300.0), pos2(300.0, 399.4));
    assert!(at(&guides, Axis::X, 400.0).is_some(), "Shift inside the edge lands on it and stays: {:?}", guides.borrow());

    let count = guides.borrow().len();
    let from = left(300.0);
    fixture.fixture().frame(vec![pointer::moved(from)]);
    fixture.fixture().frame(vec![pointer::press(from)]);
    fixture.fixture().frame(vec![pointer::moved(pos2(380.0, 300.0))]);
    fixture.fixture().frame(vec![pointer::moved(from + Vec2::new(0.0, 10.0))]);
    fixture.fixture().frame(vec![pointer::release(from + Vec2::new(0.0, 10.0))]);
    fixture.fixture().frame(Vec::new());
    assert_eq!(guides.borrow().len(), count, "a guide dropped back on its ruler is not added: {:?}", guides.borrow());

    drag_with(&mut fixture, Modifiers::NONE, top(300.0), pos2(300.0, 480.0));
    assert_eq!(guides.borrow().len(), count, "a guide dropped outside the bounds is not added: {:?}", guides.borrow());

    drag_with(&mut fixture, Modifiers::NONE, pos2(450.0, 250.3), pos2(450.0, 450.3));
    assert!(at(&guides, Axis::X, 250.3).is_none(), "a guide dragged off the bounds is removed: {:?}", guides.borrow());
    assert_eq!(guides.borrow().len(), count - 1);

    let deep: Shared<Vec<Guide>> = Rc::new(RefCell::new(Vec::new()));
    let mut fixture = ruled(8.0, &deep);
    let rect = fixture.canvas().rect;
    let centre = rect.center();
    drag_with(&mut fixture, Modifiers::SHIFT, pos2(centre.x, rect.min.y + 4.0), centre + Vec2::new(0.0, 13.0));
    let landed: Vec<f64> = deep.borrow().iter().map(|guide| guide.position).collect();
    assert!(matches!(landed.as_slice(), [tick] if tick.fract() == 0.0), "Shift deep in lands on a whole pixel: {landed:?}");
}

// A double press on a strip places a guide at once, at the pointer, with the drag-out's Alt and Shift rules. The
// pointer lies on the strip, so the guide lands only where the bounds take it: Alt turns it to the strip's across
// axis, which runs inside them. A press and drag out of a strip still adds exactly one guide.
#[trial]
fn a_double_press_on_a_ruler_places_a_guide_at_once() {
    let placed = |mods: Modifiers, at: &dyn Fn(Rect) -> Pos2| -> Vec<Guide> {
        let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(Vec::new()));
        let mut fixture = ruled(1.0, &guides);
        let rect = fixture.canvas().rect;
        fixture.fixture().hold(mods);
        fixture.double_press_at(at(rect));
        fixture.fixture().hold(Modifiers::NONE);
        guides.borrow().clone()
    };
    let top = |x: f32| move |rect: Rect| pos2(x, rect.min.y + 4.0);
    let left = |y: f32| move |rect: Rect| pos2(rect.min.x + 4.0, y);

    let turned = placed(Modifiers::ALT, &top(300.4));
    assert!(
        matches!(turned.as_slice(), [one] if one.along == Axis::Y && (one.position - 300.4).abs() < 1e-3),
        "Alt on the top ruler places a vertical guide at the pointer: {turned:?}"
    );
    let ticked = placed(Modifiers::ALT | Modifiers::SHIFT, &top(300.4));
    assert!(matches!(ticked.as_slice(), [one] if one.position.fract() == 0.0), "Shift lands on a whole pixel: {ticked:?}");
    let side = placed(Modifiers::ALT, &left(300.4));
    assert!(matches!(side.as_slice(), [one] if one.along == Axis::X), "Alt on the left ruler places a horizontal guide: {side:?}");
    assert!(placed(Modifiers::ALT, &top(650.0)).is_empty(), "a double press outside the bounds adds none");

    let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(Vec::new()));
    let mut fixture = ruled(1.0, &guides);
    let rect = fixture.canvas().rect;
    drag_with(&mut fixture, Modifiers::NONE, pos2(300.0, rect.min.y + 4.0), pos2(300.0, 247.6));
    assert_eq!(guides.borrow().len(), 1, "a press and drag out of a strip adds exactly one guide: {:?}", guides.borrow());
}

// Delete on a focused guide removes it, and the focus the removal hands to the next guide selects nothing: no guide is
// selected after a delete, as in Photoshop.
#[trial]
fn a_deleted_guide_hands_focus_on_without_selecting_the_next_guide() {
    let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(vec![guide(1, Axis::Y, 200.0), guide(2, Axis::Y, 400.0)]));
    let selected: Shared<Option<GuideKey>> = Rc::default();
    let (shown, picked) = (Rc::clone(&guides), Rc::clone(&selected));
    let mut fixture = ToolFixture::with_view("Document", Recorder::default(), view_at(1.0), move |ui, cx| {
        let mut list = shown.borrow_mut();
        let current: Vec<GuideKey> = picked.borrow().iter().copied().collect();
        let built = Guides::new("Guides", canvas_of(cx), Bind::bound(&mut *list)).interactive(true).selected_guides(&current);
        if let Some(select) = built.show_at(ui, cx.rect).select {
            *picked.borrow_mut() = select;
        }
    });
    fixture.press_at(pos2(200.0, 300.0)).release_at(pos2(200.0, 300.0));
    assert_eq!(*selected.borrow(), Some(GuideKey::new(1)), "a click selects the guide");
    fixture.fixture().key(Key::Delete, Modifiers::NONE).idle(3);
    assert!(at(&guides, Axis::Y, 200.0).is_none(), "Delete removes it: {:?}", guides.borrow());
    assert_eq!(*selected.borrow(), None, "and selects no other guide");
}

// A right-click on an unlocked guide is the guide's: the guides report its menu request and the tool host under them
// reports none. Off the guide the right-click reaches the host, which asks for its own menu.
#[trial]
fn a_right_click_on_a_guide_asks_for_the_guide_menu_not_the_tools() {
    use craft_ui::canvas::{Canvas, ToolHost, tool_pass};
    use craft_ui::testing::Fixture;
    use craft_ui::testing::tool_fixture::DOCUMENT;
    let guides: Shared<Vec<Guide>> = Rc::new(RefCell::new(vec![guide(1, Axis::Y, 200.0)]));
    let asked: Shared<(Option<GuideKey>, bool)> = Rc::new(RefCell::new((None, false)));
    let (shown, seen) = (Rc::clone(&guides), Rc::clone(&asked));
    let tool = RefCell::new(Recorder::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").id(tool_fixture::canvas_id("Document")).document(DOCUMENT).view(view_at(1.0)(room));
        let _ = canvas.show(ui, |ui, body| {
            let out = Guides::new("Guides", canvas_of(body), Bind::bound(&mut *shown.borrow_mut())).interactive(true).show_at(ui, body.rect);
            let mut seen = seen.borrow_mut();
            seen.0 = seen.0.or(out.context.map(|(key, _)| key));
            if let Some(pass) = tool_pass(&body.env) {
                seen.1 |= ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()).context.is_some();
            }
        });
    });
    fixture.settle();
    let mut right_click = |at: Pos2| {
        *asked.borrow_mut() = (None, false);
        fixture.frame(vec![pointer::moved(at)]);
        fixture.frame(vec![pointer::secondary(at, true)]);
        fixture.frame(vec![pointer::secondary(at, false)]);
        fixture.settle();
        *asked.borrow()
    };
    assert_eq!(right_click(pos2(200.0, 300.0)), (Some(GuideKey::new(1)), false), "on the guide: its menu, not the tool's");
    assert_eq!(right_click(pos2(400.0, 300.0)), (None, true), "off the guide: the tool's menu");
}

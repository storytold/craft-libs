//! The Hand drag's limit with Overscroll on (photocraft#12025): the drag may take the document past the canvas edge,
//! but stops when 100 screen points of it stay in view at the opposite edge. The rows come from the reference
//! application: a 3000 x 2000 document in an image area of about 1473 x 817 screen points, where the view centre stops
//! 636.5 points past the left and right image edge and 308.5 points past the top and bottom, the same at 100 % and at
//! 200 %.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{Axis, CanvasView};
use craft_ui::canvas::{Canvas, CanvasScrollbars, CanvasTool, HandTool, Momentary, NavigationOptions, NoTool, ToolHost, tool_pass};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Event, Key, Modifiers, Pos2, Rect};

/// The document of the issue's measured rows: 3000 by 2000 px.
const DOCUMENT: Size = Size::new(3000.0, 2000.0);

/// The image area it was measured in: about 1473 by 817 screen points.
const VIEW: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1473.0, 817.0));

/// What the reference application leaves in view at the opposite edge, in screen points.
const LEFT_IN_VIEW: f64 = 100.0;

/// The step of one pass, in seconds: the clock step `Fixture::frame` takes on its own.
const FRAME: f64 = 1.0 / 60.0;

/// The slack of a centre read after a drag, in points: the canvas shows its view at whole device pixels and a drag
/// starts from the shown view, so the centre stands under a point away from the pointer's distance.
const SHOWN: f64 = 1.0;

/// `set` applied to the default navigation options.
fn options(set: impl FnOnce(&mut NavigationOptions)) -> NavigationOptions {
    let mut options = NavigationOptions::default();
    set(&mut options);
    options
}

/// Overscroll on with no glide, so a drag's own view is what the trial reads.
fn overscroll() -> NavigationOptions {
    options(|o| (o.overscroll, o.flick_panning) = (true, false))
}

/// The document's length on `axis`.
fn length(document: Size, axis: Axis) -> f64 {
    if axis == Axis::X { document.width } else { document.height }
}

/// The canvas's own length on `axis`.
fn view_side(axis: Axis) -> f64 {
    if axis == Axis::X { f64::from(VIEW.width()) } else { f64::from(VIEW.height()) }
}

/// How far the view centre may sit past an image edge on `axis`, in document units: the view's half less
/// [`LEFT_IN_VIEW`], at `zoom`. The issue's rows give 636.5 and 308.5 points at zoom 1, and the same at zoom 2.
fn room(axis: Axis, zoom: f64) -> f64 {
    (view_side(axis) / 2.0 - LEFT_IN_VIEW) / zoom
}

/// The centre the limit leaves after the document's start edge on `axis` has been panned past.
fn past_start(axis: Axis, zoom: f64) -> f64 {
    -room(axis, zoom)
}

/// ... and after its end edge.
fn past_end(axis: Axis, document: Size, zoom: f64) -> f64 {
    length(document, axis) + room(axis, zoom)
}

/// The centre's coordinate on `axis`.
fn coord(center: Point, axis: Axis) -> f64 {
    if axis == Axis::X { center.x } else { center.y }
}

/// The other canvas axis.
fn across(axis: Axis) -> Axis {
    if axis == Axis::X { Axis::Y } else { Axis::X }
}

/// The longest drag the image area holds on `axis`, in points.
fn full_drag(axis: Axis) -> f32 {
    if axis == Axis::X { 1400.0 } else { 740.0 }
}

/// A press and a drag of `points` along `axis`, `toward_start` being toward the document's lower edge: the pointer
/// starts 20 points inside the far edge of the image area and moves `points` across it. Dragging right moves the
/// centre left.
fn gesture(axis: Axis, toward_start: bool, points: f32) -> (Pos2, Pos2) {
    let side = if axis == Axis::X { VIEW.width() } else { VIEW.height() };
    let at = |along: f32| {
        if axis == Axis::X { Pos2::new(along, VIEW.center().y) } else { Pos2::new(VIEW.center().x, along) }
    };
    let from = if toward_start { 20.0 } else { side - 20.0 };
    (at(from), at(from + if toward_start { points } else { -points }))
}

/// The middle button going down (`pressed`) or up at `at`, with the pointer there. The testing kit has no
/// middle-button helper.
fn middle(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE }
}

/// A canvas over `document` shown at `VIEW`, its tool `tool`, the momentary set `momentary` when the arm is the Space
/// Hand, and the canvas's navigation `navigation`. The view starts centred on the document at `zoom`.
struct Rig {
    fixture: Fixture,
    view: Rc<Cell<CanvasView>>,
}

impl Rig {
    fn new<T: CanvasTool + 'static>(tool: T, navigation: NavigationOptions, momentary: Option<Momentary>, document: Size, zoom: f64) -> Self {
        let tool = Rc::new(RefCell::new(tool));
        let set = Rc::new(RefCell::new(momentary));
        let start = CanvasView::new(VIEW, Point::new(document.width / 2.0, document.height / 2.0), zoom);
        let view = Rc::new(Cell::new(start));
        let (shown_tool, shown_set, shown_view) = (Rc::clone(&tool), Rc::clone(&set), Rc::clone(&view));
        let mut fixture = Fixture::new(move |ui| {
            let current = shown_view.get();
            let mut reply = None;
            let shown = Canvas::new("Document").document(document).navigation(navigation).view(current).show_at(ui, VIEW, |ui, body| {
                if let Some(pass) = tool_pass(&body.env) {
                    let mut held = shown_set.borrow_mut();
                    let mut host = ToolHost::new();
                    if let Some(set) = held.as_mut() {
                        host = host.momentary(set);
                    }
                    reply = Some(host.run(ui, &pass, &mut *shown_tool.borrow_mut()));
                }
            });
            shown_view.set(reply.as_ref().and_then(|reply| reply.view).or(shown.output.view).unwrap_or(current));
        });
        fixture.room(VIEW.size()).settle();
        Rig { fixture, view }
    }

    fn view(&self) -> CanvasView {
        self.view.get()
    }

    /// The view's rect the canvas gave back: the trials measure the issue's image area, so check the rig really has it.
    fn rect(&self) -> Rect {
        self.view().rect
    }

    /// One frame `dt` seconds after the previous one: the fixture's frame moves the clock by [`FRAME`], the rig by the
    /// rest.
    fn tick(&mut self, dt: f64, events: Vec<Event>) {
        self.fixture.advance(dt - FRAME);
        self.fixture.frame(events);
    }

    /// Hovers and presses at `at` in separate frames.
    fn press(&mut self, at: Pos2) {
        self.tick(FRAME, vec![pointer::moved(at)]);
        self.fixture.frame_held(vec![pointer::press(at)]);
    }

    /// Moves from `from` to `to` in `steps` frames of `dt` seconds.
    fn drag_to(&mut self, from: Pos2, to: Pos2, steps: usize, dt: f64) {
        for step in 1..=steps {
            let at = from + (to - from) * (step as f32 / steps as f32);
            self.tick(dt, vec![pointer::moved(at)]);
        }
    }

    /// Releases at `at` in a frame `dt` seconds on, then one idle frame of the same step.
    fn release(&mut self, at: Pos2, dt: f64) {
        self.tick(dt, vec![pointer::release(at)]);
        self.tick(dt, Vec::new());
    }

    /// One primary-button Hand drag from `from` to `to`, and idle frames after it.
    fn drag(&mut self, from: Pos2, to: Pos2, steps: usize) {
        self.press(from);
        self.drag_to(from, to, steps, FRAME);
        self.release(to, FRAME);
    }

    /// One middle-button drag from `from` to `to`.
    fn middle_drag(&mut self, from: Pos2, to: Pos2, steps: usize) {
        self.tick(FRAME, vec![pointer::moved(from)]);
        self.tick(FRAME, vec![middle(from, true)]);
        self.drag_to(from, to, steps, FRAME);
        self.tick(FRAME, vec![middle(to, false)]);
        self.tick(FRAME, Vec::new());
    }
}

/// With Overscroll on, a Hand drag stops with 100 points of the image in view at the opposite edge, on each axis, to
/// each edge, at 100 % and at 200 %: the issue's rows of 636.5 and 308.5 screen points beyond the image edge.
#[trial]
fn a_hand_drag_with_overscroll_on_stops_with_100_points_of_the_image_in_view() {
    for zoom in [1.0, 2.0] {
        for axis in [Axis::X, Axis::Y] {
            for toward_start in [true, false] {
                let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, zoom);
                assert_eq!(rig.rect(), VIEW, "the rig shows the issue's image area");
                // Five drags of the whole image area: together they pass the limit on either axis at either zoom.
                let (from, to) = gesture(axis, toward_start, full_drag(axis));
                for _ in 0..5 {
                    rig.drag(from, to, 7);
                }
                let want = if toward_start { past_start(axis, zoom) } else { past_end(axis, DOCUMENT, zoom) };
                let got = coord(rig.view().center, axis);
                let past = (if toward_start { got } else { got - length(DOCUMENT, axis) }).abs() * zoom;
                assert!(
                    (got - want).abs() < SHOWN,
                    "zoom {zoom}, {axis:?} toward start {toward_start}: the centre stops at {want}, it came to {got} \
                     ({past} points past the image edge against the measured {})",
                    if axis == Axis::X { 636.5 } else { 308.5 }
                );
            }
        }
    }
}

/// Inside that limit the drag still follows the pointer one to one, and leaves the other axis where it was.
#[trial]
fn inside_the_limit_a_hand_drag_still_pans_one_to_one_and_leaves_the_other_axis() {
    for zoom in [1.0, 2.0] {
        for axis in [Axis::X, Axis::Y] {
            for toward_start in [true, false] {
                let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, zoom);
                let before = rig.view().center;
                // One drag of the image area stays inside the limit on every row of the issue.
                let (from, to) = gesture(axis, toward_start, full_drag(axis));
                rig.drag(from, to, 7);
                let after = rig.view().center;
                let want = f64::from(full_drag(axis)) / zoom;
                let sign = if toward_start { -1.0 } else { 1.0 };
                assert!(
                    (coord(after, axis) - (coord(before, axis) + sign * want)).abs() < SHOWN,
                    "zoom {zoom}, {axis:?} toward start {toward_start}: the drag pans 1:1 and the centre moves away \
                     from the pointer: {before:?} -> {after:?}"
                );
                assert!(
                    (coord(after, across(axis)) - coord(before, across(axis))).abs() < SHOWN,
                    "zoom {zoom}: the {axis:?} drag leaves the other axis alone: {before:?} -> {after:?}"
                );
            }
        }
    }
}

/// The limit stands on its own axis: at it the other axis can still be dragged, and the axis at its limit stays there.
#[trial]
fn the_limit_clamps_one_axis_and_leaves_the_other_free() {
    for axis in [Axis::X, Axis::Y] {
        let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, 1.0);
        let (from, to) = gesture(axis, true, full_drag(axis));
        for _ in 0..5 {
            rig.drag(from, to, 7);
        }
        let at_limit = coord(rig.view().center, axis);
        assert!((at_limit - past_start(axis, 1.0)).abs() < SHOWN, "the {axis:?} drag stands at its limit: {at_limit}");
        // A shorter drag along the other axis, still inside its own limit.
        let (from, to) = gesture(across(axis), true, 400.0);
        let before = rig.view().center;
        rig.drag(from, to, 7);
        let after = rig.view().center;
        assert!(
            (coord(after, across(axis)) - (coord(before, across(axis)) - 400.0)).abs() < SHOWN,
            "the {axis:?} stands at its limit while the other pans: {before:?} -> {after:?}"
        );
        assert!((coord(after, axis) - at_limit).abs() < SHOWN, "and the {axis:?} stays at its limit: {after:?}");
    }
}

/// Space held pans with the same Hand tool, and stops at the same limit.
#[trial]
fn the_space_held_hand_stops_at_the_same_limit_with_overscroll_on() {
    let navigation = overscroll();
    let mut rig = Rig::new(NoTool, navigation, Some(Momentary::new(navigation)), DOCUMENT, 1.0);
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    rig.tick(FRAME, vec![pointer::moved(from)]);
    rig.tick(FRAME, vec![key_press(Key::Space, Modifiers::NONE)]);
    for _ in 0..5 {
        rig.press(from);
        rig.drag_to(from, to, 7, FRAME);
        rig.release(to, FRAME);
    }
    let got = rig.view().center.x;
    assert!((got - past_start(Axis::X, 1.0)).abs() < SHOWN, "Space's Hand stops 636.5 points past the image's left edge, it came to {got}");
}

/// The middle button pans the view too, and stops at the same limit.
#[trial]
fn the_middle_button_drag_stops_at_the_same_limit_with_overscroll_on() {
    let mut rig = Rig::new(NoTool, overscroll(), None, DOCUMENT, 1.0);
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    for _ in 0..5 {
        rig.middle_drag(from, to, 7);
    }
    let got = rig.view().center.x;
    assert!((got - past_start(Axis::X, 1.0)).abs() < SHOWN, "the middle drag stops 636.5 points past the image's left edge, it came to {got}");
    assert!((rig.view().center.y - 1000.0).abs() < SHOWN, "the middle drag keeps the other axis: {:?}", rig.view());
}

/// An image smaller than the view (Fit Screen) can still be panned with Overscroll on: it is not centred, and it stops
/// with the same 100 points of it in view at the far edge.
#[trial]
fn a_document_smaller_than_the_view_can_still_be_panned_with_overscroll_on() {
    let small = Size::new(1000.0, 800.0);
    let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, small, 1.0);
    let before = rig.view().center;
    // A short drag: inside the limit, so the fitting image follows the pointer.
    let (from, to) = gesture(Axis::X, true, 400.0);
    rig.drag(from, to, 7);
    let panned = rig.view().center;
    assert!((panned.x - (before.x - 400.0)).abs() < SHOWN, "a fitting image pans with the pointer: {before:?} -> {panned:?}");
    // On past the limit: the same 100 points of the small image stay in view at the far edge.
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    for _ in 0..3 {
        rig.drag(from, to, 7);
    }
    let at = rig.view().center;
    assert!((at.x - past_start(Axis::X, 1.0)).abs() < SHOWN, "and it stops there, not in the middle: {at:?}");
    assert!((at.y - before.y).abs() < SHOWN, "the axis nobody dragged stays: {before:?} -> {at:?}");
}

/// A mirrored view (View, Flip Horizontal) stops at the same limit: the pointer's drag moves the centre the mirror's
/// way, and the range is the document's own.
#[trial]
fn a_mirrored_view_stops_at_the_same_limit_with_overscroll_on() {
    let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, 1.0);
    let mut mirrored = rig.view();
    mirrored.flip_x = true;
    rig.view.set(mirrored);
    rig.tick(FRAME, Vec::new());
    // Dragging right moves the centre toward the document's end edge in a mirrored view.
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    for _ in 0..5 {
        rig.drag(from, to, 7);
    }
    let got = rig.view().center.x;
    assert!(
        (got - past_end(Axis::X, DOCUMENT, 1.0)).abs() < SHOWN,
        "the mirrored drag stops 636.5 points past the image's end edge, it came to {got}"
    );
}

/// The limit holds inside one held drag, not only at its release: the pass where the pointer passes the biting point
/// stops there with the button still down, and the release keeps it there.
#[trial]
fn the_drag_stops_at_the_limit_while_the_button_is_still_down() {
    let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, 1.0);
    // One gesture: the centre comes to 100 document units from the image's left edge, inside the range.
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    rig.drag(from, to, 7);
    assert!((rig.view().center.x - 100.0).abs() < SHOWN, "the first drag leaves the centre at 100: {:?}", rig.view().center);
    // A second gesture, held: the first half is still inside the range, the rest stops at the limit before any release.
    rig.press(from);
    let middle = egui::pos2(from.x + 700.0, from.y);
    rig.drag_to(from, middle, 4, FRAME);
    let inside = rig.view().center.x;
    assert!((inside - -600.0).abs() < SHOWN, "inside the range the held drag still follows the pointer: {inside}");
    rig.drag_to(middle, to, 4, FRAME);
    let held = rig.view().center.x;
    assert!((held - past_start(Axis::X, 1.0)).abs() < SHOWN, "the held drag stops at the limit, before its release: {held}");
    rig.release(to, FRAME);
    assert!((rig.view().center.x - held).abs() < SHOWN, "and the release keeps it there: {held} -> {}", rig.view().center.x);
}

/// The limit is the thumb's range on the x axis even when the pixels are not square (View, Pixel Aspect Ratio
/// Correction): at aspect 2 the drag stops where a thumb drag stops, at -318.25 document units on this canvas rather
/// than the -268.25 a margin not divided by the aspect would give.
#[trial]
fn an_aspect_corrected_drag_stops_where_the_thumb_does() {
    let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, 1.0);
    let mut corrected = rig.view();
    corrected.aspect = 2.0;
    rig.view.set(corrected);
    rig.tick(FRAME, Vec::new());
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    for _ in 0..5 {
        rig.drag(from, to, 7);
    }
    let panned = rig.view().center.x;
    let thumb = thumb_centre_x(2.0);
    assert!((panned - thumb).abs() < SHOWN, "at aspect 2 the drag stops where the thumb does: drag {panned}, thumb {thumb}");
}

/// Where a thumb drag at the end of its track puts the view centre on the x axis, for the same canvas at the pixel
/// aspect `aspect`: the scroll bars' own extreme.
fn thumb_centre_x(aspect: f64) -> f64 {
    let start = CanvasView::new(VIEW, Point::new(DOCUMENT.width / 2.0, DOCUMENT.height / 2.0), 1.0).aspect(aspect);
    let view = Rc::new(Cell::new(start));
    let observed = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut next = observed.get();
        let shown = Canvas::new("Document").document(DOCUMENT).navigation(overscroll()).view(next).show_at(ui, VIEW, |_, _| {});
        next = shown.output.view.unwrap_or(next);
        let doc = DocRect::from_origin_size(Point::ZERO, DOCUMENT);
        let _ = CanvasScrollbars::new("Scroll bars", Bind::bound(&mut next), doc).overscroll(true).document_chrome().show_at(ui, VIEW);
        observed.set(next);
    });
    fixture.room(VIEW.size()).settle();
    let track = fixture.by_label("Horizontal scroll bar").expect("the horizontal track").rect;
    let at = track.center();
    fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]);
    for step in 1..=20 {
        fixture.frame(vec![pointer::moved(at + egui::Vec2::new(-150.0 * step as f32, 0.0))]);
    }
    fixture.frame(vec![pointer::release(at + egui::Vec2::new(-3000.0, 0.0))]).settle();
    view.get().center.x
}

/// Overscroll off keeps today's clamp: the drag stops with the image's edge at the canvas edge, no pasteboard room.
#[trial]
fn overscroll_off_keeps_today_s_clamp_at_the_image_edges() {
    let off = options(|o| o.flick_panning = false);
    for axis in [Axis::X, Axis::Y] {
        let mut rig = Rig::new(HandTool::new(off), off, None, DOCUMENT, 1.0);
        let (from, to) = gesture(axis, true, full_drag(axis));
        for _ in 0..5 {
            rig.drag(from, to, 7);
        }
        let half = view_side(axis) / 2.0;
        let got = coord(rig.view().center, axis);
        assert!((got - half).abs() < SHOWN, "{axis:?} with Overscroll off stops at the image edge: {got}");
    }
}

/// A turned view keeps today's free pan: the issue does not measure one.
#[trial]
fn a_turned_view_keeps_today_s_free_pan_with_overscroll_on() {
    let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, 1.0);
    let mut turned = rig.view();
    turned.rotation = 30.0;
    rig.view.set(turned);
    rig.tick(FRAME, Vec::new());
    let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
    for _ in 0..5 {
        rig.drag(from, to, 7);
    }
    let got = rig.view().center.x;
    assert!(
        got < past_start(Axis::X, 1.0) - 100.0,
        "a turned view pans as freely as today, past the level view's limit: {got} against {}",
        past_start(Axis::X, 1.0)
    );
}

/// A flick glide keeps today's run: the issue does not measure whether a glide stops at the limit, so it goes on past
/// it. With Flick Panning off the same release stops where the drag stopped.
#[trial]
fn a_flick_glide_keeps_today_s_run_past_the_limit_and_flick_off_stops_at_it() {
    for flick in [true, false] {
        let label = if flick { "Flick Panning on" } else { "Flick Panning off" };
        let navigation = options(|o| (o.overscroll, o.flick_panning) = (true, flick));
        let mut rig = Rig::new(HandTool::new(navigation), navigation, None, DOCUMENT, 1.0);
        let (from, to) = gesture(Axis::X, true, full_drag(Axis::X));
        // Five fast drags: the drag stands at the limit, and the pointer is still moving at the release.
        for _ in 0..5 {
            rig.press(from);
            rig.drag_to(from, to, 4, FRAME);
            rig.release(to, FRAME);
        }
        let at_release = rig.view().center.x;
        for _ in 0..30 {
            rig.tick(FRAME, Vec::new());
        }
        let after = rig.view().center.x;
        if flick {
            assert!(after < at_release - 1.0, "{label}: the glide runs on: {at_release} -> {after}");
            assert!(after < past_start(Axis::X, 1.0), "{label}: and past the drag's limit: {after}");
        } else {
            assert_eq!(after, at_release, "{label}: the release starts no glide: {at_release} -> {after}");
        }
    }
}

/// The wheel keeps today's free scroll with Overscroll on: the issue measures a Hand drag, not a scroll.
#[trial]
fn a_wheel_scroll_with_overscroll_on_keeps_today_s_free_view() {
    let mut rig = Rig::new(HandTool::new(overscroll()), overscroll(), None, DOCUMENT, 1.0);
    rig.tick(FRAME, vec![pointer::moved(VIEW.center())]);
    for _ in 0..10 {
        rig.tick(FRAME, vec![pointer::wheel_lines(20.0)]);
    }
    let got = rig.view().center.y;
    assert!(got < past_start(Axis::Y, 1.0) - 100.0, "the wheel scrolls past the drag's limit as today: {got} against {}", past_start(Axis::Y, 1.0));
}

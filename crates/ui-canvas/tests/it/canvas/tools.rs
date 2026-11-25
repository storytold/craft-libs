//! The view tools: what a gesture asks the app's view to become, and the checks every tool owes the canvas.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Vec2 as DocVec;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_keymap::Mods;
use craft_ui::canvas::{
    Canvas, CanvasTool, HandTool, LayerBounds, Momentary, MomentaryCommands, MomentaryShift, MomentaryTool, NavigationOptions, RotateViewTool,
    ToolHost, ViewCommands, ViewTarget, ZoomTool, tool_pass,
};
use craft_ui::contract::{Bind, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{self, FirePhase, FireSource};
use craft_ui::testing::{Fixture, ToolFixture, pointer, trial};
use egui::{Event, Modifiers, Pos2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The document the rigs show.
const DOCUMENT: Size = Size::new(1200.0, 900.0);

/// The step of one pass, in seconds: the clock step `Fixture::frame` takes on its own.
const FRAME: f64 = 1.0 / 60.0;

/// A tool on a canvas whose view the rig keeps across frames as the host reports it, with the pans the host reported.
struct Rig {
    fixture: Fixture,
    view: Rc<Cell<Option<CanvasView>>>,
    pans: Rc<RefCell<Vec<(DocVec, bool)>>>,
}

impl Rig {
    fn new<T: CanvasTool + 'static>(tool: T, start: impl Fn(egui::Rect) -> CanvasView + 'static) -> Self {
        Self::scaled(tool, 1.0, NavigationOptions::default(), start)
    }

    /// A rig whose canvas shows at `ppp` pixels per point from its first frame, with the canvas's navigation options
    /// `navigation`.
    fn scaled<T: CanvasTool + 'static>(tool: T, ppp: f32, navigation: NavigationOptions, start: impl Fn(egui::Rect) -> CanvasView + 'static) -> Self {
        let tool = Rc::new(RefCell::new(tool));
        let view = Rc::new(Cell::new(None::<CanvasView>));
        let pans = Rc::new(RefCell::new(Vec::<(DocVec, bool)>::new()));
        let (shown_tool, shown_view, shown_pans) = (Rc::clone(&tool), Rc::clone(&view), Rc::clone(&pans));
        let mut fixture = Fixture::new(move |ui| {
            let room = ui.available_rect_before_wrap();
            let current = shown_view.get().unwrap_or_else(|| start(room));
            let mut reply = None;
            let shown = Canvas::new("Document").document(ViewTarget::from(DOCUMENT)).navigation(navigation).view(current).show(ui, |ui, body| {
                if let Some(pass) = tool_pass(&body.env) {
                    reply = Some(ToolHost::new().run(ui, &pass, &mut *shown_tool.borrow_mut()));
                }
            });
            let reply = reply.unwrap_or_default();
            if let Some(pan) = reply.pan {
                shown_pans.borrow_mut().push((pan.delta, pan.scroll_all));
            }
            shown_view.set(Some(reply.view.or(shown.output.view).unwrap_or(current)));
        });
        fixture.ctx().set_pixels_per_point(ppp);
        fixture.settle();
        Rig { fixture, view, pans }
    }

    fn view(&self) -> CanvasView {
        self.view.get().expect("the rig has shown a frame")
    }

    fn ppp(&self) -> f32 {
        self.fixture.ctx().pixels_per_point()
    }

    fn device(&self) -> f64 {
        zoom::device(&self.view(), self.ppp())
    }

    fn centre(&self) -> Pos2 {
        self.view().rect.center()
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

    fn click(&mut self, at: Pos2) {
        self.press(at);
        self.release(at, FRAME);
    }
}

/// `set` applied to the default navigation options.
fn options(set: impl FnOnce(&mut NavigationOptions)) -> NavigationOptions {
    let mut options = NavigationOptions::default();
    set(&mut options);
    options
}

fn plain(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(600.0, 450.0), 2.0)
}

fn turned(room: egui::Rect, rotation: f64, flip_x: bool) -> CanvasView {
    let mut view = CanvasView::new(room, Point::new(600.0, 450.0), 0.5);
    view.rotation = rotation;
    view.flip_x = flip_x;
    view
}

/// Whether the document point `before` showed at `at` is still at `at` in `after`.
fn pinned(before: &CanvasView, after: &CanvasView, at: Pos2) -> bool {
    (after.to_screen(before.to_doc(at)) - at).length() < 0.5
}

#[trial]
fn each_view_tool_owes_the_canvas_its_checks() {
    ToolFixture::new("Hand", HandTool::new(NavigationOptions::default())).check();
    ToolFixture::new("Zoom", ZoomTool::new(NavigationOptions::default())).check();
    ToolFixture::new("Rotate View", RotateViewTool::default()).check();
}

/// The view commands that read a fact of the document beyond its size: each runs with its fact, and without it is
/// disabled and leaves the view as it is.
#[trial]
fn view_commands_run_with_their_facts_and_are_disabled_without() {
    let ppp = 2.0;
    let size = Size::new(4000.0, 3000.0);
    let board = Rect::new(2500.0, 400.0, 3500.0, 1200.0);
    let layers = Rect::new(100.0, 2000.0, 300.0, 2900.0);
    let full = ViewTarget::new(size).artboard(board).ppi(300.0);
    let bare = ViewTarget::from(size);
    let bounds = || Some(layers);
    let given: Option<LayerBounds<'_>> = Some(&bounds);
    let start = CanvasView::new(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)), Point::new(2000.0, 1500.0), 0.1);
    let device = |view: &CanvasView| zoom::device(view, ppp);
    let shows = |view: &CanvasView, rect: Rect| {
        [rect.origin(), Point::new(rect.x1, rect.y1)].iter().all(|corner| view.rect.expand(0.5).contains(view.to_screen(*corner)))
    };
    // What each command owes the view it changed.
    type Owed<'a> = &'a dyn Fn(&CanvasView) -> bool;
    let rows: [(ViewCommands, Owed<'_>); 4] = [
        (ViewCommands::TwoHundredPercent, &|view| (device(view) - 2.0).abs() < 1e-9 && view.center == start.center),
        // Print Size: the screen's 72 ppi over the document's 300.
        (ViewCommands::PrintSize, &|view| (device(view) - 72.0 / 300.0).abs() < 1e-9 && view.center == start.center),
        (ViewCommands::FitArtboard, &|view| view.center == board.center() && shows(view, board) && device(view) > device(&start)),
        (ViewCommands::FitLayers, &|view| view.center == layers.center() && shows(view, layers) && device(view) > device(&start)),
    ];
    for (command, owed) in rows {
        let mut view = start;
        assert!(command.available(&start, &full, given, ppp, &craft_behavior::ZoomScheme::PHOTOSHOP), "{command:?} runs with its fact");
        assert!(command.apply(&mut view, &full, given, ppp, &craft_behavior::ZoomScheme::PHOTOSHOP) && owed(&view), "{command:?} gives {view:?}");
        if command == ViewCommands::TwoHundredPercent {
            assert!(command.available(&start, &bare, None, ppp, &craft_behavior::ZoomScheme::PHOTOSHOP), "200% needs no fact");
            continue;
        }
        let mut view = start;
        assert!(!command.available(&start, &bare, None, ppp, &craft_behavior::ZoomScheme::PHOTOSHOP), "{command:?} is disabled without its fact");
        assert!(
            !command.apply(&mut view, &bare, None, ppp, &craft_behavior::ZoomScheme::PHOTOSHOP) && view == start,
            "{command:?} leaves the view without its fact"
        );
    }
}

#[trial]
fn zoom_drag_keeps_the_press_point_and_fits_the_rectangle() {
    // Device zoom is the view's zoom times the pixels per point, so each case runs at 1 and at 2. Overscroll is on, so
    // the canvas keeps the view the tool set: off, a zoom out that leaves the document smaller than the view centres it.
    let free = options(|o| o.overscroll = true);
    for (rotation, ppp) in [(-45.0, 1.0f32), (-45.0, 2.0), (90.0, 1.0), (90.0, 2.0)] {
        for flip_x in [false, true] {
            let start = move |room: egui::Rect| turned(room, rotation, flip_x);
            let label = format!("rotation {rotation}, flip {flip_x}, {ppp} pixels per point");

            // A click zooms in (Alt: out) about the clicked point.
            for out in [false, true] {
                let mut rig = Rig::scaled(ZoomTool::new(options(|o| o.animated_zoom = false)), ppp, free, start);
                let (before, at, device) = (rig.view(), rig.centre() + egui::vec2(80.0, 50.0), rig.device());
                if out {
                    rig.fixture.hold(Modifiers::ALT);
                }
                rig.click(at);
                assert!(pinned(&before, &rig.view(), at), "{label}: the clicked point stays under the pointer (out {out})");
                assert_eq!(rig.device() > device, !out, "{label}: a click zooms {}", if out { "out" } else { "in" });
                assert_ne!(rig.device(), device, "{label}: the click changes the zoom");
            }

            // A scrub of one doubling keeps the press point.
            let mut rig = Rig::scaled(ZoomTool::new(options(|o| (o.animated_zoom, o.scrubby_zoom) = (false, true))), ppp, free, start);
            let (before, at, device) = (rig.view(), rig.centre() + egui::vec2(-70.0, 30.0), rig.device());
            rig.press(at);
            rig.drag_to(at, at + egui::vec2(100.0, 0.0), 5, FRAME);
            rig.release(at + egui::vec2(100.0, 0.0), FRAME);
            assert!(pinned(&before, &rig.view(), at), "{label}: a scrub keeps the press point");
            assert!(rig.device() > device, "{label}: dragging right scrubs the zoom in, got {}", rig.device() / device);

            // A rectangle changes nothing until the release, which centres it and fits it; Alt steps out once.
            for out in [false, true] {
                let mut rig = Rig::scaled(ZoomTool::new(options(|o| (o.animated_zoom, o.scrubby_zoom) = (false, false))), ppp, free, start);
                let (before, centre, device) = (rig.view(), rig.centre(), rig.device());
                let (a, b) = (centre + egui::vec2(-60.0, -40.0), centre + egui::vec2(60.0, 40.0));
                if out {
                    rig.fixture.hold(Modifiers::ALT);
                }
                rig.press(a);
                rig.drag_to(a, b, 6, FRAME);
                assert_eq!(rig.view(), before, "{label}: the rectangle changes no view before the release");
                rig.release(b, FRAME);
                let after = rig.view();
                if out {
                    assert!(rig.device() < device, "{label}: Alt steps the zoom out");
                    assert!(pinned(&before, &after, a), "{label}: stepping out keeps the press point");
                } else {
                    assert!(rig.device() > device, "{label}: the rectangle zooms in");
                    assert!(
                        (after.to_screen(before.to_doc(centre)) - after.rect.center()).length() < 0.5,
                        "{label}: the rectangle's centre is the view centre"
                    );
                }
            }
        }
    }
}

#[trial]
fn a_hand_pan_reports_its_document_distance_and_its_scroll_all_choice() {
    // (Shift at the press, the Scroll All Windows option) and the choice the pans carry.
    let rows = [(false, false, false), (true, false, true), (false, true, true)];
    for (shift, option, expected) in rows {
        let mut rig = Rig::new(HandTool::new(options(|o| (o.scroll_all_windows, o.flick_panning) = (option, true))), plain);
        let (before, c) = (rig.view().center, rig.centre());
        if shift {
            rig.fixture.hold(Modifiers::SHIFT);
        }
        rig.press(c);
        // Shift comes up during the drag; the choice made at the press holds through the drag and the glide.
        rig.fixture.hold(Modifiers::NONE);
        let to = c + egui::vec2(120.0, 0.0);
        rig.drag_to(c, to, 8, FRAME);
        rig.release(to, FRAME);
        let dragged = rig.pans.borrow().len();
        assert!(dragged > 0, "a drag reports its pans (shift {shift}, option {option})");
        for _ in 0..10 {
            rig.tick(FRAME, Vec::new());
        }
        let pans = rig.pans.borrow().clone();
        assert!(pans.len() > dragged, "the flick glide reports pans too (shift {shift}, option {option})");
        assert!(pans.iter().all(|(_, all)| *all == expected), "scroll all is {expected}: {pans:?}");
        // The pans are document distances: together they are how far the view centre moved, against the pointer.
        let sum = pans.iter().fold(DocVec::ZERO, |sum, (delta, _)| sum + *delta);
        let moved = rig.view().center - before;
        assert!((sum - moved).hypot() < 1e-6, "the pans sum to the centre's move: {sum:?} against {moved:?}");
        assert!(sum.x < 0.0 && sum.y.abs() < 1e-6, "dragging right moves the centre left in document units: {sum:?}");
    }

    // A view at its Overscroll limit (Overscroll is off) cannot move: dragging on outward reports a pan of 0.
    // The view starts 20 units inside the document's left edge, so the drag reaches the limit part way.
    let at_edge = |room: egui::Rect| CanvasView::new(room, Point::new(f64::from(room.width()) / 4.0 + 20.0, 450.0), 2.0);
    let mut rig = Rig::new(HandTool::new(options(|o| o.flick_panning = false)), at_edge);
    let c = rig.centre();
    rig.press(c);
    rig.drag_to(c, c + egui::vec2(240.0, 0.0), 12, FRAME);
    let pans = rig.pans.borrow().clone();
    assert!(pans.iter().any(|(delta, _)| *delta != DocVec::ZERO), "the drag moves the view before it reaches the limit");
    assert_eq!(pans.last().map(|(delta, _)| *delta), Some(DocVec::ZERO), "a drag on past the limit reports no distance: {pans:?}");
    rig.release(c + egui::vec2(240.0, 0.0), FRAME);
}

/// Rotate View Clockwise turns the image clockwise on screen (screen y points down, so the angle grows), and
/// Counterclockwise the other way, in a mirrored view as in a plain one (#4931).
#[trial]
fn the_rotate_commands_step_fifteen_degrees_on_screen_in_a_flipped_view_too() {
    let room = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
    let probe = Point::new(700.0, 480.0);
    let angle = |view: &CanvasView| {
        let v = view.to_screen(probe) - view.rect.center();
        f64::from(v.y).atan2(f64::from(v.x)).to_degrees()
    };
    let target = ViewTarget::from(DOCUMENT);
    for flip_x in [false, true] {
        for (command, screen_turn) in [(ViewCommands::RotateClockwise, 15.0), (ViewCommands::RotateCounterclockwise, -15.0)] {
            let before = turned(room, 20.0, flip_x);
            let mut view = before;
            assert!(command.apply(&mut view, &target, None, 1.0, &craft_behavior::ZoomScheme::PHOTOSHOP), "{command:?} turns the view");
            let turned_by = (angle(&view) - angle(&before) + 540.0).rem_euclid(360.0) - 180.0;
            assert!(
                (turned_by - screen_turn).abs() < 0.01,
                "flip {flip_x}: {command:?} turns the image {screen_turn} degrees on screen, got {turned_by}"
            );
        }
    }
}

#[trial]
fn a_rotate_view_drag_follows_the_pointer_in_a_flipped_view_and_shift_steps_fifteen_degrees() {
    let probe = Point::new(700.0, 480.0);
    // The screen angle of the probe about the view's centre, in degrees.
    let angle = |view: &CanvasView| {
        let v = view.to_screen(probe) - view.rect.center();
        f64::from(v.y).atan2(f64::from(v.x)).to_degrees()
    };
    for flip_x in [false, true] {
        let start = move |room: egui::Rect| {
            let mut view = CanvasView::new(room, Point::new(600.0, 450.0), 0.5);
            view.rotation = 20.0;
            view.flip_x = flip_x;
            view
        };
        let mut rig = Rig::new(RotateViewTool::default(), start);
        let (before, c) = (rig.view(), rig.centre());
        let (from, to) = (c + egui::vec2(100.0, 0.0), c + egui::vec2(100.0 * 30f32.to_radians().cos(), 100.0 * 30f32.to_radians().sin()));
        rig.press(from);
        rig.drag_to(from, to, 6, FRAME);
        rig.release(to, FRAME);
        let turned_by = (angle(&rig.view()) - angle(&before) + 540.0).rem_euclid(360.0) - 180.0;
        assert!((turned_by - 30.0).abs() < 0.01, "flip {flip_x}: a 30 degree drag turns the image 30 degrees the pointer's way, got {turned_by}");
    }

    let mut rig = Rig::new(RotateViewTool::default(), |room| CanvasView::new(room, Point::new(600.0, 450.0), 0.5));
    let c = rig.centre();
    let (from, to) = (c + egui::vec2(100.0, 0.0), c + egui::vec2(100.0 * 37f32.to_radians().cos(), 100.0 * 37f32.to_radians().sin()));
    rig.fixture.hold(Modifiers::SHIFT);
    rig.press(from);
    rig.drag_to(from, to, 6, FRAME);
    rig.release(to, FRAME);
    let rotation = rig.view().rotation;
    assert!(rotation != 0.0, "the drag turned the view");
    assert!((rotation - (rotation / 15.0).round() * 15.0).abs() < 1e-6, "Shift lands the angle on 15 degree steps, got {rotation}");
}

#[trial]
fn a_flicked_hand_release_glides_and_stops_only_when_fast() {
    // Drags 8 passes of `step` points, waits `pause` seconds, releases; returns whether the view kept moving after the
    // release and whether it came to rest.
    let run = |flick: bool, step: f32, pause: f64| {
        let mut rig = Rig::new(HandTool::new(options(|o| o.flick_panning = flick)), plain);
        let c = rig.centre();
        let to = c + egui::vec2(step * 8.0, 0.0);
        rig.press(c);
        rig.drag_to(c, to, 8, FRAME);
        if pause > 0.0 {
            rig.tick(pause, Vec::new());
        }
        rig.release(to, 0.0);
        let at = rig.view().center;
        for _ in 0..5 {
            rig.tick(FRAME, Vec::new());
        }
        let moved = rig.view().center != at;
        for _ in 0..40 {
            rig.tick(0.1, Vec::new());
        }
        let rest = rig.view().center;
        rig.tick(0.1, Vec::new());
        (moved, rig.view().center == rest)
    };
    assert_eq!(run(true, 12.0, 0.0), (true, true), "a fast release glides and decays to a stop");
    assert!(!run(true, 1.6, 0.0).0, "a slow release does not glide");
    assert!(!run(false, 12.0, 0.0).0, "with Flick Panning off a fast release does not glide");
    assert!(!run(true, 12.0, 0.3).0, "the velocity is that of the last moments: a pause before the release gives no glide");
}

#[trial]
fn a_held_zoom_press_zooms_about_its_point_at_a_frame_rate_independent_rate() {
    let held = |dt: f64, animated: bool| {
        let mut rig = Rig::new(ZoomTool::new(options(|o| o.animated_zoom = animated)), |room| turned(room, 0.0, false));
        let (before, at, device) = (rig.view(), rig.centre() + egui::vec2(80.0, 50.0), rig.device());
        rig.press(at);
        for _ in 0..(0.9 / dt).round() as usize {
            rig.tick(dt, Vec::new());
        }
        // The release comes in the pass of the last frame, so both rates end at the same time.
        rig.release(at, 0.0);
        (rig.device() / device, pinned(&before, &rig.view(), at))
    };
    let (slow, slow_pinned) = held(1.0 / 30.0, true);
    let (fast, fast_pinned) = held(1.0 / 120.0, true);
    assert!(slow_pinned && fast_pinned, "the hold zooms about the press point");
    assert!(slow > 1.0, "a hold zooms in: {slow}");
    assert!((slow / fast - 1.0).abs() < 0.03, "the same hold zooms the same at 30 and 120 passes a second: {slow} against {fast}");

    // Animated Zoom off: a hold is one step, as a click.
    let step = |animated: bool| {
        let mut rig = Rig::new(ZoomTool::new(options(|o| o.animated_zoom = animated)), |room| turned(room, 0.0, false));
        let device = rig.device();
        rig.click(rig.centre() + egui::vec2(80.0, 50.0));
        rig.device() / device
    };
    let (still, _) = held(1.0 / 30.0, false);
    assert!((still - step(false)).abs() < 1e-9, "with Animated Zoom off a hold gives one step: {still}");
    assert!(still < slow, "one step is less than a hold's zoom");
    assert!((step(true) - step(false)).abs() < 1e-9, "a quick click steps one rung with or without easing");

    // A drag within the delay scrubs.
    let mut rig = Rig::new(ZoomTool::new(NavigationOptions::default()), |room| turned(room, 0.0, false));
    let (at, device) = (rig.centre(), rig.device());
    rig.press(at);
    rig.drag_to(at, at + egui::vec2(100.0, 0.0), 4, FRAME);
    rig.release(at + egui::vec2(100.0, 0.0), FRAME);
    assert!((rig.device() / device - 2.0).abs() < 1e-3, "a drag scrubs one doubling per 100 points: {}", rig.device() / device);
}

#[trial]
fn space_then_shift_turns_the_view_only_where_the_app_asks_for_it() {
    let shift = Mods { shift: true, ..Mods::NONE };
    let cmd_shift = Mods { cmd: true, shift: true, ..Mods::NONE };
    let hand = |momentary: Momentary| {
        let mut momentary = momentary;
        momentary.fired(MomentaryCommands::Hand, FirePhase::Press);
        momentary
    };

    let mut plain = hand(Momentary::new(NavigationOptions::default()));
    assert_eq!(plain.tool(shift, false), Some(MomentaryTool::Hand), "by default Space+Shift pans");

    let mut turning = hand(Momentary::new(NavigationOptions::default()).shift(MomentaryShift::RotateView));
    assert_eq!(turning.tool(Mods::NONE, false), Some(MomentaryTool::Hand), "Space alone pans");
    assert_eq!(turning.tool(shift, false), Some(MomentaryTool::RotateView), "Space then Shift turns the view");
    assert_eq!(turning.tool(Mods::NONE, true), Some(MomentaryTool::RotateView), "a drag keeps its tool when Shift is let go");
    assert_eq!(turning.tool(cmd_shift, false), Some(MomentaryTool::ZoomIn), "Cmd is checked before Shift");
    assert_eq!(turning.tool(Mods { alt: true, shift: true, ..Mods::NONE }, false), Some(MomentaryTool::ZoomOut), "Alt is checked before Shift");

    let mut bound = Momentary::new(NavigationOptions::default());
    bound.fired(MomentaryCommands::RotateView, FirePhase::Press);
    assert_eq!(bound.tool(Mods::NONE, false), Some(MomentaryTool::RotateView), "a bound Rotate View command turns the view");
    assert_eq!(bound.tool(Mods::ALT, false), Some(MomentaryTool::RotateView), "the held command comes first");
}

/// A canvas asks the app for the layers' bounds only in the pass that runs Fit Layers: passes that only show the
/// command enabled never walk the layers.
#[trial]
fn a_canvas_asks_for_the_layer_bounds_only_when_fit_layers_runs() {
    let size = Size::new(4000.0, 3000.0);
    let layers = Rect::new(100.0, 2000.0, 300.0, 2900.0);
    let unshown = CanvasView::new(egui::Rect::NOTHING, Point::new(2000.0, 1500.0), 1.0);
    let (asked, view) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(unshown)));
    let (shown_asked, shown_view) = (Rc::clone(&asked), Rc::clone(&view));
    let mut fixture = Fixture::new(move |ui| {
        let bounds = || {
            shown_asked.set(shown_asked.get() + 1);
            Some(layers)
        };
        let mut held = shown_view.get();
        Canvas::new("Document").document(size).layer_bounds(&bounds).view(Bind::bound(&mut held)).show(ui, |_, _| {});
        shown_view.set(held);
    });
    fixture.settle();
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas).settle();
    assert_eq!(asked.get(), 0, "showing the canvas and its enabled commands asks for no bounds");
    let before = view.get();
    assert!(commands::fire(fixture.ctx(), ViewCommands::FitLayers, FireSource::Menu), "a menu row fires Fit Layers");
    fixture.frame(vec![]);
    assert!(asked.get() > 0, "the pass that runs Fit Layers asks for the bounds");
    assert_ne!(view.get(), before, "and fits them");
    let once = asked.get();
    fixture.idle(3);
    assert_eq!(asked.get(), once, "later passes ask no more");
}

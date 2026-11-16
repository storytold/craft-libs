//! Navigation rules (wheel, page keys, flick, autoscroll), the zoom motion and the momentary tool order.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CanvasView, GESTURE_GAP, MotionMode, Wheel, WheelPhase, WheelUnit};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{
    Autoscroll, Canvas, CanvasSpace, Guide, GuideKey, Momentary, MomentaryCommands, MomentaryTool, NavigationOptions, TouchpadGlide, ViewCommands,
    WheelNavigation,
};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{self, CommandRef, FirePhase, FireSource};
use craft_ui::runtime::frame;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers, Pos2, TouchPhase, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[trial]
fn a_touchpad_glide_stops_in_the_pass_the_edge_clamp_leaves_the_view_where_it_was() {
    const STEP: f64 = 0.016;
    let mut glide = TouchpadGlide::new(true);
    let mut now = 0.0;
    glide.read(&Wheel::new(Vec2::new(30.0, 0.0), Mods::NONE, WheelUnit::Point, WheelPhase::Start), now, true);
    for _ in 0..4 {
        now += STEP;
        glide.read(&Wheel::new(Vec2::new(30.0, 0.0), Mods::NONE, WheelUnit::Point, WheelPhase::Move), now, true);
    }
    glide.read(&Wheel::new(Vec2::ZERO, Mods::NONE, WheelUnit::Point, WheelPhase::End), now, true);
    now += STEP;
    assert!(glide.step(now).x > 0.0, "a fast lift glides");
    glide.moved(true);
    now += STEP;
    assert!(glide.step(now).x > 0.0, "a glide whose view moved goes on");
    glide.moved(false);
    now += STEP;
    assert_eq!(glide.step(now), Vec2::ZERO, "a pass that left the view where it was ends the glide");
}

#[trial]
fn space_is_the_hand_and_cmd_or_alt_added_in_any_order_switch_to_zoom() {
    let mut momentary = Momentary::new(NavigationOptions::default());
    assert_eq!(momentary.tool(Mods::NONE, false), None);
    momentary.fired(MomentaryCommands::Hand, FirePhase::Press);
    assert_eq!(momentary.tool(Mods::NONE, false), Some(MomentaryTool::Hand));
    assert_eq!(momentary.tool(Mods::CMD, false), Some(MomentaryTool::ZoomIn));
    assert_eq!(momentary.tool(Mods::ALT, false), Some(MomentaryTool::ZoomOut));
    momentary.fired(MomentaryCommands::ZoomIn, FirePhase::Press);
    assert_eq!(momentary.tool(Mods::CMD, false), Some(MomentaryTool::ZoomIn));
    let both = Mods { cmd: true, alt: true, ..Mods::NONE };
    assert_eq!(momentary.tool(both, false), Some(MomentaryTool::ZoomOut), "Alt added to a held Zoom In zooms out");
    assert_eq!(momentary.tool(Mods::NONE, true), Some(MomentaryTool::ZoomOut), "a drag keeps the tool it began with");
    momentary.fired(MomentaryCommands::ZoomIn, FirePhase::Release);
    assert_eq!(momentary.tool(Mods::NONE, false), None);
}

#[trial]
fn cmd_or_ctrl_with_page_down_scrolls_a_focused_canvas_sideways_on_every_os() {
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
        let shown = Rc::clone(&view);
        let mut fixture = Fixture::new(move |ui| {
            let mut given = shown.get();
            given.rect = ui.available_rect_before_wrap();
            let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).view(given).show(ui, |_, _| {});
            shown.set(canvas.output.view.unwrap_or(given));
        });
        fixture.os(os);
        fixture.settle();
        let canvas = fixture.by_role(Role::CANVAS)[0];
        fixture.click(&canvas);
        let before = view.get().center;
        // `COMMAND` is Cmd on macOS and Ctrl elsewhere.
        fixture.key(Key::PageDown, Modifiers::COMMAND);
        let sideways = view.get().center;
        assert!(sideways.x > before.x && sideways.y == before.y, "{os:?}: {before:?} -> {sideways:?}");
        fixture.key(Key::PageDown, Modifiers::NONE);
        let down = view.get().center;
        assert!(down.y > sideways.y && down.x == sideways.x, "{os:?}: plain Page Down scrolls down");
    }
}

#[trial]
fn cmd_or_ctrl_with_page_down_on_a_focused_canvas_control_scrolls_the_canvas_sideways() {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    // A horizontal guide through the document's centre, where the view starts: it stays in view as the view moves
    // sideways. Its use sets no document canvas key.
    let mut guides = vec![Guide::new(GuideKey::new(1), Axis::X, 2000.0)];
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = ui.scope_id().with("document canvas");
        let canvas = Canvas::new("Document")
            .id(canvas)
            .document(Size::new(4000.0, 4000.0))
            .view(given)
            .show(ui, |ui, body| Guides::new("Guides", canvas, Bind::bound(&mut guides)).interactive(true).show_at(ui, body.rect));
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.settle();
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    let guide = fixture.focused_stop();
    assert!(guide.is_some_and(|(id, part)| Some(id) != fixture.root_id() && part.is_some()), "the click focuses the guide, not the canvas");
    assert!(fixture.focused_role().is_some_and(|role| role.document), "a stop in the body of a canvas with a view is on a document");
    let before = view.get().center;
    fixture.key(Key::PageDown, Modifiers::COMMAND).settle();
    let sideways = view.get().center;
    assert!(sideways.x > before.x && sideways.y == before.y, "the guide leaves Cmd/Ctrl+Page Down to the canvas: {before:?} -> {sideways:?}");
    assert_eq!(fixture.focused_stop(), guide, "focus stays on the guide");
}

/// The space a canvas gives its body for a view of `device` device pixels per document unit turned `rotation`
/// degrees, centred off the pixel grid, with the app's tool moving the view with the pointer or not.
fn space_at(device: f64, rotation: f64, follows_pointer: bool) -> craft_ui::canvas::CanvasSpace {
    let space = Rc::new(Cell::new(None));
    let seen = Rc::clone(&space);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let zoom = CanvasView::points_per_unit(device, ui.ctx().pixels_per_point());
        let view = CanvasView::new(room, Point::new(100.3, 77.7), zoom).rotation(rotation);
        let mut navigation = NavigationOptions::default();
        navigation.overscroll = true;
        let shown = Canvas::new("Document")
            .document(Size::new(400.0, 300.0))
            .navigation(navigation)
            .view_follows_pointer(follows_pointer)
            .view(view)
            .show(ui, |_, _| {});
        seen.set(shown.output.space);
    });
    fixture.settle();
    space.get().expect("a canvas given a view publishes its space")
}

#[trial]
fn at_a_whole_device_zoom_with_no_rotation_the_shown_view_puts_document_pixels_on_device_pixels_unless_it_follows_the_pointer() {
    for device in [2.0, 12.0] {
        let space = space_at(device, 0.0, false);
        let ppp = space.pixels_per_point;
        for doc in [Point::ZERO, Point::new(17.0, 29.0)] {
            let p = space.view.to_screen(doc) * ppp;
            assert!((p.x - p.x.round()).abs() < 1e-3 && (p.y - p.y.round()).abs() < 1e-3, "{device}x: {doc:?} at {p:?}");
        }
        assert_eq!(space.target().center, Point::new(100.3, 77.7), "the view the app keeps is not moved");
        assert!(!space.easing(), "a settled view is not easing");
        let moved = (space.view.to_screen(Point::ZERO) - space.target().to_screen(Point::ZERO)) * ppp;
        assert!(moved.x.abs() <= 0.501 && moved.y.abs() <= 0.501, "{device}x: moved {moved:?}");
    }
    let turned = space_at(2.0, 30.0, false);
    assert_eq!(turned.view.center, turned.target().center, "a turned view is shown as it is");
    let following = space_at(2.0, 0.0, true);
    assert_eq!(following.view.center, following.target().center, "a view moving with the pointer is shown as it is");

    // A view the tool host sets in the pass is the target, shown by the same rule.
    let mut set = following;
    let mut moved = following.target();
    moved.center = Point::new(140.6, 90.2);
    set.set_view(moved);
    assert_eq!((set.view, set.target()), (moved, moved), "a view set while following the pointer is shown as it is");
    let mut set = space_at(2.0, 0.0, false);
    let mut moved = set.target();
    moved.center = Point::new(140.6, 90.2);
    set.set_view(moved);
    assert_eq!(set.target(), moved, "the target follows the view set");
    let p = set.view.to_screen(Point::new(17.0, 29.0)) * set.pixels_per_point;
    assert!((p.x - p.x.round()).abs() < 1e-3 && (p.y - p.y.round()).abs() < 1e-3, "a view set at a whole zoom is shown aligned: {p:?}");
}

/// A case of the wheel table: its name, navigation, the events it sends, whether it reads as a pan, and the zoom it ends at.
type NavCase<'a> = (&'a str, NavigationOptions, &'a dyn Fn(Pos2) -> Vec<Vec<egui::Event>>, bool, f64);

type WheelRig = (Fixture, Rc<Cell<CanvasView>>, Rc<Cell<Option<f64>>>, Rc<Cell<egui::Rect>>);

/// A canvas over a large document at zoom 1 below a framework number field and an app's own text field, on `os`: the
/// fixture, the view it shows, the number and where the app's field is.
fn wheel_rig(os: Platform) -> WheelRig {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let number = Rc::new(Cell::new(Some(10.0)));
    let app_field = Rc::new(Cell::new(egui::Rect::NOTHING));
    let (shown, value, field_at) = (Rc::clone(&view), Rc::clone(&number), Rc::clone(&app_field));
    let mut text = String::from("Layer 1");
    let mut fixture = Fixture::new(move |ui| {
        let mut current = value.get();
        NumberField::new("Smoothing", Bind::bound(&mut current)).show(ui);
        value.set(current);
        field_at.set(ui.add(egui::TextEdit::singleline(&mut text)).rect);
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.os(os);
    // As an app that gives ⌘/Ctrl + wheel to the canvas sets egui: no wheel modifier is egui's own zoom.
    fixture.ctx().options_mut(|options| options.input_options.zoom_modifier = Modifiers::NONE);
    fixture.settle();
    (fixture, view, number, app_field)
}

/// One wheel line of `dy` with `modifiers` over the middle of the canvas.
fn wheel_line(fixture: &mut Fixture, view: &Cell<CanvasView>, dy: f32, modifiers: Modifiers) {
    let at = view.get().rect.center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: egui::MouseWheelSource::Wheel,
    }]);
}

#[trial]
fn a_wheel_line_pans_the_canvas_photoshops_line_and_ctrl_alt_zooms_as_alt_does() {
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let (mut fixture, view, _, _) = wheel_rig(os);
        let before = view.get();
        wheel_line(&mut fixture, &view, -1.0, Modifiers::NONE);
        let panned = view.get();
        assert!((panned.center.y - before.center.y - f64::from(WheelNavigation::LINE)).abs() < 1e-3, "{os:?}: {before:?} -> {panned:?}");
        wheel_line(&mut fixture, &view, 1.0, Modifiers::ALT);
        let alt = view.get().zoom / panned.zoom;
        assert!((alt - WheelNavigation::NOTCH).abs() < 1e-3, "{os:?}: Alt + a line zooms a notch: {alt}");
        // egui reports Ctrl as `ctrl` with `command` off macOS, and the Control key as `ctrl` alone on macOS.
        let ctrl_alt = if os.is_mac() {
            Modifiers { alt: true, ctrl: true, ..Modifiers::NONE }
        } else {
            Modifiers { alt: true, ctrl: true, command: true, ..Modifiers::NONE }
        };
        let at = view.get();
        wheel_line(&mut fixture, &view, 1.0, ctrl_alt);
        assert!((view.get().zoom / at.zoom - WheelNavigation::NOTCH).abs() < 1e-3, "{os:?}: Ctrl + Alt zooms as Alt does");
        if os.is_mac() {
            let at = view.get();
            wheel_line(&mut fixture, &view, 1.0, Modifiers { alt: true, mac_cmd: true, command: true, ..Modifiers::NONE });
            assert_eq!(view.get().zoom, at.zoom, "⌘ + ⌥ is no zoom");
        }
    }
}

#[trial]
fn a_held_alt_key_is_in_the_wheels_modifiers_when_the_wheel_events_miss_it() {
    // Windows can report a held Alt in its key events and not in the wheel's modifier state.
    let alt_key = |pressed: bool, repeat: bool| egui::Event::Key {
        key: Key::AltLeft,
        physical_key: Some(Key::AltLeft),
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    };
    let (mut fixture, view, _, _) = wheel_rig(Platform::Windows);
    fixture.frame(vec![alt_key(true, false)]);
    let before = view.get();
    wheel_line(&mut fixture, &view, 1.0, Modifiers::NONE);
    assert!((view.get().zoom / before.zoom - WheelNavigation::NOTCH).abs() < 1e-3, "Alt held from its press: the wheel zooms");
    // A pass whose first Alt event is a repeat: Alt was already down.
    fixture.frame(vec![alt_key(true, true)]);
    let held = view.get();
    wheel_line(&mut fixture, &view, 1.0, Modifiers::NONE);
    assert!(view.get().zoom > held.zoom, "Alt still held: the wheel zooms");
    fixture.frame(vec![alt_key(false, false)]);
    let released = view.get();
    wheel_line(&mut fixture, &view, -1.0, Modifiers::NONE);
    assert_eq!(view.get().zoom, released.zoom, "Alt released: a plain wheel line does not zoom");
    assert_ne!(view.get().center, released.center, "it pans");
}

#[trial]
fn a_focused_field_takes_the_plain_wheel_over_the_canvas_and_leaves_it_the_view_keys() {
    let (mut fixture, view, number, _) = wheel_rig(Platform::Linux);
    let field = fixture.by_label("Smoothing").expect("the number field");
    fixture.click(&field);
    let before = view.get();
    wheel_line(&mut fixture, &view, 1.0, Modifiers::NONE);
    assert_eq!(view.get(), before, "the canvas neither pans nor zooms");
    assert_eq!(number.get(), Some(11.0), "the field stepped");
    wheel_line(&mut fixture, &view, 1.0, Modifiers::ALT);
    assert!(view.get().zoom > before.zoom, "Alt + wheel is still the view's zoom");
    let zoomed = view.get();
    wheel_line(&mut fixture, &view, 1.0, Modifiers::COMMAND);
    assert!(view.get().center.x != zoomed.center.x, "Cmd/Ctrl + wheel still scrolls the view sideways");
    assert_eq!(number.get(), Some(11.0), "the view's wheel does not step the field");

    let (mut fixture, view, _, app_field) = wheel_rig(Platform::Linux);
    fixture.click_at(app_field.get().center());
    let before = view.get();
    wheel_line(&mut fixture, &view, 1.0, Modifiers::NONE);
    assert_eq!(view.get(), before, "an app's own focused text field keeps the plain wheel from the canvas");
    wheel_line(&mut fixture, &view, 1.0, Modifiers::ALT);
    assert!(view.get().zoom > before.zoom, "Alt + wheel is still the view's zoom");
}

#[trial]
fn the_view_commands_a_canvas_runs_are_in_the_frame_output_whatever_fired_them() {
    let (mut fixture, view, _, _) = wheel_rig(Platform::Linux);
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    frame::take_output(fixture.ctx());
    let before = view.get();
    fixture.key(Key::Num0, Modifiers::COMMAND);
    assert!(commands::fire(fixture.ctx(), ViewCommands::ZoomIn, FireSource::Menu), "a menu row fires the canvas's command");
    fixture.frame(vec![]);
    assert_ne!(view.get(), before, "the canvas ran them");
    let delivered: Vec<_> = frame::take_output(fixture.ctx()).delivered.into_iter().map(|fired| (fired.command, fired.source)).collect();
    assert_eq!(
        delivered,
        [(CommandRef::from(ViewCommands::FitOnScreen), FireSource::Key), (CommandRef::from(ViewCommands::ZoomIn), FireSource::Menu)]
    );
}

#[trial]
fn a_wheel_zoom_keeps_the_point_under_the_pointer_where_an_app_widget_covers_the_body() {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).view(given).show(ui, |ui, body| {
            // An app's own tool widget over the whole body, as an app that runs its tools itself places one.
            ui.interact(body.rect, egui::Id::unique("app tools"), egui::Sense::click_and_drag());
        });
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.settle();
    let before = view.get();
    let at = before.rect.center() + Vec2::new(-120.0, 80.0);
    let under = before.to_doc(at);
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::ALT,
        source: egui::MouseWheelSource::Wheel,
    }]);
    let after = view.get();
    assert!(after.zoom > before.zoom, "Alt + wheel zooms");
    assert!(after.to_screen(under).distance(at) < 0.5, "the document point stays under the pointer: {before:?} -> {after:?}");
}

#[trial]
fn a_canvas_in_a_sizing_pass_of_its_parent_shows_nothing_and_reports_an_idle_press() {
    // An egui window lays its contents out in a sizing pass in its first frame, as a document window in the app's
    // main window does.
    let idle = Rc::new(Cell::new(None));
    let seen = Rc::clone(&idle);
    let mut fixture = Fixture::new(move |ui| {
        ui.scope_builder(egui::UiBuilder::new().sizing_pass(), |ui| {
            let given = CanvasView::new(ui.available_rect_before_wrap(), Point::new(50.0, 50.0), 1.0);
            let canvas = Canvas::new("Document").document(Size::new(100.0, 100.0)).view(given).show(ui, |_, _| {});
            seen.set(Some(canvas.output.press.phase.start().is_none()));
        });
    });
    fixture.settle();
    assert_eq!(idle.get(), Some(true), "the canvas ran and its press is idle");
}

/// Navigation options with overscroll on, so no edge clamp moves the view the trials measure.
fn navigation(pinch_zooms: bool, touch_gestures: bool, trackpad_rotate: bool) -> NavigationOptions {
    let mut options = NavigationOptions::default();
    options.pinch_zooms = pinch_zooms;
    options.touch_gestures = touch_gestures;
    options.trackpad_rotate = trackpad_rotate;
    options.overscroll = true;
    options
}

/// Two fingers of one touch screen in `phase`, at `center` ∓ `half`.
fn fingers(phase: TouchPhase, center: Pos2, half: Vec2) -> Vec<egui::Event> {
    [(0, center - half), (1, center + half)]
        .map(|(id, pos)| egui::Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(id), phase, pos, force: None })
        .into()
}

/// The zoom ratio and the rotation of the view a canvas shows after the frames of `input` over its middle, under
/// `options`, from a view turned `rotation` degrees and mirrored when `flip`.
fn gestured(options: NavigationOptions, rotation: f64, flip: bool, input: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>>) -> (f64, f64) {
    // A view with a rect is shown as given: no first fit.
    let start = CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0)), Point::new(2000.0, 2000.0), 1.0);
    let view = Rc::new(Cell::new(start.rotation(rotation).flip_x(flip)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.settle();
    let middle = view.get().rect.center();
    fixture.frame(vec![pointer::moved(middle)]);
    let before = view.get();
    for events in input(middle) {
        fixture.frame(events);
    }
    let after = view.get();
    (after.zoom / before.zoom, after.rotation)
}

#[trial]
fn pinch_and_rotation_follow_the_gesture_options_and_the_flip() {
    const TURN: f32 = 0.1;
    let degrees = f64::from(TURN).to_degrees();
    let pinch: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>> = &|_| vec![vec![egui::Event::Zoom(1.2)]];
    let rotate: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>> = &|_| vec![vec![egui::Event::Rotate(TURN)]];
    let spread: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>> = &|at| {
        vec![
            fingers(TouchPhase::Start, at, Vec2::new(50.0, 0.0)),
            fingers(TouchPhase::Move, at, Vec2::new(75.0, 0.0)),
            fingers(TouchPhase::End, at, Vec2::new(75.0, 0.0)),
        ]
    };
    let twist: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>> = &|at| {
        let turned = Vec2::new(TURN.cos(), TURN.sin()) * 50.0;
        vec![fingers(TouchPhase::Start, at, Vec2::new(50.0, 0.0)), fingers(TouchPhase::Move, at, turned), fingers(TouchPhase::End, at, turned)]
    };
    // (case, options (pinch, gestures, rotate), input, zooms, degrees turned)
    let cases: [NavCase<'_>; 12] = [
        ("the trackpad's pinch", navigation(true, true, false), pinch, true, 0.0),
        ("the fingers' pinch", navigation(true, true, false), spread, true, 0.0),
        ("the trackpad's pinch without Zoom with Trackpad Pinch", navigation(false, true, false), pinch, false, 0.0),
        ("the fingers' pinch without Zoom with Trackpad Pinch", navigation(false, true, false), spread, false, 0.0),
        ("the fingers' pinch without Enable Gestures", navigation(true, false, false), spread, false, 0.0),
        ("the trackpad's pinch without Enable Gestures", navigation(true, false, false), pinch, true, 0.0),
        ("the trackpad's rotation without Rotate View with Trackpad", navigation(true, true, false), rotate, false, 0.0),
        ("the trackpad's rotation", navigation(true, true, true), rotate, false, degrees),
        ("the trackpad's rotation without Enable Gestures", navigation(true, false, true), rotate, false, degrees),
        ("the fingers' twist", navigation(true, true, true), twist, false, degrees),
        ("the fingers' twist without Enable Gestures", navigation(true, false, true), twist, false, 0.0),
        ("the fingers' twist without Rotate View with Trackpad", navigation(true, true, false), twist, false, 0.0),
    ];
    for (case, options, input, zooms, turned) in cases {
        let (ratio, rotation) = gestured(options, 0.0, false, input);
        if zooms {
            assert!(ratio > 1.01, "{case} zooms: {ratio}");
        } else {
            assert!((ratio - 1.0).abs() < 1e-4, "{case} does not zoom: {ratio}");
        }
        assert!((rotation - turned).abs() < 0.05, "{case} turns the view {turned}°: {rotation}");
    }

    let (_, flipped) = gestured(navigation(true, true, true), 0.0, true, rotate);
    assert!((flipped + degrees).abs() < 0.05, "a mirrored view turns the other way: {flipped}");

    let five: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>> = &|_| vec![vec![egui::Event::Rotate(5.0_f32.to_radians())]];
    let (_, wrapped) = gestured(navigation(true, true, true), 178.0, false, five);
    assert!(wrapped > -180.0 && wrapped <= 180.0 && (wrapped + 177.0).abs() < 0.05, "178° turned 5° more is -177°: {wrapped}");
}

#[trial]
fn fractional_shift_alt_scrolls_add_up_to_whole_rungs_per_canvas() {
    let views = Rc::new([
        Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(500.0, 500.0), 1.0)),
        Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(500.0, 500.0), 1.0)),
    ]);
    let shown = Rc::clone(&views);
    let mut fixture = Fixture::new(move |ui| {
        for (left, label, view) in [(0.0_f32, "Left", &shown[0]), (200.0, "Right", &shown[1])] {
            craft_ui::testing::layout::in_region(ui, egui::Rect::from_min_size(Pos2::new(left, 0.0), Vec2::new(200.0, 300.0)), |ui| {
                let mut given = view.get();
                given.rect = ui.available_rect_before_wrap();
                let canvas = Canvas::new(label).document(Size::new(1000.0, 1000.0)).view(given).show(ui, |_, _| {});
                view.set(canvas.output.view.unwrap_or(given));
            });
        }
    });
    fixture.settle();
    let shift_alt = Modifiers { shift: true, alt: true, ..Modifiers::NONE };
    // A trackpad's part of a wheel line with Shift and Alt over one canvas: the zoom ratio it made.
    let scroll = |fixture: &mut Fixture, view: &Cell<CanvasView>| {
        let before = view.get().zoom;
        wheel_line(fixture, view, 0.4, shift_alt);
        view.get().zoom / before
    };
    let (left, right) = (&views[0], &views[1]);
    assert_eq!(scroll(&mut fixture, left), 1.0, "0.4 of a notch steps nothing");
    assert_eq!(scroll(&mut fixture, left), 1.0, "0.8 of a notch steps nothing");
    // A new gesture, so the right canvas takes the wheel under the pointer.
    fixture.advance(GESTURE_GAP);
    assert_eq!(scroll(&mut fixture, right), 1.0, "the other canvas keeps its own remainder");
    fixture.advance(GESTURE_GAP);
    assert!(scroll(&mut fixture, left) > 1.0, "1.2 notches step one rung");
    assert_eq!(scroll(&mut fixture, left), 1.0, "the step keeps what was left over: 0.6");
    fixture.advance(GESTURE_GAP);
    assert_eq!(scroll(&mut fixture, right), 1.0, "0.8 on the other canvas");
    assert!(scroll(&mut fixture, right) > 1.0, "1.2 on the other canvas steps it");
}

/// What the zoom-ease trial's app gives its canvas each pass, and the space the canvas showed.
struct EaseApp {
    view: CanvasView,
    ease: Option<(CanvasView, Option<(Point, Pos2)>)>,
    jumped: bool,
    options: NavigationOptions,
    space: Option<CanvasSpace>,
}

impl EaseApp {
    /// The app's zoom step by `factor` about document point `doc`, which stays where it is on screen, eased from the
    /// view it showed. Returns where `doc` stays.
    fn zoom_step(app: &RefCell<EaseApp>, doc: Point, factor: f64) -> Pos2 {
        let mut app = app.borrow_mut();
        let before = app.view;
        let screen = before.to_screen(doc);
        let mut next = before;
        next.zoom *= factor;
        next.center = doc - next.unmap_vec(screen - next.rect.center());
        app.view = next;
        app.ease = Some((before, Some((doc, screen))));
        screen
    }

    fn space(app: &RefCell<EaseApp>) -> CanvasSpace {
        app.borrow().space.expect("a canvas given a view publishes its space")
    }
}

/// The space shows its target: no ease runs and the view on screen is the target, up to the pixel alignment.
fn at_target(space: &CanvasSpace) -> bool {
    let (view, target) = (space.view, space.target());
    !space.easing() && (view.zoom - target.zoom).abs() < 1e-9 && (view.center - target.center).hypot() < 1.0
}

#[trial]
fn a_zoom_ease_keeps_its_anchor_snaps_when_animation_turns_off_and_stops_on_a_wheel_or_pan() {
    let app = Rc::new(RefCell::new(EaseApp {
        view: CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0),
        ease: None,
        jumped: false,
        options: navigation(true, true, false),
        space: None,
    }));
    let shared = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shared.borrow_mut();
        let mut given = app.view;
        given.rect = ui.available_rect_before_wrap();
        let mut canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(app.options).view(given);
        if let Some((from, anchor)) = app.ease.take() {
            canvas = canvas.ease_from(from, anchor);
        }
        if std::mem::take(&mut app.jumped) {
            canvas = canvas.jumped();
        }
        let shown = canvas.show(ui, |_, _| {});
        app.view = shown.output.view.unwrap_or(given);
        app.space = shown.output.space;
    });
    fixture.motion(MotionMode::Full);
    fixture.settle();
    let canvas = fixture.by_role(Role::CANVAS)[0];
    // The canvas holds focus, so a menu row's view command reaches it.
    fixture.click(&canvas);
    let at = app.borrow().view.rect.center() + Vec2::new(40.0, 30.0);
    fixture.frame(vec![pointer::moved(at)]);
    let doc = app.borrow().view.to_doc(app.borrow().view.rect.center() + Vec2::new(-120.0, 80.0));

    // An anchored step keeps its document point under its screen point all the way.
    let screen = EaseApp::zoom_step(&app, doc, 2.0);
    let mut eased = 0;
    loop {
        fixture.frame(vec![]);
        let space = EaseApp::space(&app);
        assert!(space.view.to_screen(doc).distance(screen) < 0.5, "frame {eased} of the ease: {:?}", space.view);
        if !space.easing() {
            break;
        }
        eased += 1;
        assert!(eased < 120, "the ease ends");
    }
    assert!(eased > 1, "the step eased over several frames");
    assert!(at_target(&EaseApp::space(&app)), "the ease ends at the target");

    // With Animated Zoom off the step jumps.
    app.borrow_mut().options.animated_zoom = false;
    EaseApp::zoom_step(&app, doc, 0.5);
    fixture.frame(vec![]);
    assert!(at_target(&EaseApp::space(&app)), "with Animated Zoom off the step jumps");
    app.borrow_mut().options.animated_zoom = true;

    EaseApp::zoom_step(&app, doc, 2.0);
    fixture.frame(vec![]);
    assert!(EaseApp::space(&app).easing(), "the step eases");
    app.borrow_mut().options.animated_zoom = false;
    fixture.frame(vec![]);
    assert!(at_target(&EaseApp::space(&app)), "turning Animated Zoom off mid-ease shows the target at once");
    app.borrow_mut().options.animated_zoom = true;
    fixture.frame(vec![]);
    assert!(at_target(&EaseApp::space(&app)), "turning it back on does not resume the ease");

    // A wheel, a pinch and a pan take over from an ease; a Zoom tool's scrub reaches the canvas as the tool host's
    // stop, as the app's own jump does.
    let wheel = |dy: f32, modifiers: Modifiers| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: egui::MouseWheelSource::Wheel,
    };
    let interruptions = [
        ("an Alt+wheel zoom", vec![wheel(1.0, Modifiers::ALT)], false),
        ("a trackpad pinch", vec![egui::Event::Zoom(1.2)], false),
        ("a wheel pan", vec![wheel(-1.0, Modifiers::NONE)], false),
        ("the app's own jump", vec![], true),
    ];
    for (factor, (what, events, jumped)) in [0.5, 2.0, 0.5, 2.0].into_iter().zip(interruptions) {
        EaseApp::zoom_step(&app, doc, factor);
        fixture.frame(vec![]);
        assert!(EaseApp::space(&app).easing(), "{what}: the step eases");
        app.borrow_mut().jumped = jumped;
        fixture.frame(events);
        assert!(at_target(&EaseApp::space(&app)), "{what} stops the ease at the view it leaves");
    }

    // A view command retargets a running ease from the view in between, about the centre, without the old anchor.
    let screen = EaseApp::zoom_step(&app, doc, 0.5);
    fixture.frame(vec![]);
    let before = EaseApp::space(&app);
    assert!(before.easing(), "the step eases");
    assert!(commands::fire(fixture.ctx(), ViewCommands::ZoomIn, FireSource::Menu), "a menu row fires the canvas's Zoom In");
    fixture.frame(vec![]);
    let retargeted = EaseApp::space(&app);
    assert!(retargeted.easing(), "the command eases on");
    assert!(retargeted.target().zoom > before.target().zoom, "toward the command's zoom");
    assert!((retargeted.view.center - retargeted.target().center).hypot() < 1e-6, "about the centre the command keeps");
    assert!(retargeted.view.to_screen(doc).distance(screen) > 1.0, "the old anchor is dropped");
}

#[trial]
fn autoscroll_stops_at_the_shown_image_edge() {
    let rect = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
    let square = Size::new(1000.0, 1000.0);

    // At 2x the image's right edge is 40 points past the canvas's: a step of 100 moves 40 (20 document units).
    let mut zoomed = CanvasView::new(rect, Point::new(880.0, 500.0), 2.0);
    let moved = Autoscroll::scroll(&mut zoomed, Vec2::new(100.0, 0.0), square);
    assert!(moved.is_some_and(|d| (d.x - 20.0).abs() < 1e-6 && d.y == 0.0), "{moved:?}");
    assert!((zoomed.center.x - 900.0).abs() < 1e-6, "the image edge meets the canvas edge: {:?}", zoomed.center);
    assert_eq!(Autoscroll::scroll(&mut zoomed, Vec2::new(100.0, 0.0), square), None, "at the edge it stops");

    let mut fits = CanvasView::new(rect, Point::new(50.0, 50.0), 1.0);
    assert_eq!(Autoscroll::scroll(&mut fits, Vec2::new(100.0, 100.0), Size::new(100.0, 100.0)), None, "an image that fits never scrolls");

    // The image's right edge is already 180 points inside the canvas's.
    let mut past = CanvasView::new(rect, Point::new(990.0, 500.0), 2.0);
    assert_eq!(Autoscroll::scroll(&mut past, Vec2::new(50.0, 0.0), square), None, "a view past a limit is not pulled back");
    assert_eq!(past.center, Point::new(990.0, 500.0));

    // A quarter turn at 2x with aspect 2: the document's x runs down the screen at 4 points a unit, its bottom edge
    // 40 points below the canvas's.
    let mut turned = CanvasView::new(rect, Point::new(952.5, 250.0), 2.0).aspect(2.0).rotation(90.0);
    let tall = Size::new(1000.0, 500.0);
    let moved = Autoscroll::scroll(&mut turned, Vec2::new(0.0, 100.0), tall);
    assert!(moved.is_some_and(|d| (d.x - 10.0).abs() < 1e-6 && d.y.abs() < 1e-6), "the displayed edge stops it: {moved:?}");
    assert_eq!(Autoscroll::scroll(&mut turned, Vec2::new(0.0, 100.0), tall), None, "at the displayed edge it stops");

    // An oblique turn: the image is wider than the canvas and fits its height.
    let mut oblique = CanvasView::new(rect, Point::new(1000.0, 10.0), 0.5).rotation(10.0);
    let wide = Size::new(2000.0, 20.0);
    let centre = oblique.center;
    assert!(Autoscroll::scroll(&mut oblique, Vec2::new(100.0, 50.0), wide).is_some(), "the wide axis scrolls");
    let shift = oblique.to_screen(centre) - rect.center();
    assert!((shift.x + 100.0).abs() < 1e-3 && shift.y.abs() < 1e-3, "along the screen's x, and the fitting axis stays: {shift:?}");
}

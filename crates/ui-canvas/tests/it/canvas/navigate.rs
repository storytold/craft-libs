//! Navigation rules (wheel, page keys, flick, autoscroll), the zoom motion and the momentary tool order.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CanvasView, Wheel, WheelPhase, WheelUnit};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{
    Canvas, Guide, GuideKey, Momentary, MomentaryCommands, MomentaryTool, NavigationOptions, TouchpadGlide, ViewCommands, WheelNavigation,
};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{self, CommandRef, FirePhase, FireSource};
use craft_ui::runtime::frame;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers, Vec2};
use std::cell::Cell;
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
        let canvas = Canvas::new("Document")
            .document(Size::new(4000.0, 4000.0))
            .view(given)
            .show(ui, |ui, body| Guides::new("Guides", Bind::bound(&mut guides)).interactive(true).show_at(ui, body.rect));
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

/// A canvas over a large document at zoom 1 below a framework number field and an app's own text field, on `os`: the
/// fixture, the view it shows, the number and where the app's field is.
fn wheel_rig(os: Platform) -> (Fixture, Rc<Cell<CanvasView>>, Rc<Cell<Option<f64>>>, Rc<Cell<egui::Rect>>) {
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
            ui.interact(body.rect, egui::Id::new("app tools"), egui::Sense::click_and_drag());
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

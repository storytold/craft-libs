//! Navigation rules (wheel, page keys, flick, autoscroll), the zoom motion and the momentary tool order.

use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CancelReason, CanvasView, GESTURE_GAP, MotionMode, ZoomScheme};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{
    Autoscroll, Canvas, CanvasSpace, CanvasTool, Guide, GuideKey, MiddleAutoscrollFeel, MiddleButton, Momentary, MomentaryCommands, MomentaryTool,
    NavigationOptions, ToolAction, ToolContext, ToolHost, ToolPath, ToolPointer, ViewCommands, ViewRequest, WheelNavigation, WheelScheme, tool_pass,
};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{self, AppCatalog, Binding, CommandDef, CommandInput, CommandRef, CommandSet as _, FirePhase, FireSource, Scope};
use craft_ui::runtime::frame;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers, Pos2, TouchPhase, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A canvas over a large document with Touchpad Glide set to `touchpad_glide`, whose view starts 210 points short of
/// the document's right edge with the pointer over it, so a fast [`swipe`] (150 points) lifts 60 points short of it:
/// the fixture and the view it shows.
fn glide_rig(touchpad_glide: bool) -> (Fixture, Rc<Cell<CanvasView>>) {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut options = NavigationOptions::default();
    options.touchpad_glide = touchpad_glide;
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.settle();
    let mut near = view.get();
    near.center.x = 4000.0 - f64::from(near.rect.width()) / 2.0 - 210.0;
    view.set(near);
    fixture.frame(vec![pointer::moved(near.rect.center())]);
    (fixture, view)
}

/// A two-finger swipe that scrolls the view `per_frame` points a frame toward the document's right edge over five
/// frames, and lifts `pause` seconds after the last motion. The content follows the delta, so a leftward delta moves
/// the view right.
fn swipe(fixture: &mut Fixture, per_frame: f32, pause: f64) {
    let event = |phase: TouchPhase, dx: f32| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(-dx, 0.0),
        phase,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Trackpad,
    };
    fixture.frame(vec![event(TouchPhase::Start, per_frame)]);
    for _ in 0..4 {
        fixture.frame(vec![event(TouchPhase::Move, per_frame)]);
    }
    if pause > 0.0 {
        fixture.advance(pause);
    }
    fixture.frame(vec![event(TouchPhase::End, 0.0)]);
}

#[trial]
fn a_touchpad_glide_stops_in_the_pass_the_edge_clamp_leaves_the_view_where_it_was() {
    let (mut fixture, view) = glide_rig(true);
    let start = view.get();
    swipe(&mut fixture, 30.0, 0.0);
    let lifted = view.get().center.x;
    eprintln!(
        "DIAG a_touchpad_glide_stops_in_the_pass_the_edge_clamp_leaves_the_view_where_it_was: start x {} width {} zoom {}, lifted x {lifted}",
        start.center.x,
        start.rect.width(),
        start.zoom
    );
    fixture.frame(vec![]);
    let mut last = view.get().center.x;
    assert!(last > lifted, "a fast lift glides on over later passes: {lifted} -> {last}");
    let mut frames = 0;
    loop {
        fixture.frame(vec![]);
        let x = view.get().center.x;
        if x == last {
            break;
        }
        last = x;
        frames += 1;
        assert!(frames < 120, "the glide ends");
    }
    // Slowing alone would take about a hundred frames from this speed; the edge is 60 points away.
    assert!(frames < 30, "the edge stopped the glide, not its slowing: {frames} frames");
    // The pass in which the clamp left the view where it was ended the glide: a view moved back from the edge stays.
    let mut back = view.get();
    back.center.x -= 100.0;
    view.set(back);
    fixture.idle(2);
    assert_eq!(view.get().center.x, back.center.x, "the glide ended at the edge");

    for (case, glide, per_frame, pause) in
        [("a slow lift", true, 1.0, 0.0), ("a lift after a pause", true, 30.0, 0.2), ("Touchpad Glide off", false, 30.0, 0.0)]
    {
        let (mut fixture, view) = glide_rig(glide);
        swipe(&mut fixture, per_frame, pause);
        let lifted = view.get().center.x;
        fixture.idle(3);
        assert_eq!(view.get().center.x, lifted, "{case} does not glide");
    }
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

const NEXT_ARTBOARD: &str = "canvasTrial.nextArtboard";

/// A page key the app binds to a command runs it ahead of the canvas's page scroll, with nothing focused and with the
/// canvas focused (Page Down goes to the next artboard); an unbound page key still scrolls the view.
#[trial]
fn a_bound_page_key_runs_its_command_and_an_unbound_one_pages_the_canvas() {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    let binding = Binding::parse("PageDown").expect("the trial's chord parses");
    let catalog =
        AppCatalog::new(vec![CommandDef::new(NEXT_ARTBOARD, "Next Artboard", Scope::app()).with_default(binding)], 1).expect("the catalog is valid");
    fixture.commands(CommandInput::new().catalog(catalog));
    fixture.settle();
    let page = |fixture: &mut Fixture, key: Key| {
        frame::take_output(fixture.ctx());
        let before = view.get().center;
        fixture.key(key, Modifiers::NONE).settle();
        let fired: Vec<String> = frame::take_output(fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).collect();
        (fired, view.get().center != before)
    };
    assert_eq!(fixture.focused(), None, "nothing is focused");
    assert_eq!(page(&mut fixture, Key::PageDown), (vec![NEXT_ARTBOARD.to_owned()], false), "unfocused: the bound Page Down runs its command");
    assert_eq!(page(&mut fixture, Key::PageUp), (Vec::new(), true), "unfocused: the unbound Page Up scrolls");
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    assert!(fixture.focused().is_some(), "the click focuses the canvas");
    assert_eq!(page(&mut fixture, Key::PageDown), (vec![NEXT_ARTBOARD.to_owned()], false), "focused: the bound Page Down runs its command");
    assert_eq!(page(&mut fixture, Key::PageUp), (Vec::new(), true), "focused: the unbound Page Up scrolls");
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

/// A canvas over a large document at zoom 1 under `options` below a framework number field and an app's own text
/// field, on `os`: the fixture, the view it shows, the number and where the app's field is.
fn wheel_rig(os: Platform, options: NavigationOptions) -> WheelRig {
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
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.os(os);
    // As an app that gives ⌘/Ctrl + wheel to the canvas sets egui: no wheel modifier is egui's own zoom.
    fixture.ctx().options_mut(|options| options.input_options.zoom_modifier = Modifiers::NONE);
    fixture.settle();
    (fixture, view, number, app_field)
}

/// A mouse wheel's line of `dy` with `modifiers`.
fn line_event(dy: f32, modifiers: Modifiers) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: egui::MouseWheelSource::Wheel,
    }
}

/// One wheel line of `dy` with `modifiers` over the middle of the canvas.
fn wheel_line(fixture: &mut Fixture, view: &Cell<CanvasView>, dy: f32, modifiers: Modifiers) {
    let at = view.get().rect.center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![line_event(dy, modifiers)]);
}

#[trial]
fn a_wheel_line_pans_the_canvas_photoshops_line_and_ctrl_alt_zooms_as_alt_does() {
    let line = f64::from(NavigationOptions::default().wheel_line);
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let (mut fixture, view, _, _) = wheel_rig(os, NavigationOptions::default());
        let before = view.get();
        wheel_line(&mut fixture, &view, -1.0, Modifiers::NONE);
        let panned = view.get();
        assert!((panned.center.y - before.center.y - line).abs() < 1e-3, "{os:?}: {before:?} -> {panned:?}");
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
fn the_wheel_line_sets_the_scroll_but_not_the_zoom_per_notch_in_the_acrobat_scheme() {
    const LINE: f32 = 40.0;
    for scheme in [WheelScheme::Scroll, WheelScheme::Zoom, WheelScheme::Acrobat] {
        let mut options = NavigationOptions::default();
        options.wheel = scheme;
        options.wheel_line = LINE;
        let (mut fixture, view, _, _) = wheel_rig(Platform::Linux, options);
        // The modifiers of a scrolling and of a zooming line in the scheme, and the zoom of one line.
        let (scroll, zoom, notch) = match scheme {
            WheelScheme::Zoom => (Modifiers::ALT, Modifiers::NONE, WheelNavigation::NOTCH),
            WheelScheme::Acrobat => (Modifiers::NONE, Modifiers::COMMAND, WheelScheme::ACROBAT_NOTCH),
            _ => (Modifiers::NONE, Modifiers::ALT, WheelNavigation::NOTCH),
        };
        let before = view.get();
        wheel_line(&mut fixture, &view, -1.0, scroll);
        let panned = view.get();
        assert!(
            (panned.center.y - before.center.y - f64::from(LINE)).abs() < 1e-3,
            "{scheme:?}: a line scrolls the wheel line: {before:?} -> {panned:?}"
        );
        assert_eq!(panned.zoom, before.zoom, "{scheme:?}: a scrolling line does not zoom");
        wheel_line(&mut fixture, &view, 1.0, zoom);
        let ratio = view.get().zoom / panned.zoom;
        assert!((ratio - notch).abs() < 1e-3, "{scheme:?}: a line zooms one notch whatever the wheel line: {ratio}");
        let pinched = view.get();
        fixture.frame(vec![egui::Event::Zoom(1.2)]);
        assert!(view.get().zoom / pinched.zoom > 1.01, "{scheme:?}: a pinch zooms");
    }

    let mut options = NavigationOptions::default();
    options.wheel = WheelScheme::Acrobat;
    let (mut fixture, view, _, _) = wheel_rig(Platform::Linux, options);
    let before = view.get();
    wheel_line(&mut fixture, &view, -1.0, Modifiers::SHIFT);
    let sideways = view.get();
    assert!(sideways.center.x != before.center.x && sideways.center.y == before.center.y, "Shift scrolls sideways: {before:?} -> {sideways:?}");
    wheel_line(&mut fixture, &view, 1.0, Modifiers::ALT);
    assert_eq!(view.get().zoom, sideways.zoom, "Alt + wheel is no zoom in the Acrobat scheme");
}

type LatchRig = (Fixture, Rc<Cell<CanvasView>>, Rc<Cell<Option<egui::Id>>>);

/// A canvas over a large document whose middle button latches a scroll: the fixture, the view it shows and the
/// canvas's id.
fn latch_rig() -> LatchRig {
    let mut options = NavigationOptions::default();
    options.middle_button = MiddleButton::AutoScroll;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let canvas = Rc::new(Cell::new(None));
    let (shown, seen) = (Rc::clone(&view), Rc::clone(&canvas));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
        seen.set(Some(canvas.output.response.id));
    });
    fixture.settle();
    (fixture, view, canvas)
}

/// A latched middle scroll ends when the window loses focus, when the pointer leaves the window and when the canvas
/// is not shown for a pass (another tab), and does not come back with it; a press, a move and a release that arrive in
/// one frame latch it as a click does, measured from the press.
#[trial]
fn a_latched_middle_scroll_ends_on_blur_pointer_gone_and_a_hidden_canvas_and_latches_from_one_frame() {
    let mut options = NavigationOptions::default();
    options.middle_button = MiddleButton::AutoScroll;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let open = Rc::new(Cell::new(true));
    let (shown, visible) = (Rc::clone(&view), Rc::clone(&open));
    let mut fixture = Fixture::new(move |ui| {
        if !visible.get() {
            return;
        }
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.settle();
    let anchor = view.get().rect.center();
    let below = anchor + Vec2::new(0.0, 80.0);
    let latch = |fixture: &mut Fixture| {
        middle(fixture, anchor, true);
        middle(fixture, anchor, false);
        fixture.frame(vec![pointer::moved(below)]);
    };

    latch(&mut fixture);
    assert!(scrolls(&mut fixture, &view), "a click latches the scroll");
    fixture.frame(vec![pointer::window_blurred()]);
    assert!(!scrolls(&mut fixture, &view), "a window blur ends the latch");
    fixture.frame(vec![egui::Event::WindowFocused(true), pointer::moved(below)]);
    assert!(!scrolls(&mut fixture, &view), "the window's return does not bring it back");

    latch(&mut fixture);
    assert!(scrolls(&mut fixture, &view), "a click latches again");
    fixture.frame(vec![egui::Event::PointerGone]);
    assert!(!scrolls(&mut fixture, &view), "the pointer leaving the window ends the latch");

    let button = |pressed, at| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![pointer::moved(anchor), button(true, anchor), pointer::moved(below), button(false, below)]);
    assert!(scrolls(&mut fixture, &view), "a press, a move and a release in one frame latch the scroll");
    fixture.key(Key::Escape, Modifiers::NONE);

    latch(&mut fixture);
    assert!(scrolls(&mut fixture, &view), "a click latches again");
    open.set(false);
    fixture.idle(2);
    open.set(true);
    fixture.frame(vec![pointer::moved(below)]);
    assert!(!scrolls(&mut fixture, &view), "a canvas shown again does not resume the latch");
}

/// The middle button going down (`pressed`) or up at `at`, with the pointer there. The testing kit has no
/// middle-button helper.
fn middle(fixture: &mut Fixture, at: Pos2, pressed: bool) {
    let button = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![pointer::moved(at), button]);
}

/// Whether the view moves over five frames with no input.
fn scrolls(fixture: &mut Fixture, view: &Cell<CanvasView>) -> bool {
    let before = view.get().center;
    fixture.idle(5);
    view.get().center != before
}

#[trial]
fn middle_press_move_and_release_in_one_frame_latches_at_the_press() {
    let (mut fixture, view, _) = latch_rig();
    let at = view.get().rect.center();
    let below = at + Vec2::new(0.0, 80.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![pointer::moved(at), button(at, true), pointer::moved(below), button(below, false)]);
    assert!(scrolls(&mut fixture, &view), "the pointer remains below the initial anchor");
}

#[trial]
fn a_latched_middle_scroll_ends_on_escape_a_second_press_or_the_wheel_and_a_held_press_scrolls_until_its_release() {
    let (mut fixture, view, canvas) = latch_rig();
    let anchor = view.get().rect.center();
    let below = anchor + Vec2::new(0.0, 80.0);
    // A middle click latches the scroll; the pointer resting below the anchor, past the dead zone, scrolls the view.
    let latch = |fixture: &mut Fixture| {
        middle(fixture, anchor, true);
        middle(fixture, anchor, false);
        fixture.frame(vec![pointer::moved(below)]);
    };

    latch(&mut fixture);
    assert!(
        fixture.focused_stop().is_some_and(|(id, _)| Some(id) == canvas.get()),
        "the middle press focuses the canvas, which its Escape claim needs"
    );
    assert!(scrolls(&mut fixture, &view), "a latched scroll moves the view");
    fixture.key(Key::Escape, Modifiers::NONE);
    assert!(!scrolls(&mut fixture, &view), "Escape ends the latch");

    latch(&mut fixture);
    assert!(scrolls(&mut fixture, &view), "a click latches again");
    middle(&mut fixture, below, true);
    middle(&mut fixture, below, false);
    assert!(!scrolls(&mut fixture, &view), "a second press ends the latch");

    latch(&mut fixture);
    assert!(scrolls(&mut fixture, &view), "a click latches again");
    fixture.frame(vec![line_event(-1.0, Modifiers::NONE)]);
    assert!(!scrolls(&mut fixture, &view), "the wheel ends the latch");

    // A held press resting past the dead zone keeps scrolling with no pointer motion; its release ends the latch.
    middle(&mut fixture, anchor, true);
    fixture.frame(vec![pointer::moved(below)]);
    assert!(scrolls(&mut fixture, &view), "a held press resting past the dead zone scrolls with no pointer motion");
    middle(&mut fixture, below, false);
    assert!(!scrolls(&mut fixture, &view), "the release of a held press ends the latch");
}

/// The seconds of one fixture frame.
const FRAME: f64 = 1.0 / 60.0;

/// A canvas over a large document whose middle button autoscrolls with `feel`: the fixture and the view it shows.
fn feel_rig(feel: MiddleAutoscrollFeel) -> (Fixture, Rc<Cell<CanvasView>>) {
    let mut options = NavigationOptions::default();
    options.middle_button = MiddleButton::AutoScroll;
    options.middle_autoscroll = feel;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.settle();
    (fixture, view)
}

/// Under `MiddleAutoscrollFeel::Reader` (a document reader's autoscroll) a held press dragged past the drag threshold
/// but let go inside the dead zone keeps the run, where the default feel ends it; and the speed grows with the
/// pointer's whole distance from the mark, |d|^2.2 x 0.04 points per second.
#[trial]
fn a_reader_autoscroll_keeps_a_short_hold_and_speeds_up_with_the_whole_distance() {
    for feel in [MiddleAutoscrollFeel::Edge, MiddleAutoscrollFeel::Reader] {
        let (mut fixture, view) = feel_rig(feel);
        let anchor = view.get().rect.center();
        let near = anchor + Vec2::new(0.0, 12.0);
        middle(&mut fixture, anchor, true);
        fixture.frame(vec![pointer::moved(anchor + Vec2::new(0.0, 6.0))]);
        fixture.frame(vec![pointer::moved(near)]);
        middle(&mut fixture, near, false);
        fixture.frame(vec![pointer::moved(anchor + Vec2::new(0.0, 80.0))]);
        let reader = feel == MiddleAutoscrollFeel::Reader;
        assert_eq!(scrolls(&mut fixture, &view), reader, "{feel:?}: a release inside the dead zone keeps the run only for the reader");
        fixture.key(Key::Escape, Modifiers::NONE);
    }

    let (mut fixture, view) = feel_rig(MiddleAutoscrollFeel::Reader);
    let anchor = view.get().rect.center();
    middle(&mut fixture, anchor, true);
    middle(&mut fixture, anchor, false);
    fixture.frame(vec![pointer::moved(anchor + Vec2::new(0.0, 50.0))]);
    fixture.idle(2);
    let before = view.get().center.y;
    fixture.idle(60);
    let travelled = view.get().center.y - before;
    let expected = 50f64.powf(2.2) * 0.04 * 60.0 * FRAME;
    assert!((travelled - expected).abs() < 1.0, "a second 50 points below the mark scrolls {expected}, not {travelled}");
}

type EasedRig = (Fixture, Rc<Cell<CanvasView>>, Rc<Cell<bool>>);

/// A canvas over a large document with the eased wheel, whose app reruns the pass of a frame while the flag is set:
/// the fixture, the view it shows and the flag.
fn eased_rig() -> EasedRig {
    let mut options = NavigationOptions::default();
    options.eased_wheel = true;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let rerun = Rc::new(Cell::new(false));
    let (shown, again) = (Rc::clone(&view), Rc::clone(&rerun));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
        if again.replace(false) && frame::discardable(ui.ctx()) {
            frame::discard_pass(ui.ctx(), "the trial's second pass");
        }
    });
    fixture.settle();
    (fixture, view, rerun)
}

/// One frame of `events`, run as two passes when `twice`.
fn eased_frame(fixture: &mut Fixture, rerun: &Cell<bool>, twice: bool, events: Vec<egui::Event>) {
    rerun.set(twice);
    fixture.frame(events);
    assert!(!twice || fixture.passes() >= 2, "the frame ran a second pass");
}

/// How far down the document the view travels from one wheel line through the `frames` frames of `step` seconds
/// after it, each frame run as two passes when `twice`; and whether the first frame after the line still moved it.
fn eased_travel(step: f64, frames: usize, twice: bool) -> (f64, bool) {
    let (mut fixture, view, rerun) = eased_rig();
    fixture.frame(vec![pointer::moved(view.get().rect.center())]);
    let start = view.get().center.y;
    eased_frame(&mut fixture, &rerun, twice, vec![line_event(-1.0, Modifiers::NONE)]);
    let notched = view.get().center.y;
    let mut moved_after = None;
    for _ in 0..frames {
        if step > FRAME {
            fixture.advance(step - FRAME);
        }
        eased_frame(&mut fixture, &rerun, twice, vec![]);
        moved_after.get_or_insert(view.get().center.y != notched);
    }
    (view.get().center.y - start, moved_after.unwrap_or(false))
}

#[trial]
fn an_eased_wheel_pays_the_same_distance_at_any_frame_rate_keeps_paying_after_the_notch_and_stops_on_a_key_move() {
    // A tenth of a second after the notch: six frames at 60 Hz, two at 20 Hz.
    let (fast, moved_after) = eased_travel(FRAME, 6, false);
    let (slow, _) = eased_travel(0.05, 2, false);
    let (rerun, _) = eased_travel(FRAME, 6, true);
    assert!(fast > 0.0, "the line moves the view: {fast}");
    assert!(moved_after, "the view still moves on a pass after the notch");
    assert!((fast - slow).abs() < 0.5, "the same distance at 60 Hz and at 20 Hz: {fast} and {slow}");
    assert!((fast - rerun).abs() < 0.5, "a frame's second pass pays nothing more: {fast} and {rerun}");

    let (mut fixture, view, _) = eased_rig();
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    fixture.frame(vec![pointer::moved(view.get().rect.center())]);
    fixture.frame(vec![line_event(-1.0, Modifiers::NONE)]);
    fixture.key(Key::PageDown, Modifiers::NONE);
    let paged = view.get().center;
    fixture.idle(5);
    assert_eq!(view.get().center, paged, "a key move ends the owed motion");
}

/// A canvas over a large document whose app reruns the pass of a frame while the flag is set, with `eased_wheel_zoom`
/// as `eased`: the fixture, the view it shows and the flag.
fn zoom_rig(eased: bool) -> EasedRig {
    let mut options = NavigationOptions::default();
    options.eased_wheel_zoom = eased;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let rerun = Rc::new(Cell::new(false));
    let (shown, again) = (Rc::clone(&view), Rc::clone(&rerun));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
        if again.replace(false) && frame::discardable(ui.ctx()) {
            frame::discard_pass(ui.ctx(), "the trial's second pass");
        }
    });
    fixture.settle();
    (fixture, view, rerun)
}

/// A zoom-modifier wheel notch of 60 points.
fn zoom_notch() -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, 60.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::COMMAND,
        source: egui::MouseWheelSource::Wheel,
    }
}

/// The zoom after each of the notch's frame and the `frames` frames after it, each run as two passes when `twice`.
fn zooms_after_notch(eased: bool, frames: usize, twice: bool) -> Vec<f64> {
    let (mut fixture, view, rerun) = zoom_rig(eased);
    fixture.frame(vec![pointer::moved(view.get().rect.center())]);
    let mut zooms = Vec::new();
    eased_frame(&mut fixture, &rerun, twice, vec![zoom_notch()]);
    zooms.push(view.get().zoom);
    for _ in 0..frames {
        eased_frame(&mut fixture, &rerun, twice, vec![]);
        zooms.push(view.get().zoom);
    }
    zooms
}

#[trial]
fn an_eased_wheel_zoom_spreads_a_notch_over_the_frames_after_it_and_lands_where_the_plain_one_jumps() {
    let plain = zooms_after_notch(false, 3, false);
    assert!(plain[0] > 1.0, "the notch zooms in: {plain:?}");
    assert!(plain.windows(2).all(|pair| pair[0] == pair[1]), "upstream's default zooms at once: {plain:?}");

    let eased = zooms_after_notch(true, 60, false);
    let (first, last) = (eased[0], eased[eased.len() - 1]);
    assert!(first > 1.0 && first < plain[0], "the notch's frame pays part of it: {first} of {}", plain[0]);
    assert!(eased[2] > eased[1], "a frame after the notch still zooms: {eased:?}");
    assert!((last / plain[0] - 1.0).abs() < 1e-3, "the ease lands on the plain zoom: {last} and {}", plain[0]);

    let rerun = zooms_after_notch(true, 60, true);
    assert!((rerun[1] - eased[1]).abs() < 1e-9, "a frame's second pass pays nothing more: {} and {}", rerun[1], eased[1]);

    // A key move drops what is still owed.
    let (mut fixture, view, _) = zoom_rig(true);
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    fixture.frame(vec![pointer::moved(view.get().rect.center())]);
    fixture.frame(vec![zoom_notch()]);
    fixture.key(Key::PageDown, Modifiers::NONE);
    let paged = view.get().zoom;
    fixture.idle(5);
    assert_eq!(view.get().zoom, paged, "a key move ends the owed zoom");
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
    let (mut fixture, view, _, _) = wheel_rig(Platform::Windows, NavigationOptions::default());
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
    let (mut fixture, view, number, _) = wheel_rig(Platform::Linux, NavigationOptions::default());
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

    let (mut fixture, view, _, app_field) = wheel_rig(Platform::Linux, NavigationOptions::default());
    fixture.click_at(app_field.get().center());
    let before = view.get();
    wheel_line(&mut fixture, &view, 1.0, Modifiers::NONE);
    assert_eq!(view.get(), before, "an app's own focused text field keeps the plain wheel from the canvas");
    wheel_line(&mut fixture, &view, 1.0, Modifiers::ALT);
    assert!(view.get().zoom > before.zoom, "Alt + wheel is still the view's zoom");
}

#[trial]
fn the_view_commands_a_canvas_runs_are_in_the_frame_output_whatever_fired_them() {
    let (mut fixture, view, _, _) = wheel_rig(Platform::Linux, NavigationOptions::default());
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

/// The app command the zoom-chord trials bind to Cmd/Ctrl+=, as a word processor's Subscript.
const SUBSCRIPT: &str = "canvasTrial.subscript";

/// What Cmd/Ctrl+= does with a focused canvas, over a document with `document` (without one the canvas offers no view
/// command), under an app catalog binding it to `SUBSCRIPT` that drops Zoom In's default chords with `dropped`: the app
/// commands it fired, and whether the canvas zoomed in.
fn ctrl_equals(document: bool, dropped: bool) -> (Vec<String>, bool) {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").view(given);
        let canvas = if document { canvas.document(Size::new(4000.0, 4000.0)) } else { canvas };
        shown.set(canvas.show(ui, |_, _| {}).output.view.unwrap_or(given));
    });
    fixture.os(Platform::Linux);
    let binding = Binding::parse("Cmd+=").expect("the trial's chord parses");
    let catalog =
        AppCatalog::new(vec![CommandDef::new(SUBSCRIPT, "Subscript", Scope::app()).with_default(binding)], 1).expect("the catalog is valid");
    let catalog = if dropped { catalog.replace_defaults(ViewCommands::ZoomIn.id(), Vec::new()) } else { catalog };
    fixture.commands(CommandInput::new().catalog(catalog));
    fixture.settle();
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    frame::take_output(fixture.ctx());
    let before = view.get().zoom;
    fixture.key(Key::Equals, Modifiers::COMMAND);
    fixture.frame(vec![]);
    let fired = frame::take_output(fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).collect();
    (fired, view.get().zoom > before)
}

/// By default a focused canvas's Zoom In takes Cmd/Ctrl+= from the app's binding of it: its scope is around the focus.
#[trial]
fn a_focused_canvas_zooms_in_on_its_chord_by_default() {
    assert_eq!(ctrl_equals(true, false), (Vec::new(), true));
}

/// An app whose own command owns the chord (Word's Ctrl+= is Subscript) drops Zoom In's default through its catalog:
/// the chord is the app's while the canvas has the focus, and the canvas's Zoom In stays for menus and buttons.
#[trial]
fn an_app_catalog_drops_the_canvas_s_zoom_chord_for_its_own_command() {
    assert_eq!(ctrl_equals(true, true), (vec![SUBSCRIPT.to_owned()], false));
}

/// A canvas that offers no Zoom In (no document) leaves its chord to the app's binding instead of swallowing it.
#[trial]
fn a_canvas_s_unavailable_view_command_leaves_its_chord_to_the_app() {
    assert_eq!(ctrl_equals(false, false), (vec![SUBSCRIPT.to_owned()], false));
}

#[trial]
fn a_canvas_runs_the_view_commands_its_app_queues_before_the_fired_ones_and_reports_only_the_fired() {
    const DOCUMENT: Size = Size::new(4000.0, 4000.0);
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let on_screen = Rc::new(Cell::new(None::<CanvasView>));
    let queue = Rc::new(RefCell::new(Vec::<ViewRequest>::new()));
    let (shown, seen, asked) = (Rc::clone(&view), Rc::clone(&on_screen), Rc::clone(&queue));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let commands = asked.borrow().clone();
        let canvas = Canvas::new("Document").document(DOCUMENT).view(given).queued(&commands).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
        seen.set(canvas.output.space.map(|space| space.view));
        // As an app does: the queue is dropped once a pass showed the canvas.
        asked.borrow_mut().clear();
    });
    fixture.settle();
    frame::take_output(fixture.ctx());
    let ppp = fixture.ctx().pixels_per_point();

    // The app placing the view jumps there, also while a zoom the user asked for eases, and Fit on Screen leaves room
    // around the document.
    queue.borrow_mut().push(ViewRequest::Command(ViewCommands::ZoomIn));
    fixture.frame(vec![]);
    queue.borrow_mut().push(ViewRequest::Place(ViewCommands::FitOnScreen));
    fixture.frame(vec![]);
    let fitted = view.get();
    assert_eq!(on_screen.get().map(|shown| (shown.zoom, shown.center)), Some((fitted.zoom, fitted.center)), "a placed view jumps");
    let extent = fitted.extent(craft_behavior::kurbo::Rect::from_origin_size(Point::ORIGIN, DOCUMENT));
    assert!(extent.x < fitted.rect.width() && extent.y < fitted.rect.height(), "the fit leaves room on every side");

    // Two asked Zoom Ins and a fired Zoom Out: the queue runs first, in order, then the fired command.
    let start = zoom::device(&fitted, ppp);
    queue.borrow_mut().extend([ViewRequest::Command(ViewCommands::ZoomIn), ViewRequest::Command(ViewCommands::ZoomIn)]);
    assert!(commands::fire(fixture.ctx(), ViewCommands::ZoomOut, FireSource::Menu));
    fixture.frame(vec![]);
    let one_in = ZoomScheme::PHOTOSHOP.step(start, 1, DOCUMENT);
    assert!((zoom::device(&view.get(), ppp) / one_in - 1.0).abs() < 1e-6, "in, in, out: one rung in");
    let delivered: Vec<_> = frame::take_output(fixture.ctx()).delivered.into_iter().map(|fired| fired.command).collect();
    assert_eq!(delivered, [CommandRef::from(ViewCommands::ZoomOut)], "a queued command is the app's, not delivered");

    // A dropped queue runs nothing more.
    let settled = view.get();
    fixture.frame(vec![]);
    assert_eq!(view.get().zoom, settled.zoom);
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

/// The view a canvas shows before and after the frames of `input` over its middle, under `options` with `held`
/// modifiers down, from a view turned `rotation` degrees and mirrored when `flip`.
fn views_after(
    options: NavigationOptions,
    rotation: f64,
    flip: bool,
    held: Modifiers,
    input: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>>,
) -> (CanvasView, CanvasView) {
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
    if held != Modifiers::NONE {
        fixture.hold(held);
    }
    let before = view.get();
    for events in input(middle) {
        fixture.frame(events);
    }
    (before, view.get())
}

/// The zoom ratio and the rotation of the view a canvas shows after the frames of `input`, as [`views_after`].
fn gestured(options: NavigationOptions, rotation: f64, flip: bool, input: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>>) -> (f64, f64) {
    let (before, after) = views_after(options, rotation, flip, Modifiers::NONE, input);
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
fn two_fingers_moving_together_pan_the_view_only_with_enable_gestures_and_before_the_modifier_rules() {
    let shift = Vec2::new(30.0, 0.0);
    let half = Vec2::new(50.0, 0.0);
    let drag: &dyn Fn(Pos2) -> Vec<Vec<egui::Event>> =
        &|at| vec![fingers(TouchPhase::Start, at, half), fingers(TouchPhase::Move, at + shift, half), fingers(TouchPhase::End, at + shift, half)];
    // (case, Enable Gestures, modifiers held, pans)
    let cases = [
        ("with Enable Gestures", true, Modifiers::NONE, true),
        ("without Enable Gestures", false, Modifiers::NONE, false),
        ("with Cmd held", true, Modifiers::COMMAND, true),
    ];
    for (case, gestures, held, pans) in cases {
        let (before, after) = views_after(navigation(true, gestures, false), 0.0, false, held, drag);
        if pans {
            assert!(
                after.center.x < before.center.x - 1.0 && (after.center.y - before.center.y).abs() < 1e-3,
                "{case}: the content follows the fingers: {before:?} -> {after:?}"
            );
        } else {
            assert_eq!(after.center, before.center, "{case}: the view stays");
        }
        assert!((after.zoom / before.zoom - 1.0).abs() < 1e-4, "{case}: a pan is no zoom");
    }
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

#[trial]
fn the_document_scheme_zooms_by_rungs_on_ctrl_or_cmd_wheel_and_drops_the_residual_on_a_scroll() {
    const DOCUMENT: Size = Size::new(4000.0, 4000.0);
    let mut options = NavigationOptions::default();
    options.wheel = WheelScheme::Document;
    options.overscroll = true;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(DOCUMENT).navigation(options).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.os(Platform::Linux);
    // As an app that gives Ctrl + wheel to the canvas sets egui: no wheel modifier is egui's own zoom.
    fixture.ctx().options_mut(|options| options.input_options.zoom_modifier = Modifiers::NONE);
    fixture.settle();
    let ppp = fixture.ctx().pixels_per_point();
    let device = |view: &Cell<CanvasView>| zoom::device(&view.get(), ppp);

    let start = device(&view);
    wheel_line(&mut fixture, &view, 1.0, Modifiers::COMMAND);
    let one_in = ZoomScheme::PHOTOSHOP.step(start, 1, DOCUMENT);
    assert!((device(&view) / one_in - 1.0).abs() < 1e-6, "Ctrl + one notch steps one rung in: {start} -> {}", device(&view));
    wheel_line(&mut fixture, &view, -1.0, Modifiers::COMMAND);
    assert!((device(&view) / ZoomScheme::PHOTOSHOP.step(one_in, -1, DOCUMENT) - 1.0).abs() < 1e-6, "and one notch back steps one rung out");

    let before = view.get();
    wheel_line(&mut fixture, &view, -1.0, Modifiers::NONE);
    let scrolled = view.get();
    assert_eq!(scrolled.zoom, before.zoom, "a plain wheel does not zoom");
    assert!(scrolled.center.y != before.center.y && scrolled.center.x == before.center.x, "it scrolls up and down: {before:?} -> {scrolled:?}");
    wheel_line(&mut fixture, &view, -1.0, Modifiers::SHIFT);
    let sideways = view.get();
    assert_eq!(sideways.zoom, scrolled.zoom, "Shift + wheel does not zoom");
    assert!(sideways.center.x != scrolled.center.x && sideways.center.y == scrolled.center.y, "it scrolls sideways: {sideways:?}");

    // Half notches with Ctrl add up only while nothing else comes between them.
    let at = device(&view);
    wheel_line(&mut fixture, &view, 0.5, Modifiers::COMMAND);
    assert_eq!(device(&view), at, "half a notch steps nothing");
    wheel_line(&mut fixture, &view, -1.0, Modifiers::NONE);
    wheel_line(&mut fixture, &view, 0.5, Modifiers::COMMAND);
    assert_eq!(device(&view), at, "the scroll between them dropped the first half");
    wheel_line(&mut fixture, &view, 0.5, Modifiers::COMMAND);
    assert!((device(&view) / ZoomScheme::PHOTOSHOP.step(at, 1, DOCUMENT) - 1.0).abs() < 1e-6, "two halves in a row step one rung");
}

type ScrollRig = (Fixture, Rc<Cell<CanvasView>>, Rc<Cell<Option<Pos2>>>, Rc<RefCell<Vec<Point>>>, Rc<Cell<bool>>, Rc<Cell<bool>>);

/// A canvas filling a 400 by 300 room over a 1000-unit square document at zoom 1, its middle button set to autoscroll:
/// the fixture, the view it shows, the autoscroll mark it reported, the view centre of each pass, and whether the next
/// frame runs a second pass, and whether any pass reported a press to the tool since the flag was last cleared.
fn autoscroll_rig() -> ScrollRig {
    let mut options = NavigationOptions::default();
    options.middle_button = MiddleButton::AutoScroll;
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(500.0, 500.0), 1.0)));
    let mark = Rc::new(Cell::new(None));
    let passes = Rc::new(RefCell::new(Vec::new()));
    let rerun = Rc::new(Cell::new(false));
    let reached = Rc::new(Cell::new(false));
    let (shown, marked, seen, again, tool) = (Rc::clone(&view), Rc::clone(&mark), Rc::clone(&passes), Rc::clone(&rerun), Rc::clone(&reached));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let canvas = Canvas::new("Document").document(Size::new(1000.0, 1000.0)).navigation(options).view(given).show(ui, |_, _| {});
        let now = canvas.output.view.unwrap_or(given);
        shown.set(now);
        marked.set(canvas.output.autoscroll);
        if canvas.output.press.phase.start().is_some() {
            tool.set(true);
        }
        seen.borrow_mut().push(now.center);
        if again.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs the frame twice");
        }
    });
    fixture.room(Vec2::new(400.0, 300.0));
    // The window's focus is stated, not left to the input's default: losing it ends an autoscroll.
    fixture.frame(vec![egui::Event::WindowFocused(true)]);
    fixture.settle();
    (fixture, view, mark, passes, rerun, reached)
}

/// A middle click at `at`: hover, press and release in separate frames.
fn middle_click(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::middle(at, true)]);
    fixture.frame(vec![pointer::middle(at, false)]);
}

#[trial]
fn a_middle_click_starts_autoscroll_that_scrolls_toward_the_pointer_and_any_input_ends_it() {
    let (mut fixture, view, mark, passes, rerun, reached) = autoscroll_rig();
    let start = view.get();
    let at = start.rect.center();
    let (below, above) = (at + Vec2::new(0.0, 100.0), at - Vec2::new(0.0, 100.0));

    middle_click(&mut fixture, at);
    assert!(mark.get().is_some_and(|mark| mark.distance(at) < 0.5), "a middle click sets the mark where it came up: {:?}", mark.get());
    assert_eq!(view.get().center, start.center, "the click itself moves nothing");
    fixture.frame(vec![pointer::moved(at + Vec2::new(0.0, 10.0))]).idle(2);
    assert_eq!(view.get().center, start.center, "inside the dead zone nothing scrolls");

    fixture.frame(vec![pointer::moved(below)]).idle(1);
    let down = view.get();
    assert!(down.center.y > start.center.y && down.center.x == start.center.x, "the pointer below the mark scrolls down: {down:?}");

    // A second pass of one frame has no time of its own: it scrolls zero.
    passes.borrow_mut().clear();
    rerun.set(true);
    fixture.frame(Vec::new());
    rerun.set(false);
    assert_eq!(fixture.passes(), 2, "the frame ran twice");
    let seen = passes.borrow().clone();
    assert_eq!(seen.len(), 2, "one view per pass");
    assert!(seen[0].y > down.center.y, "the frame's first pass scrolls");
    assert_eq!(seen[1], seen[0], "its second pass does not");

    // On to the image's edge, where it stops and keeps running.
    fixture.idle(120);
    let edge = view.get();
    let bottom = edge.center.y + f64::from(edge.rect.height()) / 2.0 / edge.zoom;
    assert!((bottom - 1000.0).abs() < 0.5, "the view stops at the image's bottom edge: {bottom}");
    assert!(mark.get().is_some(), "the autoscroll still runs at the edge");

    // A key ends it: the pointer above the mark then scrolls nothing.
    fixture.key(Key::Escape, Modifiers::NONE);
    assert_eq!(mark.get(), None, "a key ends the autoscroll");
    fixture.frame(vec![pointer::moved(above)]).idle(3);
    assert_eq!(view.get().center, edge.center, "an ended autoscroll scrolls nothing");

    // A press ends it, a middle press too, and that press's release starts nothing.
    fixture.advance(1.0);
    middle_click(&mut fixture, at);
    fixture.frame(vec![pointer::moved(above)]).idle(1);
    let up = view.get();
    assert!(up.center.y < edge.center.y, "the pointer above the mark scrolls up");
    reached.set(false);
    fixture.frame(vec![pointer::press(above)]);
    assert_eq!(mark.get(), None, "a primary press ends the autoscroll");
    fixture.frame(vec![pointer::release(above)]).idle(3);
    assert!(!reached.get(), "the press that stopped the autoscroll reaches no tool");
    let pressed = view.get();
    fixture.idle(3);
    assert_eq!(view.get().center, pressed.center, "nothing scrolls after the press");
    fixture.advance(1.0);
    middle_click(&mut fixture, at);
    assert!(mark.get().is_some(), "a middle click starts it again");
    fixture.advance(1.0);
    middle_click(&mut fixture, at);
    assert_eq!(mark.get(), None, "a second middle click ends it and does not start another");

    // The pointer leaving the window ends it.
    fixture.advance(1.0);
    middle_click(&mut fixture, at);
    assert!(mark.get().is_some());
    fixture.frame(vec![pointer::gone()]);
    assert_eq!(mark.get(), None, "the pointer leaving the window ends the autoscroll");
    let left = view.get();
    fixture.frame(vec![pointer::moved(above)]).idle(3);
    assert_eq!(view.get().center, left.center, "the pointer back above the mark scrolls nothing");

    // A middle drag under Autoscroll scrolls at the autoscroll speed, does not pan, and its release ends it.
    fixture.advance(1.0);
    let before = view.get();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::middle(at, true)]);
    assert!(mark.get().is_some_and(|mark| mark.distance(at) < 0.5), "the press starts it where it came down: {:?}", mark.get());
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(at - Vec2::new(0.0, 25.0 * step as f32))]);
    }
    fixture.idle(2);
    let dragged = view.get();
    assert!(dragged.center.y < before.center.y, "the held drag above the mark scrolls up: {before:?} -> {dragged:?}");
    assert_eq!(dragged.center.x, before.center.x, "and does not pan sideways");
    fixture.frame(vec![pointer::middle(above, false)]);
    assert_eq!(mark.get(), None, "letting go after a drag ends it");
    let ended = view.get();
    fixture.idle(3);
    assert_eq!(view.get().center, ended.center, "nothing scrolls after the release");
}

/// A tool that records where its drag samples fall in the document, through the canvas space of the pass that took them.
#[derive(Default)]
struct Trace {
    drags: Vec<Point>,
}

impl CanvasTool for Trace {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(space) = cx.space() {
            self.drags.push(space.view.to_doc(pointer.pos));
        }
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the trace only follows the pointer")
    }
}

#[trial]
fn the_wheel_scrolls_during_a_tool_drag_and_the_drag_follows_the_new_view() {
    let tool = Rc::new(RefCell::new(Trace::default()));
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let (shown_tool, shown) = (Rc::clone(&tool), Rc::clone(&view));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let mut reply = None;
        let canvas = Canvas::new("Document").document(Size::new(4000.0, 4000.0)).view(given).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *shown_tool.borrow_mut()));
            }
        });
        let reply = reply.unwrap_or_default();
        shown.set(reply.view.or(canvas.output.view).unwrap_or(given));
    });
    fixture.settle();
    let at = view.get().rect.center();
    let to = at + Vec2::new(60.0, 0.0);
    fixture.drag_hold_at(at, to - at);
    assert!(!tool.borrow().drags.is_empty(), "the tool's drag runs");

    let before = view.get();
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, -1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    }]);
    let after = view.get();
    assert!(
        (after.center.y - before.center.y - f64::from(NavigationOptions::default().wheel_line)).abs() < 1e-3,
        "a wheel line scrolls the view while the drag runs: {before:?} -> {after:?}"
    );

    let next = to + Vec2::new(1.0, 0.0);
    fixture.frame(vec![pointer::moved(next)]);
    let last = *tool.borrow().drags.last().expect("the drag goes on");
    assert!((last - after.to_doc(next)).hypot() < 1e-3, "the drag's point is read through the scrolled view: {last:?}");
    assert!((last - before.to_doc(next)).hypot() > 1.0, "not through the view before the wheel");
    fixture.release_at(next);
}

//! Navigation rules (wheel, page keys, flick, autoscroll), the zoom motion and the momentary tool order.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CanvasView, Wheel, WheelPhase, WheelUnit};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{Canvas, Guide, GuideKey, Momentary, MomentaryCommands, MomentaryTool, NavigationOptions, TouchpadGlide};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::FirePhase;
use craft_ui::testing::{Fixture, trial};
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
/// degrees, centred off the pixel grid.
fn space_at(device: f64, rotation: f64) -> craft_ui::canvas::CanvasSpace {
    let space = Rc::new(Cell::new(None));
    let seen = Rc::clone(&space);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let zoom = CanvasView::points_per_unit(device, ui.ctx().pixels_per_point());
        let view = CanvasView::new(room, Point::new(100.3, 77.7), zoom).rotation(rotation);
        let mut navigation = NavigationOptions::default();
        navigation.overscroll = true;
        let shown = Canvas::new("Document").document(Size::new(400.0, 300.0)).navigation(navigation).view(view).show(ui, |_, _| {});
        seen.set(shown.output.space);
    });
    fixture.settle();
    space.get().expect("a canvas given a view publishes its space")
}

#[trial]
fn at_a_whole_device_zoom_with_no_rotation_the_shown_view_puts_document_pixels_on_device_pixels() {
    for device in [2.0, 12.0] {
        let space = space_at(device, 0.0);
        let ppp = space.pixels_per_point;
        for doc in [Point::ZERO, Point::new(17.0, 29.0)] {
            let p = space.view.to_screen(doc) * ppp;
            assert!((p.x - p.x.round()).abs() < 1e-3 && (p.y - p.y.round()).abs() < 1e-3, "{device}x: {doc:?} at {p:?}");
        }
        assert_eq!(space.target().center, Point::new(100.3, 77.7), "the view the app keeps is not moved");
        let moved = (space.view.to_screen(Point::ZERO) - space.target().to_screen(Point::ZERO)) * ppp;
        assert!(moved.x.abs() <= 0.501 && moved.y.abs() <= 0.501, "{device}x: moved {moved:?}");
    }
    let turned = space_at(2.0, 30.0);
    assert_eq!(turned.view.center, turned.target().center, "a turned view is shown as it is");
}

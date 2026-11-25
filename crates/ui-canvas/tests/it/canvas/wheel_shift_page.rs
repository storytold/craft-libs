//! Shift + wheel over the canvas: one page up or down, a page being the canvas's height less one wheel line
//! (production's #12029). The sideways, zoom, ladder and clamp neighbours of that gesture are pinned here too.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, Wheel, WheelPhase, WheelUnit, ZoomScheme};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{Canvas, NavigationOptions, WheelAction, WheelInput, WheelNavigation, WheelScheme};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Modifiers, Vec2, pos2};
use std::cell::Cell;
use std::rc::Rc;

/// The document under the measured canvas: 4000 x 4000 units at zoom 1.
const DOCUMENT: Size = Size::new(4000.0, 4000.0);

/// The wheel line the tests read and page with: the 16-point default of [`NavigationOptions::wheel_line`],
/// which the fixture's canvas runs on.
const LINE: f32 = 16.0;

/// A page of the measured canvas: its 817 points less one wheel line, the 801 px production's #12029 measured.
const PAGE: f64 = 817.0 - LINE as f64;

/// The measured view: a canvas 1473 x 817 points, centred on the document at zoom 1.
fn view() -> CanvasView {
    CanvasView::new(egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1473.0, 817.0)), Point::new(2000.0, 2000.0), 1.0)
}

/// One wheel line on a canvas whose line is [`LINE`] points, as the wheel service reports it. `down`
/// is the notch that pans the view down the document (production's #12028): it reaches the canvas as a negative scroll,
/// and the view's centre moves to a larger document y.
fn line(down: bool) -> Vec2 {
    Vec2::new(0.0, if down { -LINE } else { LINE })
}

/// The wheel share of one line, as the canvas reads it: `scroll` in points the device reported in `unit`, with `mods`,
/// on `os`.
fn pass(scroll: Vec2, unit: WheelUnit, mods: Mods, os: Platform) -> WheelInput {
    WheelInput::of(&Wheel::new(scroll, mods, unit, WheelPhase::Move), LINE, os)
}

/// What one pass of the wheel does to the view under `options` on `os`.
fn classify(scroll: Vec2, unit: WheelUnit, mods: Mods, os: Platform, options: &NavigationOptions) -> Option<WheelAction> {
    WheelNavigation::default().classify(pass(scroll, unit, mods, os), options)
}

/// The measured view after [`WheelNavigation::apply`] ran `action`, and the document centre it moved by.
fn moved(action: WheelAction) -> (CanvasView, Point) {
    let before = view();
    let at = before.rect.center();
    let mut after = before;
    WheelNavigation::apply(&mut after, action, at, DOCUMENT, 1.0, &ZoomScheme::PHOTOSHOP);
    (after, Point::new(after.center.x - before.center.x, after.center.y - before.center.y))
}

/// The page count a wheel line gave the view, or a panic naming what the wheel did instead.
fn pages_of(action: Option<WheelAction>, os: Platform, mods: Mods) -> f32 {
    let Some(WheelAction::Page(pages, _)) = action else {
        panic!("{os:?}: {mods:?} + a wheel line pages the view, not {action:?}");
    };
    pages
}

/// A canvas over a large document on `os`, with Overscroll as `overscroll` and the wheel modifiers a host app asks the
/// runtime for app-wide: Shift and Cmd/Ctrl fold to a sideways scroll in the runtime's own scroll areas, which is why
/// the canvas has to read the gesture itself. The fixture, and the view it shows.
fn rig(os: Platform, overscroll: bool) -> (Fixture, Rc<Cell<CanvasView>>) {
    let view = Rc::new(Cell::new(CanvasView::new(egui::Rect::ZERO, Point::new(2000.0, 2000.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        given.rect = ui.available_rect_before_wrap();
        let mut navigation = NavigationOptions::default();
        navigation.overscroll = overscroll;
        let canvas = Canvas::new("Document").document(DOCUMENT).navigation(navigation).view(given).show(ui, |_, _| {});
        shown.set(canvas.output.view.unwrap_or(given));
    });
    fixture.os(os);
    fixture.ctx().options_mut(|options| {
        // No wheel modifier is egui's own zoom, as an app that owns the wheel sets it.
        options.input_options.zoom_modifier = Modifiers::NONE;
        options.input_options.horizontal_scroll_modifier = Modifiers::SHIFT | Modifiers::COMMAND;
    });
    fixture.settle();
    (fixture, view)
}

/// One wheel line of `dy` with `modifiers`, over the middle of the canvas.
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

/// The canvas's own page distance for the view it shows.
fn page_of_the_shown(view: &CanvasView) -> f64 {
    f64::from(view.rect.height()) - f64::from(LINE)
}

#[trial]
fn shift_with_a_wheel_line_pages_the_canvas_by_its_height_less_one_line() {
    let options = NavigationOptions::default();
    for os in [Platform::Windows, Platform::Linux] {
        let notches = pages_of(classify(line(true), WheelUnit::Line, Mods::SHIFT, os, &options), os, Mods::SHIFT);
        assert_eq!(notches, -1.0, "{os:?}: one wheel line is one page");
        let (after, by) = moved(WheelAction::Page(notches, LINE));
        assert!((by.y - PAGE).abs() < 1e-3, "{os:?}: a page is the canvas height less one line, down the document: {by:?}");
        assert_eq!(by.x, 0.0, "{os:?}: nothing moves sideways: {:?} -> {after:?}", view().center);
    }
}

#[trial]
fn shift_with_a_wheel_line_the_other_way_pages_the_same_distance_back() {
    let options = NavigationOptions::default();
    for os in [Platform::Windows, Platform::Linux] {
        let notches = pages_of(classify(line(false), WheelUnit::Line, Mods::SHIFT, os, &options), os, Mods::SHIFT);
        assert_eq!(notches, 1.0, "{os:?}: the other notch is one page back");
        let (_, by) = moved(WheelAction::Page(notches, LINE));
        assert!((by.y + PAGE).abs() < 1e-3 && by.x == 0.0, "the page is symmetric: {by:?}");
    }
}

#[trial]
fn a_page_travels_the_way_a_plain_wheel_line_travels_only_a_page_long() {
    let options = NavigationOptions::default();
    for os in [Platform::Windows, Platform::Linux] {
        let plain = classify(line(true), WheelUnit::Line, Mods::NONE, os, &options);
        let Some(WheelAction::Pan(scroll)) = plain else {
            panic!("{os:?}: a plain wheel line still pans, not {plain:?}");
        };
        assert_eq!(scroll, line(true), "{os:?}: the plain wheel line is unchanged");
        let (_, panned) = moved(WheelAction::Pan(scroll));
        assert!((panned.y - f64::from(LINE)).abs() < 1e-3 && panned.x == 0.0, "{os:?}: a line pans 16 points");
        let notches = pages_of(classify(line(true), WheelUnit::Line, Mods::SHIFT, os, &options), os, Mods::SHIFT);
        let (paged, page) = moved(WheelAction::Page(notches, LINE));
        assert_eq!(page.y.signum(), panned.y.signum(), "{os:?}: the page follows the plain wheel's direction");
        assert!(page.y.abs() > panned.y.abs(), "{os:?}: a page is longer than a line: {page:?} vs {panned:?}");
        assert_eq!(paged.center.x, view().center.x, "{os:?}: a page moves the view along one axis only: {paged:?}");
    }
}

#[trial]
fn ctrl_or_cmd_with_a_wheel_line_still_scrolls_the_canvas_sideways_on_every_os() {
    let options = NavigationOptions::default();
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        // `CMD` is ⌘ on macOS and Ctrl on Windows and Linux; `ctrl` is the Control key, a modifier of its own on macOS
        // only.
        let held = [Mods::CMD, if os.is_mac() { Mods::CTRL } else { Mods::NONE }];
        for mods in held {
            if mods.is_none() {
                continue;
            }
            let sideways = classify(line(true), WheelUnit::Line, mods, os, &options);
            let Some(WheelAction::Pan(scroll)) = sideways else {
                panic!("{os:?}: {mods:?} + a wheel line stays a sideways pan, not {sideways:?}");
            };
            assert_eq!(scroll, Vec2::new(-LINE, 0.0), "{os:?}: {mods:?} + a wheel line goes sideways");
        }
    }
}

#[trial]
fn a_plain_wheel_line_still_pans_the_canvas_and_the_mac_keeps_shift_sideways() {
    let options = NavigationOptions::default();
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let panned = classify(line(true), WheelUnit::Line, Mods::NONE, os, &options);
        let Some(WheelAction::Pan(scroll)) = panned else {
            panic!("{os:?}: a plain wheel line still pans, not {panned:?}");
        };
        assert_eq!(scroll, line(true), "{os:?}: the plain wheel line is unchanged");
    }
    // The Mac keeps today's fold: the system itself reports Shift + wheel sideways there.
    let mac = classify(line(true), WheelUnit::Line, Mods::SHIFT, Platform::Mac, &options);
    let Some(WheelAction::Pan(scroll)) = mac else {
        panic!("macOS keeps Shift + a wheel line sideways, not {mac:?}");
    };
    assert_eq!(scroll, Vec2::new(-LINE, 0.0), "macOS: Shift + a wheel line is still the sideways pan");
}

#[trial]
fn shift_with_alt_with_a_wheel_line_still_steps_the_zoom_ladder() {
    let options = NavigationOptions::default();
    let alt_shift = Mods { alt: true, shift: true, ..Mods::NONE };
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let stepped = classify(line(true), WheelUnit::Line, alt_shift, os, &options);
        assert_eq!(stepped, Some(WheelAction::SnapZoom(-1)), "{os:?}: Shift + Alt + a wheel line steps the ladder as before");
    }
}

#[trial]
fn shift_with_a_wheel_line_still_steps_the_ladder_when_the_wheel_zooms() {
    let mut options = NavigationOptions::default();
    options.wheel = WheelScheme::Zoom;
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let stepped = classify(line(true), WheelUnit::Line, Mods::SHIFT, os, &options);
        assert_eq!(stepped, Some(WheelAction::SnapZoom(-1)), "{os:?}: Zoom with Scroll Wheel keeps Shift on the ladder");
    }
}

#[trial]
fn shift_with_a_trackpad_scroll_or_a_sideways_report_keeps_todays_pan() {
    let options = NavigationOptions::default();
    for os in [Platform::Windows, Platform::Linux] {
        // A trackpad reports its scroll in points, not notches: not measured against the reference application, so it
        // keeps today's sideways fold.
        let points = classify(Vec2::new(0.0, -30.0), WheelUnit::Point, Mods::SHIFT, os, &options);
        assert_eq!(points, Some(WheelAction::Pan(Vec2::new(-30.0, 0.0))), "{os:?}: Shift + a point-unit scroll stays sideways");
        // A device that already reports the gesture across the canvas keeps its own pan.
        let across = Vec2::new(LINE, 0.0);
        assert_eq!(
            classify(across, WheelUnit::Line, Mods::SHIFT, os, &options),
            Some(WheelAction::Pan(across)),
            "{os:?}: a sideways report pans as it is"
        );
        // Alt with Shift is the ladder, not a page.
        let alt_shift = Mods { alt: true, shift: true, ..Mods::NONE };
        assert_eq!(
            classify(line(true), WheelUnit::Line, alt_shift, os, &options),
            Some(WheelAction::SnapZoom(-1)),
            "{os:?}: Alt + Shift stays the ladder"
        );
    }
}

#[trial]
fn a_page_unit_wheel_under_shift_keeps_todays_sideways_pan() {
    // A device that reports whole pages is not the notched wheel production's #12029 measured: it keeps today's
    // sideways fold, and no page of its own comes out of the new rule.
    let options = NavigationOptions::default();
    for os in [Platform::Windows, Platform::Linux] {
        let sideways = classify(Vec2::new(0.0, -1.0), WheelUnit::Page, Mods::SHIFT, os, &options);
        assert_eq!(sideways, Some(WheelAction::Pan(Vec2::new(-1.0, 0.0))), "{os:?}: a page-unit wheel under Shift keeps today's sideways pan");
    }
}

#[trial]
fn the_document_schemes_shift_wheel_stays_sideways() {
    let mut options = NavigationOptions::default();
    options.wheel = WheelScheme::Document;
    for os in [Platform::Mac, Platform::Windows, Platform::Linux] {
        let sideways = classify(line(true), WheelUnit::Line, Mods::SHIFT, os, &options);
        assert_eq!(sideways, Some(WheelAction::Pan(Vec2::new(-LINE, 0.0))), "{os:?}: the Document scheme keeps Shift sideways");
        let zoom = classify(line(true), WheelUnit::Line, Mods::CMD, os, &options);
        assert_eq!(zoom, Some(WheelAction::SnapZoom(-1)), "{os:?}: the Document scheme keeps Cmd/Ctrl on the ladder");
    }
}

#[trial]
fn a_page_moves_a_turned_or_mirrored_view_the_way_a_pan_of_the_same_screen_distance_does() {
    // The page goes through `pan`, the same move a wheel line, a Hand drag and a page key use, so a view that is
    // turned or mirrored follows the screen's vertical axis exactly as it does for those.
    let ratio = PAGE / f64::from(LINE);
    for turned in [0.0, 30.0, 90.0, 180.0] {
        for mirrored in [false, true] {
            let before = view().rotation(turned).flip_x(mirrored);
            let at = before.rect.center();
            let mut panned = before;
            WheelNavigation::apply(&mut panned, WheelAction::Pan(line(true)), at, DOCUMENT, 1.0, &ZoomScheme::PHOTOSHOP);
            let mut paged = before;
            WheelNavigation::apply(&mut paged, WheelAction::Page(-1.0, LINE), at, DOCUMENT, 1.0, &ZoomScheme::PHOTOSHOP);
            let by = |moved: &CanvasView| Point::new(moved.center.x - before.center.x, moved.center.y - before.center.y);
            let (panned_by, paged_by) = (by(&panned), by(&paged));
            assert!(
                (paged_by.x - panned_by.x * ratio).abs() < 1e-3 && (paged_by.y - panned_by.y * ratio).abs() < 1e-3,
                "{turned} degrees, mirrored {mirrored}: a page is {ratio:.1} wheel lines of the same move: {panned_by:?} -> {paged_by:?}"
            );
        }
    }
}

#[trial]
fn a_tiny_canvas_and_a_hostile_page_count_move_the_view_nowhere() {
    // A canvas shorter than one wheel line has no page left after the line comes off, and a page count that is not a
    // number is no move at all: neither jumps the view.
    let mut narrow = CanvasView::new(egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 12.0)), Point::new(2000.0, 2000.0), 1.0);
    let at = narrow.rect.center();
    assert!(
        !WheelNavigation::apply(&mut narrow, WheelAction::Page(1.0, LINE), at, DOCUMENT, 1.0, &ZoomScheme::PHOTOSHOP),
        "a canvas under one line has no page to scroll"
    );
    let upright = view();
    let mut hostile = upright;
    assert!(
        !WheelNavigation::apply(&mut hostile, WheelAction::Page(f32::NAN, LINE), at, DOCUMENT, 1.0, &ZoomScheme::PHOTOSHOP),
        "a page count that is not a number moves nothing"
    );
    assert_eq!(hostile, upright, "and leaves the view as it was");
}

#[trial]
fn shift_with_a_wheel_notch_over_the_canvas_pages_it_by_its_height_less_one_line() {
    for os in [Platform::Windows, Platform::Linux] {
        let (mut fixture, view) = rig(os, false);
        let before = view.get();
        let page = page_of_the_shown(&before);
        wheel_line(&mut fixture, &view, -1.0, Modifiers::SHIFT);
        let after = view.get();
        assert!(
            (after.center.y - before.center.y - page).abs() < 1e-3,
            "{os:?}: one Shift + wheel notch scrolls the canvas height less one line: {before:?} -> {after:?}"
        );
        assert_eq!(after.center.x, before.center.x, "{os:?}: the page leaves the canvas where it was sideways: {before:?} -> {after:?}");
        assert_eq!(after.zoom, before.zoom, "{os:?}: the page leaves the zoom as it was");
        // The next notch pages back to where the view was.
        wheel_line(&mut fixture, &view, 1.0, Modifiers::SHIFT);
        let back = view.get();
        assert!(
            (back.center.y - before.center.y).abs() < 1e-3 && back.center.x == before.center.x,
            "{os:?}: the next notch pages back: {before:?} -> {back:?}"
        );
    }
}

#[trial]
fn a_page_at_the_documents_edge_stops_there_with_overscroll_off() {
    // The page goes through the same view a pan does, and the canvas clamps that view to the document with Overscroll
    // off: a page past the last row of the document leaves the view at the edge instead of showing a band of pasteboard.
    let (mut fixture, view) = rig(Platform::Windows, false);
    let half = f64::from(view.get().rect.height()) / 2.0;
    let mut at_edge = view.get();
    // The lowest centre the clamp allows: the document's bottom edge on the canvas's bottom edge.
    at_edge.center.y = 4000.0 - half;
    view.set(at_edge);
    fixture.settle();
    let held = view.get();
    assert!((held.center.y - at_edge.center.y).abs() < 1.0, "the edge is a place the canvas keeps: {held:?}");
    // The notch that pages past that edge leaves the view at it.
    wheel_line(&mut fixture, &view, -1.0, Modifiers::SHIFT);
    let at_edge = view.get();
    assert!((at_edge.center.y - held.center.y).abs() < 1.0, "a page past the document's bottom edge stops at it: {held:?} -> {at_edge:?}");
    assert_eq!(at_edge.center.x, held.center.x, "and nothing moves sideways: {held:?} -> {at_edge:?}");
    // The other notch pages a whole page back into the document.
    wheel_line(&mut fixture, &view, 1.0, Modifiers::SHIFT);
    let back = view.get();
    let page = page_of_the_shown(&back);
    assert!((back.center.y - at_edge.center.y + page).abs() < 1.0, "the other notch pages a whole page back: {at_edge:?} -> {back:?}");
}

#[trial]
fn a_page_runs_past_the_documents_edge_with_overscroll_on() {
    let (mut fixture, view) = rig(Platform::Windows, true);
    let mut at_edge = view.get();
    at_edge.center.y = 4000.0 - f64::from(at_edge.rect.height()) / 2.0;
    view.set(at_edge);
    fixture.settle();
    let before = view.get();
    let page = page_of_the_shown(&before);
    // The notch that pages off the end: with Overscroll on the canvas is free to show the pasteboard past it.
    wheel_line(&mut fixture, &view, -1.0, Modifiers::SHIFT);
    let after = view.get();
    assert!((after.center.y - before.center.y - page).abs() < 1.0, "Overscroll on runs the whole page past the edge: {before:?} -> {after:?}");
}

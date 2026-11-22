//! `CanvasScrollbars`: the clamp of the view and the bars' drag and page press.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{Axis, CanvasView};
use craft_ui::canvas::{Canvas, CanvasScrollbars, NavigationOptions, Rulers, canvas_space};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::env::Label;
use craft_ui::testing::input::slop;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Rect, Shape, Vec2, pos2, vec2};

/// The view's rect in the fixture, in points.
const VIEW: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 300.0));

/// The slack of a rect comparison after f32 maths, in points.
const TOLERANCE: f32 = 0.01;

fn view_at(center: Point, zoom: f64) -> CanvasView {
    CanvasView::new(VIEW, center, zoom)
}

#[trial]
fn a_rotated_view_is_clamped_by_the_document_as_it_is_shown() {
    let doc = DocRect::new(0.0, 0.0, 1000.0, 800.0);
    let mut view = view_at(Point::new(-50.0, 100.0), 1.0).rotation(90.0);
    assert!(CanvasScrollbars::clamp(&mut view, doc));
    let quad = view.quad(doc);
    let shown = quad.iter().skip(1).fold(Rect::from_min_max(quad[0], quad[0]), |rect, &corner| rect.union(Rect::from_min_max(corner, corner)));
    assert!(shown.expand(TOLERANCE).contains_rect(VIEW), "the turned document covers the view: {shown:?}");
}

#[trial]
fn overscroll_off_below_fit_keeps_the_image_centred() {
    let shown = |view: &CanvasView, doc: DocRect| {
        let quad = view.quad(doc);
        quad.iter().skip(1).fold(Rect::from_min_max(quad[0], quad[0]), |rect, &corner| rect.union(Rect::from_min_max(corner, corner)))
    };
    // Narrower and shorter than the view on both axes, zoomed out off centre, level and turned.
    let small = DocRect::new(0.0, 0.0, 200.0, 100.0);
    for rotation in [0.0, 30.0] {
        let mut view = view_at(Point::new(-60.0, 140.0), 1.0).rotation(rotation);
        assert!(CanvasScrollbars::clamp(&mut view, small));
        let image = shown(&view, small);
        assert!((image.center() - VIEW.center()).length() < TOLERANCE, "rotation {rotation}: {image:?} is centred in {VIEW:?}");
    }
    // Wider than the view but shorter: the wide axis keeps its edge, the short one is centred.
    let wide = DocRect::new(0.0, 0.0, 1000.0, 100.0);
    let mut view = view_at(Point::new(-60.0, 140.0), 1.0);
    assert!(CanvasScrollbars::clamp(&mut view, wide));
    let image = shown(&view, wide);
    assert!((image.left() - VIEW.left()).abs() < TOLERANCE, "the wide axis stops at its edge: {image:?}");
    assert!((image.center().y - VIEW.center().y).abs() < TOLERANCE, "the short axis is centred: {image:?}");
}

const FITTING_DOCUMENT: Size = Size::new(400.0, 300.0);
const DOCUMENT_WINDOW: Rect = Rect::from_min_max(Pos2::new(12.0, 12.0), Pos2::new(732.0, 512.0));

fn document_scrollbars() -> (Fixture, Rc<Cell<CanvasView>>, Rc<Cell<bool>>) {
    let over = Rc::new(Cell::new(false));
    let observed_over = Rc::clone(&over);
    let view = Rc::new(Cell::new(CanvasView::new(DOCUMENT_WINDOW, Point::new(200.0, 150.0), 1.0)));
    let observed = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut navigation = NavigationOptions::default();
        navigation.overscroll = true;
        let mut zero = Point::ZERO;
        let rulers = Rulers::new("Rulers", Bind::bound(&mut zero));
        let mut next = observed.get();
        Canvas::new("Document").document(FITTING_DOCUMENT).view(next).navigation(navigation).with_rulers(Some(rulers)).show_at(
            ui,
            DOCUMENT_WINDOW,
            |_, body| {
                if let Some(space) = canvas_space(&body.env) {
                    next = space.view;
                }
            },
        );
        let doc = DocRect::from_origin_size(Point::ZERO, FITTING_DOCUMENT);
        let bars = CanvasScrollbars::new("Scroll bars", Bind::bound(&mut next), doc).overscroll(true).document_chrome().show_at(ui, DOCUMENT_WINDOW);
        observed_over.set(bars.over);
        observed.set(next);
    });
    fixture.room(vec2(760.0, 560.0)).settle();
    (fixture, view, over)
}

/// #15529: the document bars leave room for arrows and a corner, and put a thinner thumb inside each track.
#[trial]
fn document_bars_show_arrows_and_an_inset_thumb_for_a_fitting_image() {
    let (mut fixture, view, _) = document_scrollbars();
    let horizontal = fixture.by_label("Horizontal scroll bar").expect("the horizontal bar").rect;
    let vertical = fixture.by_label("Vertical scroll bar").expect("the vertical bar").rect;
    assert!((horizontal.height() - 16.0).abs() < TOLERANCE, "horizontal track: {horizontal:?}");
    assert!((vertical.width() - 16.0).abs() < TOLERANCE, "vertical track: {vertical:?}");

    let left = fixture.by_label("Scroll left").expect("the left arrow").rect;
    let right = fixture.by_label("Scroll right").expect("the right arrow").rect;
    let up = fixture.by_label("Scroll up").expect("the up arrow").rect;
    let down = fixture.by_label("Scroll down").expect("the down arrow").rect;
    assert!((left.right() - horizontal.left()).abs() < TOLERANCE && (right.left() - horizontal.right()).abs() < TOLERANCE);
    assert!((up.bottom() - vertical.top()).abs() < TOLERANCE && (down.top() - vertical.bottom()).abs() < TOLERANCE);
    assert!((up.top() - DOCUMENT_WINDOW.top()).abs() < TOLERANCE, "the bar starts beside the ruler row");
    assert!(up.top() < view.get().rect.top(), "the visible document starts below the ruler row");

    let corner = Rect::from_min_size(pos2(right.right(), down.bottom()), Vec2::splat(16.0));
    let shapes = fixture.frame_shapes();
    assert!(shapes.iter().any(|shape| matches!(shape, Shape::Rect(paint) if paint.rect == corner && paint.fill.a() > 0)), "the corner is painted");
    assert!(
        shapes.iter().any(|shape| matches!(shape, Shape::Rect(paint)
            if horizontal.contains_rect(paint.rect)
                && (paint.rect.top() - horizontal.top() - 2.0).abs() < TOLERANCE
                && (paint.rect.height() - 12.0).abs() < TOLERANCE
                && paint.rect.width() < horizontal.width()
                && paint.fill.a() > 0)),
        "the fitting document's thumb is shorter than the track and inset on both sides",
    );
}

/// #2980: docked canvas scrollbars (without document chrome) provide comfortable thickness
/// (`size.icon.md` / 16 pt) and a breathing margin along the viewport edge.
#[trial]
fn docked_canvas_scrollbars_provide_comfortable_thickness_and_edge_margin() {
    let large = Size::new(4000.0, 3000.0);
    let view = Rc::new(Cell::new(CanvasView::new(DOCUMENT_WINDOW, Point::new(2000.0, 1500.0), 1.0)));
    let observed = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut next = observed.get();
        let doc = DocRect::from_origin_size(Point::ZERO, large);
        let _ = CanvasScrollbars::new("Scroll bars", Bind::bound(&mut next), doc).show_at(ui, DOCUMENT_WINDOW);
        observed.set(next);
    });
    fixture.room(vec2(760.0, 560.0)).settle();

    let horizontal = fixture.by_label("Horizontal scroll bar").expect("the horizontal bar").rect;
    let vertical = fixture.by_label("Vertical scroll bar").expect("the vertical bar").rect;

    // Comfortable desktop thickness (16 pt in standard compact density, matching size.icon.md):
    assert!(vertical.width() >= 14.0, "vertical bar width should be comfortable (>= 14 pt): {vertical:?}");
    assert!(horizontal.height() >= 14.0, "horizontal bar height should be comfortable (>= 14 pt): {horizontal:?}");

    // Breathing room along viewport edge (not merging directly into window border):
    assert!(vertical.right() < DOCUMENT_WINDOW.right(), "vertical bar has margin from right edge: {vertical:?} vs {DOCUMENT_WINDOW:?}");
    assert!(vertical.right() >= DOCUMENT_WINDOW.right() - 4.0, "vertical bar margin is subtle: {vertical:?}");
    assert!(horizontal.bottom() < DOCUMENT_WINDOW.bottom(), "horizontal bar has margin from bottom edge: {horizontal:?} vs {DOCUMENT_WINDOW:?}");
    assert!(horizontal.bottom() >= DOCUMENT_WINDOW.bottom() - 4.0, "horizontal bar margin is subtle: {horizontal:?}");
}

/// #15529: a press is one small step, a held press repeats, and the rest of the track still pages.
#[trial]
fn a_document_scroll_arrow_steps_and_repeats_while_its_track_pages() {
    let (mut fixture, view, _) = document_scrollbars();
    let start = view.get().center;
    let arrow = fixture.by_label("Scroll right").expect("the right arrow").center();
    fixture.frame(vec![pointer::moved(arrow)]).frame(vec![pointer::press(arrow)]);
    let stepped = view.get().center;
    assert!((stepped.x - start.x - 16.0).abs() < f64::from(TOLERANCE), "the arrow moves 16 screen pixels at 100%: {start:?} -> {stepped:?}");
    assert_eq!(stepped.y, start.y);
    fixture.advance(0.05).frame(vec![]);
    assert_eq!(view.get().center, stepped, "the first step precedes the hold delay");
    fixture.advance(0.5).frame(vec![]);
    let repeated = view.get().center;
    assert!(repeated.x > stepped.x && repeated.y == start.y, "the held arrow repeats on its own axis");
    fixture.frame(vec![pointer::release(arrow)]).advance(1.0).frame(vec![]);
    assert_eq!(view.get().center, repeated, "releasing the arrow stops the repeat");

    let track = fixture.by_label("Horizontal scroll bar").expect("the horizontal track").rect;
    fixture.click_at(pos2(track.right() - 8.0, track.center().y));
    fixture.settle();
    assert!(view.get().center.x > repeated.x + 16.0, "the track still pages instead of stepping");
}

#[trial]
fn the_scrollbar_corner_keeps_pointer_presses_off_the_canvas() {
    let (mut fixture, view, over) = document_scrollbars();
    let right = fixture.by_label("Scroll right").expect("the right arrow").rect;
    let down = fixture.by_label("Scroll down").expect("the down arrow").rect;
    let corner = pos2(right.right() + 4.0, down.bottom() + 4.0);
    fixture.frame(vec![pointer::moved(corner)]);
    assert!(over.get(), "the corner belongs to the scrollbars");
    let center = view.get().center;
    fixture.frame(vec![pointer::press(corner)]);
    assert!(over.get(), "a press on the corner must not reach the document");
    assert_eq!(view.get().center, center, "the corner never scrolls");
    fixture.frame(vec![pointer::release(corner)]).frame(vec![pointer::moved(DOCUMENT_WINDOW.center())]);
    assert!(!over.get(), "the document area still receives pointer input");
}

/// The document the bar trials scroll: ten times the view's height, so a page is a third of a thumb's travel or less.
const DOCUMENT: DocRect = DocRect::new(0.0, 0.0, 1000.0, 1000.0);

type BarsRig = (Fixture, Rc<Cell<CanvasView>>, Rc<RefCell<Vec<Option<Axis>>>>);

/// Scroll bars over [`DOCUMENT`] in a canvas of [`VIEW`] at zoom 1, the view at the document's top left, so both thumbs
/// sit at the start of their tracks; `tip` is the bars' tip. The fixture, the bars' view as they set it, and the thumb
/// each pass reported dragging.
fn bars_rig(tip: Option<&'static str>) -> BarsRig {
    let view = Rc::new(Cell::new(view_at(Point::new(200.0, 150.0), 1.0)));
    let dragging = Rc::new(RefCell::new(Vec::new()));
    let (shown, seen) = (Rc::clone(&view), Rc::clone(&dragging));
    let mut fixture = Fixture::new(move |ui| {
        let current = shown.get();
        let mut bar_view = current;
        let mut reported = None;
        let _ = Canvas::new("Document").document(Size::new(DOCUMENT.width(), DOCUMENT.height())).view(current).show_at(ui, VIEW, |ui, body| {
            let text = |_: &CanvasView| Label::from(tip.unwrap_or_default());
            let mut bars = CanvasScrollbars::new("Scroll bars", Bind::bound(&mut bar_view), DOCUMENT);
            if tip.is_some() {
                bars = bars.tip(&text);
            }
            reported = bars.show_at(ui, body.rect).dragging;
        });
        seen.borrow_mut().push(reported);
        shown.set(bar_view);
    });
    fixture.room(VIEW.size());
    fixture.settle();
    (fixture, view, dragging)
}

/// The track of the bar along `axis`, from the access tree.
fn track(fixture: &Fixture, axis: Axis) -> Rect {
    let bars = fixture.by_role(Role::SCROLL_BAR);
    let along = |rect: &Rect| if axis == Axis::Y { rect.height() > rect.width() } else { rect.width() > rect.height() };
    bars.into_iter().map(|bar| bar.rect).find(along).expect("the bars show both tracks")
}

/// Where a thumb at the start of the track `track` along `axis` is gripped: a little past the track's start.
fn grip(track: Rect, axis: Axis, slop: f32) -> Pos2 {
    if axis == Axis::Y { track.center_top() + Vec2::new(0.0, slop) } else { track.left_center() + Vec2::new(slop, 0.0) }
}

#[trial]
fn a_track_press_pages_one_screen_and_a_press_beside_the_bar_does_not() {
    let (mut fixture, view, dragging) = bars_rig(None);
    let start = view.get();
    let bar = track(&fixture, Axis::Y);
    let below_thumb = Pos2::new(bar.center().x, bar.top() + bar.height() * 0.75);

    dragging.borrow_mut().clear();
    fixture.click_at(below_thumb);
    let paged = view.get();
    let screen = f64::from(VIEW.height()) / start.zoom;
    assert!((paged.center.y - start.center.y - screen).abs() < 0.5, "a track press pages one screen down: {start:?} -> {paged:?}");
    assert_eq!(paged.center.x, start.center.x, "and not sideways");
    assert!(dragging.borrow().iter().all(Option::is_none), "a page press is no thumb drag");

    // A press beside the bar, dragged onto its track and let go there, pages nothing.
    let beside = Pos2::new(bar.left() - 20.0, below_thumb.y);
    fixture.drag_at(beside, Vec2::new(20.0 + bar.width() / 2.0, 0.0));
    assert_eq!(view.get().center, paged.center, "a press that began beside the bar pages nothing");
    fixture.click_at(beside);
    assert_eq!(view.get().center, paged.center, "nor does a click beside it");

    // The thumb drag is reported while it runs and not after.
    let (mut fixture, _, dragging) = bars_rig(None);
    let threshold = slop(fixture.ctx());
    let bar = track(&fixture, Axis::Y);
    fixture.drag_hold_at(grip(bar, Axis::Y, threshold), Vec2::new(0.0, 3.0 * threshold));
    assert_eq!(dragging.borrow().last().copied().flatten(), Some(Axis::Y), "a held thumb drag is reported on its axis");
    fixture.release_at(grip(bar, Axis::Y, threshold) + Vec2::new(0.0, 3.0 * threshold));
    assert_eq!(dragging.borrow().last().copied().flatten(), None, "the released thumb drags no more");
}

#[trial]
fn a_dragged_vertical_thumb_shows_the_tip_beside_it_and_nothing_when_it_is_not_dragged() {
    const TIP: &str = "Page 3 of 12";
    let shows_tip = |fixture: &Fixture| fixture.texts().iter().find(|text| text.text == TIP).map(|text| text.rect);

    let (mut fixture, _, _) = bars_rig(Some(TIP));
    assert_eq!(shows_tip(&fixture), None, "no tip while nothing drags");
    let threshold = slop(fixture.ctx());
    let bar = track(&fixture, Axis::Y);
    let from = grip(bar, Axis::Y, threshold);
    let by = Vec2::new(0.0, 3.0 * threshold);
    fixture.drag_hold_at(from, by);
    let tip = shows_tip(&fixture).expect("the dragged vertical thumb shows the tip");
    assert!(tip.right() <= bar.left() + 0.5, "the tip is beside the bar on the reading-start side: {tip:?} by {bar:?}");
    assert!(tip.bottom() > bar.top() && tip.top() < bar.bottom(), "level with the track: {tip:?}");
    fixture.release_at(from + by);
    // The tip leaves as every popup does, through its exit fade; the fixture's clock moves only when told to.
    fixture.advance(1.0).settle();
    assert_eq!(shows_tip(&fixture), None, "the release ends the tip");
    fixture.idle(2);
    assert_eq!(shows_tip(&fixture), None, "nothing is kept after it");

    let bottom = track(&fixture, Axis::X);
    let from = grip(bottom, Axis::X, threshold);
    fixture.drag_hold_at(from, Vec2::new(3.0 * threshold, 0.0));
    assert_eq!(shows_tip(&fixture), None, "a horizontal thumb drag shows no tip");
    fixture.release_at(from + Vec2::new(3.0 * threshold, 0.0));

    // Without a tip the drag paints no more text than the bars at rest.
    let (mut fixture, _, _) = bars_rig(None);
    let at_rest = fixture.texts().len();
    let bar = track(&fixture, Axis::Y);
    fixture.drag_hold_at(grip(bar, Axis::Y, threshold), by);
    assert_eq!(fixture.texts().len(), at_rest, "bars with no tip show none");
    assert_eq!(shows_tip(&fixture), None);
}

/// The step arrows scroll under the pointer but take no keyboard focus: Tab never stops on them, so the canvas keeps
/// its own keys and an Escape reaches what the canvas shows (a tool's edit), as in the reference application.
#[trial]
fn the_document_scroll_arrows_take_no_keyboard_focus() {
    let (mut fixture, view, _) = document_scrollbars();
    let start = view.get().center;
    let arrow = fixture.by_label("Scroll right").expect("the right arrow").center();
    fixture.frame(vec![pointer::moved(arrow)]).frame(vec![pointer::press(arrow)]).frame(vec![pointer::release(arrow)]);
    assert!(view.get().center.x > start.x, "the arrow still steps");
    for step in 0..8 {
        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
        let on_arrow = fixture
            .focused_stop()
            .and_then(|(_, part)| part)
            .is_some_and(|part| matches!(format!("{:?}", part.key).as_str(), "PartKey(\"vertical\")" | "PartKey(\"horizontal\")"));
        assert!(!on_arrow, "Tab {step} stopped on a scroll bar part: {:?}", fixture.focused_stop());
    }
}

/// Document scroll bars over a document larger than the window, at 100%, with or without the document chrome.
fn large_document_scrollbars(chrome: bool) -> (Fixture, Rc<Cell<CanvasView>>) {
    let large = Size::new(4000.0, 3000.0);
    let view = Rc::new(Cell::new(CanvasView::new(DOCUMENT_WINDOW, Point::new(2000.0, 1500.0), 1.0)));
    let observed = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut next = observed.get();
        Canvas::new("Document").document(large).view(next).show_at(ui, DOCUMENT_WINDOW, |_, body| {
            if let Some(space) = canvas_space(&body.env) {
                next = space.view;
            }
        });
        let doc = DocRect::from_origin_size(Point::ZERO, large);
        let bars = CanvasScrollbars::new("Scroll bars", Bind::bound(&mut next), doc);
        let bars = if chrome { bars.document_chrome() } else { bars };
        let _ = bars.show_at(ui, DOCUMENT_WINDOW);
        observed.set(next);
    });
    fixture.room(vec2(760.0, 560.0)).settle();
    (fixture, view)
}

/// photocraft#16006: a drag on a thumb moves the view on every pointer move until the release, not only on the first.
#[trial]
fn a_thumb_drag_follows_every_pointer_move() {
    for chrome in [false, true] {
        let (mut fixture, view) = large_document_scrollbars(chrome);
        let track = fixture.by_label("Horizontal scroll bar").expect("the horizontal track").rect;
        let start = pos2(track.center().x, track.center().y);
        let before = view.get().center;
        fixture.frame(vec![pointer::moved(start)]).frame(vec![pointer::press(start)]);
        let mut seen = vec![view.get().center.x];
        for step in 1..=10 {
            fixture.frame(vec![pointer::moved(start + vec2(20.0 * step as f32, 0.0))]);
            seen.push(view.get().center.x);
        }
        fixture.frame(vec![pointer::release(start + vec2(200.0, 0.0))]).settle();
        let moves = seen.windows(2).filter(|pair| pair[1] > pair[0]).count();
        assert!(moves >= 9, "chrome {chrome}: the view follows each of the ten moves: {seen:?} from {before:?}");
        assert_eq!(view.get().center.y, before.y, "chrome {chrome}: a horizontal drag keeps the vertical position");
    }
}

/// Document scroll bars over a `document` centred in the window at `zoom`, with the canvas and the bars following
/// `overscroll`.
fn centred_scrollbars(document: Size, zoom: f64, overscroll: bool) -> (Fixture, Rc<Cell<CanvasView>>) {
    let view = Rc::new(Cell::new(CanvasView::new(DOCUMENT_WINDOW, Point::new(document.width / 2.0, document.height / 2.0), zoom)));
    let observed = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut next = observed.get();
        let mut navigation = NavigationOptions::default();
        navigation.overscroll = overscroll;
        Canvas::new("Document").document(document).view(next).navigation(navigation).show_at(ui, DOCUMENT_WINDOW, |_, body| {
            if let Some(space) = canvas_space(&body.env) {
                next = space.view;
            }
        });
        let doc = DocRect::from_origin_size(Point::ZERO, document);
        let _ =
            CanvasScrollbars::new("Scroll bars", Bind::bound(&mut next), doc).overscroll(overscroll).document_chrome().show_at(ui, DOCUMENT_WINDOW);
        observed.set(next);
    });
    fixture.room(vec2(760.0, 560.0)).settle();
    (fixture, view)
}

/// Presses the middle of the track `label` (where a centred view's thumb is), drags by `by` and lets go.
fn drag_thumb(fixture: &mut Fixture, label: &str, by: Vec2) {
    let track = fixture.by_label(label).expect("the track").rect;
    let start = track.center();
    fixture.frame(vec![pointer::moved(start)]).frame(vec![pointer::press(start)]);
    for step in 1..=20 {
        fixture.frame(vec![pointer::moved(start + by * (step as f32 / 20.0))]);
    }
    fixture.frame(vec![pointer::release(start + by)]).settle();
}

/// The first and last document coordinate the view shows along `axis`.
fn shown_range(view: &CanvasView, axis: Axis) -> (f64, f64) {
    let (center, size) = match axis {
        Axis::X => (view.center.x, f64::from(view.rect.width())),
        _ => (view.center.y, f64::from(view.rect.height())),
    };
    let half = size / view.zoom / 2.0;
    (center - half, center + half)
}

/// With Overscroll on, a thumb at either end of its track leaves 100 screen points of the document in view at the
/// opposite edge, as the reference application does, at any zoom, for documents larger and smaller than the window.
/// With Overscroll off the ends stay the document's own edges.
#[trial]
fn a_thumb_at_either_end_leaves_100_points_of_the_document_in_view_with_overscroll() {
    for (document, zoom) in
        [(Size::new(4000.0, 3000.0), 1.0), (Size::new(4000.0, 3000.0), 2.0), (Size::new(4000.0, 3000.0), 0.5), (Size::new(300.0, 200.0), 1.0)]
    {
        for (label, axis, toward_start, toward_end) in [
            ("Horizontal scroll bar", Axis::X, vec2(-3000.0, 0.0), vec2(3000.0, 0.0)),
            ("Vertical scroll bar", Axis::Y, vec2(0.0, -3000.0), vec2(0.0, 3000.0)),
        ] {
            let len = if axis == Axis::X { document.width } else { document.height };
            let kept = 100.0 / zoom;
            let (mut fixture, view) = centred_scrollbars(document, zoom, true);
            drag_thumb(&mut fixture, label, toward_start);
            let (_, last) = shown_range(&view.get(), axis);
            assert!((last - kept).abs() < 1.0, "{document:?} at {zoom}, {axis:?} start: the first {kept} units stay in view, shows up to {last}");
            let (mut fixture, view) = centred_scrollbars(document, zoom, true);
            drag_thumb(&mut fixture, label, toward_end);
            let (first, _) = shown_range(&view.get(), axis);
            assert!(
                (first - (len - kept)).abs() < 1.0,
                "{document:?} at {zoom}, {axis:?} end: the last {kept} units stay in view, shows from {first}"
            );
        }
    }
    let large = Size::new(4000.0, 3000.0);
    let (mut fixture, view) = centred_scrollbars(large, 1.0, false);
    drag_thumb(&mut fixture, "Horizontal scroll bar", vec2(-3000.0, 0.0));
    assert!(shown_range(&view.get(), Axis::X).0.abs() < 1.0, "Overscroll off stops at the document's left edge: {:?}", view.get());
    let (mut fixture, view) = centred_scrollbars(large, 1.0, false);
    drag_thumb(&mut fixture, "Horizontal scroll bar", vec2(3000.0, 0.0));
    assert!((shown_range(&view.get(), Axis::X).1 - large.width).abs() < 1.0, "and at its right edge: {:?}", view.get());
}

/// A thumb dragged past the end of its track and back in the same drag moves again once the pointer is back over it,
/// and lands where a drag straight to that point puts it: the thumb stays under the pointer, as in the reference
/// application.
#[trial]
fn a_thumb_dragged_past_the_end_and_back_follows_the_pointer_in_the_same_drag() {
    let large = Size::new(4000.0, 3000.0);
    let (mut fixture, straight) = centred_scrollbars(large, 1.0, true);
    drag_thumb(&mut fixture, "Horizontal scroll bar", vec2(60.0, 0.0));
    let straight = straight.get().center.x;

    let (mut fixture, view) = centred_scrollbars(large, 1.0, true);
    let track = fixture.by_label("Horizontal scroll bar").expect("the track").rect;
    let start = track.center();
    fixture.frame(vec![pointer::moved(start)]).frame(vec![pointer::press(start)]);
    let mut seen = Vec::new();
    for step in 1..=30 {
        fixture.frame(vec![pointer::moved(start + vec2(40.0 * step as f32, 0.0))]);
        seen.push(view.get().center.x);
    }
    let at_end = view.get().center.x;
    let mut back = Vec::new();
    for step in (0..30).rev() {
        fixture.frame(vec![pointer::moved(start + vec2(40.0 * step as f32 + 60.0, 0.0))]);
        back.push(view.get().center.x);
    }
    fixture.frame(vec![pointer::release(start + vec2(60.0, 0.0))]).settle();
    assert!(back.iter().any(|&x| x < at_end - 1.0), "the drag moves back: past the end {seen:?}, back {back:?}");
    assert!((view.get().center.x - straight).abs() < 1.0, "back where a straight drag puts it: {} vs {straight}", view.get().center.x);
}

/// A thumb's pressed extent lasts only as long as its press: after the release the bars follow the view again. Here
/// the view zooms out after a thumb drag, and page presses on the track reach the new end (100 screen points of the
/// document left in view), as they do when no drag came first. The kept extent used to hold the range at the drag's
/// zoom (photocraft#16328).
#[trial]
fn after_a_thumb_drag_the_bars_follow_a_new_zoom() {
    let large = Size::new(4000.0, 3000.0);
    let zoomed_out = |view: CanvasView| CanvasView::new(view.rect, Point::new(large.width / 2.0, large.height / 2.0), 0.25);
    let page_to_the_end = |fixture: &mut Fixture| {
        let track = fixture.by_label("Horizontal scroll bar").expect("the track").rect;
        for _ in 0..40 {
            fixture.click_at(pos2(track.right() - 3.0, track.center().y));
            fixture.settle();
        }
    };
    let (mut fixture, view) = centred_scrollbars(large, 1.0, true);
    view.set(zoomed_out(view.get()));
    fixture.settle();
    page_to_the_end(&mut fixture);
    let fresh = shown_range(&view.get(), Axis::X).0;
    assert!((fresh - (large.width - 400.0)).abs() < 1.0, "paging reaches the end: the last 400 units stay in view, shows from {fresh}");

    let (mut fixture, view) = centred_scrollbars(large, 1.0, true);
    drag_thumb(&mut fixture, "Horizontal scroll bar", vec2(40.0, 0.0));
    view.set(zoomed_out(view.get()));
    fixture.settle();
    page_to_the_end(&mut fixture);
    let first = shown_range(&view.get(), Axis::X).0;
    assert!((first - fresh).abs() < 1.0, "after a thumb drag paging reaches the same end: shows from {first}, without the drag {fresh}");
}

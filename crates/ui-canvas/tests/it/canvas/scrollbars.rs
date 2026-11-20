//! `CanvasScrollbars`: the clamp of the view and the bars' drag and page press.

use std::{cell::Cell, rc::Rc};

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_ui::canvas::{Canvas, CanvasScrollbars, NavigationOptions, Rulers, canvas_space};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
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

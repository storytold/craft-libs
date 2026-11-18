//! `CanvasScrollbars`: the clamp of the view and the bars' drag and page press.

use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_behavior::{Axis, CanvasView};
use craft_ui::canvas::{Canvas, CanvasScrollbars};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::env::Label;
use craft_ui::testing::input::slop;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

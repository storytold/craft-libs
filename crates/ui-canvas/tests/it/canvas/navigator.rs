//! The Navigator's preview keys: Home centres the document. A zoom typed in the Navigator lands exactly on the canvas.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::navigator_panel::NavigatorPanel;
use craft_ui::canvas::{AppPaint, Canvas, CanvasSpace, NavigationOptions, Navigator, NavigatorPreview};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, Rect, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;

const DOCUMENT: Size = Size::new(2000.0, 1500.0);

#[trial]
fn home_on_the_focused_preview_centres_the_document_and_end_leaves_the_view() {
    let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let view = Rc::new(Cell::new(CanvasView::new(canvas, Point::new(400.0, 300.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        let _ = Navigator::new("Navigator", Bind::bound(&mut given), DOCUMENT).show(ui);
        shown.set(given);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    assert!(fixture.focused_stop().is_some(), "Tab reaches the preview");
    let before = view.get().center;
    fixture.key(Key::End, Modifiers::NONE);
    assert_eq!(view.get().center, before, "End leaves the view where it is");
    fixture.key(Key::Home, Modifiers::NONE);
    assert_eq!(view.get().center, Point::new(DOCUMENT.width / 2.0, DOCUMENT.height / 2.0), "Home centres the document");
}

/// What the app of [`panel_over_canvas`] keeps between passes.
#[derive(Clone)]
struct Kept {
    /// The view the app keeps: the canvas's target.
    view: Rc<Cell<CanvasView>>,
    /// The space the canvas showed last pass.
    space: Rc<Cell<Option<CanvasSpace>>>,
    /// The interface scale the panel's field reads.
    pixels_per_point: Rc<Cell<f32>>,
}

/// A Navigator panel above a canvas of [`DOCUMENT`], both over one view, as an app wires them: a zoom the panel makes
/// eases on the canvas from the view drawn before it, and the canvas jumps instead with Animated Zoom off.
fn panel_over_canvas(animated: bool) -> (Fixture, Kept) {
    let kept = Kept {
        view: Rc::new(Cell::new(CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0))),
        space: Rc::new(Cell::new(None)),
        pixels_per_point: Rc::new(Cell::new(1.0)),
    };
    let mut navigation = NavigationOptions::default();
    navigation.animated_zoom = animated;
    let app = kept.clone();
    let mut fixture = Fixture::new(move |ui| {
        app.pixels_per_point.set(Env::of(ui).scale.0);
        let before = app.view.get();
        let mut given = before;
        let _ = NavigatorPanel::new("Navigator", Bind::bound(&mut given), DOCUMENT).show(ui);
        let mut canvas = Canvas::new("Document").document(DOCUMENT).navigation(navigation).view(given);
        if given.zoom != before.zoom {
            canvas = canvas.ease_from(before, None);
        }
        let shown = canvas.show(ui, |_, _| {});
        app.view.set(shown.output.view.unwrap_or(given));
        app.space.set(shown.output.space);
    });
    fixture.room(vec2(900.0, 900.0));
    fixture.settle();
    (fixture, kept)
}

/// A typed zoom lands on the exact percent with Animated Zoom on and off: the view the app keeps, the view the canvas
/// shows once its ease ends, and the field's text. An ease that ends a step short of `zoom::with_device`'s target
/// would show 1599.99 %.
#[trial]
fn a_typed_zoom_lands_exactly_after_its_ease() {
    for animated in [true, false] {
        let (mut fixture, kept) = panel_over_canvas(animated);
        for typed in [1600.0, 200.0, 6400.0] {
            let text = format!("{typed}");
            let field = fixture.by_label("Zoom").expect("the Navigator's zoom field");
            fixture.click(&field);
            fixture.key(Key::A, Modifiers::COMMAND);
            fixture.type_text(&text);
            fixture.key(Key::Enter, Modifiers::NONE);
            // Frames well past the ease's length, with the clock moving as an app's does.
            for _ in 0..40 {
                fixture.advance(0.05).frame(Vec::new());
            }
            fixture.settle();
            let ppp = kept.pixels_per_point.get();
            let percent = |view: &CanvasView| zoom::device(view, ppp) * 100.0;
            assert_eq!(percent(&kept.view.get()), typed, "the kept view; animated: {animated}");
            let space = kept.space.get().expect("a canvas given a view publishes its space");
            assert!(!space.easing(), "the ease has ended; animated: {animated}");
            assert_eq!(percent(&space.view), typed, "the shown view; animated: {animated}");
            let field = fixture.by_label("Zoom").expect("the Navigator's zoom field");
            assert_eq!(fixture.tree().run_texts_of(field.node).concat(), text, "the field's text; animated: {animated}");
        }
    }
}

/// The fitted rect the panel's preview paints the document into, in a room of `room`.
fn fitted_in(room: egui::Vec2) -> Rect {
    let fitted = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&fitted);
    let view = Rc::new(Cell::new(CanvasView::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)), Point::new(1000.0, 750.0), 1.0)));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = view.get();
        let seen = Rc::clone(&seen);
        let paint = AppPaint::new(move |_, rect: Rect| seen.set(rect));
        let _ = NavigatorPanel::new("Navigator", Bind::bound(&mut given), DOCUMENT).preview(NavigatorPreview::Paint(paint)).show(ui);
        view.set(given);
    });
    fixture.room(room);
    fixture.settle();
    fitted.get()
}

#[trial]
fn panel_preview_grows_with_the_room_and_keeps_the_document_aspect() {
    let aspect = DOCUMENT.width as f32 / DOCUMENT.height as f32;
    let small = fitted_in(vec2(200.0, 260.0));
    let large = fitted_in(vec2(400.0, 700.0));
    assert!(large.width() > small.width() && large.height() > small.height(), "the preview grows with the panel: {small:?} -> {large:?}");
    for fitted in [small, large] {
        assert!((fitted.width() / fitted.height() - aspect).abs() < 0.02, "the document keeps its aspect: {fitted:?}");
    }
    let tall = fitted_in(vec2(200.0, 700.0));
    assert!(tall.height() > small.height() || tall.width() <= small.width() + 0.5, "extra height alone never widens the fit: {small:?} {tall:?}");
    assert!(fitted_in(vec2(200.0, 260.0)).width() <= small.width() + 0.5, "the preview shrinks back with the panel");
}

/// photocraft#16403: a drag of the view box moves the canvas on every pointer move, not only on the first and the
/// release. The canvas settles the view it is given to whole device pixels, so the view the app keeps differs from the
/// preview's proposal by a fraction of a pixel; that is the drag's own value, not the app changing the view under it.
#[trial]
fn a_view_box_drag_follows_every_pointer_move_over_a_settling_canvas() {
    let view = Rc::new(Cell::new(CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0)));
    let fitted = Rc::new(Cell::new(Rect::NOTHING));
    let (app, seen) = (Rc::clone(&view), Rc::clone(&fitted));
    let mut fixture = Fixture::new(move |ui| {
        let mut given = app.get();
        let seen = Rc::clone(&seen);
        let paint = AppPaint::new(move |_, rect: Rect| seen.set(rect));
        let _ = NavigatorPanel::new("Navigator", Bind::bound(&mut given), DOCUMENT).preview(NavigatorPreview::Paint(paint)).show(ui);
        let shown = Canvas::new("Document").document(DOCUMENT).view(given).show(ui, |_, _| {});
        // The app keeps the view as the canvas settles it: its centre on whole device pixels (zoom 1, one pixel a point).
        let mut kept = shown.output.view.unwrap_or(given);
        kept.center = Point::new(kept.center.x.round(), kept.center.y.round());
        app.set(kept);
    });
    fixture.room(vec2(900.0, 900.0));
    fixture.settle();
    let box_rect = fitted.get();
    assert!(box_rect.is_positive(), "the preview painted the document: {box_rect:?}");
    let before = view.get();
    let at = |center: Point| {
        pos2(
            box_rect.min.x + (center.x / DOCUMENT.width) as f32 * box_rect.width(),
            box_rect.min.y + (center.y / DOCUMENT.height) as f32 * box_rect.height(),
        )
    };
    let start = at(before.center);
    fixture.frame(vec![pointer::moved(start)]).frame(vec![pointer::press(start)]);
    let mut seen = vec![view.get().center.x];
    for step in 1..=10 {
        fixture.frame(vec![pointer::moved(start + vec2(3.0 * step as f32, 0.0))]);
        seen.push(view.get().center.x);
    }
    fixture.frame(vec![pointer::release(start + vec2(30.0, 0.0))]).settle();
    let moves = seen.windows(2).filter(|pair| pair[1] > pair[0]).count();
    assert!(moves >= 9, "the canvas follows each of the ten moves: {seen:?} from {:?}", before.center);
}

/// A Navigator shown over a view that the canvas, its scroll bars or the Hand put past the document (the pasteboard,
/// with Overscroll on) leaves that view where it is: pulling it back into the document every pass stopped the scroll
/// bars at the document's edge and ended their drags (photocraft#16328). A move of its own still keeps the centre
/// inside the document.
#[trial]
fn a_view_past_the_document_stays_while_the_navigator_shows_it() {
    let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let past = Point::new(DOCUMENT.width + 300.0, -200.0);
    let view = Rc::new(Cell::new(CanvasView::new(canvas, past, 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        let _ = Navigator::new("Navigator", Bind::bound(&mut given), DOCUMENT).show(ui);
        shown.set(given);
    });
    fixture.settle();
    fixture.idle(3);
    assert_eq!(view.get().center, past, "the Navigator leaves a view past the document where it is");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::Home, Modifiers::NONE);
    assert_eq!(view.get().center, Point::new(DOCUMENT.width / 2.0, DOCUMENT.height / 2.0), "Home still centres the document");
}

//! The Navigator's preview keys: Home centres the document. A zoom typed in the Navigator lands exactly on the canvas.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::navigator_panel::NavigatorPanel;
use craft_ui::canvas::{Canvas, CanvasSpace, NavigationOptions, Navigator};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
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
            assert_eq!(fixture.tree().value_of(field.node), Some(text.as_str()), "the field's text; animated: {animated}");
        }
    }
}

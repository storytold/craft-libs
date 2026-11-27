//! The Navigator's preview keys: Home centres the document. A zoom typed in the Navigator lands exactly on the canvas.

use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, ColumnWidth, EditEvent};
use craft_ui::buttons::Button;
use craft_ui::canvas::navigator_panel::NavigatorPanel;
use craft_ui::canvas::{AppPaint, Canvas, CanvasSpace, NavigationOptions, Navigator, NavigatorPreview, NavigatorZoomRow};
use craft_ui::contract::{Bind, Scroll, Show as _, ShowContainer as _, WrapWidth};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Target, pointer, trial};
use craft_ui::value_drag::Slider;
use egui::{Key, Modifiers, Rect, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

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
    /// Whether the panel has cancelled an edit.
    navigator_cancelled: Rc<Cell<bool>>,
}

/// A Navigator panel above a canvas of [`DOCUMENT`], both over one view, as an app wires them: a zoom the panel makes
/// eases on the canvas from the view drawn before it, and the canvas jumps instead with Animated Zoom off.
fn panel_over_canvas(animated: bool) -> (Fixture, Kept) {
    let kept = Kept {
        view: Rc::new(Cell::new(CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0))),
        space: Rc::new(Cell::new(None)),
        pixels_per_point: Rc::new(Cell::new(1.0)),
        navigator_cancelled: Rc::new(Cell::new(false)),
    };
    let mut navigation = NavigationOptions::default();
    navigation.animated_zoom = animated;
    let app = kept.clone();
    let mut fixture = Fixture::new(move |ui| {
        app.pixels_per_point.set(Env::of(ui).scale.0);
        let before = app.view.get();
        let mut given = before;
        let panel = NavigatorPanel::new("Navigator", Bind::bound(&mut given), DOCUMENT).show(ui);
        if panel.view.into_iter().any(|event| matches!(event, EditEvent::Cancel { .. })) {
            app.navigator_cancelled.set(true);
        }
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

fn zoom_controls(fixture: &Fixture) -> [Target; 4] {
    [
        fixture.by_role(Role::SPIN_BUTTON)[0],
        fixture.by_label("Zoom Out").expect("Zoom Out is labeled"),
        fixture.by_role(Role::SLIDER)[0],
        fixture.by_label("Zoom In").expect("Zoom In is labeled"),
    ]
}

#[trial]
fn a_narrow_navigator_keeps_every_zoom_control_in_view() {
    let (mut fixture, kept) = panel_over_canvas(false);
    let percent = || zoom::device(&kept.view.get(), kept.pixels_per_point.get()) * 100.0;
    let field = fixture.by_role(Role::SPIN_BUTTON)[0];
    fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("250").settle();
    let editing = fixture.focused_stop();
    assert!(editing.is_some());
    for width in (72..=224).rev().map(|width| width as f32) {
        let room = vec2(width, 380.0);
        fixture.room(room).settle();
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, room);
        let [field, minus, slider, plus] = zoom_controls(&fixture);
        if width == 120.0 {
            assert!(slider.rect.y_range().contains(minus.rect.center().y), "the slider fits beside Zoom Out at {width} points");
            assert!(slider.rect.y_range().contains(plus.rect.center().y), "the slider fits beside Zoom In at {width} points");
        }
        for (name, target) in [("numeric zoom", field), ("slider", slider), ("Zoom Out", minus), ("Zoom In", plus)] {
            assert!(bounds.contains_rect(target.rect), "{name} is outside {bounds:?}: {:?}", target.rect);
        }
        assert_eq!(fixture.focused_stop(), editing, "resizing keeps keyboard focus in the field");
    }
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(percent(), 250.0, "the entry survives the reflow and applies on the canvas");

    let minus = fixture.by_label("Zoom Out").unwrap();
    fixture.click(&minus).settle();
    let reduced = percent();
    assert!(reduced < 250.0, "Zoom Out steps the canvas: {reduced}");
    let plus = fixture.by_label("Zoom In").unwrap();
    fixture.click(&plus).settle();
    let increased = percent();
    assert!(increased > reduced, "Zoom In steps the canvas: {increased}");

    let slider = fixture.by_role(Role::SLIDER)[0];
    let thumb = pointer::part_rect_of_node(fixture.ctx(), slider.node.into(), Slider::THUMB).expect("the slider thumb is shown");
    fixture.drag_at(thumb.center(), vec2((slider.rect.width() * 0.3).max(12.0), 0.0)).settle();
    assert_ne!(percent(), increased, "dragging the slider changes the canvas zoom: {:?}, thumb {thumb:?}", slider.rect);
    let dragged = percent();
    fixture.key(Key::ArrowLeft, Modifiers::NONE).settle();
    assert!(percent() < dragged, "the slider keeps keyboard input after a drag");
    let slider_focus = fixture.focused_stop();
    fixture.room(vec2(120.0, 380.0)).settle();
    assert_eq!(fixture.focused_stop(), slider_focus, "rejoining the row keeps the slider focused");

    let mut external = kept.view.get();
    external.zoom = 1.5;
    kept.view.set(external);
    fixture.settle();
    let field = fixture.by_role(Role::SPIN_BUTTON)[0];
    let expected = percent().round().to_string();
    assert_eq!(fixture.tree().run_texts_of(field.node).concat(), expected, "the field follows the canvas zoom");

    fixture.room(vec2(120.0, 175.0)).settle();
    let plus = fixture.by_label("Zoom In").unwrap();
    fixture.access(AccessActionKind::Focus, &plus).settle();
    let button_focus = fixture.focused_stop();
    assert!(button_focus.is_some(), "the zoom button takes keyboard focus");
    let room = vec2(72.0, 110.0);
    fixture.room(room).settle();
    let bounds = Rect::from_min_size(egui::Pos2::ZERO, room);
    for target in zoom_controls(&fixture) {
        assert!(bounds.contains_rect(target.rect), "zoom controls fit below a short preview: {:?}", target.rect);
    }
    assert_eq!(fixture.focused_stop(), button_focus, "Zoom In keeps focus when its row wraps");
    let before = percent();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(percent() > before, "the focused button still zooms from the keyboard");
}

#[trial]
fn a_very_short_navigator_reveals_its_focused_zoom_button() {
    let (mut fixture, kept) = panel_over_canvas(false);
    fixture.room(vec2(72.0, 135.0)).settle();
    let plus = fixture.by_label("Zoom In").expect("Zoom In remains available to assistive technology");
    fixture.access(AccessActionKind::Focus, &plus).settle();
    let room = vec2(72.0, 80.0);
    fixture.room(room).settle();
    let plus = fixture.by_label("Zoom In").unwrap();
    let bounds = Rect::from_min_size(egui::Pos2::ZERO, room);
    assert!(bounds.contains_rect(plus.rect), "resizing reveals the focused Zoom In button: {plus:?}");
    assert_eq!(fixture.tree().focused_key(), plus.node);
    let before = kept.view.get().zoom;
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(kept.view.get().zoom > before);
}

#[trial]
fn a_zoom_slider_narrower_than_its_themed_thumb_still_has_pointer_travel() {
    let (mut fixture, kept) = panel_over_canvas(false);
    let sizes = br#"{"all":{"size.icon.lg":32,"size.icon.md":32,"size.icon.glyph_md":32}}"#;
    let mut root = RootEnv::default();
    root.sizes = Arc::new(craft_theme::AppSizes::parse(sizes).unwrap());
    fixture.root(root).room(vec2(32.0, 380.0)).settle();
    let slider = fixture.by_role(Role::SLIDER)[0];
    let thumb = pointer::part_rect_of_node(fixture.ctx(), slider.node.into(), Slider::THUMB).expect("the thumb is drawn");
    assert!(slider.rect.width() > thumb.width(), "the narrow viewport can scroll a track with pointer travel");
    let before = kept.view.get().zoom;
    fixture.drag_at(thumb.center(), vec2(12.0, 0.0)).settle();
    assert!(kept.view.get().zoom > before, "dragging the visible thumb still changes zoom");
}

#[trial]
fn a_standalone_navigator_zoom_row_leaves_room_for_a_wrapping_sibling() {
    let view = CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        Row::new().fill_width().wrap(true).show(ui, |ui, _| {
            let _ = NavigatorZoomRow::new(Bind::given(view), DOCUMENT).show(ui);
            Button::new("Sibling").show(ui);
        });
        Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |_, _| {});
    });
    fixture.room(vec2(420.0, 300.0)).settle();
    let [field, _, _, plus] = zoom_controls(&fixture);
    let sibling = fixture.by_label("Sibling").unwrap();
    assert!(sibling.rect.left() > plus.rect.right(), "the next control fits to the right of the zoom row: {sibling:?}");
    assert!(field.rect.y_range().contains(sibling.center().y), "the next control stays on the same line");
}

#[trial]
fn a_navigator_zoom_row_yields_width_to_an_unwrapped_sibling() {
    let view = CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        Row::new().fill_width().show(ui, |ui, _| {
            let _ = NavigatorZoomRow::new(Bind::given(view), DOCUMENT).show(ui);
            Button::new("Sibling").show(ui);
        });
        Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |_, _| {});
    });
    let room = vec2(180.0, 300.0);
    fixture.room(room).settle();
    let bounds = Rect::from_min_size(egui::Pos2::ZERO, room);
    let sibling = fixture.by_label("Sibling").unwrap();
    assert!(bounds.contains_rect(sibling.rect), "the sibling remains inside the room: {sibling:?}");
    for control in zoom_controls(&fixture) {
        assert!(control.rect.right() <= sibling.rect.left(), "the zoom control fits to the left of the sibling: {control:?}, {sibling:?}");
    }
}

#[trial]
fn a_preview_that_collapses_during_a_drag_accepts_navigation_again() {
    for modifiers in [Modifiers::NONE, Modifiers::COMMAND] {
        let (mut fixture, kept) = panel_over_canvas(false);
        let room = vec2(240.0, 380.0);
        let mut view = kept.view.get();
        view.center = Point::new(400.0, 300.0);
        kept.view.set(view);
        fixture.room(room).settle();
        let preview = fixture.by_label("Navigator").expect("the preview is visible");
        fixture.access(AccessActionKind::Focus, &preview).settle();
        let from = preview.center();
        let delta = vec2(26.0, 14.0);
        fixture.hold(modifiers).drag_hold_at(from, delta);
        assert_eq!(fixture.tree().focused_key(), preview.node, "the preview has focus before it collapses");
        fixture.room(vec2(72.0, 112.0)).settle();
        assert!(fixture.by_label("Navigator").is_none(), "the inset leaves too little room to expose the preview");
        let field = fixture.by_role(Role::SPIN_BUTTON)[0];
        assert_eq!(fixture.tree().focused_key(), field.node, "focus moves to numeric zoom");
        assert!(fixture.by_label("Visible area horizontal position").is_none(), "the collapsed preview stops exposing its scroll bars");
        if modifiers == Modifiers::NONE {
            assert!(kept.navigator_cancelled.get(), "a pan ends while the preview is collapsed");
        }
        fixture.release_at(from + delta).hold(Modifiers::NONE).settle();
        fixture.room(room).settle();
        let preview = fixture.by_label("Navigator").expect("the preview is visible again");
        fixture.access(AccessActionKind::Focus, &preview).settle();
        assert_ne!(kept.view.get().center, Point::new(DOCUMENT.width / 2.0, DOCUMENT.height / 2.0));
        fixture.key(Key::Home, Modifiers::NONE).settle();
        assert_eq!(kept.view.get().center, Point::new(DOCUMENT.width / 2.0, DOCUMENT.height / 2.0), "Home works after {modifiers:?} drag");
    }
}

#[trial]
fn a_navigator_zoom_row_reflows_to_its_grid_cell() {
    const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Fixed(76.0), ColumnWidth::Fixed(300.0)];
    let view = CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        Grid::new(COLUMNS).show(ui, |ui, _| {
            let _ = NavigatorZoomRow::new(Bind::given(view), DOCUMENT).show(ui);
            Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |_, _| {});
        });
    });
    fixture.text_size(craft_theme::TextSize::Large).room(vec2(400.0, 300.0)).settle();
    for target in zoom_controls(&fixture) {
        assert!(target.rect.right() <= 76.0, "zoom control stays in its cell: {:?}", target.rect);
    }
}

#[trial]
fn a_navigator_zoom_row_keeps_one_line_when_its_controls_fit() {
    let view = CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        let _ = NavigatorZoomRow::new(Bind::given(view), DOCUMENT).show(ui);
        Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |_, _| {});
    });
    fixture.density(craft_theme::Size::Comfortable).text_size(craft_theme::TextSize::Medium);
    for width in [213.0, 214.0, 240.0, 300.0] {
        fixture.room(vec2(width, 300.0)).settle();
        let [field, minus, slider, plus] = zoom_controls(&fixture);
        for control in [minus, slider, plus] {
            assert!(field.rect.y_range().contains(control.rect.center().y), "all controls fit on one line at {width} points: {field:?} {control:?}");
            assert!(control.rect.right() <= width, "control fits its room at {width} points: {control:?}");
        }
    }
    let room = vec2(96.0, 300.0);
    fixture.room(room).settle();
    let bounds = Rect::from_min_size(egui::Pos2::ZERO, room);
    for control in zoom_controls(&fixture) {
        assert!(bounds.contains_rect(control.rect), "large controls still fit a narrow room: {control:?}");
    }

    let mut root = RootEnv::default();
    root.sizes = Arc::new(craft_theme::AppSizes::parse(br#"{"all":{"space.gap.row":32}}"#).unwrap());
    fixture.root(root).room(vec2(144.0, 300.0)).settle();
    let [_, minus, slider, plus] = zoom_controls(&fixture);
    for button in [minus, plus] {
        assert!(slider.rect.y_range().contains(button.rect.center().y), "the large row gap stays outside the slider: {button:?}");
    }
}

#[trial]
fn a_navigator_zoom_row_keeps_its_natural_width_in_content_hosts() {
    let view = CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0);
    for scrolling in [false, true] {
        let measured = Rc::new(Cell::new(Rect::NOTHING));
        let seen = Rc::clone(&measured);
        let mut fixture = Fixture::new(move |ui| {
            let host = Stack::new();
            let host = if scrolling { host.scroll(Scroll::horizontal()) } else { host };
            let shown = host.show(ui, |ui, _| {
                let _ = NavigatorZoomRow::new(Bind::given(view), DOCUMENT).show(ui);
            });
            seen.set(shown.output.rect);
            Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |_, _| {});
        });
        fixture.room(vec2(900.0, 300.0)).settle();
        let [field, _, slider, plus] = zoom_controls(&fixture);
        assert!(slider.rect.width() <= 148.0, "the slider keeps its natural track in a content host: {slider:?}");
        assert!(plus.rect.right() - field.rect.left() <= 300.0, "Zoom In stays beside the natural track: {plus:?}; scrolling: {scrolling}");
        if !scrolling {
            assert!(measured.get().width() <= 300.0, "the host sizes to the controls: {:?}", measured.get());
        }
    }
}

#[trial]
fn a_navigator_zoom_row_follows_the_host_line_and_themed_button_sizes() {
    let view = CanvasView::new(Rect::ZERO, Point::new(1000.0, 750.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().wrap_width(WrapWidth::Column).show(ui, |ui, _| {
            let _ = NavigatorZoomRow::new(Bind::given(view), DOCUMENT).show(ui);
        });
        Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |_, _| {});
    });
    let sizes = br#"{
        "all": { "size.dock.column": 210, "size.icon.lg": 32, "size.icon.md": 32, "size.icon.glyph_md": 32 }
    }"#;
    let mut root = RootEnv::default();
    root.sizes = Arc::new(craft_theme::AppSizes::parse(sizes).unwrap());
    fixture.root(root).room(vec2(900.0, 300.0)).settle();
    let [field, minus, slider, plus] = zoom_controls(&fixture);
    for control in [minus, slider, plus] {
        assert!(field.rect.y_range().contains(control.rect.center().y), "the controls share the host's 210-point line: {control:?}");
        assert!(control.rect.right() <= 210.0, "control fits the host's line: {control:?}");
    }
    for width in [84.0, 98.0] {
        let room = vec2(width, 300.0);
        fixture.room(room).settle();
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, room);
        let [_, minus, slider, plus] = zoom_controls(&fixture);
        assert!(minus.rect.width() > 24.0, "the larger glyph makes the button wider than a default control");
        let thumb = pointer::part_rect_of_node(fixture.ctx(), slider.node.into(), Slider::THUMB).expect("the themed thumb is drawn");
        assert!(slider.rect.width() > thumb.width() && slider.rect.height() >= thumb.height(), "the themed thumb has room to travel: {thumb:?}");
        assert!(minus.rect.bottom() <= slider.rect.top() && slider.rect.bottom() <= plus.rect.top(), "larger buttons and the slider stack");
        for control in [minus, slider, plus] {
            assert!(bounds.contains_rect(control.rect), "control fits the narrow room: {control:?}");
        }
    }
}

/// The fitted rect the panel's preview paints the document into, in a room of `room`.
fn fitted_in(room: egui::Vec2, content_height: bool) -> Rect {
    let fitted = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&fitted);
    let view = Rc::new(Cell::new(CanvasView::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)), Point::new(1000.0, 750.0), 1.0)));
    let show = move |ui: &mut egui::Ui| {
        let mut given = view.get();
        let seen = Rc::clone(&seen);
        let paint = AppPaint::new(move |_, rect: Rect| seen.set(rect));
        let _ = NavigatorPanel::new("Navigator", Bind::bound(&mut given), DOCUMENT).preview(NavigatorPreview::Paint(paint)).show(ui);
        view.set(given);
    };
    let mut fixture = Fixture::new(move |ui| {
        if content_height {
            Stack::new().fill_width().show(ui, |ui, _| show(ui));
        } else {
            show(ui);
        }
    });
    fixture.room(room);
    fixture.settle();
    fitted.get()
}

#[trial]
fn panel_preview_grows_with_the_room_and_keeps_the_document_aspect() {
    let aspect = DOCUMENT.width as f32 / DOCUMENT.height as f32;
    let small = fitted_in(vec2(200.0, 260.0), false);
    let large = fitted_in(vec2(400.0, 700.0), false);
    assert!(large.width() > small.width() && large.height() > small.height(), "the preview grows with the panel: {small:?} -> {large:?}");
    for fitted in [small, large] {
        assert!((fitted.width() / fitted.height() - aspect).abs() < 0.02, "the document keeps its aspect: {fitted:?}");
    }
    let tall = fitted_in(vec2(200.0, 700.0), false);
    assert!(tall.height() > small.height() || tall.width() <= small.width() + 0.5, "extra height alone never widens the fit: {small:?} {tall:?}");
    assert!(fitted_in(vec2(200.0, 260.0), false).width() <= small.width() + 0.5, "the preview shrinks back with the panel");
    let content = fitted_in(vec2(400.0, 700.0), true);
    assert!(content.height() >= small.height() * 0.5, "a content-height host still gets a useful preview: {content:?}");
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

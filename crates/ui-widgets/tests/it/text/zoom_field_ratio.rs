//! photocraft#16784: the zoom field reads a ratio typed with a colon, `a:b`, as the zoom `a ÷ b × 100` %, as it reads
//! `a/b`; plain numbers keep their reading.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::ZoomField;
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// The document the field zooms: its zoom range runs from below 1 % to 12800 %.
const DOCUMENT: Size = Size::new(2000.0, 1500.0);

/// The field's label.
const ZOOM: &str = "Zoom";

/// The zoom in percent after `entry` is typed into the focused field at 100 % and committed with Enter, and whether
/// the commit moved the document point at the view's centre.
fn zoom_after(entry: &str) -> (f64, bool) {
    let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let centre = Point::new(1000.0, 750.0);
    let view = Rc::new(Cell::new(CanvasView::new(canvas, centre, 1.0)));
    let pixels_per_point = Rc::new(Cell::new(1.0f32));
    let (app, scale) = (view.clone(), pixels_per_point.clone());
    let mut fixture = Fixture::new(move |ui| {
        scale.set(Env::of(ui).scale.0);
        let out = ZoomField::new(ZOOM, Some(app.get()), DOCUMENT).show(ui);
        if let Some(landed) = out.view.landed() {
            app.set(*landed);
        }
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(entry);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    let view = view.get();
    let moved = (view.to_doc(canvas.center()) - centre).hypot() > 1e-6;
    (zoom::device(&view, pixels_per_point.get()) * 100.0, moved)
}

/// Each row: the entry and the zoom it commits. A ratio past the zoom range is held to it, as every other entry is.
#[trial]
fn a_colon_ratio_is_the_zoom_a_over_b() {
    let rows = [("1:2", 50.0), ("1:4", 25.0), ("2:1", 200.0), ("1000:1", 12800.0), ("1/2", 50.0), ("45", 45.0)];
    for (entry, want) in rows {
        let (zoom, moved) = zoom_after(entry);
        assert!((zoom - want).abs() < 1e-6, "{entry}: {zoom} %, want {want} %");
        assert!(!moved, "{entry}: the zoom keeps the view's centre");
    }
}

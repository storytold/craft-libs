//! photocraft#16780: with `clamp_out_of_range` on, a zoom typed outside the range, zero and negative ones included,
//! applies the nearest end and reports the range for the "A number between ... is required" alert. Off, the field keeps
//! its rule: zero and negative entries keep the view, and only a cut entry is reported.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{InvalidZoom, ZoomField};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A document whose zoom range runs from below 1 % to 12800 %.
const LARGE: Size = Size::new(2000.0, 1500.0);

/// The field's label.
const ZOOM: &str = "Zoom";

/// What an entry left: the zoom in percent after Enter, the range reported and its message.
#[derive(Debug)]
struct After {
    zoom: f64,
    invalid: Option<InvalidZoom>,
    message: Option<String>,
}

/// Types `entry` into a focused zoom field at 100 % over `document` and presses Enter.
fn enter(document: Size, clamp: bool, entry: &str) -> After {
    let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let view = Rc::new(Cell::new(CanvasView::new(canvas, Point::new(document.width / 2.0, document.height / 2.0), 1.0)));
    let pixels_per_point = Rc::new(Cell::new(1.0f32));
    let reported = Rc::new(RefCell::new(None));
    let (app, scale, seen) = (view.clone(), pixels_per_point.clone(), reported.clone());
    let mut fixture = Fixture::new(move |ui| {
        scale.set(Env::of(ui).scale.0);
        let out = ZoomField::new(ZOOM, Some(app.get()), document).clamp_out_of_range(clamp).show(ui);
        if let Some(landed) = out.view.landed() {
            app.set(*landed);
        }
        if let Some(invalid) = out.invalid_zoom {
            let message = invalid.message(&Env::of(ui));
            *seen.borrow_mut() = Some((invalid, message));
        }
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(entry);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    let (invalid, message) = reported.borrow_mut().take().unzip();
    After { zoom: zoom::device(&view.get(), pixels_per_point.get()) * 100.0, invalid, message }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Past the top, the zoom is the top and the range is reported with the number message.
#[trial]
fn an_entry_past_the_top_applies_the_top_and_reports_the_range() {
    for entry in ["99999", "13000"] {
        let after = enter(LARGE, true, entry);
        assert!(close(after.zoom, 12800.0), "{entry}: {after:?}");
        let invalid = after.invalid.unwrap_or_else(|| panic!("{entry}: the range is reported"));
        assert!(invalid.low < 1.0 && close(invalid.high, 12800.0), "{entry}: {invalid:?}");
        let message = after.message.unwrap_or_default();
        assert!(message.starts_with("A number between ") && message.ends_with(" is required. Closest value inserted."), "{entry}: {message}");
    }
}

/// Below the bottom, zero and negative entries apply the bottom and report the range. The entries carry no decimal
/// separator, so they read the same in every locale.
#[trial]
fn zero_negative_and_tiny_entries_apply_the_bottom_and_report_the_range() {
    for entry in ["0", "1/1000", "-50"] {
        let after = enter(LARGE, true, entry);
        let invalid = after.invalid.clone().unwrap_or_else(|| panic!("{entry}: the range is reported: {after:?}"));
        assert!(close(after.zoom, invalid.low) && invalid.low < 1.0, "{entry}: {after:?}");
    }
    // A 10 x 10 document's range starts at 20 %.
    let small = enter(Size::new(10.0, 10.0), true, "1");
    let invalid = small.invalid.clone().unwrap_or_else(|| panic!("the range is reported: {small:?}"));
    assert!(close(small.zoom, 20.0) && close(invalid.low, 20.0), "{small:?}");
}

/// An entry at an end or inside the range is no alert.
#[trial]
fn an_entry_in_the_range_reports_nothing() {
    for (entry, zoom) in [("12800", 12800.0), ("250", 250.0), ("1/2", 50.0)] {
        let after = enter(LARGE, true, entry);
        assert!(close(after.zoom, zoom) && after.invalid.is_none(), "{entry}: {after:?}");
    }
}

/// Off (the default), the field keeps its rule: no report past the range, and zero or a negative keeps the view.
#[trial]
fn off_the_field_keeps_its_rule() {
    let top = enter(LARGE, false, "99999");
    assert!(close(top.zoom, 12800.0) && top.invalid.is_none(), "{top:?}");
    for entry in ["0", "-50"] {
        let after = enter(LARGE, false, entry);
        assert!(close(after.zoom, 100.0) && after.invalid.is_none(), "{entry}: {after:?}");
    }
}

//! photocraft#16773: Shift + Page Up / Page Down scroll 10 document pixels, rounded to whole device pixels, at any zoom
//! and display scaling; Cmd/Ctrl + Shift do the same sideways; the page keys without Shift keep their canvas page.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::{Canvas, PageKey};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// A focused canvas over a 3000 × 2000 document centred on (1500, 1000) at `zoom` (100 % = one device pixel per
/// document pixel) on a screen of `ppp`: how far the view centre moves, in document pixels, for `key` with `mods`, and
/// the canvas size in document pixels.
fn moved(ppp: f32, zoom: f64, key: Key, mods: Modifiers) -> ((f64, f64), (f64, f64)) {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let given = shown.get().unwrap_or_else(|| {
            let zoom = CanvasView::points_per_unit(zoom, ui.ctx().pixels_per_point());
            CanvasView::new(ui.available_rect_before_wrap(), Point::new(1500.0, 1000.0), zoom)
        });
        let canvas = Canvas::new("Document").document(Size::new(3000.0, 2000.0)).view(given).show(ui, |_, _| {});
        shown.set(Some(canvas.output.view.unwrap_or(given)));
    });
    fixture.pixels_per_point(ppp);
    fixture.settle();
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    let before = view.get().expect("the canvas shows a view");
    fixture.key(key, mods);
    let after = view.get().expect("the canvas shows a view");
    let page = (f64::from(before.rect.width()) / before.zoom, f64::from(before.rect.height()) / before.zoom);
    ((after.center.x - before.center.x, after.center.y - before.center.y), page)
}

/// Rounds to 1/1000 of a document pixel, so a step that went through points compares exactly.
fn round(d: (f64, f64)) -> (f64, f64) {
    ((d.0 * 1000.0).round() / 1000.0, (d.1 * 1000.0).round() / 1000.0)
}

#[trial]
fn shift_page_keys_scroll_ten_document_pixels_rounded_to_whole_device_pixels() {
    let shift_cmd = Modifiers::SHIFT | Modifiers::COMMAND;
    for ppp in [1.0, 1.25, 2.0] {
        // 25 %: 2.5 device pixels round to 3, which are 12 document pixels.
        for (zoom, step) in [(0.25, 12.0), (0.5, 10.0), (1.0, 10.0), (2.0, 10.0), (4.0, 10.0)] {
            // A 3000 × 2000 document at 25 % fits a small canvas, which then has nothing to scroll.
            if zoom < 0.5 && ppp > 1.0 {
                continue;
            }
            let at = format!("{}x display scale, {} % zoom", ppp, zoom * 100.0);
            assert_eq!(round(moved(ppp, zoom, Key::PageDown, Modifiers::SHIFT).0), (0.0, step), "Shift+Page Down, {at}");
            assert_eq!(round(moved(ppp, zoom, Key::PageUp, Modifiers::SHIFT).0), (0.0, -step), "Shift+Page Up, {at}");
            assert_eq!(round(moved(ppp, zoom, Key::PageDown, shift_cmd).0), (step, 0.0), "Ctrl+Shift+Page Down, {at}");
            assert_eq!(round(moved(ppp, zoom, Key::PageUp, shift_cmd).0), (-step, 0.0), "Ctrl+Shift+Page Up, {at}");
        }
    }
}

#[trial]
fn page_keys_without_shift_still_scroll_a_canvas_page() {
    for (ppp, zoom) in [(1.0, 1.0), (1.25, 2.0), (2.0, 2.0)] {
        let (down, page) = moved(ppp, zoom, Key::PageDown, Modifiers::NONE);
        assert_eq!(round(down), round((0.0, page.1)), "Page Down at {ppp}x, {zoom}");
        let (right, page) = moved(ppp, zoom, Key::PageDown, Modifiers::COMMAND);
        assert_eq!(round(right), round((page.0, 0.0)), "Ctrl+Page Down at {ppp}x, {zoom}");
    }
}

#[trial]
fn the_small_step_is_whole_device_pixels_and_never_zero() {
    assert_eq!(PageKey::small_step(1.0, 1.0), 10.0);
    // 125 % display scaling at 100 %: 10 device pixels are 8 points.
    assert_eq!(PageKey::small_step(0.8, 1.25), 8.0);
    // 25 % on a 1x screen: 2.5 device pixels round to 3.
    assert_eq!(PageKey::small_step(0.25, 1.0), 3.0);
    // A tiny zoom still steps one device pixel; bad numbers do not panic or make the step zero.
    assert_eq!(PageKey::small_step(0.001, 2.0), 0.5);
    for (zoom, ppp) in [(f64::NAN, 1.0), (f64::INFINITY, 1.0), (1.0, 0.0), (1.0, f32::NAN), (-1.0, 1.0)] {
        let step = PageKey::small_step(zoom, ppp);
        assert!(step.is_finite() && step > 0.0, "{zoom} {ppp}: {step}");
    }
}

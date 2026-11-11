//! Hover and press looks: a part declared `state_layer` takes the theme's hover fill under the pointer, and a check
//! indicator shows the press.

use craft_ui::choice::{Checkbox, Rating};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Rect, Shape, pos2, vec2};
use std::cell::Cell;

/// The filled rects inside `within`.
fn plates(shapes: &[Shape], within: Rect) -> usize {
    shapes.iter().filter(|shape| matches!(shape, Shape::Rect(rect) if rect.fill.a() > 0 && within.contains_rect(rect.rect))).count()
}

#[trial]
fn hovering_a_star_fills_its_cell_and_leaving_it_clears_the_fill() {
    let mut frames = Frames::new();
    let shown = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0));
        shown.set(in_region(ui, region, |ui| Rating::new(Bind::given(0u8), 5).id_salt("stars").show(ui).response.rect));
    };
    let rest = frames.frame_shapes(&mut show);
    let rect = shown.get();
    assert_eq!(plates(&rest, rect), 0, "a rating at rest paints no fill");
    frames.frame(vec![pointer::moved(pos2(rect.min.x + 2.0, rect.center().y))], &mut show);
    let hovered = frames.frame_shapes(&mut show);
    assert_eq!(plates(&hovered, rect), 1, "the hovered star's cell, and only it, takes the hover fill");
    frames.frame(vec![pointer::gone()], &mut show);
    let left = frames.frame_shapes(&mut show);
    assert_eq!(plates(&left, rect), 0, "the fill goes with the pointer");
}

/// The fill colours painted inside `within`.
fn fills(shapes: &[Shape], within: Rect) -> Vec<egui::Color32> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if rect.fill.a() > 0 && within.contains_rect(rect.rect) => Some(rect.fill),
            _ => None,
        })
        .collect()
}

#[trial]
fn pressing_a_check_box_changes_its_fill_whether_off_or_on() {
    for on in [false, true] {
        let mut frames = Frames::new();
        let shown = Cell::new(Rect::NOTHING);
        let mut show = |ui: &mut egui::Ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 40.0));
            shown.set(in_region(ui, region, |ui| Checkbox::new(Bind::given(on)).label("Snap").show(ui).response.rect));
        };
        let rest = frames.frame_shapes(&mut show);
        let rect = shown.get();
        frames.frame(vec![pointer::moved(rect.center()), pointer::press(rect.center())], &mut show);
        let pressed = frames.frame_shapes(&mut show);
        assert!(!fills(&rest, rect).is_empty(), "the box paints a fill (on: {on})");
        assert_ne!(fills(&pressed, rect), fills(&rest, rect), "a press shows on the box (on: {on})");
    }
}

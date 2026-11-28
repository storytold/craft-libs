//! `PaintChip`'s menu mark, the arrow after the well that is part of the chip, its swatch size and its ring.

use craft_color_ui::ui::{GradientPreview, Paint, PaintChip, PaintChipSize};
use craft_theme::Srgba;
use craft_ui::buttons::PopupMark;
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Shape};
use std::cell::Cell;
use std::rc::Rc;

/// Every shape of the frame, groups opened.
fn flat(shapes: Vec<Shape>) -> Vec<Shape> {
    shapes.into_iter().flat_map(|shape| if let Shape::Vec(inner) = shape { flat(inner) } else { vec![shape] }).collect()
}

/// A ring chip wider than it is high keeps an open middle: the hole inside the band is at least a third of the chip's
/// height, and the band is as thick at the sides as at the top and bottom. Checked for a solid paint, drawn as a band
/// of the colour, and for a gradient, whose middle is covered by the surface the chip sits on.
#[trial]
fn a_wide_ring_chip_keeps_an_open_middle() {
    let gradient = GradientPreview::new(vec![(0.0, Srgba::rgb(0.2, 0.4, 0.8)), (1.0, Srgba::WHITE)]);
    for paint in [Paint::Solid(Srgba::rgb(0.2, 0.4, 0.8)), Paint::Gradient(gradient)] {
        let mut fixture = Fixture::new(move |ui| {
            let mut paint = paint.clone();
            let _ = PaintChip::new("Stroke", Bind::bound(&mut paint), Srgba::BLACK).ring(true).show(ui);
        });
        fixture.settle();
        let well = fixture.by_role(Role::COLOR_WELL).first().copied().expect("the chip is a colour well").rect;
        assert!(well.width() > well.height(), "the chip is wider than it is high");
        // The hole is the smallest rect drawn round the well's centre.
        let hole = flat(fixture.frame_shapes())
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Rect(rect) => Some(rect.rect),
                _ => None,
            })
            .filter(|rect: &Rect| rect.center().distance(well.center()) < 0.5 && rect.width() < well.width() - 2.0)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("a hole inside the band");
        assert!(hole.height() >= well.height() / 3.0, "the hole is {:?} in a {:?} well", hole.size(), well.size());
        let (side, top) = (hole.left() - well.left(), hole.top() - well.top());
        assert!((side - top).abs() < 0.5, "the band is {side} at the sides and {top} at the top");
    }
}

/// A click on the menu mark, past the well, opens the chip's popover, as a click on the well does.
#[trial]
fn a_click_on_the_menu_mark_opens_the_popover() {
    let mut fixture = Fixture::new(|ui| {
        let mut paint = Paint::Solid(Srgba::rgb(0.2, 0.4, 0.8));
        let _ = PaintChip::new("Fill", Bind::bound(&mut paint), Srgba::BLACK).popup_mark(PopupMark::Menu).show(ui);
    });
    fixture.settle();
    let chip = fixture.by_role(Role::COLOR_WELL).first().copied().expect("the chip is a colour well");
    // The mark's room is at the chip's end edge, past the well.
    fixture.click_at(egui::pos2(chip.rect.right() - 2.0, chip.rect.center().y)).idle(4);
    assert!(!fixture.by_role(Role::RADIO_BUTTON).is_empty(), "the popover's kind switch shows");
}

/// A swatch chip opens no popover: a click reports its modifiers for the host's command.
#[trial]
fn a_click_on_a_swatch_chip_opens_nothing_and_reports_the_click() {
    let clicked = Rc::new(Cell::new(false));
    let seen = Rc::clone(&clicked);
    let mut fixture = Fixture::new(move |ui| {
        let mut paint = Paint::None;
        let out = PaintChip::new("None", Bind::bound(&mut paint), Srgba::BLACK).chip_size(PaintChipSize::Swatch).show(ui);
        seen.set(seen.get() || out.clicked.is_some());
    });
    fixture.settle();
    let chip = fixture.by_role(Role::COLOR_WELL).first().copied().expect("the chip is a colour well");
    fixture.click(&chip).idle(4);
    assert!(clicked.get(), "the click is reported");
    assert!(fixture.by_role(Role::RADIO_BUTTON).is_empty(), "no popover opens");
}

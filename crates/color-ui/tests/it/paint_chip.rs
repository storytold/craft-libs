//! `PaintChip`'s menu mark, the arrow after the well that is part of the chip, and its swatch size.

use craft_color_ui::ui::{Paint, PaintChip, PaintChipSize};
use craft_theme::Srgba;
use craft_ui::buttons::PopupMark;
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

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

//! `PaintChip`'s menu mark: the arrow after the well is part of the chip.

use craft_color_ui::ui::{Paint, PaintChip};
use craft_theme::Srgba;
use craft_ui::buttons::PopupMark;
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, trial};

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

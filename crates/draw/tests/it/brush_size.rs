//! The brush Size row: a reset sets the tip's original size, and follows the original the host shows.

use craft_draw::ui::brush_settings::BrushSizeField;
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use std::cell::RefCell;
use std::rc::Rc;

struct Host {
    diameter: f64,
    original: f64,
}

#[trial]
fn a_reset_sets_the_original_size_of_the_tip_shown() {
    let host = Rc::new(RefCell::new(Host { diameter: 120.0, original: 40.0 }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { diameter, original } = &mut *host;
        let original = *original;
        let _ = BrushSizeField::new("Size", Bind::bound(diameter), 500.0).restore(original).show(ui);
    });
    fixture.room(Vec2::new(320.0, 120.0)).settle();
    let slider = fixture.by_role(Role::SLIDER).first().copied().expect("the size slider shows");
    let at = slider.rect.left_center() + Vec2::new(slider.rect.width() * 0.8, 0.0);

    fixture.click_at(at).click_at(at).settle();
    assert_eq!(host.borrow().diameter, 40.0, "a double click resets to the original size");

    host.borrow_mut().original = 75.0;
    host.borrow_mut().diameter = 200.0;
    fixture.settle();
    fixture.click_at(at).click_at(at).settle();
    assert_eq!(host.borrow().diameter, 75.0, "the reset follows the tip the host shows");
}

//! `PaintList` rows with their fields: a typed hex keeps the paint's alpha.

use std::cell::RefCell;
use std::rc::Rc;

use craft_color::ui::{Paint, PaintList, PaintListEvent, PaintListOptions};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, vec2};

/// A hex typed into a translucent row's field reports the typed colour with the row's alpha.
#[trial]
fn a_typed_hex_keeps_the_rows_alpha() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let paints = [(String::from("Shadow"), Paint::Solid(Srgba::new(0.2, 0.4, 0.6, 0.5))), (String::from("Sunrise"), Paint::None)];
        let out = PaintList::new("Paints", &paints, Bind::given(None)).options(PaintListOptions::default().hex(true)).show(ui);
        held.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(400.0, 200.0));
    fixture.settle();
    let field = fixture.by_label("Color").expect("the solid row has a hex field");
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text("ff0000");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    let hex = seen.borrow().iter().find_map(|event| match event {
        PaintListEvent::Hex { row: 0, color } => Some(*color),
        _ => None,
    });
    let hex = hex.unwrap_or_else(|| panic!("the field reports the typed colour: {:?}", seen.borrow()));
    assert!(hex.r > 0.99 && hex.g < 0.01 && hex.b < 0.01, "the typed colour: {hex:?}");
    assert!((hex.a - 0.5).abs() < 0.01, "the row's alpha is kept: {hex:?}");
}

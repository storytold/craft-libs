//! Trials of the guide dialogs.

use super::{GUIDE_LAYOUT_COLOR_KEY, GuideLayoutDialog, GuideLayoutValues, GuideRect, GuideUnits};
use crate::canvas_data::GuideColor;
use crate::contract::Bind;
use crate::runtime::env::UnitKind;
use crate::runtime::requests::ColorRequest;
use crate::testing::{Fixture, trial};
use craft_theme::Rgba;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The owner-less Preview box starts on at every open: turned off, it stays off while the dialog is open only.
#[trial]
fn the_preview_check_box_under_the_side_buttons_starts_on_at_every_open() {
    let open = Rc::new(Cell::new(true));
    let previewing = Rc::new(Cell::new(false));
    let (opened, seen) = (Rc::clone(&open), Rc::clone(&previewing));
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let document = GuideRect::new(0.0, 0.0, 3000.0, 2000.0);
        let mut values = GuideLayoutValues::default();
        let mut is_open = opened.get();
        let shown = GuideLayoutDialog::new(units, document, Bind::bound(&mut values)).opened(Bind::bound(&mut is_open)).show(ui);
        opened.set(is_open);
        seen.set(shown.preview.is_some());
    });
    fixture.settle();
    assert!(previewing.get(), "Preview starts on and the layout previews");
    let (Some(preview), Some(ok)) = (fixture.by_label("Preview"), fixture.by_label("OK")) else {
        panic!("the Preview check box and OK are shown");
    };
    assert!(preview.rect.min.y >= ok.rect.max.y && (preview.rect.min.x - ok.rect.min.x).abs() < 0.5, "Preview sits under the buttons");
    fixture.click(&preview);
    fixture.settle();
    assert!(!previewing.get(), "with Preview off the dialog previews nothing");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).idle(4);
    assert!(!open.get(), "Escape closes the dialog");
    // A closed dialog's shown memory goes after 3 passes and 0.5 s.
    fixture.advance(1.0).idle(4);
    open.set(true);
    fixture.settle();
    assert!(previewing.get(), "the next open starts with Preview on");
}

/// With a custom colour chosen, a swatch of it follows the colour list and a click on it asks the host's picker again,
/// at that colour; a named colour shows no swatch.
#[trial]
fn a_click_on_the_custom_colour_swatch_asks_the_picker_at_that_colour() {
    let values = Rc::new(RefCell::new(GuideLayoutValues::new(Default::default(), GuideColor::Custom(Rgba::rgb(10, 200, 30)))));
    let asked: Rc<RefCell<Vec<ColorRequest>>> = Rc::default();
    let (held, seen) = (Rc::clone(&values), Rc::clone(&asked));
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let document = GuideRect::new(0.0, 0.0, 3000.0, 2000.0);
        let mut values = held.borrow_mut();
        let mut shown = GuideLayoutDialog::new(units, document, Bind::bound(&mut *values)).opened(Bind::given(true)).show(ui);
        seen.borrow_mut().extend(shown.requests.take_color());
    });
    fixture.settle();
    assert!(asked.borrow().is_empty(), "nothing asks before the click");
    let swatch = fixture.by_label("Custom…").expect("the custom colour's swatch follows the list");
    fixture.click(&swatch);
    fixture.settle();
    let asked_now = asked.borrow();
    let [request] = asked_now.as_slice() else { panic!("one colour request, got {}", asked_now.len()) };
    assert_eq!(request.key, GUIDE_LAYOUT_COLOR_KEY);
    assert!(request.color.is_some(), "the picker opens at the custom colour");
    drop(asked_now);

    values.borrow_mut().color = GuideColor::Cyan;
    fixture.settle();
    assert!(fixture.by_label("Custom…").is_none(), "a named colour shows no swatch");
}

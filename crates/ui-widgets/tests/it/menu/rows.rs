//! Menu rows: the detail line under the label and the row's own tooltip.

use craft_ui::buttons::Button;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, pointer, trial};

#[trial]
fn a_detail_shows_as_a_second_line_under_the_label() {
    let mut fixture = Fixture::new(|ui| {
        Menu::inline("Paper").show(ui, |ui, _| {
            MenuItem::new("Letter").show(ui);
            MenuItem::new("A4").detail("210 × 297 mm").show(ui);
        });
    });
    fixture.settle();
    let plain = fixture.by_label("Letter").expect("the plain row").rect;
    let detailed = fixture.by_label("A4").expect("the row with a detail").rect;
    assert!(detailed.height() > plain.height(), "the row with a detail is a line taller");
    let text = |wanted: &str| fixture.texts().iter().find(|run| run.text == wanted).map(|run| run.rect).expect("the run is painted");
    let (label, detail) = (text("A4"), text("210 × 297 mm"));
    assert!(detail.min.y >= label.max.y - 0.5, "the detail lies under the label");
    assert!((detail.min.x - label.min.x).abs() < 1.0, "the detail starts where the label starts");
}

#[trial]
fn a_row_shows_its_tooltip_when_the_pointer_rests_on_it_in_an_open_menu() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Edit").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Undo").tooltip("Take back the last change".to_owned()).show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let row = fixture.by_label("Undo").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(row)]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Take back the last change"), "the row's tooltip shows");
}

#[trial]
fn a_letter_no_row_takes_while_a_menu_is_open_does_not_reach_the_text_field_under_it() {
    let text = std::rc::Rc::new(std::cell::RefCell::new("hello".to_owned()));
    let held = text.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut value = held.borrow_mut();
        let field = craft_ui::text::TextField::new("Name", craft_ui::contract::Bind::bound(&mut *value)).own_context_menu().show(ui);
        Menu::context("Field menu", &field.response).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
            MenuItem::new("Paste").show(ui);
        });
    });
    fixture.settle();
    let at = fixture.by_label("Name").expect("the field").center();
    fixture.click_at(at);
    fixture.settle();
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    assert!(fixture.by_label("Copy").is_some(), "the context menu is open over the field");
    fixture.type_text("q");
    fixture.settle();
    assert_eq!(*text.borrow(), "hello", "the letter does not reach the field");
}

/// The fills painted as rects inside `row`, a row's highlight among them.
fn fills_in(shapes: &[egui::Shape], row: egui::Rect) -> Vec<egui::Color32> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.fill.a() > 0 && row.expand(0.5).contains_rect(rect.rect) && rect.rect.height() > row.height() / 2.0 => {
                Some(rect.fill)
            }
            _ => None,
        })
        .collect()
}

/// A menu highlight switches rows in the frame that moves the pointer onto the next row
/// itself: the row it reaches paints the full highlight and the row it left none, with no time passing in between, so the
/// highlight never trails the pointer (photocraft#15477).
#[trial]
fn the_menu_highlight_moves_to_the_next_row_in_the_frame_the_pointer_reaches_it() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Edit").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Undo").show(ui);
            MenuItem::new("Redo").show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let (undo, redo) = (fixture.by_label("Undo").expect("the menu is open").rect, fixture.by_label("Redo").expect("the second row").rect);
    fixture.frame(vec![pointer::moved(undo.center())]);
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let [lit] = fills_in(&shapes, undo)[..] else { panic!("the row under the pointer paints one highlight: {:?}", fills_in(&shapes, undo)) };
    assert!(fills_in(&shapes, redo).is_empty(), "the other row paints no highlight");
    let shapes = fixture.frame_shapes_with(vec![pointer::moved(redo.center())]);
    assert_eq!(fills_in(&shapes, redo), [lit], "the row the pointer reached paints the full highlight at once");
    assert!(fills_in(&shapes, undo).is_empty(), "the row the pointer left paints no highlight at once: {:?}", fills_in(&shapes, undo));
}

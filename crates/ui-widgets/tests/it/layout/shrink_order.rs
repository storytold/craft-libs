//! How a row too short for its children shares the shortfall: a wrapping text keeps its words whole.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};

const ROOM: Vec2 = Vec2::new(800.0, 600.0);
const SLACK: f32 = 0.5;

/// The rect of the widget labelled `label` in a row of `children`, in a room `width` wide.
fn rect_in_row(width: f32, label: &'static str, children: impl Fn(&mut egui::Ui) + 'static) -> Rect {
    let mut fixture = Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| children(ui));
    });
    fixture.room(Vec2::new(width, ROOM.y)).settle();
    fixture.by_label(label).map_or(Rect::NOTHING, |target| target.rect)
}

/// A wrapping text after a wide sibling, which leaves it less room than its widest word, breaks between words only:
/// it is at least as wide as that word, and the row narrows the sibling to make the room.
#[trial]
fn a_wrapping_text_in_a_short_row_is_never_narrower_than_its_widest_word() {
    const WORD: &str = "following";
    const TEXT: &str = "of the following rules:";
    const BUTTON: &str = "A button with a long label";
    let word = rect_in_row(ROOM.x, WORD, |ui| {
        Text::new(WORD).show(ui);
    });
    let button = rect_in_row(ROOM.x, BUTTON, |ui| {
        Button::new(BUTTON).show(ui);
    });
    assert!(word.width() > 0.0 && button.width() > 0.0, "both measure: {word:?}, {button:?}");
    // The button takes all but half the word's width.
    let width = button.width() + word.width() / 2.0;
    let text = rect_in_row(width, TEXT, |ui| {
        Button::new(BUTTON).show(ui);
        Text::new(TEXT).show(ui);
    });
    assert!(text.width() >= word.width() - SLACK, "the text {text:?} is narrower than its widest word {word:?}");
}

//! `SelectableText`: read-only text the user focuses with a press to select and copy, which Tab passes over.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{SelectableText, TextField};
use egui::{Event, Id, Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

fn stop(fixture: &Fixture) -> Option<Id> {
    fixture.focused_stop().map(|(id, _)| id)
}

/// Tab goes from the field before the text to the one after it; a press on the text focuses it, and Select All then
/// Copy puts the whole text on the clipboard.
#[trial]
fn tab_passes_over_read_only_text_and_a_press_selects_it_for_copy() {
    let ids = Rc::new([Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL)]);
    let seen = ids.clone();
    let mut fixture = Fixture::new(move |ui| {
        seen[0].set(TextField::new("Before", Bind::given(String::new())).id_salt("before").show(ui).response.id);
        seen[1].set(SelectableText::new("Path", "/projects/poster.indd").show(ui).response.id);
        seen[2].set(TextField::new("After", Bind::given(String::new())).id_salt("after").show(ui).response.id);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(stop(&fixture), Some(ids[0].get()));
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(stop(&fixture), Some(ids[2].get()), "Tab passes over the read-only text");

    let text = fixture.by_label("Path").expect("the text has a node");
    fixture.click(&text);
    assert_eq!(stop(&fixture), Some(ids[1].get()), "a press focuses the text");
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.frame(vec![Event::Copy]);
    assert_eq!(fixture.copied(), ["/projects/poster.indd"]);
}

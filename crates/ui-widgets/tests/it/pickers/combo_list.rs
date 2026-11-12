//! The keys in a ComboBox's open list put the active option's text in the field at once, and the list's Escape puts
//! back what was typed.

use craft_ui::contract::{Bind, Entry, Role, Show as _};
use craft_ui::pickers::ComboBox;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Ui};

const FAMILIES: [&str; 3] = ["Garamond", "Georgia", "Arial"];

fn combo(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options: Vec<Entry<String>> = FAMILIES.iter().map(|family| Entry::new((*family).to_owned(), *family)).collect();
    ComboBox::new("Font", Bind::bound(data.value("font", "Arial".to_owned())), &options).show(ui).into()
}

fn font(fixture: &mut Fixture) -> String {
    fixture.data().value("font", String::new()).clone()
}

fn list_open(fixture: &mut Fixture) -> bool {
    !fixture.by_role(Role::LIST_BOX).is_empty()
}

#[trial]
fn keys_in_the_open_list_put_the_option_in_and_escape_puts_the_typed_text_back() {
    let mut fixture = Fixture::story(&Story::new("combo_list", combo));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text("G");
    fixture.settle();
    assert!(list_open(&mut fixture), "typing opens the list on the matches");
    assert_eq!(font(&mut fixture), "G", "the typed text is the value");
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    let keyed = font(&mut fixture);
    assert!(FAMILIES.contains(&keyed.as_str()) && keyed.starts_with('G'), "Down puts a matching option's text in at once: {keyed:?}");
    assert!(list_open(&mut fixture), "and the list stays open on the typed text's matches");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    assert!(!list_open(&mut fixture), "Escape closes the list");
    assert_eq!(font(&mut fixture), "G", "and puts back the typed text");
}

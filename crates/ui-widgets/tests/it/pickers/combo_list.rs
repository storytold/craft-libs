//! The keys in a ComboBox's open list put the active option's text in the field at once, and the list's Escape puts
//! back what was typed. The option under the pointer is previewed without changing the field.

use craft_behavior::Preview;
use craft_ui::contract::{Entry, Role, Show as _};
use craft_ui::pickers::ComboBox;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Pos2, Ui};

const FAMILIES: [&str; 3] = ["Garamond", "Georgia", "Arial"];

fn combo(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options: Vec<Entry<String>> = FAMILIES.iter().map(|family| Entry::new((*family).to_owned(), *family)).collect();
    ComboBox::new("Font", data.bind("font", "Arial".to_owned()), &options).show(ui).edit.into()
}

/// The combo box that stores the last preview change it reported and how many passes reported one; with `twice` set,
/// every frame runs two passes.
fn previewing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options: Vec<Entry<String>> = FAMILIES.iter().map(|family| Entry::new((*family).to_owned(), *family)).collect();
    let shown = ComboBox::new("Font", data.bind("font", "Arial".to_owned()), &options).show(ui);
    if shown.preview != Preview::Unchanged {
        *data.value("reports", 0_u32) += 1;
        *data.value("preview", Preview::<String>::Unchanged) = shown.preview.clone();
    }
    if *data.value("twice", false) && ui.ctx().current_pass_index() == 0 {
        ui.ctx().request_discard("the trial runs the frame twice");
    }
    shown.edit.into()
}

fn preview(fixture: &mut Fixture) -> Preview<String> {
    fixture.data().value("preview", Preview::<String>::Unchanged).clone()
}

fn reports(fixture: &mut Fixture) -> u32 {
    *fixture.data().value("reports", 0_u32)
}

#[trial]
fn the_hovered_option_is_previewed_until_the_pointer_leaves_the_list() {
    let mut fixture = Fixture::story(&Story::new("combo_preview", previewing));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");

    let garamond = fixture.by_label("Garamond").expect("the Garamond row").center();
    fixture.frame(vec![moved(garamond)]);
    fixture.settle();
    assert_eq!(preview(&mut fixture), Preview::Shows("Garamond".to_owned()), "the row under the pointer is previewed");
    assert_eq!(font(&mut fixture), "Arial", "and the field's value stays");

    // A frame that runs two passes reports the move once: the second pass has no input of its own.
    *fixture.data().value("twice", false) = true;
    let before = reports(&mut fixture);
    let georgia = fixture.by_label("Georgia").expect("the Georgia row").center();
    fixture.frame(vec![moved(georgia)]);
    assert!(fixture.passes() > 1, "the frame ran twice");
    fixture.settle();
    assert_eq!(preview(&mut fixture), Preview::Shows("Georgia".to_owned()), "another row previews its text");
    assert_eq!(reports(&mut fixture), before + 1, "one report for one move, and none for passes that change nothing");
    *fixture.data().value("twice", false) = false;

    let list = fixture.by_role(Role::LIST_BOX).first().copied().expect("the open list").rect;
    fixture.frame(vec![moved(Pos2::new(list.center().x, list.max.y + 40.0))]);
    fixture.settle();
    assert!(list_open(&mut fixture), "leaving the rows keeps the list open");
    assert_eq!(preview(&mut fixture), Preview::Ended, "and ends the preview");
    assert_eq!(font(&mut fixture), "Arial", "the field's value never followed the preview");

    let georgia = fixture.by_label("Georgia").expect("the Georgia row");
    fixture.click(&georgia);
    fixture.settle();
    assert_eq!(font(&mut fixture), "Georgia", "a click picks the row");
    assert_eq!(preview(&mut fixture), Preview::Ended, "and the pick ends the preview the hover started");
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

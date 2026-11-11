//! Stories of `ComboBox`.

use super::ComboBox;
use crate::contract::{Bind, Entry, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::{Key, Modifiers, Ui};

const FAMILIES: [&str; 5] = ["Arial", "Charter", "Garamond", "Verdana", "Zapfino"];

fn options() -> Vec<Entry<String>> {
    FAMILIES.iter().map(|family| Entry::new((*family).to_owned(), *family)).collect()
}

#[story(ComboBox<'static>, covers = [Default], drive = Drive::Type("Garamond"))]
fn combo_box(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    ComboBox::new("Font", Bind::bound(data.value("font", "Charter".to_owned())), &options).placeholder("Choose a font").show(ui).into()
}

#[story(ComboBox<'static>, covers = [LongText], drive = Drive::Never)]
fn combo_box_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    let long = "A family name that runs far past the room the field has left beside its chevron".to_owned();
    ComboBox::new("Font", Bind::bound(data.value("long", long)), &options).show(ui).into()
}

#[story(ComboBox<'static>, covers = [Mixed], drive = Drive::Never)]
fn combo_box_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    ComboBox::new("Font", Bind::bound_mixed(data.value("mixed", Mixed::<String>::Mixed)), &options).show(ui).into()
}

#[story(ComboBox<'static>, covers = [Invalid], drive = Drive::Never)]
fn combo_box_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    let refuse_zapfino =
        |family: &String| if family == "Zapfino" { Validity::Refuse(Message::new("Zapfino is not installed")) } else { Validity::Ok };
    ComboBox::new("Font", Bind::bound(data.value("invalid", "Zapfino".to_owned())).check(refuse_zapfino), &options).show(ui).into()
}

/// Focuses the field and opens the list on every option with Alt+Down.
fn open_list(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.idle(4);
}

#[story(ComboBox<'static>, covers = [Open], drive = Drive::Custom(open_list))]
fn combo_box_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut options = options();
    options.insert(2, Entry::separator());
    ComboBox::new("Font", Bind::bound(data.value("open", "Charter".to_owned())), &options).show(ui).into()
}

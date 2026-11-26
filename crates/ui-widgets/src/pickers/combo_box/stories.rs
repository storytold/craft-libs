//! Stories of `ComboBox`.

use super::ComboBox;
use crate::contract::{Entry, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::pointer::moved;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{Mixed, Preview};
use egui::{Key, Modifiers, Ui};

const FAMILIES: [&str; 5] = ["Arial", "Charter", "Garamond", "Verdana", "Zapfino"];

fn options() -> Vec<Entry<String>> {
    FAMILIES.iter().map(|family| Entry::new((*family).to_owned(), *family)).collect()
}

#[story(ComboBox<'static>, covers = [Default], drive = Drive::Type("Garamond"))]
fn combo_box(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    ComboBox::new("Font", data.bind("font", "Charter".to_owned()), &options).placeholder("Choose a font").show(ui).edit.into()
}

#[story(ComboBox<'static>, covers = [LongText], drive = Drive::Never)]
fn combo_box_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    let long = "A family name that runs far past the room the field has left beside its chevron".to_owned();
    ComboBox::new("Font", data.bind("long", long), &options).show(ui).edit.into()
}

#[story(ComboBox<'static>, covers = [Mixed], drive = Drive::Never)]
fn combo_box_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    ComboBox::new("Font", data.bind_mixed("mixed", Mixed::<String>::Mixed), &options).show(ui).edit.into()
}

#[story(ComboBox<'static>, covers = [Invalid], drive = Drive::Never)]
fn combo_box_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    let refuse_zapfino =
        |family: &String| if family == "Zapfino" { Validity::Refuse(Message::new("Zapfino is not installed")) } else { Validity::Ok };
    ComboBox::new("Font", data.bind("invalid", "Zapfino".to_owned()).check(refuse_zapfino), &options).show(ui).edit.into()
}

#[story(ComboBox<'static>, covers = [], drive = Drive::Never, group = "States", title = "No options")]
fn combo_box_no_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ComboBox::new("Font", data.bind("none", String::new()), &[]).show(ui).into()
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
    ComboBox::new("Font", data.bind("open", "Charter".to_owned()), &options).show(ui).edit.into()
}

/// Opens the list and rests the pointer on the Garamond row, which the list previews.
fn hover_option(fixture: &mut Fixture) {
    open_list(fixture);
    if let Some(row) = fixture.by_label("Garamond") {
        fixture.frame(vec![moved(row.center())]);
    }
    fixture.idle(4);
}

#[story(ComboBox<'static>, covers = [Open], drive = Drive::Custom(hover_option))]
fn combo_box_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = options();
    let shown = ComboBox::new("Font", data.bind("preview_font", "Charter".to_owned()), &options).show(ui);
    // The field keeps "Charter" while the hovered option is previewed. The preview is no edit, so the story says what
    // it shows in its readout, not in its data, which a drive reads as a changed value.
    let output = StoryOutput::from(shown.edit);
    match shown.preview {
        Preview::Shows(text) => output.readout(format!("Previewing {text}")),
        Preview::Ended => output.readout("Preview ended"),
        _ => output,
    }
}

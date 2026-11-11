//! Stories of `TextField`.

use super::TextField;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _, Validity};
use crate::contract::{LabelAlign, SideLabel};
use crate::form::{Field, FieldGroup};
use crate::primitives::{Completer, Completion, IconSpec};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed, PlainText};
use craft_theme::ControlSize;
use egui::{Key, Modifiers, Ui};
use std::ops::Range;

const SIZES: [(&str, ControlSize); 2] = [("Sm", ControlSize::Sm), ("Md", ControlSize::Md)];
const LAYER_LONG: &str = "A layer name long enough to scroll inside the field while the caret moves along it";
const LONG: &str = "A name that runs on well past the end of the field, so the field scrolls its text under the caret";

/// A row of the states story: the label column starts at the left edge, as the form list does.
fn row(label: &'static str) -> Field {
    Field::new(label).side(SideLabel::new().align(LabelAlign::Start)).id_salt(label)
}

#[story(TextField<'static, PlainText>, covers = [Default])]
fn text_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let placeholder = data.value("placeholder", String::from("Layer name")).clone();
    let out = FieldGroup::new("Text field states").show(ui, |ui, _| {
        let first = row("Empty").show(ui, |ui, _| {
            TextField::new("Name", Bind::bound(data.value("name", String::new()))).placeholder(placeholder.clone()).size(size).show(ui)
        });
        row("Filled").show(ui, |ui, _| {
            TextField::new("Layer", Bind::bound(&mut String::from("Background"))).placeholder(placeholder.clone()).size(size).show(ui);
        });
        row("Long text").show(ui, |ui, _| {
            TextField::new("Layer name", Bind::bound(&mut String::from(LAYER_LONG))).size(size).show(ui);
        });
        let refuse_slash =
            |text: &String| if text.contains('/') { Validity::Refuse(Message::new("A name cannot contain a slash")) } else { Validity::Ok };
        row("Invalid").show(ui, |ui, _| {
            TextField::new("Layer path", Bind::bound(&mut String::from("Layer/1")).check(refuse_slash)).size(size).show(ui);
        });
        row("Password").show(ui, |ui, _| {
            TextField::new("Password", Bind::bound(&mut String::from("correct horse"))).masked().size(size).show(ui);
        });
        row("Leading icon").show(ui, |ui, _| {
            TextField::new("Layer icon", Bind::bound(&mut String::from("Layers"))).leading(IconSpec::svg(IconId::new("layers"))).size(size).show(ui);
        });
        row("Multiline").show(ui, |ui, _| {
            TextField::new("Notes", Bind::bound(data.value("notes", String::from("Shot list\nGolden hour, wide\nDetail of the hands"))))
                .lines(3)
                .size(size)
                .show(ui);
        });
        for (at, text) in ["", "Locked layer"].into_iter().enumerate() {
            let name = if text.is_empty() { "Disabled, empty" } else { "Disabled, filled" };
            Field::new(name).side(SideLabel::new().align(LabelAlign::Start)).id_salt(("disabled", at)).show(ui, |ui, _| {
                TextField::new(name, Bind::bound(&mut text.to_owned())).placeholder(placeholder.clone()).size(size).enabled(false).show(ui);
            });
        }
        first.inner
    });
    out.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(TextField<'static, PlainText>, covers = [LongText], drive = Drive::Never)]
fn text_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", Bind::bound(data.value("long", LONG.to_owned()))).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [Mixed], drive = Drive::Never)]
fn text_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", Bind::bound_mixed(data.value("mixed", Mixed::<String>::Mixed))).placeholder("Mixed").show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [Invalid], drive = Drive::Never)]
fn text_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_empty = |text: &String| if text.is_empty() { Validity::Refuse(Message::new("A name is required")) } else { Validity::Ok };
    TextField::new("Name", Bind::bound(data.value("empty", String::new())).check(refuse_empty)).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", Bind::bound(data.value("name", "Locked".to_owned()))).read_only(true).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", Bind::bound(data.value("name", "Off".to_owned()))).enabled(false).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Type("query"))]
fn text_field_search(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let placeholder = data.value("placeholder", String::from("Search")).clone();
    TextField::search("Search", Bind::bound(data.value("query", String::new()))).placeholder(placeholder).size(size).show(ui).into()
}

/// Completes the words that start with the text before the caret.
struct Prefixes;

impl Completer for Prefixes {
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>> {
        let before = text.get(..caret)?;
        let hits: Vec<_> = ["alpha", "alps", "beta"].into_iter().filter(|word| !before.is_empty() && word.starts_with(before)).collect();
        out.extend(hits.iter().map(|word| Completion::new(*word, *word)));
        (!hits.is_empty()).then_some(0..caret)
    }
}

/// Focuses the field and types the start of a word, which opens the completion list.
fn open_completions(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("al");
    fixture.idle(4);
}

#[story(TextField<'static, PlainText>, covers = [Open], drive = Drive::Custom(open_completions))]
fn text_field_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TextField::new("Name", Bind::given(String::new())).complete(Prefixes).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_search_filled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::search("Search", Bind::bound(data.value("filled", "poster".to_owned()))).placeholder("Search").show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_adorned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Font", Bind::bound(data.value("font", "Inter".to_owned())))
        .leading(IconSpec::svg(IconId::new("search")))
        .trailing_arrow()
        .show(ui)
        .into()
}

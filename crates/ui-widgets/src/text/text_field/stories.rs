//! Stories of `TextField`.

use super::TextField;
use crate::contract::LabelScheme;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _, Sizing, Validity};
use crate::display::{Shade, Text};
use crate::form::{Field, FieldGroup};
use crate::layout_components::Stack;
use crate::primitives::{Completer, Completion, IconSpec, SubmitKey};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed, PlainText};
use craft_theme::ControlSize;
use egui::{Key, Modifiers, Ui};
use std::ops::Range;

const SIZES: [(&str, ControlSize); 2] = [("Sm", ControlSize::Sm), ("Md", ControlSize::Md)];
const LAYER_LONG: &str = "A layer name long enough to scroll inside the field while the caret moves along it";
const LONG: &str = "A name that runs on well past the end of the field, so the field scrolls its text under the caret";

/// A row of the states story, in the group's inspector rows.
fn row(label: &'static str) -> Field {
    Field::new(label).id_salt(label)
}

#[story(TextField<'static, PlainText>, covers = [Default])]
fn text_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let placeholder = data.value("placeholder", String::from("Layer name")).clone();
    let out = FieldGroup::new("Text field states").labels(LabelScheme::Inspector).show(ui, |ui, _| {
        let first = row("Empty")
            .show(ui, |ui, _| TextField::new("Name", data.bind("name", String::new())).placeholder(placeholder.clone()).size(size).show(ui));
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
            TextField::new("Notes", data.bind("notes", String::from("Shot list\nGolden hour, wide\nDetail of the hands")))
                .lines(3)
                .size(size)
                .show(ui);
        });
        for (at, text) in ["", "Locked layer"].into_iter().enumerate() {
            let name = if text.is_empty() { "Disabled, empty" } else { "Disabled, filled" };
            Field::new(name).id_salt(("disabled", at)).show(ui, |ui, _| {
                TextField::new(name, Bind::bound(&mut text.to_owned())).placeholder(placeholder.clone()).size(size).enabled(false).show(ui);
            });
        }
        first.inner
    });
    out.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(TextField<'static, PlainText>, covers = [LongText], drive = Drive::Never)]
fn text_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", data.bind("long", LONG.to_owned())).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [Mixed], drive = Drive::Never)]
fn text_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", data.bind_mixed("mixed", Mixed::<String>::Mixed)).placeholder("Mixed").show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [Invalid], drive = Drive::Never)]
fn text_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_empty = |text: &String| if text.is_empty() { Validity::Refuse(Message::new("A name is required")) } else { Validity::Ok };
    TextField::new("Name", data.bind("empty", String::new()).check(refuse_empty)).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", data.bind("name", "Locked".to_owned())).read_only(true).show(ui).into()
}

/// A reply box: Enter submits and Shift+Enter breaks the line; the readout counts the submits.
#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_submits_on_enter(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = TextField::new("Reply", Bind::bound(data.value("reply", String::from("Looks good"))))
        .lines(3)
        .submits()
        .submit_key(SubmitKey::Enter)
        .show(ui);
    let submits = data.note("submits", 0u32);
    *submits += u32::from(shown.submitted);
    let readout = format!("Submitted {submits} times");
    StoryOutput::from(shown).readout(readout)
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Name", data.bind("name", "Off".to_owned())).enabled(false).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Type("query"))]
fn text_field_search(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let placeholder = data.value("placeholder", String::from("Search")).clone();
    TextField::search("Search", data.bind("query", String::new())).placeholder(placeholder).size(size).show(ui).into()
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
    TextField::search("Search", data.bind("filled", "poster".to_owned())).placeholder("Search").show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_adorned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TextField::new("Font", data.bind("font", "Inter".to_owned())).leading(IconSpec::svg(IconId::new("search"))).trailing_arrow().show(ui).into()
}

/// Fields that ask for a width in characters over the field length: a short one, the field length, and a wide one
/// (a watermark's text), one over the other so their ends show the difference.
#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_width_chars(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (text, text_at) = data.value_at("text", "CONFIDENTIAL".to_owned());
    let shown = Stack::new().show(ui, |ui, _| {
        let short = TextField::new("Short, 12 characters", text_at.bind(&mut *text)).width_chars(12).show(ui);
        TextField::new("Field length", text_at.bind(&mut *text)).show(ui);
        TextField::new("Wide, 40 characters", text_at.bind(&mut *text)).width_chars(40).show(ui);
        short
    });
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}

const NOTES: &str = "Shot list\nGolden hour, wide\nDetail of the hands\nThe crowd at the gate\nClosing frame on the sign";

/// A multi-line field that grows to four lines and then scrolls, and one that fills its room, both over one text and
/// one bound caret shown under them: moving the caret in either moves it in the other.
#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_filling(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((notes, notes_at), (caret, caret_at)) = data.pair_at(("notes", NOTES.to_owned()), ("caret", 0usize));
    let shown = Stack::new().show(ui, |ui, _| {
        let capped = TextField::new("Notes", notes_at.bind(&mut *notes)).lines(2).max_lines(4).caret(caret_at.bind(&mut *caret)).show(ui);
        TextField::new("Notes, filling", notes_at.bind(&mut *notes)).lines(2).caret(caret_at.bind(&mut *caret)).sizing(Sizing::Fill).show(ui);
        Text::new(format!("Caret at byte {caret}")).shade(Shade::Muted).show(ui);
        capped
    });
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}

/// A field whose Tab stays in it and is reported: the readout shows the direction of the last one.
#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn text_field_tab_reports(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((cell, cell_at), (last, _)) = data.pair_at(("cell", "Revenue".to_owned()), ("last", String::from("none")));
    let shown = Stack::new().show(ui, |ui, _| {
        let field = TextField::new("Cell", cell_at.bind(&mut *cell)).tab_reports().show(ui);
        if let Some(forward) = field.tab {
            *last = String::from(if forward { "Tab" } else { "Shift+Tab" });
        }
        Text::new(format!("Last key: {last}")).shade(Shade::Muted).show(ui);
        field
    });
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}

/// Completes spreadsheet functions: each candidate marked "ƒ", with what it does under the list.
struct Functions;

impl Completer for Functions {
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>> {
        const FUNCTIONS: [(&str, &str); 3] = [
            ("SUM", "Adds the numbers in a range."),
            ("SUMIF", "Adds the numbers in a range that meet a condition."),
            ("SUBSTITUTE", "Replaces text in a string."),
        ];
        let before = text.get(..caret)?.trim_start_matches('=');
        let start = caret - before.len();
        let hits: Vec<_> = FUNCTIONS.into_iter().filter(|(name, _)| !before.is_empty() && name.starts_with(&before.to_uppercase())).collect();
        out.extend(hits.iter().map(|(name, what)| Completion::new(*name, format!("{name}(")).mark("ƒ").description(*what)));
        (!hits.is_empty()).then_some(start..caret)
    }
}

/// Focuses the field and types the start of a function name, which opens the marked list.
fn open_functions(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("=SU");
    fixture.idle(4);
}

/// A formula cell: the marked list with the highlighted function's description; Enter commits the cell, Tab completes.
#[story(TextField<'static, PlainText>, covers = [Open], drive = Drive::Custom(open_functions))]
fn text_field_formula(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TextField::new("Formula", Bind::given(String::new())).complete(Functions).show(ui).into()
}

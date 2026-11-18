//! Stories of `FormDialog`.

use super::{FormButtons, FormDialog};
use crate::buttons::Button;
use crate::sections::{DialogRole, FooterButton};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, clicked};
use crate::dialog::HeldAnswer;
use crate::runtime::env::Label;
use crate::text::TextField;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// A held OK the app refused for the name `"Untitled"`: the dialog shows the reason until the name changes.
#[story(FormDialog<'static, bool>, covers = [], drive = Drive::Never)]
fn form_dialog_held(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut name = data.value("name", "Untitled".to_owned()).clone();
    let mut open = *data.value("open", false);
    if clicked(&Button::new("Rename…").show(ui)) {
        open = true;
    }
    let held = data.note("held", HeldAnswer::<String>::new());
    let output = FormDialog::new("Rename", Bind::bound(&mut name))
        .opened(Bind::bound(&mut open))
        .held(&mut *held)
        .show(ui, |ui, _, value| {
            TextField::new("Name", Bind::bound(value)).show(ui);
            true
        });
    if let Some(filed) = held.take() {
        let answer = if filed == "Untitled" { Err(Label::from("That name is taken.")) } else { Ok(()) };
        held.file(filed);
        held.answer(answer);
    }
    *data.value("open", false) = open;
    if let Some(value) = output.value {
        *data.value("name", String::new()) = value;
    }
    output.response.into()
}

/// A held form whose body activation is OK: a double click on a cell files the cell's name, as the OK button does,
/// and the app accepts every name but `"Moon"`.
#[story(FormDialog<'static, bool>, covers = [], drive = Drive::Never)]
fn form_dialog_body_accept(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("open", false);
    if clicked(&Button::new("Insert Symbol…").show(ui)) {
        open = true;
    }
    let held = data.note("held", HeldAnswer::<String>::new());
    let output = FormDialog::own("Insert Symbol")
        .opened(Bind::bound(&mut open))
        .held(&mut *held)
        .accept_stays_open(true)
        .show(ui, |ui, cx, value: &mut String| {
            for cell in ["Sun", "Moon", "Star"] {
                let shown = Button::new(cell).show(ui);
                if clicked(&shown) {
                    *value = cell.to_owned();
                }
                if shown.clicks == 2 {
                    cx.accept();
                }
            }
            !value.is_empty()
        });
    if let Some(filed) = held.take() {
        held.answer(if filed == "Moon" { Err(Label::from("The moon is not in this font.")) } else { Ok(()) });
    }
    *data.value("open", false) = open;
    output.response.into()
}

/// A form without OK: Close, and the Mark (Enter) and Mark All buttons, which count what they marked.
#[story(FormDialog<'static, bool>, covers = [], drive = Drive::Never)]
fn form_dialog_close(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("open", false);
    if clicked(&Button::new("Mark Entry…").show(ui)) {
        open = true;
    }
    let mut entry = data.value("entry", "Craft".to_owned()).clone();
    let output = FormDialog::new("Mark Entry", Bind::bound(&mut entry))
        .opened(Bind::bound(&mut open))
        .buttons(FormButtons::Close)
        .extra_buttons([FooterButton::new("Mark", DialogRole::Accept), FooterButton::new("Mark All", DialogRole::Other)])
        .show(ui, |ui, _, value| {
            TextField::new("Entry", Bind::bound(value)).show(ui);
            true
        });
    let marked = match output.picked {
        Some(2) => 1,
        Some(3) => 10,
        _ => 0,
    };
    *data.value("marked", 0_u32) += marked;
    *data.value("open", false) = open;
    if let Some(value) = output.value {
        *data.value("entry", String::new()) = value;
    }
    output.response.into()
}

#[story(FormDialog<'static, bool>, covers = [], drive = Drive::Never)]
fn form_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ok_enabled = *data.value("ok_enabled", true);
    FormDialog::own("Options")
        .opened(Bind::bound(data.value("open", true)))
        .show(ui, |ui, _, value| {
            Checkbox::new(Bind::bound(value)).label(Label::from("Smooth")).show(ui);
            ok_enabled
        })
        .response
        .into()
}

//! Stories of `FormDialog`.

use super::FormDialog;
use crate::buttons::Button;
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

//! Stories of `FormDialog`.

use super::FormDialog;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _};
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

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

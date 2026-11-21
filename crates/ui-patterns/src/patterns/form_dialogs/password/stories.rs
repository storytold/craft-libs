//! Stories of `PasswordDialog`.

use super::{PasswordDialog, PasswordPurpose};
use crate::buttons::Button;
use crate::contract::{Show as _, clicked};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[derive(Clone, Copy, PartialEq)]
enum Asks {
    Open,
    Permissions,
}

#[story(PasswordDialog<'static>, covers = [], drive = Drive::Never)]
fn password_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let asks = *data.choice("purpose", Asks::Open, &[("Open", Asks::Open), ("Permissions", Asks::Permissions)]);
    let wrong = *data.value("wrong password", false);
    let purpose = match asks {
        Asks::Open => PasswordPurpose::open("brochure.pdf"),
        Asks::Permissions => PasswordPurpose::permissions("brochure.pdf"),
    };
    let dialog = PasswordDialog::new(purpose).opened(data.bind("open", true));
    let dialog = if wrong { dialog.error(crate::runtime::env::Label::Static("That password is not correct.")) } else { dialog };
    dialog.show(ui).into()
}

/// A new password that two empty fields clear; the dialog opens from its button.
#[story(PasswordDialog<'static>, covers = [], drive = Drive::Never)]
fn set_or_clear(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (open, open_at) = data.value_at("open", false);
    if clicked(&Button::new("Set Password…").show(ui)) {
        *open = true;
    }
    PasswordDialog::new(PasswordPurpose::SetOrClear).opened(open_at.bind(open)).show(ui).into()
}

#[story(PasswordDialog<'static>, covers = [], drive = Drive::Never)]
fn password_dialog_new_password(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PasswordDialog::new(PasswordPurpose::Set).opened(data.bind("open", true)).show(ui).into()
}

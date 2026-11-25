//! Stories of `PasswordDialog` and `PasswordFields`.

use super::{PasswordDialog, PasswordFields, PasswordPurpose};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, clicked};
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

#[story(PasswordDialog<'static>, covers = [], drive = Drive::Never)]
fn password_dialog_several(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let asks = *data.choice("purpose", Asks::Open, &[("Open", Asks::Open), ("Permissions", Asks::Permissions)]);
    let purpose = match asks {
        Asks::Open => PasswordPurpose::open_several(3),
        Asks::Permissions => PasswordPurpose::permissions_several(3),
    };
    PasswordDialog::new(purpose).note("The files share one password.").opened(data.bind("open", true)).show(ui).into()
}

#[story(PasswordFields<'static>, covers = [], drive = Drive::Never)]
fn password_fields(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let wrong = *data.value("wrong password", false);
    let input = data.value("input", String::new());
    let error = wrong.then_some(crate::runtime::env::Label::Static("That password is not correct."));
    PasswordFields::new("Password", Bind::bound(input)).error(error).show(ui).into()
}

#[story(PasswordFields<'static>, covers = [], drive = Drive::Never)]
fn password_fields_new_password(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut input = data.value("input", String::new()).clone();
    let mut confirm = data.value("confirm", String::new()).clone();
    let shown = PasswordFields::new("Type Password", Bind::bound(&mut input)).confirm(Bind::bound(&mut confirm)).show(ui);
    *data.value("input", String::new()) = input;
    *data.value("confirm", String::new()) = confirm;
    shown.into()
}

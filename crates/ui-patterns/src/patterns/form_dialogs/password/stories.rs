//! Stories of `PasswordDialog`.

use super::{PasswordDialog, PasswordPurpose};
use crate::contract::Bind;
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
    let dialog = PasswordDialog::new(purpose).opened(Bind::bound(data.value("open", true)));
    let dialog = if wrong { dialog.error(crate::runtime::env::Label::Static("That password is not correct.")) } else { dialog };
    dialog.show(ui).into()
}

#[story(PasswordDialog<'static>, covers = [], drive = Drive::Never)]
fn password_dialog_new_password(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PasswordDialog::new(PasswordPurpose::Set).opened(Bind::bound(data.value("open", true))).show(ui).into()
}

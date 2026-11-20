//! The password dialog keeps the keyboard in its field.

use craft_ui::patterns::form_dialogs::{PasswordDialog, PasswordPurpose};
use craft_ui::testing::{Fixture, trial};

fn focused_on(fixture: &Fixture, label: &str) -> bool {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    fixture.by_label(label).is_some_and(|target| target.node == focused)
}

/// Whether the OK button of the dialog is enabled.
fn ok_enabled(fixture: &Fixture) -> bool {
    let ok = fixture.by_label("OK").expect("the OK button");
    !fixture.tree().is_disabled_key(ok.node)
}

// Two empty fields clear the password and pass for `SetOrClear` where `Set` refuses them; one typed and one empty never
// pass, and two that match do.
#[trial]
fn set_or_clear_accepts_two_empty_fields_but_not_one_empty_and_one_typed() {
    let mut clearing = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::SetOrClear).show(ui);
    });
    clearing.idle(30);
    assert!(ok_enabled(&clearing), "two empty fields clear the password");
    clearing.type_text("secret").idle(4);
    assert!(!ok_enabled(&clearing), "one typed and one empty fields do not match");
    let confirm = clearing.by_label("Re-type Password").expect("the confirmation field");
    clearing.click(&confirm).idle(2);
    clearing.type_text("secret").idle(4);
    assert!(ok_enabled(&clearing), "two matching entries pass");

    let mut setting = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::Set).show(ui);
    });
    setting.idle(30);
    assert!(!ok_enabled(&setting), "Set refuses two empty fields");
}

#[trial]
fn the_reveal_button_hands_the_focus_back_to_the_field() {
    let mut fixture = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::open("poster.pdf")).show(ui);
    });
    fixture.idle(30);
    assert!(focused_on(&fixture, "Password"), "the dialog opens on the field");
    let reveal = fixture.by_label("Show password").expect("the reveal button is shown");
    fixture.click(&reveal).idle(4);
    assert!(fixture.by_label("Hide password").is_some(), "the press revealed the text");
    assert!(focused_on(&fixture, "Password"), "the field has the focus back, so typing goes on");
}

//! The password dialog keeps the keyboard in its field.

use craft_ui::patterns::form_dialogs::{PasswordDialog, PasswordPurpose};
use craft_ui::testing::{Fixture, trial};

fn focused_on(fixture: &Fixture, label: &str) -> bool {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    fixture.by_label(label).is_some_and(|target| target.node == focused)
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

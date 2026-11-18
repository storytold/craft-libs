//! The password dialog keeps the keyboard in its field and rates a new password as it is typed.

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

/// The strength meter's fill and the top of its range: the fill grows with the rating and Strong reaches the top.
fn meter(fixture: &Fixture) -> (f64, f64) {
    let meter = fixture.by_label("Password strength").expect("the strength meter shows once something is typed");
    let numeric = fixture.tree().numeric_of(meter.node).expect("the meter reports how full it is");
    (numeric.value.expect("the meter reports how full it is"), *numeric.range.end())
}

#[trial]
fn a_password_is_rated_by_its_length_and_its_character_classes() {
    let mut fixture = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::Set).show(ui);
    });
    fixture.idle(30);
    assert!(focused_on(&fixture, "Type Password"), "the dialog opens on the first field");
    assert!(fixture.by_label("Password strength").is_none(), "an empty field has no rating");
    // Each step adds characters, so the text only grows longer and gains classes.
    fixture.type_text("abc").idle(2);
    let (short, top) = meter(&fixture);
    assert!(short < top, "three lower-case letters are not the top rating");
    fixture.type_text("DEFGH").idle(2);
    let (mixed, _) = meter(&fixture);
    assert!(mixed >= short, "eight characters over two classes rate no lower than three over one");
    fixture.type_text("12!x").idle(2);
    let (long, _) = meter(&fixture);
    assert!(long >= mixed, "more length and classes never lower the rating");
    assert!(long >= top, "twelve characters over three classes fill the meter");
    assert!(short < long, "the rating rose from weak to strong");
}

//! The password dialog keeps the keyboard in its field and rates a new password as it is typed.

use craft_ui::patterns::form_dialogs::{PasswordDialog, PasswordPurpose};
use craft_ui::testing::{Fixture, trial};
use egui::Modifiers;
use std::cell::RefCell;
use std::rc::Rc;

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

/// Enter in the field answers Open with the typed password; a new password answers only once both entries match.
#[trial]
fn enter_in_the_field_answers_while_the_entries_are_valid() {
    for (purpose, retyped, answers) in [
        (PasswordPurpose::open("poster.pdf"), None, true),
        (PasswordPurpose::Set, Some("other"), false),
        (PasswordPurpose::Set, Some("secret"), true),
    ] {
        let answer = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&answer);
        let mut fixture = Fixture::new(move |ui| {
            let shown = PasswordDialog::new(purpose.clone()).show(ui);
            if let Some(given) = shown.answer {
                *seen.borrow_mut() = Some(given.secret.expose().to_owned());
            }
        });
        fixture.idle(30);
        fixture.type_text("secret").idle(2);
        if let Some(retyped) = retyped {
            let field = fixture.by_label("Re-type Password").expect("a new password is typed twice");
            fixture.click(&field).idle(2);
            fixture.type_text(retyped).idle(2);
        }
        fixture.key(egui::Key::Enter, Modifiers::NONE).idle(4);
        let expected = answers.then(|| "secret".to_owned());
        assert_eq!(*answer.borrow(), expected, "Enter after {retyped:?}");
    }
}

#[trial]
fn retry_starts_with_an_empty_password() {
    let answers = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&answers);
    let mut fixture = Fixture::new(move |ui| {
        let mut open = true;
        let shown = PasswordDialog::new(PasswordPurpose::permissions("poster.pdf")).opened(craft_ui::contract::Bind::bound(&mut open)).show(ui);
        if let Some(answer) = shown.answer {
            seen.borrow_mut().push(answer.secret.expose().to_owned());
        }
    });
    fixture.idle(30).type_text("wrong").key(egui::Key::Enter, Modifiers::NONE).idle(30);
    fixture.type_text("right").key(egui::Key::Enter, Modifiers::NONE).idle(4);
    assert_eq!(*answers.borrow(), vec!["wrong", "right"]);
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

/// Opening a protected file asks for the password in a bare field, as production's prompt does: Open waits for a typed
/// password, and the field stands at the dialog's content start with no caption column before it (its name is its
/// placeholder).
#[trial]
fn an_open_prompt_waits_for_a_typed_password_in_a_bare_field() {
    let answers = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&answers);
    let mut fixture = Fixture::new(move |ui| {
        let shown = PasswordDialog::new(PasswordPurpose::open("poster.pdf")).show(ui);
        if let Some(answer) = shown.answer {
            seen.borrow_mut().push(answer.secret.expose().to_owned());
        }
    });
    fixture.idle(30);
    let open = fixture.by_label("Open").expect("the Open button");
    fixture.click(&open).idle(4);
    assert!(answers.borrow().is_empty(), "Open does not answer an empty password: {:?}", answers.borrow());
    let field = fixture.by_label("Password").expect("the password field");
    // The lock stands before the title, so the message (the content's first line) marks where the content starts.
    let title = fixture.texts().iter().find(|run| run.text.contains("poster.pdf")).map(|run| run.rect.min.x).expect("the message");
    assert!((field.rect.min.x - title).abs() < 6.0, "the field {:?} starts where the dialog's content does ({title})", field.rect);
}

/// The lock stands by the title, as production's does; the message under it starts at the content's edge.
#[trial]
fn the_lock_icon_stands_by_the_title_not_the_message() {
    let mut fixture = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::open("poster.pdf")).show(ui);
    });
    fixture.idle(30);
    let at = |starts: &str| fixture.texts().iter().find(|run| run.text.starts_with(starts)).map(|run| run.rect).unwrap_or(egui::Rect::NOTHING);
    let (title, message) = (at("Password required"), at("\u{201c}poster.pdf"));
    let message = if message.is_positive() { message } else { at("poster.pdf") };
    assert!(title.is_positive() && message.is_positive(), "both texts are painted: {title:?} {message:?}");
    assert!(title.left() > message.left() + 8.0, "the title starts after the lock: {title:?} vs the message {message:?}");
}

/// The lock before the title is in the accent colour, as production's is.
#[trial]
fn the_title_lock_is_drawn_in_the_accent_colour() {
    let mut fixture = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::open("poster.pdf")).show(ui);
    });
    fixture.idle(30);
    let title = fixture.texts().iter().find(|run| run.text.starts_with("Password required")).map(|run| run.rect).expect("the title");
    let colours: Vec<egui::Color32> = fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Mesh(mesh)
                if mesh.calc_bounds().max.x <= title.left() + 1.0 && (mesh.calc_bounds().center().y - title.center().y).abs() < 8.0 =>
            {
                mesh.vertices.first().map(|v| v.color)
            }
            _ => None,
        })
        .collect();
    let accent = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).color.accent.text;
    assert!(!colours.is_empty(), "the lock is drawn before the title");
    let accent = egui::Color32::from_rgb(accent.0[0], accent.0[1], accent.0[2]);
    assert!(colours.contains(&accent), "the lock is {colours:?}, not the accent {accent:?}");
}

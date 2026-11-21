//! The Find and Replace dialog's protocol: fields set by automation are what the next request carries.

use std::cell::RefCell;
use std::rc::Rc;

use craft_text::find::FindOptions;
use craft_text::ui::find::{FindCheck, FindReplaceDialog, FindRequest, FindTab, ReplaceButtons};
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Fixture, trial};
use serde_json::json;

/// Presses the button named `name`.
fn press(fixture: &mut Fixture, name: &str) {
    let button = fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .find(|button| fixture.tree().name_of(button.node) == Some(name))
        .unwrap_or_else(|| panic!("a button named {name}"));
    fixture.click(&button).settle();
}

#[trial]
fn find_fields_set_by_automation_reach_the_next_request() {
    let dialog = Rc::new(RefCell::new(
        FindReplaceDialog::new("craft", FindTab::Replace).word(true).replace_buttons(ReplaceButtons::Separate).checks(vec![FindCheck::new(
            "backwards",
            "Search Backwards",
            false,
        )]),
    ));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.settle();
    let ctx = fixture.ctx().clone();
    let set = |field: &str, value: serde_json::Value| dialog.borrow_mut().set_json_field(&ctx, field, value);
    set("find", json!("teh")).unwrap();
    set("replace", json!("the")).unwrap();
    set("matchCase", json!(true)).unwrap();
    set("backwards", json!(true)).unwrap();
    assert_eq!(dialog.borrow().checked("backwards"), Some(true));
    assert!(set("message", json!("x")).unwrap_err().contains("read only"));
    assert!(set("nope", json!(true)).unwrap_err().contains("unknown field"));
    assert!(set("matchCase", json!("yes")).unwrap_err().contains("can't take"));
    let options = FindOptions { match_case: true, ..FindOptions::default() };

    press(&mut fixture, "Find Next");
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::find_next("teh", options)));
    press(&mut fixture, "Replace");
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::replace_only("teh", "the", options)), "Separate's Replace replaces only");
    press(&mut fixture, "Replace & Find");
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::replace("teh", "the", options)));

    dialog.borrow_mut().set_matched(false);
    fixture.settle();
    press(&mut fixture, "Replace");
    assert_eq!(dialog.borrow_mut().take_request(), None, "no match, no Replace");
    dialog.borrow_mut().set_message("nothing found");
    let fields = dialog.borrow().json_fields().expect("fields");
    assert_eq!(fields["message"], json!("nothing found"));
    assert_eq!(fields["backwards"], json!(true));
}

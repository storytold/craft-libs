//! The Find and Replace dialog's protocol: fields set by automation are what the next request carries.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_text::find::FindOptions;
use craft_text::ui::find::{FindCheck, FindReplaceDialog, FindRequest, FindTab, ReplaceButtons};
use craft_ui::buttons::Button;
use craft_ui::contract::{Outcome as _, Role, Show as _};
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::access::{focused_key, name};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
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

// Escape in Find what closes the dialog and puts the focus back on the control that opened it, also where Escape
// otherwise takes a field's focus (`escape_leaves_fields`). Typing in the field is reverted by the first Escape, which
// keeps the focus in the field, and the next Escape closes the dialog.
#[trial]
fn escape_in_find_what_closes_the_dialog_and_returns_the_focus() {
    for leaves in [false, true] {
        for typed in [false, true] {
            let open = Rc::new(Cell::new(false));
            let opener = Rc::new(Cell::new(None));
            let (shown, seen) = (Rc::clone(&open), Rc::clone(&opener));
            let dialog = RefCell::new(FindReplaceDialog::new("craft", FindTab::Find));
            let mut fixture = Fixture::new(move |ui| {
                let button = Button::new("Find").show(ui);
                seen.set(Some(button.response().id));
                if shown.get() && dialog.borrow_mut().show(ui, &DialogStack::default()).is_some() {
                    shown.set(false);
                }
            });
            let mut root = RootEnv::new();
            root.escape_leaves_fields = leaves;
            fixture.root(root).settle();
            request(fixture.ctx(), FocusTarget::Id(opener.get().expect("the opener is shown")));
            fixture.settle();
            assert_eq!(fixture.focused(), opener.get(), "the opener has the focus");
            open.set(true);
            fixture.settle();
            let focused_name = |fixture: &Fixture| name(fixture.tree(), focused_key(fixture.tree())).map(str::to_owned);
            assert_eq!(focused_name(&fixture).as_deref(), Some("Find what:"), "the dialog opens with the focus in Find what");
            let in_field = fixture.focused();
            if typed {
                fixture.type_text("teh").settle();
                fixture.key(Key::Escape, Modifiers::NONE).settle();
                assert!(open.get(), "leaves {leaves}: the first Escape reverts the typing and keeps the dialog open");
                assert_eq!(fixture.focused(), in_field, "leaves {leaves}: the focus stays in Find what");
            }
            fixture.key(Key::Escape, Modifiers::NONE).settle();
            assert!(!open.get(), "leaves {leaves}, typed {typed}: Escape closes the dialog");
            assert_eq!(fixture.focused(), opener.get(), "leaves {leaves}, typed {typed}: the focus goes back to the opener");
        }
    }
}

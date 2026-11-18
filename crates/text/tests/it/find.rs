//! Go To in Find and Replace: the requests the app glue answers.

use craft_text::ui::find::{FindReplaceDialog, FindRequest, FindTab, GoToKind};
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, Target, trial};
use std::cell::RefCell;
use std::rc::Rc;

fn open(kinds: Vec<GoToKind>) -> (Fixture, Rc<RefCell<FindReplaceDialog>>) {
    let dialog = Rc::new(RefCell::new(FindReplaceDialog::new("", FindTab::GoTo).word(true).go_to_kinds(kinds)));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shown.borrow_mut().show(ui, &DialogStack::default());
    });
    fixture.idle(30);
    (fixture, dialog)
}

/// The footer button named `label`: the Go To tab carries the same words, and `by_label` finds the tab first.
fn button(fixture: &Fixture, label: &str) -> Option<Target> {
    let tree = fixture.tree();
    fixture.by_role(Role::BUTTON).into_iter().find(|target| craft_ui::testing::access::name(tree, target.node) == Some(label))
}

#[trial]
fn go_to_steps_on_an_empty_page_and_names_a_bookmark() {
    let (mut fixture, dialog) = open(vec![GoToKind::typed("Page", "Enter a page number", true)]);
    let next = fixture.by_label("Next").expect("an empty stepping target offers Next");
    fixture.click(&next).idle(4);
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::go_to_step(0, true)));
    let previous = fixture.by_label("Previous").expect("an empty stepping target offers Previous");
    fixture.click(&previous).idle(4);
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::go_to_step(0, false)));

    let field = fixture.by_label("Page").expect("the target field");
    fixture.click(&field).type_text("12").idle(4);
    assert!(fixture.by_label("Previous").is_none(), "a typed target leaves Previous out");
    let go = button(&fixture, "Go To").expect("a typed target turns Next back into Go To");
    fixture.click(&go).idle(4);
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::go_to(0, "12")));

    // A kind change clears the typed target.
    let (mut fixture, dialog) = open(vec![GoToKind::typed("Page", "Enter a page number", true), GoToKind::typed("Line", "Enter a line number", false)]);
    let field = fixture.by_label("Page").expect("the target field");
    fixture.click(&field).type_text("7").idle(4);
    let kinds = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the Go to what choice");
    fixture.click(&kinds).idle(10);
    let line = fixture.by_label("Line").expect("the Line kind");
    fixture.click(&line).idle(10);
    let fields = dialog.borrow().json_fields().expect("the dialog's fields");
    assert_eq!(fields["goToKind"], 1);
    assert_eq!(fields["goToTarget"], "", "a kind change clears the target");
    let go = button(&fixture, "Go To").expect("Go To is shown");
    fixture.click(&go).idle(4);
    assert_eq!(dialog.borrow_mut().take_request(), Some(FindRequest::go_to(1, "")));

    let (mut fixture, dialog) = open(vec![GoToKind::named("Bookmark", Vec::new(), "No bookmarks")]);
    assert!(fixture.by_label("No bookmarks").is_some(), "no names shows the empty line");
    let go = button(&fixture, "Go To").expect("Go To is shown");
    fixture.click(&go).idle(4);
    assert_eq!(dialog.borrow_mut().take_request(), None, "Go To is disabled without names");
}

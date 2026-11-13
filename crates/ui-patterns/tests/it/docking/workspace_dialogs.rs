//! New Workspace and Delete Workspace answer what `WorkspaceAsk` asks, with the set's refusals.

use craft_behavior::dock::{NamedWorkspace, Workspace, WorkspaceChrome, WorkspaceKeeps, WorkspaceSet};
use craft_ui::patterns::dock::{DeleteWorkspaceDialog, NewWorkspace, NewWorkspaceDialog};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

/// Essentials built in; Review and Mine saved, Mine current.
fn workspaces() -> WorkspaceSet<u32> {
    let mut set = WorkspaceSet::new(vec![NamedWorkspace::new("essentials", "Essentials", Workspace::default())]);
    for name in ["Review", "Mine"] {
        set.save(name, Workspace::default(), WorkspaceChrome::default(), WorkspaceKeeps::NONE).unwrap();
    }
    set
}

fn shows(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().any(|painted| painted.text == text)
}

fn disabled(fixture: &Fixture, label: &str) -> bool {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.tree().is_disabled_key(target.node)
}

#[trial]
fn new_workspace_refuses_a_built_in_name_and_answers_a_new_one_with_its_captures() {
    let answers: Rc<RefCell<Vec<NewWorkspace>>> = Rc::default();
    let seen = answers.clone();
    let set = workspaces();
    let mut fixture = Fixture::new(move |ui| seen.borrow_mut().extend(NewWorkspaceDialog::new(&set).show(ui).accepted));
    fixture.idle(30);
    assert!(disabled(&fixture, "OK"), "OK waits for a name");
    let name = fixture.by_label("Name").expect("the dialog asks for a name");
    fixture.click(&name).idle(4);
    fixture.type_text("essentials").idle(4);
    assert!(shows(&fixture, "A built-in workspace has this name."), "a built-in's name says why it is refused");
    assert!(disabled(&fixture, "OK"), "and OK refuses it");
    fixture.type_text(" 2").idle(4);
    let toolbar = fixture.by_label("Toolbar").expect("the Capture group offers Toolbar");
    fixture.click(&toolbar).idle(4);
    let ok = fixture.by_label("OK").unwrap();
    fixture.click(&ok).idle(10);
    let answers = answers.borrow();
    assert_eq!(answers.len(), 1, "OK answers once");
    assert_eq!(answers[0].name, "essentials 2");
    assert_eq!(answers[0].keeps, WorkspaceKeeps::new(false, false, true), "the boxes that were on are the captures");
}

#[trial]
fn delete_workspace_answers_the_picked_saved_workspace_and_refuses_the_current_one() {
    let answers: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen = answers.clone();
    let set = workspaces();
    let mut fixture = Fixture::new(move |ui| seen.borrow_mut().extend(DeleteWorkspaceDialog::new(&set).show(ui).accepted));
    fixture.idle(30);
    let list = fixture.by_label("Workspace").expect("the dialog lists the saved workspaces");
    fixture.click(&list).idle(10);
    assert!(disabled(&fixture, "Mine"), "the current workspace is listed disabled");
    assert!(!disabled(&fixture, "Review"), "the others can be picked");
    let review = fixture.by_label("Review").unwrap();
    fixture.click(&review).idle(10);
    let ok = fixture.by_label("OK").unwrap();
    fixture.click(&ok).idle(10);
    assert_eq!(*answers.borrow(), ["review"], "the pick starts on the first saved workspace that is not current");
}

//! New Workspace and Delete Workspace answer what `WorkspaceAsk` asks, with the set's refusals.

use craft_behavior::dock::{NamedWorkspace, Workspace, WorkspaceChrome, WorkspaceKeeps, WorkspaceSet};
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog};
use craft_ui::docking::WorkspaceCommands;
use craft_ui::patterns::dock::{DeleteWorkspaceDialog, HostedNewWorkspace, NewWorkspace, NewWorkspaceDialog, WorkspaceSwitcher};
use craft_ui::runtime::commands::CommandRef;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use serde_json::json;
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

#[trial]
fn the_hosted_new_workspace_refuses_a_built_in_name_and_answers_through_the_host() {
    type Hosted = HostedNewWorkspace<u32>;
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let id = host.borrow_mut().open(HostedNewWorkspace::new(&workspaces()));
    fixture.idle(10);
    let ctx = fixture.ctx().clone();

    let fields = host.borrow().get::<Hosted>().and_then(|dialog| dialog.json_fields()).expect("the dialog lists its fields");
    let mut names: Vec<&str> = fields.as_object().expect("an object").keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, ["keyboardShortcuts", "menus", "name", "toolbar"]);

    host.borrow_mut().set_json_field(&ctx, id, "name", json!("essentials")).expect("the name field takes text");
    assert!(!host.borrow_mut().confirm::<Hosted>(&ctx), "a built-in's name is refused as the drawn OK refuses it");
    assert!(host.borrow().is_open::<Hosted>(), "a refused confirm keeps the dialog open");

    host.borrow_mut().set_json_field(&ctx, id, "name", json!("Mine 2")).expect("the name field takes text");
    host.borrow_mut().set_json_field(&ctx, id, "toolbar", json!(true)).expect("the toolbar box is offered");
    assert!(host.borrow_mut().set_json_field(&ctx, id, "menus", json!("yes")).is_err(), "a box takes a bool");
    assert!(host.borrow_mut().confirm::<Hosted>(&ctx), "a new name is accepted");
    let answer = host.borrow_mut().take::<Hosted>().and_then(DialogResult::ok).expect("the answer reaches the app");
    assert_eq!(answer.name, "Mine 2");
    assert_eq!(answer.keeps, WorkspaceKeeps::new(false, false, true), "the boxes set through the fields are the captures");
}

/// Whether the switcher's header menu shows Reset Workspace and New Workspace…: the default workspace commands, or the
/// rows the app gives in their place.
fn switcher_rows(commands: Option<Vec<CommandRef>>) -> (bool, bool) {
    let set = workspaces();
    let mut fixture = Fixture::new(move |ui| {
        let switcher = WorkspaceSwitcher::new(&set);
        let _ = match commands.clone() {
            Some(rows) => switcher.commands(rows),
            None => switcher,
        }
        .show(ui);
    });
    fixture.settle();
    let select = fixture.by_label("Choose a workspace").expect("the switcher");
    fixture.click(&select).settle();
    let more = fixture.by_label("More options").expect("the list's header menu button");
    fixture.click(&more).settle();
    (shows(&fixture, "Reset Workspace"), shows(&fixture, "New Workspace…"))
}

// The switcher's list has the default workspace commands, and only the app's rows when it gives its own.
#[trial]
fn the_switcher_lists_the_apps_command_rows_in_place_of_the_default_ones() {
    assert_eq!(switcher_rows(None), (true, true), "the default rows");
    assert_eq!(switcher_rows(Some(vec![WorkspaceCommands::New.into()])), (false, true), "only the app's rows");
}

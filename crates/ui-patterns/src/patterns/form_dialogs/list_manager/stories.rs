//! Stories of `ListManager`.

use super::{ActionEnabled, HostedListManager, ListManager, ListManagerAction, ListManagerEntry, ListManagerWords};
use crate::buttons::Button;
use crate::choice::{Checkbox, Radio, RadioGroup};
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use crate::dialog::stories::hosted;
use crate::layout_components::Stack;
use crate::runtime::env::Env;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::cell::Cell;

fn entries() -> Vec<ListManagerEntry> {
    vec![
        ListManagerEntry::new("a", "Align Layers").detail("Ctrl+Shift+A").description("Aligns the selected layers to the key layer."),
        ListManagerEntry::new("b", "Batch Export").description("Exports every artboard."),
        ListManagerEntry::new("c", "Flatten Image").detail("script, 3 steps").description("Merges all layers into one."),
    ]
}

#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let actions =
        vec![ListManagerAction::run(&words), ListManagerAction::edit(&words), ListManagerAction::create(&words), ListManagerAction::delete(&words)];
    ListManager::new("Macros", "Macro name:", entries(), actions)
        .name("Batch Export")
        .searchable(true)
        .note("This document also has a script project.".into())
        .opened(data.bind("open", true))
        .show(ui)
        .into()
}

/// Entries of kind 1 rename in place: F2 on a row edits its name there.
#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager_renames(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let actions = vec![ListManagerAction::run(&words), ListManagerAction::delete(&words)];
    let entries: Vec<ListManagerEntry> = entries().into_iter().enumerate().map(|(at, entry)| if at == 1 { entry } else { entry.kind(1) }).collect();
    ListManager::new("Macros", "Macro name:", entries, actions).renames(1).opened(data.bind("open", true)).show(ui).into()
}

/// A list manager whose name box the caller owns: the story keeps the name, and a button under the list sets it on
/// the next pass, as a script sets the Macros dialog's name. The dialog opens from its button.
#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager_name_box(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let ((name, name_at), (open, open_at)) = data.pair_at(("name", String::new()), ("open", false));
    if clicked(&Button::new("Macros…").show(ui)) {
        *open = true;
    }
    let first = Cell::new(false);
    let actions = vec![ListManagerAction::run(&words), ListManagerAction::create(&words)];
    let shown = ListManager::new("Macros", "Macro name:", entries(), actions)
        .name_box(name_at.bind(&mut *name))
        .under_list(|ui| {
            if clicked(&Button::new("Name the first macro").show(ui)) {
                first.set(true);
            }
        })
        .opened(open_at.bind(&mut *open))
        .show(ui);
    if first.get() {
        *name = "Align Layers".to_owned();
    }
    shown.into()
}

/// A bookmark name is valid while it is not empty and fits the name box.
fn bookmark_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 32
}

/// A bookmark manager: Add follows the name check, Go To and Delete keep the dialog open, Close ends it. A Sort by
/// choice and a Hidden box stand under the list. The dialog opens from its button.
#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager_bookmark(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let mut by_name = *data.value("sort by name", true);
    let mut hidden = *data.value("hidden", false);
    let mut gone = data.value("deleted", Vec::<String>::new()).clone();
    let (open, open_at) = data.value_at("open", false);
    if clicked(&Button::new("Bookmarks…").show(ui)) {
        *open = true;
    }
    let actions = vec![
        ListManagerAction::new("add", "Add", ActionEnabled::OnValidName).keeps_open(),
        ListManagerAction::new("goTo", "Go To", ActionEnabled::OnEntry { kind: None }).keeps_open(),
        ListManagerAction::delete(&words),
    ];
    let marks: Vec<ListManagerEntry> =
        ["Cover", "Chapter 1", "Index"].iter().map(|name| ListManagerEntry::new(*name, *name)).filter(|entry| !gone.contains(&entry.id)).collect();
    let shown = ListManager::new("Bookmarks", "Bookmark name:", marks, actions)
        .empty("There are no bookmarks.".into())
        .name_check(bookmark_name)
        .cancel_label("Close")
        .under_list(|ui| {
            let sorted = RadioGroup::new("Sort by", Bind::given(by_name)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    Radio::new(true).label("Name").show(ui);
                    Radio::new(false).label("Location").show(ui);
                });
            });
            if let Some(on) = sorted.output.value().copied() {
                by_name = on;
            }
            if let Some(on) = Checkbox::new(Bind::given(hidden)).label("Hidden").show(ui).value().copied() {
                hidden = on;
            }
        })
        .opened(open_at.bind(open))
        .show(ui);
    if let Some(choice) = &shown.choice
        && choice.action == "delete"
        && let Some(id) = &choice.entry
    {
        gone.push(id.clone());
    }
    *data.value("deleted", Vec::<String>::new()) = gone;
    *data.value("sort by name", true) = by_name;
    *data.value("hidden", false) = hidden;
    shown.into()
}

#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let actions = vec![ListManagerAction::run(&words), ListManagerAction::create(&words)];
    ListManager::new("Macros", "Macro name:", Vec::new(), actions)
        .empty("This document has no scripts or macros.".into())
        .opened(data.bind("open", true))
        .show(ui)
        .into()
}

/// The list manager on the dialog host: Run closes it with its choice, Delete keeps it open for the host.
#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn hosted_list_manager(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    hosted(ui, data, (), || {
        HostedListManager::new("macros", "Macros", "Macro name:", vec![ListManagerAction::run(&words), ListManagerAction::delete(&words)])
            .entries(entries())
    })
}

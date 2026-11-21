//! Stories of `ListManager`.

use super::{ListManager, ListManagerAction, ListManagerEntry, ListManagerWords};
use crate::contract::Bind;
use crate::runtime::env::Env;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

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
        .opened(Bind::bound(data.value("open", true)))
        .show(ui)
        .into()
}

/// Entries of kind 1 rename in place: F2 on a row edits its name there.
#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager_renames(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let actions = vec![ListManagerAction::run(&words), ListManagerAction::delete(&words)];
    let entries: Vec<ListManagerEntry> = entries().into_iter().enumerate().map(|(at, entry)| if at == 1 { entry } else { entry.kind(1) }).collect();
    ListManager::new("Macros", "Macro name:", entries, actions).renames(1).opened(Bind::bound(data.value("open", true))).show(ui).into()
}

#[story(ListManager<'static>, covers = [], drive = Drive::Never)]
fn list_manager_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ListManagerWords>();
    let actions = vec![ListManagerAction::run(&words), ListManagerAction::create(&words)];
    ListManager::new("Macros", "Macro name:", Vec::new(), actions)
        .empty("This document has no scripts or macros.".into())
        .opened(Bind::bound(data.value("open", true)))
        .show(ui)
        .into()
}

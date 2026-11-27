//! Stories of `PresetManager`.

use super::PresetManager;
use crate::contract::{Bind, Entry};
use crate::patterns::presets::{PresetFns, PresetRow, PresetSource};
use crate::runtime::env::Env;
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The preset types the stories offer.
fn kinds() -> Vec<Entry<u32>> {
    vec![Entry::new(0, "Brushes"), Entry::new(1, "Custom Shapes"), Entry::new(2, "Patterns")]
}

/// Shows a manager over `rows` of the picked kind; `load` adds the host's Load… button. The preset keyed 1 is
/// built in: it cannot be renamed or deleted.
fn show_manager(ui: &mut Ui, data: &mut StoryData, rows: fn(&(), &Env) -> Vec<PresetRow<u32>>, load: bool) -> StoryOutput {
    let kinds = kinds();
    let source = PresetSource::new((), PresetFns::new(rows).can_rename(not_built_in).can_delete(not_built_in));
    // `data` hands out one value at a time, so the picked kind is copied out and written back.
    let mut kind = *data.value("kind", 0u32);
    let mut manager = PresetManager::new("Preset Manager", &kinds, Bind::bound(&mut kind), &source).opened(data.bind("open", true));
    if load {
        manager = manager.actions(vec![(Label::from("Load…"), Some(Label::from("Import brushes (.abr)")))]);
    }
    let out = manager.show(ui);
    *data.value("kind", 0u32) = kind;
    let readout = if !out.taken.actions.is_empty() {
        String::from("Load… pressed.")
    } else {
        match out.taken.kind {
            Some(kind) => format!("Type {kind} picked."),
            None => String::from("Nothing taken."),
        }
    };
    StoryOutput::from(out).readout(readout)
}

/// The preset keyed 1 is built in.
fn not_built_in(_: &(), key: &u32) -> bool {
    *key != 1
}

#[story(PresetManager<'static, u32, u32>, covers = [], drive = Drive::Never)]
fn preset_manager(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_manager(ui, data, |_, _| vec![PresetRow::item(2, "Soft Round", 0), PresetRow::item(3, "Chalk", 0), PresetRow::item(4, "Hard Edge", 0)], true)
}

#[story(PresetManager<'static, u32, u32>, covers = [], drive = Drive::Never)]
fn preset_manager_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_manager(ui, data, |_, _| Vec::new(), false)
}

#[story(PresetManager<'static, u32, u32>, covers = [], drive = Drive::Never)]
fn preset_manager_built_in(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_manager(ui, data, |_, _| vec![PresetRow::item(1, "Round (built-in)", 0), PresetRow::item(2, "Soft Round", 0)], false)
}

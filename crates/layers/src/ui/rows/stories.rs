//! Stories of `BlendRows`.

use super::{BlendRows, LayerPropertyEdit};
use crate::ui::mock::FakeLayers;
use crate::ui::tree::{LayerProperties, LayerTree as _, LockKind};
use craft_ui::contract::Entry;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// Photoshop's blend modes, as the app offers them for a layer or, with Pass Through, a group.
fn modes(group: bool) -> Vec<Entry<String>> {
    FakeLayers::sample().blend_modes(group, "normal")
}

/// The opacity of the story's layer.
const OPACITY: f64 = 80.0;

/// Applies what the rows report to `properties`, as an app applies it to its layer.
fn apply(properties: &mut LayerProperties, events: &[LayerPropertyEdit]) {
    for event in events {
        match event {
            LayerPropertyEdit::Blend(edit) => {
                if let Some(mode) = edit.value() {
                    properties.blend_key.clone_from(mode);
                }
            }
            LayerPropertyEdit::Opacity(edit) => {
                if let Some(value) = edit.value() {
                    properties.opacity = *value;
                }
            }
            LayerPropertyEdit::Fill(edit) => {
                if let Some(value) = edit.value() {
                    properties.fill = *value;
                }
            }
            LayerPropertyEdit::Lock { kind, on } => match kind {
                LockKind::Transparency => properties.locks.transparency = *on,
                LockKind::Pixels => properties.locks.pixels = *on,
                LockKind::Position => properties.locks.position = *on,
                LockKind::Artboard => properties.locks.artboard = *on,
                LockKind::All => properties.locks.all = *on,
            },
        }
    }
}

fn show(ui: &mut Ui, properties: &mut Option<LayerProperties>) -> StoryOutput {
    let modes = modes(properties.as_ref().is_some_and(|properties| properties.group));
    let out = BlendRows::new(properties.as_ref(), &modes).show(ui);
    if let Some(properties) = properties {
        apply(properties, &out.events);
    }
    out.into()
}

#[story(BlendRows<'static>, covers = [Default], drive = Drive::Never, group = "States", title = "Normal layer")]
fn a_layer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let properties = data.value("properties", Some(LayerProperties { opacity: OPACITY, ..LayerProperties::new("multiply") }));
    show(ui, properties)
}

#[story(BlendRows<'static>, covers = [], drive = Drive::Never, group = "States", title = "Background")]
fn the_background(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let properties = data.value("properties", Some(LayerProperties { editable: false, ..LayerProperties::new("normal") }));
    show(ui, properties)
}

#[story(BlendRows<'static>, covers = [], drive = Drive::Never, group = "States", title = "Several layers selected")]
fn mixed_locks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut properties = LayerProperties::new("normal");
    properties.locks.pixels = true;
    properties.locks.position = true;
    let properties = data.value("properties", Some(properties));
    show(ui, properties)
}

#[story(BlendRows<'static>, covers = [], drive = Drive::Never, group = "States", title = "No active layer")]
fn no_layer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let properties = data.value("properties", None::<LayerProperties>);
    show(ui, properties)
}

#[story(BlendRows<'static>, covers = [], drive = Drive::Never, title = "Group")]
fn a_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let properties = data.value("properties", Some(LayerProperties { group: true, ..LayerProperties::new("passThrough") }));
    show(ui, properties)
}

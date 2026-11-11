//! Stories of `Inspector`.

use super::Inspector;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::primitives::IconSpec;
use crate::sections::{Section, SectionStack};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::value_drag::Slider;
use craft_behavior::IconId;
use egui::Ui;

const TITLES: [&str; 3] = ["Transform", "Appearance", "Effects"];

#[story(Inspector<'static>, covers = [], drive = Drive::Never)]
fn inspector(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let order = data.value("order", vec![0_u8, 1, 2]).clone();
    let layer = *data.choice("target", true, &[("Layer", true), ("Document", false)]);
    let inspector = if layer {
        Inspector::new("Properties").target("layer-1").header("Layer 1").icon(IconSpec::svg(IconId::new("settings"))).kind("Pixel layer")
    } else {
        Inspector::new("Properties").target("document").header("Document")
    };
    let output = inspector.show(ui, |ui, _| {
        let stack = SectionStack::new(&order);
        let rows = stack.rows();
        let shown = stack
            .show(ui, |ui, _| {
                rows.each(ui, |ui, key, slot| {
                    let open = Bind::bound(data.value(TITLES[usize::from(*key)], true));
                    Section::new(TITLES[usize::from(*key)])
                        .open(open)
                        .in_stack(slot)
                        .show(ui, |ui, _| {
                            Slider::new(Bind::given(30.0), 0.0..=100.0).label("Opacity").show(ui);
                        })
                        .output
                });
            })
            .output;
        let order = data.value("order", vec![0_u8, 1, 2]);
        for edit in &shown.moves {
            if let Some(moved) = edit.value()
                && let Some(from) = order.iter().position(|key| *key == moved.key)
            {
                let key = order.remove(from);
                order.insert(moved.to.min(order.len()), key);
            }
        }
    });
    output.into()
}

#[story(Inspector<'static>, covers = [], drive = Drive::Never)]
fn inspector_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Inspector::new("Properties").show(ui, |_, _| {}).into()
}

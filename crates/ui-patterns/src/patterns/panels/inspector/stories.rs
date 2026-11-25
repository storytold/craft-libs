//! Stories of `Inspector`.

use super::Inspector;
use crate::contract::{Bind, Props as _, QuietHeader, Show as _, ShowContainer as _};
use crate::primitives::IconSpec;
use crate::runtime::env::with_env;
use crate::sections::{HeaderStrength, Section, SectionStack};
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
                    let open = data.bind(TITLES[usize::from(*key)], true);
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

/// A quiet title: the target's kind as a muted line over its sections, with no rule under it.
#[story(Inspector<'static>, covers = [], drive = Drive::Never)]
fn inspector_quiet_title(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let inspector = Inspector::new("Properties").target("ellipse").header("Ellipse").title_strength(HeaderStrength::Quiet);
    inspector
        .show(ui, |ui, _| {
            Section::fixed("Transform").show(ui, |ui, _| {
                Slider::new(Bind::given(30.0), 0.0..=100.0).label("Opacity").show(ui);
            });
        })
        .into()
}

/// A quiet title in an app whose quiet titles head their body (`QuietHeader::Heads`): the first section under the kind
/// line draws no rule above it; the second keeps its rule.
#[story(Inspector<'static>, covers = [], drive = Drive::Never)]
fn inspector_quiet_title_heads(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    with_env(
        ui,
        |overrides| {
            overrides.values.set(QuietHeader::Heads);
        },
        |ui| {
            let inspector = Inspector::new("Properties").target("ellipse").header("Ellipse").title_strength(HeaderStrength::Quiet);
            inspector
                .show(ui, |ui, _| {
                    for title in &TITLES[..2] {
                        Section::fixed(*title).show(ui, |ui, _| {
                            Slider::new(Bind::given(30.0), 0.0..=100.0).label("Opacity").show(ui);
                        });
                    }
                })
                .into()
        },
    )
}

#[story(Inspector<'static>, covers = [], drive = Drive::Never)]
fn inspector_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Inspector::new("Properties").show(ui, |_, _| {}).into()
}

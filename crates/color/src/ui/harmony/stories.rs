//! Stories of `HarmonyWheel` and `HarmonyPanel`.

use super::{HarmonyPanel, HarmonyPanelState, HarmonyPicked, HarmonyWheel};
use crate::harmony::Harmony;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const BASE: Srgba = Srgba::rgb(0.86, 0.24, 0.16);

#[story(HarmonyPanel<'static>, covers = [], drive = Drive::Never)]
fn harmony_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, picked) = data.value("guide", (triad(), HarmonyPicked::default()));
    let colors = state.rule.apply(BASE);
    // The wheel above the panel shows the panel's rule on the base colour, as a Color Guide works with the wheel.
    Stack::new()
        .show(ui, |ui, _| {
            let _ = HarmonyWheel::new("Harmony", &colors).selected_marker(Some(0)).show(ui);
            StoryOutput::from(HarmonyPanel::new(Bind::bound(&mut *state), Bind::bound(&mut *picked), BASE).show(ui))
        })
        .inner
        .unwrap_or_default()
}

/// The panel set to a triad.
fn triad() -> HarmonyPanelState {
    HarmonyPanelState { rule: Harmony::Triad, ..HarmonyPanelState::default() }
}

#[story(HarmonyWheel<'static>, covers = [], drive = Drive::Never)]
fn harmony_wheel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let colors = Harmony::Triad.apply(BASE);
    HarmonyWheel::new("Harmony", &colors).selected_marker(Some(1)).show(ui).into()
}

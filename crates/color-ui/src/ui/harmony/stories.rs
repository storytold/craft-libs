//! Stories of `HarmonyWheel` and `HarmonyPanel`.

use super::{DiscLook, GuideCellAt, HarmonyPanel, HarmonyPanelState, HarmonyPicked, HarmonyWheel};
use crate::harmony::Harmony;
use craft_theme::Srgba;
use craft_ui::contract::ShowContainer as _;
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const BASE: Srgba = Srgba::rgb(0.86, 0.24, 0.16);

#[story(HarmonyPanel<'static>, covers = [], drive = Drive::Never)]
fn harmony_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((state, picked), guide_at) = data.value_at("guide", (triad(), HarmonyPicked::default()));
    let colors = state.rule.apply(BASE);
    // The wheel above the panel shows the panel's rule on the base colour, as a Color Guide works with the wheel.
    Stack::new()
        .show(ui, |ui, _| {
            let _ = HarmonyWheel::new("Harmony", &colors).selected_marker(Some(0)).show(ui);
            StoryOutput::from(HarmonyPanel::new(guide_at.bind(&mut *state), guide_at.bind(&mut *picked), BASE).show(ui))
        })
        .inner
        .unwrap_or_default()
}

/// The spot tint the host shows in the strip's cells.
const SPOT: Srgba = Srgba::rgb(0.95, 0.65, 0.1);

#[story(HarmonyPanel<'static>, covers = [], drive = Drive::Never, title = "Host cell colors")]
fn harmony_panel_host_cells(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((state, state_at), (picked, picked_at)) = data.pair_at(("guide", triad()), ("picked", HarmonyPicked::default()));
    HarmonyPanel::new(state_at.bind(state), picked_at.bind(picked), BASE)
        .cell_color(|cell| match cell.at {
            GuideCellAt::Strip(_) => SPOT,
            GuideCellAt::Grid { .. } => cell.color,
        })
        .drag_payload(|cell| Some(cell.color))
        .show(ui)
        .into()
}

#[story(HarmonyWheel<'static>, covers = [], drive = Drive::Never, title = "Segmented")]
fn harmony_wheel_segmented(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let colors = Harmony::Triad.apply(BASE);
    HarmonyWheel::new("Harmony", &colors).look(DiscLook::Segmented).selected_marker(Some(1)).show(ui).into()
}

/// The panel set to a triad.
fn triad() -> HarmonyPanelState {
    HarmonyPanelState { rule: Harmony::Triad, ..HarmonyPanelState::default() }
}

/// Recolor's wheel: the base colour drawn large, the selected one ringed.
#[story(HarmonyWheel<'static>, covers = [], drive = Drive::Never, title = "Base")]
fn harmony_wheel_base(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let colors = Harmony::Triad.apply(BASE);
    HarmonyWheel::new("Harmony", &colors).selected_marker(Some(1)).base(Some(0)).show(ui).into()
}

#[story(HarmonyWheel<'static>, covers = [], drive = Drive::Never)]
fn harmony_wheel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let colors = Harmony::Triad.apply(BASE);
    HarmonyWheel::new("Harmony", &colors).selected_marker(Some(1)).show(ui).into()
}

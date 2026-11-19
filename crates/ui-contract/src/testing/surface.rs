//! The egui behaviours the runtime relies on, as scenarios on a bare egui context: the `egui_surface` trials
//! assert on what they return.

pub use crate::runtime::egui::surface::{
    HookFacts, LayerOrder, PassFacts, PressHits, child_viewport_hooks, discard_passes, focus_survives_escape, layer_order, preset_style_direction,
    press_hits, registered_parent_owns_member_node,
};

/// The root widget every member resolves through, built with the runtime's own `InteractOptions`, and whether the
/// pass clicked it. The `activation` trial pins Space's route with it: Space must not click a focused control.
/// Exposed here because the library target runs no tests of its own.
#[doc(hidden)]
pub fn root_widget(ui: &egui::Ui, id: egui::Id, rect: egui::Rect, sense: craft_behavior::Sense) -> bool {
    let response = crate::runtime::egui::response::widget(ui, id, rect, sense);
    crate::runtime::egui::response::clicked(&response)
}

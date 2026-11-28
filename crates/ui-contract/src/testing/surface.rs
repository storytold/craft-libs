//! The egui behaviours the runtime relies on, as scenarios on a bare egui context: the `egui_surface` trials
//! assert on what they return.

pub use crate::runtime::egui::surface::{
    HookFacts, LayerOrder, PassFacts, PressHits, child_viewport_hooks, discard_passes, focus_survives_escape, layer_order, preset_style_direction,
    press_hits, registered_parent_owns_member_node,
};

//! `contract/layers`: egui layers, popups and tooltips outside `runtime::egui::areas`.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/layers";

/// Permission to use `Popup::swallow_outside_press`: no file has it.
const NEVER: &str = "contract/layers:never";

const LAYER_TYPES: [&str; 3] = ["Area", "Popup", "Modal"];

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    let why = "opens or orders an egui layer; go through `runtime::egui::areas`";
    cx.flag_methods(out, ID, ID, why, |name, _| {
        matches!(name, "move_to_top" | "set_sublayer" | "set_transform_layer" | "on_hover_text" | "menu_button" | "context_menu")
    });
    cx.flag_paths(out, ID, ID, why, |p| {
        p.windows(2).any(|w| w[0] == "Order" || (LAYER_TYPES.contains(&w[0].as_str()) && w[1].starts_with(char::is_lowercase)))
    });
    cx.flag_methods(out, ID, NEVER, "is not allowed in any layer; the pipeline owns outside presses", |name, _| name == "swallow_outside_press");
}

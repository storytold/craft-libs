//! `contract/access`: AccessKit node building and reads outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/access";

const METHODS: [&str; 6] =
    ["widget_info", "labelled_by", "register_accesskit_parent", "set_accesskit_focus", "accesskit_node_builder", "has_accesskit_action_request"];

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    let why = "builds or reads the accessibility tree; go through `runtime::egui::access`";
    cx.flag_methods(out, ID, ID, why, |name, _| {
        METHODS.contains(&name)
            || name.starts_with("accessibility_")
            || name.starts_with("accesskit_")
            || name.starts_with("set_accessibility_")
            || matches!(name, "set_live" | "set_container_live")
    });
}

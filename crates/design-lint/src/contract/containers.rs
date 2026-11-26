//! `contract/containers`: egui containers outside `runtime::egui`; `ScrollArea` nowhere.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/containers";

/// Permission to use `ScrollArea`: no file has it, scrolling is scroll parts (8.6).
const NEVER: &str = "contract/containers:never";

const CONTAINERS: [&str; 7] = ["Panel", "CentralPanel", "SidePanel", "TopBottomPanel", "Window", "CollapsingHeader", "Resize"];

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    cx.flag_paths(out, ID, NEVER, "scrolls with egui's `ScrollArea`; scrolling is scroll parts", |p| p.iter().any(|s| s == "ScrollArea"));
    cx.flag_paths(out, ID, ID, "is an egui container; go through `runtime::egui`", |p| {
        p.iter().any(|s| CONTAINERS.contains(&s.as_str()) || s == "TableBuilder")
    });
    cx.flag_methods(out, ID, ID, "is an egui virtual-list body; go through `runtime::egui`", |name, _| {
        name == "show_rows" || name == "show_viewport"
    });
}

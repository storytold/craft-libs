//! `contract/allocate`: allocation and hand-placed widgets outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/allocate";

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    let why = "allocates or places a widget by hand; go through `runtime::egui::alloc`";
    cx.flag_methods(out, ID, ID, why, |name, args| {
        name.starts_with("allocate_")
            || (name == "interact" && args.count == 3)
            || name == "interact_opt"
            || (matches!(name, "put" | "add_sized") && args.count == 2)
            || (name == "new_child" && args.idents.iter().any(|i| i == "max_rect"))
    });
}

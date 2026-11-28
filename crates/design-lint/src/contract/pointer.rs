//! `contract/pointer`: drag and click queries, press-origin reads and hover-position reads outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/pointer";

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    cx.flag_methods(
        out,
        ID,
        ID,
        "reads egui's pointer position; `Cx::hover_pos` and `pointer::hover_point_in` read it and mark what follows the pointer",
        |name, args| matches!(name, "pointer_hover_pos" | "pointer_latest_pos" | "pointer_interact_pos") || (name == "hover_pos" && args.count == 0),
    );
    cx.flag_methods(out, ID, ID, "reads egui's pointer state; the pointer frame carries presses and drags", |name, _| {
        matches!(
            name,
            "dragged"
                | "dragged_by"
                | "drag_started"
                | "drag_started_by"
                | "drag_stopped"
                | "drag_stopped_by"
                | "drag_delta"
                | "drag_motion"
                | "dragged_id"
                | "is_being_dragged"
                | "clicked"
                | "clicked_by"
                | "double_clicked"
                | "triple_clicked"
                | "secondary_clicked"
                | "middle_clicked"
                | "long_touched"
                | "interact_pointer_pos"
                | "is_pointer_button_down_on"
                | "press_origin"
        )
    });
}

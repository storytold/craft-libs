//! `contract/pointer`: drag and click queries and press-origin reads outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;

pub const ID: &str = "contract/pointer";

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
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

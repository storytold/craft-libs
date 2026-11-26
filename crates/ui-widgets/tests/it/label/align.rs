//! A content-sized group around a wide control under a short label.

use super::group::Group;
use super::kit::{Dial, Seen};
use craft_ui::contract::{Props as _, ShowContainer as _, SideLabel};
use craft_ui::testing::{Frames, trial};
use egui::Rect;
use std::cell::RefCell;

/// A content-sized group is as wide as its rows at the widest label: a wide control under a short label, measured
/// before the long label of a later row, stays inside the group.
#[trial]
fn a_content_sized_group_holds_a_wide_control_under_a_short_label() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::ContainerProps;
    let mut frames = Frames::new();
    let wide = RefCell::new(Seen::default());
    let mut value = 1.0;
    let mut group = Rect::NOTHING;
    frames.idle(3, |ui| {
        group = Group::new(ContainerProps::new().label_column(true))
            .id_salt("held")
            .show(ui, |ui, cx| {
                cx.labelled(ui, "short", "A".into(), SideLabel::new(), Dial::new(&mut value, &wide));
                cx.labelled(ui, "long", "Hue and saturation".into(), SideLabel::new(), Button::new("OK"));
            })
            .rect;
    });
    let control = wide.borrow().rect;
    let label = wide.borrow().label.clone().map_or(Rect::NOTHING, |label| label.rect);
    assert!(control.min.x > label.max.x, "the control sits beside its label: {label:?} {control:?}");
    assert!(group.contains_rect(control), "the group holds the control: {group:?} {control:?}");
}

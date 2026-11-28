//! A label after its control adds the gap and its own width to the container, not the rest of the row.

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, FieldLayout, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::form::Field;
use craft_ui::testing::snapshot::layer_rects;
use craft_ui::testing::{Fixture, trial};
use egui::{Vec2, vec2};

/// The width of a content-sized dialog holding one checkbox field laid out as `layout`, in a window of `room`.
fn dialog_width(layout: FieldLayout, room: Vec2) -> f32 {
    let mut fixture = Fixture::new(move |ui| {
        Dialog::new("Grid").show(ui, |ui, _| {
            Field::new("Snap to grid").layout(layout).show(ui, |ui, _| {
                Checkbox::new(Bind::given(true)).show(ui);
            });
        });
    });
    fixture.room(room);
    fixture.idle(20);
    let layers = layer_rects(fixture.ctx());
    assert_eq!(layers.len(), 1, "one dialog layer");
    layers[0].width()
}

#[trial]
fn a_dialog_with_a_label_after_its_checkbox_is_as_wide_as_its_content() {
    let (small, large) = (vec2(800.0, 600.0), vec2(1600.0, 1000.0));
    let after = dialog_width(FieldLayout::After, small);
    assert_eq!(after, dialog_width(FieldLayout::After, large), "the dialog's width does not follow the window's");
    // Beside, the label column and the gap come before the control; after, the gap and the label come past it.
    let beside = dialog_width(FieldLayout::Beside, small);
    assert!((after - beside).abs() <= 8.0, "after {after}, beside {beside}");
}

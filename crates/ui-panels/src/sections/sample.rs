//! The body the sections stories put in their boxes.

use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Stack;
use crate::value_drag::Slider;
use egui::Ui;

/// A check box and a slider of fixed values.
pub(super) fn body(ui: &mut Ui) {
    Stack::new().show(ui, |ui, _| {
        Checkbox::new(Bind::given(true)).label("Snap to grid").show(ui);
        Slider::new(Bind::given(30.0), 0.0..=100.0).label("Spacing").show(ui);
    });
}

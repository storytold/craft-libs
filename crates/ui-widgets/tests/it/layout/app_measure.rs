//! An app that sizes a row itself, measuring the component inside `runtime::measure::run` and then showing it with
//! `show_at` (LightCraft's slider rows), leaves the body around it settled: a settled frame runs one pass.

use craft_behavior::Proposal;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::SliderField;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::measure;
use craft_ui::testing::{Frames, trial};
use egui::{Ui, vec2};

fn rows(ui: &mut Ui) {
    let _ = Stack::new().id_salt("rows").show(ui, |ui, _| {
        for (row, label) in ["Exposure", "Contrast", "Highlights"].into_iter().enumerate() {
            let mut field = SliderField::new(label, Bind::given(0.5), -5.0..=5.0).step(0.01).id_salt(row);
            let width = ui.available_width().max(1.0);
            let ctx = ui.ctx().clone();
            let size = measure::run(&ctx, || field.measure_in(ui, Proposal::Width(width)));
            let (_, room) = ui.allocate_space(vec2(width, size.y));
            field.show_at(ui, room);
        }
    });
}

#[trial]
fn rows_an_app_measures_itself_and_shows_at_a_rect_settle_to_one_pass() {
    let mut frames = Frames::new();
    frames.idle(4, rows);
    assert_eq!(frames.frame(vec![], rows), 1, "a settled frame runs one pass");
    assert_eq!(frames.frame(vec![], rows), 1, "and the next one too");
}

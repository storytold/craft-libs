//! A caption above its control counts whole toward its field's least width.

use craft_behavior::Proposal;
use craft_theme::Srgba;
use craft_ui::choice::Swatch;
use craft_ui::contract::{FieldLayout, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::form::Field;
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

const CAPTION: &str = "Preview of the page";
const GREY: Srgba = Srgba { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
/// How far two widths may differ and still count as the same, in points.
const SLACK: f32 = 0.5;

/// A row that narrows a field to its least narrows it no further than its caption above a small control (the
/// Preview of the Columns dialog), so the caption is not cut.
#[trial]
fn a_caption_above_a_small_control_counts_whole_toward_the_least_width() {
    let least = Rc::new(Cell::new(0.0));
    let seen = least.clone();
    let field = || Field::new(CAPTION).layout(FieldLayout::Above).sizing(Sizing::Content);
    let mut fixture = Fixture::new(move |ui| {
        let body = |ui: &mut egui::Ui, _: &mut craft_ui::contract::BodyCx<'_>| {
            Swatch::new("Page", GREY).show(ui);
        };
        seen.set(field().measure_in(ui, Proposal::Min, body).x);
        field().show(ui, body);
    });
    fixture.settle();
    let caption = fixture.texts().iter().find(|run| run.text == CAPTION).map(|run| run.rect).expect("the caption is painted whole");
    assert!(least.get() >= caption.width() - SLACK, "the field's least width holds its caption: {} < {}", least.get(), caption.width());
}

//! A row of continuous and levelled units: the continuous unit narrows first, the levelled one then steps down a level
//! at a time, and the row fits its room.

use std::cell::Cell;
use std::rc::Rc;

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::display::{Elide, Text};
use craft_ui::layout::levelled_unit;
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, is_measuring, trial};
use egui::{Id, Shape};

/// The continuous unit's text: an elided line that cuts from its end.
const LINE: &str = "Quarterly report of the northern region, second draft";

/// The levelled unit's labels, widest first.
const LEVELS: [&str; 3] = ["A wide label for the widest level", "Middle level", "S"];

/// The level the unit showed at in the real run, and the text the line showed, for a row `width` wide.
fn shown(width: f32) -> (usize, String) {
    let level = Rc::new(Cell::new(usize::MAX));
    let seen = level.clone();
    let mut fixture = Fixture::new(move |ui| {
        Row::new().fill_width().show(ui, |ui, _| {
            Text::new(LINE).elide(Elide::End).show(ui);
            levelled_unit(ui, Id::unique("levelled"), LEVELS.len(), |ui, at| {
                if !is_measuring(ui.ctx()) && !ui.is_sizing_pass() {
                    seen.set(at);
                }
                Button::new(LEVELS[at]).show(ui);
            });
        });
    });
    fixture.room(egui::vec2(width, 60.0)).settle();
    let text = fixture
        .frame_shapes()
        .iter()
        .find_map(|shape| match shape {
            Shape::Text(text) if text.galley.job.text.starts_with("Quarterly") => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    (level.get(), text)
}

#[trial]
fn a_mixed_row_narrows_its_continuous_unit_first_then_steps_its_levelled_unit_down() {
    let (wide, whole) = shown(900.0);
    assert_eq!(wide, 0, "a row with room shows the levelled unit at its widest level");
    assert_eq!(whole, LINE, "a row with room cuts no text");
    let mut last = 0;
    let mut cut_at_widest = false;
    for step in 0..=42 {
        let width = 900.0 - step as f32 * 20.0;
        let (level, text) = shown(width);
        assert!(level < LEVELS.len(), "the unit showed in the real run ({width})");
        assert!(level >= last, "a narrower row never shows the levelled unit wider ({width}): {level} after {last}");
        cut_at_widest |= level == 0 && text != LINE;
        last = level;
    }
    assert!(cut_at_widest, "the continuous unit gives way before the levelled unit steps down");
    assert_eq!(last, LEVELS.len() - 1, "a row with no room shows the levelled unit at its narrowest level");
}

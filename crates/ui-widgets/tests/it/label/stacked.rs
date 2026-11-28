//! A group whose label column is too wide for its room puts every row's control under its label: the rows measured
//! before the widest label take the stacked height too.

use craft_theme::Size;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Rect, Vec2};

/// How far two edges may differ and still line up, in points.
const SLACK: f32 = 0.5;

/// The rows of the group, with the widest label last, and the value each field shows.
const ROWS: [(&str, f64); 3] = [("Width", 11.0), ("Corner radius", 22.0), ("Maximum number of layers", 33.0)];

fn control(fixture: &Fixture, name: &str) -> Rect {
    fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} names its control")).rect
}

fn caption(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text:?} is painted"))
}

/// In a group 240 pt wide, the last row's label is too wide to sit beside a control, so every row stacks. Each field
/// shows under its label and above the next row, and its value shows whole, not clipped to the row it measured
/// beside a narrower column.
#[trial]
fn every_row_of_a_group_that_stacks_takes_the_stacked_height() {
    let mut fixture = Fixture::new(|ui| {
        FieldGroup::new("Shape").show(ui, |ui, _| {
            for (name, value) in ROWS {
                Field::new(name).show(ui, |ui, _| NumberField::new(name, Bind::given(Some(value))).show(ui));
            }
        });
    });
    fixture.room(Vec2::new(240.0, 600.0)).density(Size::Comfortable);
    fixture.settle();
    let mut above = f32::NEG_INFINITY;
    for (name, value) in ROWS {
        let (label, field) = (caption(&fixture, name), control(&fixture, name));
        assert!(label.max.y <= field.min.y + SLACK, "{name}'s field shows under its label: {label:?} {field:?}");
        assert!(label.min.y >= above - SLACK, "{name}'s row starts under the field before: {label:?} {above}");
        above = field.max.y;
        let digits = format!("{value}");
        let texts = fixture.texts();
        let run = texts.iter().find(|run| run.text.starts_with(&digits)).unwrap_or_else(|| panic!("{digits:?} is painted: {texts:?}"));
        assert!(
            run.clip.min.y <= run.rect.min.y + SLACK && run.clip.max.y >= run.rect.max.y - SLACK,
            "{name}'s value shows whole: {:?} under the clip {:?}",
            run.rect,
            run.clip
        );
    }
}

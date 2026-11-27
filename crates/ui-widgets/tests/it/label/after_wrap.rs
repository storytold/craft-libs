//! A label that follows its control and wraps in a narrow room: the row takes the wrapped label's height.

use craft_theme::Size;
use craft_ui::choice::{Checkbox, Switch};
use craft_ui::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Ui, Vec2};

/// How far two edges may differ and still line up, in points.
const SLACK: f32 = 0.5;

/// The widest beside label of the form: it sets the label column the row after its control starts past.
const WIDE: &str = "Spacing";
const SNAP: &str = "Snap to grid";
const NEXT: &str = "Name";

/// The control a row shows with `SNAP` after it.
#[derive(Clone, Copy, Debug)]
enum Row {
    /// A field laid out `After` around a check box.
    FieldCheckbox,
    /// A field laid out `After` around a switch.
    FieldSwitch,
    /// A switch that carries its own label.
    Switch,
}

impl Row {
    fn show(self, ui: &mut Ui) {
        match self {
            Row::FieldCheckbox => {
                Field::new(SNAP).layout(FieldLayout::After).id_salt("snap").show(ui, |ui, _| {
                    Checkbox::new(Bind::given(true)).show(ui);
                });
            }
            Row::FieldSwitch => {
                Field::new(SNAP).layout(FieldLayout::After).id_salt("snap").show(ui, |ui, _| {
                    Switch::new(Bind::given(true)).show(ui);
                });
            }
            Row::Switch => {
                Switch::new(Bind::given(true)).label(SNAP).show(ui);
            }
        }
    }
}

fn number(ui: &mut Ui, name: &'static str, value: f64) {
    Field::new(name).show(ui, |ui, _| NumberField::new(name, Bind::given(Some(value))).show(ui));
}

/// In a form 130 pt wide at comfortable density, the row's label wraps after its control in the room the label
/// column leaves, whether the form's widest label comes before the row or after it. Every line shows whole inside its
/// clip, and the row after starts under the wrapped label.
#[trial]
fn a_wrapped_label_after_its_control_takes_its_rows_height() {
    for row in [Row::FieldCheckbox, Row::FieldSwitch, Row::Switch] {
        for wide_first in [true, false] {
            let mut fixture = Fixture::new(move |ui| {
                FieldGroup::new("Canvas").show(ui, |ui, _| {
                    if wide_first {
                        number(ui, WIDE, 1.0);
                    }
                    row.show(ui);
                    number(ui, NEXT, 2.0);
                    if !wide_first {
                        number(ui, WIDE, 1.0);
                    }
                });
            });
            fixture.room(Vec2::new(130.0, 600.0)).density(Size::Comfortable);
            fixture.settle();
            let case = format!("{row:?}, widest label first: {wide_first}");
            let texts = fixture.texts();
            let snap = texts.iter().find(|run| run.text == SNAP).unwrap_or_else(|| panic!("{case}: {SNAP:?} is painted: {texts:?}"));
            let line = texts.iter().find(|run| run.text == WIDE).map_or(0.0, |run| run.rect.height());
            assert!(snap.rect.height() > line + SLACK, "{case}: the label wraps in the room after its control: {:?}", snap.rect);
            assert!(
                snap.clip.min.y <= snap.rect.min.y + SLACK && snap.clip.max.y >= snap.rect.max.y - SLACK,
                "{case}: every line of the label shows whole: {:?} under the clip {:?}",
                snap.rect,
                snap.clip
            );
            let next = texts.iter().find(|run| run.text == NEXT).map(|run| run.rect).unwrap_or_else(|| panic!("{case}: {NEXT:?} is painted"));
            assert!(next.min.y >= snap.rect.max.y - SLACK, "{case}: the row after starts under the wrapped label: {:?} {next:?}", snap.rect);
        }
    }
}

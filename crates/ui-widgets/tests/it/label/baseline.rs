//! A side label beside a row of controls of different heights lines up with the text of the control it names.

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use craft_ui::value_drag::AngleDial;
use egui::Ui;

/// How far two baselines may differ and still line up, in points.
const SLACK: f32 = 1.0;

/// The painted baseline of the run whose text is `text`.
fn baseline_of(fixture: &Fixture, text: &str) -> f32 {
    let texts = fixture.texts();
    let run = texts.iter().find(|run| run.text.trim() == text).unwrap_or_else(|| panic!("{text:?} is painted: {texts:?}"));
    run.baseline
}

/// The rows of a parameter form: a field whose control is a row of `body`, in a field group.
fn settled(name: &'static str, mut body: impl FnMut(&mut Ui) + 'static) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        FieldGroup::new("Parameters").show(ui, |ui, _| {
            Field::new(name).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| body(ui));
            });
        });
    });
    fixture.settle();
    fixture
}

/// A row centres its shorter piece beside a taller one: the label lines up with the text of the piece that reports
/// the first baseline, not with the top of that piece.
#[trial]
fn a_side_label_lines_up_with_the_text_of_a_piece_a_row_centres() {
    let fixture = settled("Angle", |ui| {
        AngleDial::new(Bind::given(45.0)).label("Angle").id_salt("dial").show(ui);
        NumberField::new("Angle in degrees", Bind::given(Some(45.0))).id_salt("field").show(ui);
    });
    let (label, field) = (baseline_of(&fixture, "Angle"), baseline_of(&fixture, "45"));
    assert!((label - field).abs() <= SLACK, "the label lines up with the number beside the tall dial: label {label}, field {field}");

    let fixture = settled("Quality", |ui| {
        Checkbox::new(Bind::given(true)).label("Custom").id_salt("custom").show(ui);
        NumberField::new("Quality", Bind::given(Some(80.0))).id_salt("field").show(ui);
    });
    let (label, field) = (baseline_of(&fixture, "Quality"), baseline_of(&fixture, "80"));
    assert!((label - field).abs() <= SLACK, "the label lines up with the number of the field beside the check box: label {label}, field {field}");
}

/// A control whose text sits on a later line than its first piece (a note under a swatch, "Active" after a wrapped
/// row of chips): the label stays level with the first line.
#[trial]
fn a_side_label_stays_on_the_first_line_of_its_control() {
    use craft_theme::Srgba;
    use craft_ui::choice::Swatch;
    use craft_ui::display::Text;
    use craft_ui::layout_components::Stack;
    const GREY: Srgba = Srgba { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
    let fixture = settled_in(|ui| {
        Field::new("Light theme").show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Swatch::new("Light", GREY).show(ui);
                Text::new("Active").show(ui);
            });
        });
    });
    let swatch = fixture.by_label("Light").expect("the swatch is shown").rect;
    let label = fixture.texts().iter().find(|run| run.text == "Light theme").map(|run| run.rect).expect("the label is painted");
    assert!(
        label.center().y >= swatch.top() && label.center().y <= swatch.bottom(),
        "the label sits level with the swatch, not with the note below it: label {label:?}, swatch {swatch:?}"
    );
}

/// `body` in a field group, settled.
fn settled_in(mut body: impl FnMut(&mut Ui) + 'static) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        FieldGroup::new("Appearance").show(ui, |ui, _| body(ui));
    });
    fixture.settle();
    fixture
}

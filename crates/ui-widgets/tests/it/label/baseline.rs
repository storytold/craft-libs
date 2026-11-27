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
                Swatch::new("Light", GREY).keeps_name(true).show(ui);
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

/// A check box's label reads in body type, as a select's face does, and shares its baseline: in an options bar,
/// "Anti-alias" after its box lines up with the select beside it. Guards against the label's text taking the caption
/// role, which set it smaller and off the face's line.
#[trial]
fn a_check_box_label_shares_the_line_of_a_face_beside_it() {
    use craft_ui::bars::{Bar, BarKind};
    use craft_ui::contract::Entry;
    use craft_ui::pickers::Select;
    let mut fixture = Fixture::new(|ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            Checkbox::new(Bind::given(true)).label("Anti-alias").id_salt("smooth").show(ui);
            let entries = [Entry::new(0u8, "Layer"), Entry::new(1, "Group")];
            Select::new("Target", Bind::given(0u8), &entries).show(ui);
        });
    });
    fixture.settle();
    let (label, face) = (baseline_of(&fixture, "Anti-alias"), baseline_of(&fixture, "Layer"));
    assert!((label - face).abs() <= 0.5, "the check box label lines up with the select's text: label {label}, select {face}");
}

/// In an options bar, a number field's mono digits, its smaller unit and the text of a select beside it share one
/// baseline. Guards against each run centring its own line box: the mono face's and the caption's lines put their
/// baselines elsewhere than the body face's, and the "100" and the "%" sat off the select's line.
#[trial]
fn a_bar_number_field_sets_its_digits_and_unit_on_the_line_of_a_select() {
    use craft_ui::bars::{Bar, BarKind};
    use craft_ui::contract::Entry;
    use craft_ui::pickers::Select;
    use craft_ui::runtime::env::UnitKind;
    let mut fixture = Fixture::new(|ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let entries = [Entry::new(0u8, "Normal"), Entry::new(1, "Multiply")];
            Select::new("Mode", Bind::given(0u8), &entries).show(ui);
            NumberField::new("Opacity", Bind::given(Some(100.0))).unit_kind(UnitKind::Percent).show(ui);
        });
    });
    fixture.settle();
    let (face, digits, unit) = (baseline_of(&fixture, "Normal"), baseline_of(&fixture, "100.0"), baseline_of(&fixture, "%"));
    assert!((digits - face).abs() < 0.5, "the digits sit on the select's baseline: digits {digits}, select {face}");
    assert!((unit - face).abs() < 0.5, "the unit sits on the select's baseline: unit {unit}, select {face}");
}

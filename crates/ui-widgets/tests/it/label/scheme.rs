//! The label scheme a container sets: a form's rows and an inspector's rows, and the captions of a bar.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, GlyphLabel, LabelKind, LabelScheme, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Rect, Ui};

const SHORT: &str = "Size";
const LONG: &str = "Corner radius";
/// How far two edges may differ and still line up, in points.
const SLACK: f32 = 0.5;

/// Two fields of different caption lengths in a group under `scheme`.
fn rows(scheme: LabelScheme) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        FieldGroup::new("Shape").labels(scheme).show(ui, |ui, _| {
            for name in [SHORT, LONG] {
                Field::new(name).show(ui, |ui, _| {
                    NumberField::new(name, Bind::given(Some(8.0))).show(ui);
                });
            }
        });
    });
    fixture.settle();
    fixture
}

fn colons(fixture: &Fixture) -> usize {
    fixture.texts().iter().filter(|run| run.text.trim() == ":").count()
}

fn caption(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text:?} is painted"))
}

fn control(fixture: &Fixture, name: &str) -> Rect {
    fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} names its control")).rect
}

/// A form sets each caption against its control with a colon and keeps the control's width; an inspector starts each
/// caption at the row's start with no colon and lets the control fill the rest of the row.
#[trial]
fn a_form_and_an_inspector_lay_out_the_same_rows_their_own_way() {
    let form = rows(LabelScheme::Form);
    assert_eq!(colons(&form), 2, "a form's captions end with a colon");
    assert!(caption(&form, SHORT).min.x > caption(&form, LONG).min.x + SLACK, "a form's shorter caption sits against its control");
    let kept = control(&form, SHORT);

    let inspector = rows(LabelScheme::Inspector);
    assert_eq!(colons(&inspector), 0, "an inspector's captions take no colon");
    assert!((caption(&inspector, SHORT).min.x - caption(&inspector, LONG).min.x).abs() <= SLACK, "an inspector's captions start at the row's start");
    let (short, long) = (control(&inspector, SHORT), control(&inspector, LONG));
    assert!((short.min.x - long.min.x).abs() <= SLACK, "the controls share one column: {short:?} {long:?}");
    assert!(short.width() > kept.width() + SLACK, "an inspector's control fills the row, a form's keeps its width: {short:?} {kept:?}");
}

fn glyph_field(ui: &mut Ui) {
    let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text("X")));
    Field::new("Horizontal position").side(side).show(ui, |ui, _| {
        NumberField::new("Horizontal position", Bind::given(Some(10.0))).show(ui);
    });
}

/// In a bar every caption ends with a colon, a glyph's as well ("X:"); elsewhere a glyph takes none.
#[trial]
fn a_glyph_caption_takes_a_colon_in_a_bar_only() {
    let mut bar = Fixture::new(|ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| glyph_field(ui));
    });
    bar.settle();
    assert_eq!(colons(&bar), 1, "the glyph caption in a bar ends with a colon");
    let colon = bar.texts().iter().find(|run| run.text.trim() == ":").map(|run| run.rect).unwrap_or(Rect::NOTHING);
    let field = control(&bar, "Horizontal position");
    assert!(
        colon.min.x >= caption(&bar, "X").max.x - SLACK && colon.max.x <= field.min.x + SLACK,
        "the colon follows the glyph, clear of the control: {colon:?} {field:?}"
    );
    let mut plain = Fixture::new(glyph_field);
    plain.settle();
    assert_eq!(colons(&plain), 0, "a glyph caption outside a bar takes no colon");
}

/// A check box row of a form starts its box in the control column, where the rows labelled beside it start their
/// controls. In an inspector, in a form with no labelled row, and as a radio group's member, it stays at the row's
/// start.
#[trial]
fn a_forms_check_box_row_starts_in_the_control_column() {
    let check = |scheme: LabelScheme, labelled: bool| {
        let mut fixture = Fixture::new(move |ui| {
            FieldGroup::new("Export").labels(scheme).show(ui, |ui, _| {
                if labelled {
                    Field::new(LONG).show(ui, |ui, _| {
                        NumberField::new(LONG, Bind::given(Some(8.0))).show(ui);
                    });
                }
                Checkbox::new(Bind::given(true)).label("Embed Color Profile").show(ui);
                RadioGroup::new("Artboards", Bind::given(true)).show(ui, |ui, _| {
                    Radio::new(true).label("All").show(ui);
                });
            });
        });
        fixture.settle();
        let boxed = fixture.by_label("Embed Color Profile").expect("the check box").rect;
        let start = fixture.texts().iter().find(|run| run.text == "Embed Color Profile").map(|run| run.rect).expect("the label is painted");
        let field = labelled.then(|| fixture.by_label(LONG).expect("the field").rect);
        let radio = fixture.by_label("All").expect("the radio button").rect;
        (boxed, start, field, radio)
    };
    let (form, text, field, radio) = check(LabelScheme::Form, true);
    let field = field.expect("the labelled row");
    assert!(radio.min.x < field.min.x - SLACK, "a radio group's member stays at the row's start: {radio:?}");
    assert!((form.min.x - field.min.x).abs() <= SLACK, "a form's check box starts at the control edge: {form:?} {field:?}");
    assert!(text.min.x > form.min.x, "its label follows the box: {text:?}");
    let (inspector, _, field, _) = check(LabelScheme::Inspector, true);
    assert!(inspector.min.x < field.expect("the labelled row").min.x - SLACK, "an inspector's check box stays at the row's start");
    let (alone, _, _, _) = check(LabelScheme::Form, false);
    assert!((alone.min.x - inspector.min.x).abs() <= SLACK, "with no labelled row the box stays at the row's start: {alone:?}");
}

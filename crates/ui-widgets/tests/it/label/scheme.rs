//! The label scheme a container sets: a form's rows and an inspector's rows, and the captions of a bar.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::{Bind, GlyphLabel, LabelKind, LabelScheme, Show as _, ShowContainer as _, SideLabel};
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

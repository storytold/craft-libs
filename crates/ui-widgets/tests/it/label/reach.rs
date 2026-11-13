//! A group's label column reaches the labelled rows of the plain containers inside it: a stack, a row, a nested
//! box. A nested group keeps its own column, and a field's control keeps its own labels. The value and unit columns
//! of content-sized number fields reach through a field's body.

use super::group::Group;
use super::kit::{Dial, Seen};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, ContainerProps, LabelScheme, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, Frames, last_pass_measuring_runs, trial};
use craft_ui::text::NumberField;
use egui::Rect;
use std::cell::RefCell;

/// How far two edges may differ and still line up, in points.
const SLACK: f32 = 0.5;

fn number(ui: &mut egui::Ui, caption: &'static str) {
    Field::new(caption).show(ui, |ui, _| {
        NumberField::new(caption, Bind::given(Some(8.0))).show(ui);
    });
}

/// A field `caption` holding a content-sized number field over `range`, with `unit` after the number.
fn sized(ui: &mut egui::Ui, caption: &'static str, range: std::ops::RangeInclusive<f64>, unit: Option<UnitKind>) {
    Field::new(caption).show(ui, |ui, _| {
        let field = NumberField::new(caption, Bind::given(Some(8.0))).range(range.clone()).sizing(Sizing::Content);
        match unit {
            Some(unit) => field.unit_kind(unit),
            None => field,
        }
        .show(ui);
    });
}

fn control(fixture: &Fixture, name: &str) -> Rect {
    fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} names its control")).rect
}

fn caption(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text:?} is painted"))
}

/// The rows directly in a group, in a stack inside it and in a row inside it share one control edge; a nested group
/// lines up its own rows; the labelled row inside a field's control reads no column.
#[trial]
fn a_groups_label_column_reaches_the_rows_of_its_plain_containers() {
    let mut fixture = Fixture::new(|ui| {
        FieldGroup::new("Outer").labels(LabelScheme::Inspector).show(ui, |ui, _| {
            number(ui, "Size");
            Stack::new().show(ui, |ui, _| number(ui, "Corner radius"));
            Row::new().show(ui, |ui, _| {
                number(ui, "Width");
                Button::new("Link").show(ui);
            });
            FieldGroup::new("Inner").show(ui, |ui, _| number(ui, "X"));
            Field::new("Fill").show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| number(ui, "A"));
            });
        });
    });
    fixture.settle();
    let edge = control(&fixture, "Size").min.x;
    for name in ["Corner radius", "Width"] {
        let at = control(&fixture, name).min.x;
        assert!((at - edge).abs() <= SLACK, "{name:?} starts at the group's control edge: {at} {edge}");
    }
    let nested = control(&fixture, "X").min.x;
    assert!(nested < edge - SLACK, "a nested group's short label keeps its own column: {nested} {edge}");
    let inside = control(&fixture, "A").min.x - caption(&fixture, "A").min.x;
    let long = caption(&fixture, "Corner radius").width();
    assert!(inside < long - SLACK, "the field's own labelled row reads no group column: {inside} {long}");
}

/// A content-sized row holding a stack of labelled rows measures at the group's widest label: the member after the
/// stack starts clear of a wide control under a short label that measured before the long one.
#[trial]
fn a_row_after_a_stack_of_labelled_rows_clears_their_controls() {
    let mut frames = Frames::new();
    let wide = RefCell::new(Seen::default());
    let mut value = 1.0;
    let mut link = Rect::NOTHING;
    frames.idle(4, |ui| {
        Group::new(ContainerProps::new().label_column(true)).id_salt("held").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, cx| {
                    cx.labelled(ui, "short", "A".into(), SideLabel::new(), Dial::new(&mut value, &wide));
                    cx.labelled(ui, "long", "Hue and saturation".into(), SideLabel::new(), Button::new("OK"));
                });
                link = Button::new("Link").show(ui).response.rect;
            });
        });
    });
    let control = wide.borrow().rect;
    assert!(control.is_positive(), "the wide control is shown");
    assert!(link.min.x >= control.max.x - SLACK, "the row's next member starts clear of the wide control: {link:?} {control:?}");
}

/// A content-sized row of labelled rows that runs before the group's widest label measures at the group's final
/// column: its labels stay beside their controls, the member after them starts clear, and a settled frame measures
/// nothing again.
#[trial]
fn a_row_before_the_groups_widest_label_measures_at_its_final_column() {
    let mut fixture = Fixture::new(|ui| {
        FieldGroup::new("Image size").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    number(ui, "Width");
                    number(ui, "Height");
                });
                Button::new("Link").show(ui);
            });
            number(ui, "Resample method");
        });
    });
    fixture.settle();
    let (label, field) = (caption(&fixture, "Width"), control(&fixture, "Width"));
    assert!(label.max.x <= field.min.x + SLACK, "Width's label sits before its field: {label:?} {field:?}");
    assert!(label.center().y > field.min.y && label.center().y < field.max.y, "Width's label sits on its field's line: {label:?} {field:?}");
    let resample = control(&fixture, "Resample method").min.x;
    assert!((field.min.x - resample).abs() <= SLACK, "Width's field starts at the group's control edge: {} {resample}", field.min.x);
    let link = fixture.by_label("Link").expect("the link button is shown").rect;
    assert!(link.min.x >= field.max.x - SLACK, "the row's next member starts clear of the field: {link:?} {field:?}");
    fixture.frame(vec![]);
    let runs = last_pass_measuring_runs(fixture.ctx());
    assert!(runs.is_empty(), "a settled frame takes the kept measurements: {runs:?}");
}

/// Content-sized number fields share the widest value and the widest unit across the fields of a group: they end at
/// one edge and their units start at one place, "°" beside "%" and "-128" beside "100"; the same fields outside a
/// group keep their own widths.
#[trial]
fn content_sized_fields_line_up_their_values_and_units_across_a_group() {
    let mut fixture = Fixture::new(|ui| {
        FieldGroup::new("HSB").show(ui, |ui, _| {
            sized(ui, "Hue", 0.0..=360.0, Some(UnitKind::Degrees));
            sized(ui, "Saturation", 0.0..=100.0, Some(UnitKind::Percent));
        });
        FieldGroup::new("Lab").show(ui, |ui, _| {
            sized(ui, "L", 0.0..=100.0, None);
            sized(ui, "a", -128.0..=127.0, None);
        });
        Stack::new().show(ui, |ui, _| {
            sized(ui, "Lightness", 0.0..=100.0, None);
            sized(ui, "Green-red", -128.0..=127.0, None);
        });
    });
    fixture.settle();
    for (one, other) in [("Hue", "Saturation"), ("L", "a")] {
        let (one, other) = (control(&fixture, one), control(&fixture, other));
        assert!((one.max.x - other.max.x).abs() <= SLACK, "the fields of a group end at one edge: {one:?} {other:?}");
    }
    let (degrees, percent) = (caption(&fixture, "°"), caption(&fixture, "%"));
    assert!((degrees.min.x - percent.min.x).abs() <= SLACK, "the units start at one place: {degrees:?} {percent:?}");
    let (short, long) = (control(&fixture, "Lightness"), control(&fixture, "Green-red"));
    assert!(long.width() > short.width() + SLACK, "outside a group each field takes its own value: {short:?} {long:?}");
}

/// A field that measured before a wider value of a later field in its group shows at the group's width, and a
/// content-sized row around it makes room: the member after the row's stack starts clear of both fields, and a
/// settled frame measures nothing again.
#[trial]
fn a_field_before_the_groups_widest_value_shows_at_its_width() {
    let mut fixture = Fixture::new(|ui| {
        FieldGroup::new("Channels").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    sized(ui, "L", 0.0..=100.0, None);
                    sized(ui, "a", -12800.0..=12700.0, None);
                });
                Button::new("Link").show(ui);
            });
        });
    });
    fixture.settle();
    let (first, wide) = (control(&fixture, "L"), control(&fixture, "a"));
    assert!((first.max.x - wide.max.x).abs() <= SLACK, "the first field takes the later field's width: {first:?} {wide:?}");
    let link = fixture.by_label("Link").expect("the link button is shown").rect;
    assert!(link.min.x >= wide.max.x - SLACK, "the row's next member starts clear of the fields: {link:?} {wide:?}");
    fixture.frame(vec![]);
    let runs = last_pass_measuring_runs(fixture.ctx());
    assert!(runs.is_empty(), "a settled frame takes the kept measurements: {runs:?}");
}

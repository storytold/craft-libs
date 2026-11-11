//! Stories of `Field`.

use super::Help;
use super::{Field, FieldGroup};
use crate::contract::{Bind, FieldLayout, GlyphLabel, LabelKind, Show as _, ShowContainer as _, SideLabel, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::text::NumberField;
use craft_behavior::IconId;
use craft_theme::variant::Tone;
use egui::Ui;

const SIZE: f64 = 12.0;

#[story(Field, covers = [Default], drive = Drive::Never)]
fn field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("Size")
        .help(Help::Text("In points".into()))
        .show(ui, |ui, _| NumberField::new("Size", Bind::bound(data.value("size", Some(SIZE)))).show(ui))
        .into()
}

#[story(Field, covers = [], drive = Drive::Never)]
fn field_above_with_status(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("Size")
        .layout(FieldLayout::Above)
        .help(Help::Status(Tone::Warn, "Large sizes slow the preview".into()))
        .show(ui, |ui, _| NumberField::new("Size", Bind::bound(data.value("above", Some(SIZE)))).show(ui))
        .into()
}

#[story(Field, covers = [LongText], drive = Drive::Never)]
fn field_long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("A label much longer than the column a form usually gives it")
        .help(Help::Text("A help text much longer than the room under the control, so it wraps onto more than one line".into()))
        .show(ui, |ui, _| NumberField::new("Size", Bind::bound(data.value("long", Some(SIZE)))).show(ui))
        .into()
}

/// A glyph label: the icon shows, the text only names the control.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_icon_label(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("Width")
        .side(SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::icon(IconId::new("list")))))
        .show(ui, |ui, _| NumberField::new("Width", Bind::bound(data.value("width", Some(SIZE)))).show(ui))
        .into()
}

#[story(Field, covers = [], drive = Drive::Never)]
fn field_with_refused_value(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_negative = |size: &Option<f64>| match size {
        Some(size) if *size < 0.0 => Validity::Refuse(Message::new("Size cannot be negative")),
        _ => Validity::Ok,
    };
    Field::new("Size")
        .help(Help::Text("In points".into()))
        .show(ui, |ui, _| NumberField::new("Size", Bind::bound(data.value("refused", Some(-SIZE))).check(refuse_negative)).show(ui))
        .into()
}

/// An error that needs two fields: the end day may not come before the start day, and the line under the end field
/// says so.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_with_cross_field_error(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = *data.value("start_day", Some(20.0));
    let end_before_start = move |end: &Option<f64>| match end {
        Some(end) if start.is_some_and(|start| *end < start) => Validity::Refuse(Message::new("The end day comes before the start day")),
        _ => Validity::Ok,
    };
    let mut out = None;
    FieldGroup::new("Period").show(ui, |ui, _| {
        Field::new("Start day").show(ui, |ui, _| NumberField::new("Start day", Bind::bound(data.value("start_day", Some(20.0)))).show(ui));
        out = Some(
            Field::new("End day")
                .show(ui, |ui, _| NumberField::new("End day", Bind::bound(data.value("end_day", Some(5.0))).check(end_before_start)).show(ui)),
        );
    });
    out.map(StoryOutput::from).unwrap_or_default()
}

#[story(FieldGroup, covers = [Default], drive = Drive::Never)]
fn field_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [("Width", "width"), ("Corner radius", "radius"), ("Maximum number of layers", "layers")];
    FieldGroup::new("Shape")
        .show(ui, |ui, _| {
            for (label, name) in rows {
                Field::new(label).show(ui, |ui, _| NumberField::new(label, Bind::bound(data.value(name, Some(SIZE)))).show(ui));
            }
        })
        .into()
}

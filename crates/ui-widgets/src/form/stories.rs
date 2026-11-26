//! Stories of `Field`.

use super::Help;
use super::{Field, FieldGroup};
use crate::buttons::Link;
use crate::choice::Swatch;
use crate::contract::{
    CaptionInk, FieldLayout, GlyphColon, GlyphLabel, LabelKind, LabelScheme, LinkLabelInk, Props as _, Show as _, ShowContainer as _, SideLabel,
    Validity,
};
use crate::layout_components::{Row, Stack};
use crate::runtime::env::{Message, with_env};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::text::{HexField, NumberField};
use craft_behavior::IconId;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::Ui;

const SIZE: f64 = 12.0;

#[story(Field, covers = [Default], drive = Drive::Never)]
fn field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("Size").help(Help::Text("In points".into())).show(ui, |ui, _| NumberField::new("Size", data.bind("size", Some(SIZE))).show(ui)).into()
}

#[story(Field, covers = [], drive = Drive::Never)]
fn field_above_with_status(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("Size")
        .layout(FieldLayout::Above)
        .help(Help::Status(Tone::Warn, "Large sizes slow the preview".into()))
        .show(ui, |ui, _| NumberField::new("Size", data.bind("above", Some(SIZE))).show(ui))
        .into()
}

#[story(Field, covers = [LongText], drive = Drive::Never)]
fn field_long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("A label much longer than the column a form usually gives it")
        .help(Help::Text("A help text much longer than the room under the control, so it wraps onto more than one line".into()))
        .show(ui, |ui, _| NumberField::new("Size", data.bind("long", Some(SIZE))).show(ui))
        .into()
}

/// A number field whose side label is a link: a click reports to the host and leaves focus alone, a drag scrubs.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_link_label(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Field::new("Opacity")
        .side(SideLabel::new().link().scrub(true))
        .show(ui, |ui, _| NumberField::new("Opacity", data.bind("link", Some(100.0))).scrub_by_label().show(ui));
    // The field the label names reports the link click; the story is its host.
    let link = shown.inner.as_ref().is_some_and(|field| field.link);
    let facts = StoryOutput::from(shown);
    StoryOutput { clicked: facts.clicked || link, ..facts }
}

/// A field's link label under each `LinkLabelInk`, each with a free link to options under it: the accent (the default)
/// with a plain `Link`, then the body ink VectorCraft sets at its root with a `Link` in `Tone::Neutral`, the same ink.
/// Both stay underlined.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_link_label_ink(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut rows = |ui: &mut Ui, key: &'static str, tone: Option<Tone>| {
        Stack::new()
            .id_salt(key)
            .show(ui, |ui, _| {
                let side = SideLabel::new().link();
                let field = Field::new("Opacity").side(side).show(ui, |ui, _| NumberField::new("Opacity", data.bind(key, Some(100.0))).show(ui));
                let link = Link::new("Stroke");
                let link = match tone {
                    Some(tone) => link.tone(tone),
                    None => link,
                }
                .show(ui);
                // The field the label names reports the link click; the story is its host.
                field.inner.as_ref().is_some_and(|field| field.link) || StoryOutput::from(link).clicked
            })
            .inner
            == Some(true)
    };
    let accent = rows(ui, "accent", None);
    let neutral = with_env(
        ui,
        |overrides| {
            overrides.values.set(LinkLabelInk::Neutral);
        },
        |ui| rows(ui, "neutral", Some(Tone::Neutral)),
    );
    StoryOutput { clicked: accent || neutral, ..StoryOutput::default() }
}

/// A field's label under each `CaptionInk`, enabled and disabled: the muted ink (the default), then the normal ink
/// VectorCraft sets at its root. A disabled label reads in the disabled ink under both.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_caption_ink(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut rows = |ui: &mut Ui, key: &'static str| {
        Stack::new().id_salt(key).show(ui, |ui, _| {
            for (salt, enabled) in [("enabled", true), ("disabled", false)] {
                Field::new("Angle")
                    .id_salt(salt)
                    .enabled(enabled)
                    .show(ui, |ui, _| NumberField::new("Angle", data.bind(key, Some(SIZE))).unit("°").show(ui));
            }
        });
    };
    rows(ui, "muted");
    with_env(
        ui,
        |overrides| {
            overrides.values.set(CaptionInk::Normal);
        },
        |ui| rows(ui, "normal"),
    );
    StoryOutput::default()
}

/// A glyph label: the icon shows, the text only names the control.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_icon_label(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Field::new("Width")
        .side(SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::icon(IconId::new("list")))))
        .show(ui, |ui, _| NumberField::new("Width", data.bind("width", Some(SIZE))).show(ui))
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
        .show(ui, |ui, _| NumberField::new("Size", data.bind("refused", Some(-SIZE)).check(refuse_negative)).show(ui))
        .into()
}

/// An error that needs two fields: the end day may not come before the start day, and the line under the end field
/// says so.
#[story(Field, covers = [], drive = Drive::Never)]
fn field_with_cross_field_error(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = *data.value("start_day", Some(20.0));
    let end_before_start = |start: &Option<f64>, end: &Option<f64>| match end {
        Some(end) if start.is_some_and(|start| *end < start) => Validity::Refuse(Message::new("The end day comes before the start day")),
        _ => Validity::Ok,
    };
    let mut out = None;
    FieldGroup::new("Period").show(ui, |ui, _| {
        Field::new("Start day").show(ui, |ui, _| NumberField::new("Start day", data.bind("start_day", Some(20.0))).show(ui));
        out = Some(
            Field::new("End day")
                .show(ui, |ui, _| NumberField::new("End day", data.bind("end_day", Some(5.0)).check_with(start, end_before_start)).show(ui)),
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
                Field::new(label).show(ui, |ui, _| NumberField::new(label, data.bind(name, Some(SIZE))).show(ui));
            }
        })
        .into()
}

/// A form under `GlyphColon::Form`: the glyph captions X, Y, W and H end with a colon as the word label does, and the
/// "#" before the hex value, a mark of its own, stays bare.
#[story(FieldGroup, covers = [], drive = Drive::Never)]
fn field_glyph_captions_with_colons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const BLUE: craft_theme::Rgba = craft_theme::Rgba([0x33, 0x66, 0x99, 0xff]);
    with_env(
        ui,
        |overrides| {
            overrides.values.set(GlyphColon::Form);
        },
        |ui| {
            FieldGroup::new("Transform").labels(LabelScheme::Form).show(ui, |ui, _| {
                for (glyph, name, key, value) in
                    [("X", "X position", "x", 24.0), ("Y", "Y position", "y", 36.0), ("W", "Width", "w", 640.0), ("H", "Height", "h", 480.0)]
                {
                    let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::caption(glyph))).scrub(true);
                    Field::new(name).side(side).show(ui, |ui, _| NumberField::new(name, data.bind(key, Some(value))).show(ui));
                }
                Field::new("Opacity").show(ui, |ui, _| NumberField::new("Opacity", data.bind("opacity", Some(100.0))).show(ui));
                Field::new("Hex color").side(HexField::hash_label()).show(ui, |ui, _| HexField::hex("Hex color", data.bind("hex", BLUE)).show(ui));
            })
        },
    )
    .into()
}

/// The rows of an inspector: glyph labels W and H sharing a row, word labels at the row's start without a colon, and
/// controls that fill the rest of the row.
#[story(FieldGroup, covers = [], drive = Drive::Never)]
fn field_inspector_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const FILL: Srgba = Srgba { r: 0.80, g: 0.32, b: 0.24, a: 1.0 };
    FieldGroup::new("Properties")
        .labels(LabelScheme::Inspector)
        .show(ui, |ui, _| {
            Row::new().fill_width().show(ui, |ui, _| {
                for (glyph, name, key, value) in [("W", "Width", "w", 640.0), ("H", "Height", "h", 480.0)] {
                    let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(glyph))).scrub(true);
                    Field::new(name).side(side).show(ui, |ui, _| NumberField::new(name, data.bind(key, Some(value))).show(ui));
                }
            });
            for (label, key, value) in [("Opacity", "opacity", 100.0), ("Angle", "angle", 0.0)] {
                Field::new(label).side(SideLabel::new().scrub(true)).show(ui, |ui, _| NumberField::new(label, data.bind(key, Some(value))).show(ui));
            }
            Field::new("Fill").show(ui, |ui, _| Swatch::new("Fill", FILL).show(ui));
        })
        .into()
}

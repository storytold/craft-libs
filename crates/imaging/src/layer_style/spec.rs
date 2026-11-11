//! The effect tables: which fields an effect page shows, in which order, with which ranges, and
//! the effect's factory defaults.

use std::borrow::Cow;
use std::collections::HashMap;

use craft_theme::Srgba;

use super::contour::Contour;
use super::model::{EffectKind, StyleParams, StyleValue};

/// A slider field's unit, shown after its number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldUnit {
    /// "%".
    Percent,
    /// "px".
    Pixels,
    /// "°".
    Degrees,
}

/// One entry of a choice field: the value key stored in the parameters and the English label.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChoiceOption {
    /// Stored as `StyleValue::Text` (`"softer"`).
    pub value: Cow<'static, str>,
    /// The English label; the dialog shows the `layerStyle.choice.*` word for the built-in
    /// tables.
    pub label: Cow<'static, str>,
}

/// How a field is edited.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FieldKind {
    /// A slider row over `min..=max` in `unit`; values are whole numbers.
    Slider {
        /// Lowest value, in `unit`.
        min: f64,
        /// Highest value, in `unit`.
        max: f64,
        /// The unit the slider shows.
        unit: FieldUnit,
    },
    /// A colour swatch; a click asks the app for a colour (`ColorRequest`).
    Color,
    /// A blend-mode select over the host's blend modes.
    Blend,
    /// A select over `options`; the first is the fallback for a missing value.
    Choice(Cow<'static, [ChoiceOption]>),
    /// Gradient Overlay's Method: Smooth, Perceptual, Linear, Classic, and Custom first while the
    /// stored value names none of them (an imported curve kept until the user picks).
    GradientMethod,
    #[allow(missing_docs)]
    Check,
    /// An angle dial and field, −180..=180°. With the effect's `useGlobalLight` on, it edits the
    /// shared light angle.
    Angle,
    /// A pattern from the host's pattern list; the first entry when the value is empty.
    Pattern,
    /// A contour thumbnail with the preset picker and the Contour Editor; `anti_alias` is the
    /// key of the effect's Anti-aliased check shown beside it, when the effect has one.
    Contour {
        #[allow(missing_docs)]
        anti_alias: Option<Cow<'static, str>>,
    },
}

/// One row of an effect page.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectField {
    /// The parameter key (`"distance"`).
    pub key: Cow<'static, str>,
    /// The English label ("Distance"); empty for a row that continues the one above (Bevel &
    /// Emboss's contour under its Contour check). The dialog shows the `layerStyle.field.*` word
    /// for the built-in tables.
    pub label: Cow<'static, str>,
    /// How the field is edited.
    pub kind: FieldKind,
}

/// An effect's page: its fields in order and its factory defaults. An app with effects of its
/// own builds its specs by hand; Photoshop's ten come from [`EffectSpec::photoshop`].
#[derive(Clone, Debug, PartialEq)]
pub struct EffectSpec {
    /// The effect this page edits.
    pub kind: EffectKind,
    /// The English name ("Drop Shadow"); the dialog shows the `layerStyle.effect.*` word for the
    /// built-in kinds.
    pub label: Cow<'static, str>,
    /// The page's rows, top to bottom.
    pub fields: Vec<EffectField>,
    /// The values a new instance starts from.
    pub defaults: StyleParams,
}

impl EffectSpec {
    /// The spec with every Blend field's default renamed through `map` (English name to the
    /// host's blend key); names `map` lacks stay as they are.
    pub fn with_blend_keys(mut self, map: &HashMap<String, String>) -> Self {
        for field in self.fields.iter().filter(|field| matches!(field.kind, FieldKind::Blend)) {
            let mapped = match self.defaults.get(&field.key) {
                Some(StyleValue::Text(name)) => map.get(name).cloned(),
                _ => None,
            };
            if let Some(key) = mapped {
                self.defaults.set(field.key.to_string(), StyleValue::Text(key));
            }
        }
        self
    }

    /// Photoshop's table and factory defaults for `kind`, as PhotoCraft has them. Keys and labels,
    /// ranges in the field's unit (Slider min..max unit):
    ///
    /// - Drop Shadow: blend Blend Mode; color Color; opacity Opacity 0..100 %; angle Angle;
    ///   useGlobalLight Use Global Light; distance Distance 0..300 px; spread Spread 0..100 %;
    ///   size Size 0..250 px; contour Contour (anti-alias key `antiAlias`); noise Noise 0..100 %;
    ///   knocksOut Layer Knocks Out Drop Shadow. Defaults: Multiply, #000000, 75, 120, true, 5, 0,
    ///   5, Linear, antiAlias false, 0, true.
    /// - Inner Shadow: as Drop Shadow with choke Choke 0..100 % in place of Spread and no
    ///   knocksOut. Defaults: Multiply, #000000, 75, 120, true, 5, choke 0, 5, Linear, false, 0.
    /// - Outer Glow (Technique before Spread and Range after Contour, Photoshop's order, #6698):
    ///   blend; opacity 0..100 %; color; technique Technique (softer Softer, precise Precise);
    ///   spread 0..100 %; size 0..250 px; contour; range Range 1..100 %; noise 0..100 %.
    ///   Defaults: Screen, 75, #ffffbe, softer, 0, 5, Linear, false, range 50, 0.
    /// - Inner Glow: blend; opacity; color; technique; source Source (edge Edge, center Center);
    ///   choke 0..100 %; size 0..250 px; contour; range 1..100 %; noise 0..100 %. Defaults:
    ///   Screen, 75, #ffffbe, softer, edge, 0, 5, Linear, false, 50, 0.
    /// - Stroke: size Size 1..250 px; position Position (outside Outside, inside Inside, center
    ///   Center); blend; opacity 0..100 %; overprint Overprint; color. Defaults: 1, inside,
    ///   Normal, 100, false, #000000.
    /// - Color Overlay: blend; color; opacity 0..100 %. Defaults: Normal, #ff0000, 100.
    /// - Gradient Overlay: blend; opacity; from From (Color); to To (Color); reverse Reverse; style
    ///   Style (linear Linear, radial Radial, angle Angle, reflected Reflected, diamond Diamond);
    ///   angle; scale Scale 10..150 %; method Method (GradientMethod). Defaults: Normal, 100,
    ///   #000000, #ffffff, false, linear, 90, 100, custom.
    /// - Pattern Overlay: blend; opacity; pattern Pattern; angle; scale Scale 1..1000 %; link Link
    ///   with Layer. Defaults: Normal, 100, "", 0, 100, true.
    /// - Bevel & Emboss: style Style (inner Inner Bevel, outer Outer Bevel, emboss Emboss, pillow
    ///   Pillow Emboss); technique Technique (smooth Smooth, chiselHard Chisel Hard, chiselSoft
    ///   Chisel Soft); depth Depth 1..1000 %; direction Direction (up Up, down Down); size 0..250
    ///   px; soften Soften 0..16 px; angle; useGlobalLight; altitude Altitude 0..90 °;
    ///   glossContour Gloss Contour (anti-alias `glossAntiAlias`); highlightBlend Highlight Mode;
    ///   highlightColor Highlight Color; highlightOpacity Highlight Opacity 0..100 %; shadowBlend
    ///   Shadow Mode; shadowColor Shadow Color; shadowOpacity Shadow Opacity 0..100 %; contour
    ///   Contour (Check); contourName "" (Contour, anti-alias `contourAntiAlias`); contourRange
    ///   Range 1..100 %. Defaults: inner, smooth, 100, up, 5, 0, 120, true, 30, Linear, false,
    ///   Screen, #ffffff, 75, Multiply, #000000, 75, false, Linear, 50, false.
    /// - Satin: blend; color; opacity; angle; distance Distance 0..250 px; size 0..250 px;
    ///   contour; invert Invert. Defaults: Multiply, #000000, 50, 19, 11, 14, Linear, antiAlias
    ///   true, invert true.
    ///
    /// The defaults name blend modes by Photoshop's English names (`"Multiply"`, `"Screen"`,
    /// `"Normal"`), as PhotoCraft's engine stores them; a host whose `BlendChoice` keys differ
    /// maps them when it builds the draft. Contours default to
    /// [`Contour::linear`](super::Contour::linear).
    pub fn photoshop(kind: EffectKind) -> EffectSpec {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:90-216 `spec` (fields, ranges, units);
        // crates/engine/src/layer_style.rs:82-110 `effect_defaults` (defaults).
        use FieldUnit::{Degrees, Percent, Pixels};
        let percent = || slider(0.0, 100.0, Percent);
        let contour = |anti_alias: &'static str| FieldKind::Contour { anti_alias: Some(Cow::Borrowed(anti_alias)) };
        let glow_technique = || choice(&[("softer", "Softer"), ("precise", "Precise")]);
        let (label, fields, defaults): (&'static str, Vec<EffectField>, Vec<(&'static str, StyleValue)>) = match kind {
            EffectKind::DropShadow => (
                "Drop Shadow",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("color", "Color", FieldKind::Color),
                    field("opacity", "Opacity", percent()),
                    field("angle", "Angle", FieldKind::Angle),
                    field("useGlobalLight", "Use Global Light", FieldKind::Check),
                    field("distance", "Distance", slider(0.0, 300.0, Pixels)),
                    field("spread", "Spread", percent()),
                    field("size", "Size", slider(0.0, 250.0, Pixels)),
                    field("contour", "Contour", contour("antiAlias")),
                    field("noise", "Noise", percent()),
                    field("knocksOut", "Layer Knocks Out Drop Shadow", FieldKind::Check),
                ],
                vec![
                    ("blend", text("Multiply")),
                    ("color", hex("#000000")),
                    ("opacity", num(75.0)),
                    ("angle", num(120.0)),
                    ("useGlobalLight", StyleValue::Bool(true)),
                    ("distance", num(5.0)),
                    ("spread", num(0.0)),
                    ("size", num(5.0)),
                    ("contour", linear()),
                    ("antiAlias", StyleValue::Bool(false)),
                    ("noise", num(0.0)),
                    ("knocksOut", StyleValue::Bool(true)),
                ],
            ),
            EffectKind::InnerShadow => (
                "Inner Shadow",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("color", "Color", FieldKind::Color),
                    field("opacity", "Opacity", percent()),
                    field("angle", "Angle", FieldKind::Angle),
                    field("useGlobalLight", "Use Global Light", FieldKind::Check),
                    field("distance", "Distance", slider(0.0, 300.0, Pixels)),
                    field("choke", "Choke", percent()),
                    field("size", "Size", slider(0.0, 250.0, Pixels)),
                    field("contour", "Contour", contour("antiAlias")),
                    field("noise", "Noise", percent()),
                ],
                vec![
                    ("blend", text("Multiply")),
                    ("color", hex("#000000")),
                    ("opacity", num(75.0)),
                    ("angle", num(120.0)),
                    ("useGlobalLight", StyleValue::Bool(true)),
                    ("distance", num(5.0)),
                    ("choke", num(0.0)),
                    ("size", num(5.0)),
                    ("contour", linear()),
                    ("antiAlias", StyleValue::Bool(false)),
                    ("noise", num(0.0)),
                ],
            ),
            EffectKind::OuterGlow => (
                "Outer Glow",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("opacity", "Opacity", percent()),
                    field("color", "Color", FieldKind::Color),
                    field("technique", "Technique", glow_technique()),
                    field("spread", "Spread", percent()),
                    field("size", "Size", slider(0.0, 250.0, Pixels)),
                    field("contour", "Contour", contour("antiAlias")),
                    field("range", "Range", slider(1.0, 100.0, Percent)),
                    field("noise", "Noise", percent()),
                ],
                vec![
                    ("blend", text("Screen")),
                    ("opacity", num(75.0)),
                    ("color", hex("#ffffbe")),
                    ("technique", text("softer")),
                    ("spread", num(0.0)),
                    ("size", num(5.0)),
                    ("range", num(50.0)),
                    ("contour", linear()),
                    ("antiAlias", StyleValue::Bool(false)),
                    ("noise", num(0.0)),
                ],
            ),
            EffectKind::InnerGlow => (
                "Inner Glow",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("opacity", "Opacity", percent()),
                    field("color", "Color", FieldKind::Color),
                    field("technique", "Technique", glow_technique()),
                    field("source", "Source", choice(&[("edge", "Edge"), ("center", "Center")])),
                    field("choke", "Choke", percent()),
                    field("size", "Size", slider(0.0, 250.0, Pixels)),
                    field("contour", "Contour", contour("antiAlias")),
                    field("range", "Range", slider(1.0, 100.0, Percent)),
                    field("noise", "Noise", percent()),
                ],
                vec![
                    ("blend", text("Screen")),
                    ("opacity", num(75.0)),
                    ("color", hex("#ffffbe")),
                    ("technique", text("softer")),
                    ("source", text("edge")),
                    ("choke", num(0.0)),
                    ("size", num(5.0)),
                    ("range", num(50.0)),
                    ("contour", linear()),
                    ("antiAlias", StyleValue::Bool(false)),
                    ("noise", num(0.0)),
                ],
            ),
            EffectKind::Stroke => (
                "Stroke",
                vec![
                    field("size", "Size", slider(1.0, 250.0, Pixels)),
                    field("position", "Position", choice(&[("outside", "Outside"), ("inside", "Inside"), ("center", "Center")])),
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("opacity", "Opacity", percent()),
                    field("overprint", "Overprint", FieldKind::Check),
                    field("color", "Color", FieldKind::Color),
                ],
                vec![
                    ("size", num(1.0)),
                    ("position", text("inside")),
                    ("blend", text("Normal")),
                    ("opacity", num(100.0)),
                    ("overprint", StyleValue::Bool(false)),
                    ("color", hex("#000000")),
                ],
            ),
            EffectKind::ColorOverlay => (
                "Color Overlay",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("color", "Color", FieldKind::Color),
                    field("opacity", "Opacity", percent()),
                ],
                vec![("blend", text("Normal")), ("color", hex("#ff0000")), ("opacity", num(100.0))],
            ),
            EffectKind::GradientOverlay => (
                "Gradient Overlay",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("opacity", "Opacity", percent()),
                    field("from", "From", FieldKind::Color),
                    field("to", "To", FieldKind::Color),
                    field("reverse", "Reverse", FieldKind::Check),
                    field(
                        "style",
                        "Style",
                        choice(&[("linear", "Linear"), ("radial", "Radial"), ("angle", "Angle"), ("reflected", "Reflected"), ("diamond", "Diamond")]),
                    ),
                    field("angle", "Angle", FieldKind::Angle),
                    field("scale", "Scale", slider(10.0, 150.0, Percent)),
                    field("method", "Method", FieldKind::GradientMethod),
                ],
                vec![
                    ("blend", text("Normal")),
                    ("opacity", num(100.0)),
                    ("from", hex("#000000")),
                    ("to", hex("#ffffff")),
                    ("reverse", StyleValue::Bool(false)),
                    ("style", text("linear")),
                    ("angle", num(90.0)),
                    ("scale", num(100.0)),
                    ("method", text("custom")),
                ],
            ),
            EffectKind::PatternOverlay => (
                "Pattern Overlay",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("opacity", "Opacity", percent()),
                    field("pattern", "Pattern", FieldKind::Pattern),
                    field("angle", "Angle", FieldKind::Angle),
                    field("scale", "Scale", slider(1.0, 1000.0, Percent)),
                    field("link", "Link with Layer", FieldKind::Check),
                ],
                vec![
                    ("blend", text("Normal")),
                    ("opacity", num(100.0)),
                    ("pattern", text("")),
                    ("angle", num(0.0)),
                    ("scale", num(100.0)),
                    ("link", StyleValue::Bool(true)),
                ],
            ),
            EffectKind::BevelEmboss => (
                "Bevel & Emboss",
                vec![
                    field(
                        "style",
                        "Style",
                        choice(&[("inner", "Inner Bevel"), ("outer", "Outer Bevel"), ("emboss", "Emboss"), ("pillow", "Pillow Emboss")]),
                    ),
                    field("technique", "Technique", choice(&[("smooth", "Smooth"), ("chiselHard", "Chisel Hard"), ("chiselSoft", "Chisel Soft")])),
                    field("depth", "Depth", slider(1.0, 1000.0, Percent)),
                    field("direction", "Direction", choice(&[("up", "Up"), ("down", "Down")])),
                    field("size", "Size", slider(0.0, 250.0, Pixels)),
                    field("soften", "Soften", slider(0.0, 16.0, Pixels)),
                    field("angle", "Angle", FieldKind::Angle),
                    field("useGlobalLight", "Use Global Light", FieldKind::Check),
                    field("altitude", "Altitude", slider(0.0, 90.0, Degrees)),
                    field("glossContour", "Gloss Contour", contour("glossAntiAlias")),
                    field("highlightBlend", "Highlight Mode", FieldKind::Blend),
                    field("highlightColor", "Highlight Color", FieldKind::Color),
                    field("highlightOpacity", "Highlight Opacity", percent()),
                    field("shadowBlend", "Shadow Mode", FieldKind::Blend),
                    field("shadowColor", "Shadow Color", FieldKind::Color),
                    field("shadowOpacity", "Shadow Opacity", percent()),
                    field("contour", "Contour", FieldKind::Check),
                    field("contourName", "", contour("contourAntiAlias")),
                    field("contourRange", "Range", slider(1.0, 100.0, Percent)),
                ],
                vec![
                    ("style", text("inner")),
                    ("technique", text("smooth")),
                    ("depth", num(100.0)),
                    ("direction", text("up")),
                    ("size", num(5.0)),
                    ("soften", num(0.0)),
                    ("angle", num(120.0)),
                    ("useGlobalLight", StyleValue::Bool(true)),
                    ("altitude", num(30.0)),
                    ("glossContour", linear()),
                    ("glossAntiAlias", StyleValue::Bool(false)),
                    ("highlightBlend", text("Screen")),
                    ("highlightColor", hex("#ffffff")),
                    ("highlightOpacity", num(75.0)),
                    ("shadowBlend", text("Multiply")),
                    ("shadowColor", hex("#000000")),
                    ("shadowOpacity", num(75.0)),
                    ("contour", StyleValue::Bool(false)),
                    ("contourName", linear()),
                    ("contourRange", num(50.0)),
                    ("contourAntiAlias", StyleValue::Bool(false)),
                ],
            ),
            EffectKind::Satin => (
                "Satin",
                vec![
                    field("blend", "Blend Mode", FieldKind::Blend),
                    field("color", "Color", FieldKind::Color),
                    field("opacity", "Opacity", percent()),
                    field("angle", "Angle", FieldKind::Angle),
                    field("distance", "Distance", slider(0.0, 250.0, Pixels)),
                    field("size", "Size", slider(0.0, 250.0, Pixels)),
                    field("contour", "Contour", contour("antiAlias")),
                    field("invert", "Invert", FieldKind::Check),
                ],
                vec![
                    ("blend", text("Multiply")),
                    ("color", hex("#000000")),
                    ("opacity", num(50.0)),
                    ("angle", num(19.0)),
                    ("distance", num(11.0)),
                    ("size", num(14.0)),
                    ("contour", linear()),
                    ("antiAlias", StyleValue::Bool(true)),
                    ("invert", StyleValue::Bool(true)),
                ],
            ),
        };
        let defaults = StyleParams(defaults.into_iter().map(|(k, v)| (k.to_owned(), v)).collect());
        EffectSpec { kind, label: Cow::Borrowed(label), fields, defaults }
    }

    /// The field with `key`.
    pub fn field(&self, key: &str) -> Option<&EffectField> {
        self.fields.iter().find(|f| f.key == key)
    }
}

fn field(key: &'static str, label: &'static str, kind: FieldKind) -> EffectField {
    EffectField { key: Cow::Borrowed(key), label: Cow::Borrowed(label), kind }
}

fn slider(min: f64, max: f64, unit: FieldUnit) -> FieldKind {
    FieldKind::Slider { min, max, unit }
}

fn choice(options: &[(&'static str, &'static str)]) -> FieldKind {
    let options = options.iter().map(|&(value, label)| ChoiceOption { value: Cow::Borrowed(value), label: Cow::Borrowed(label) }).collect();
    FieldKind::Choice(Cow::Owned(options))
}

fn num(n: f64) -> StyleValue {
    StyleValue::Number(n)
}

fn text(s: &str) -> StyleValue {
    StyleValue::Text(s.to_owned())
}

fn hex(s: &str) -> StyleValue {
    StyleValue::Color(Srgba::from_hex(s).unwrap_or(Srgba::BLACK))
}

fn linear() -> StyleValue {
    StyleValue::Contour(Contour::linear())
}

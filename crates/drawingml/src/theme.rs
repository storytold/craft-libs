//! Themes: colour scheme, font scheme and format scheme (the fill/line/effect "style matrix").

use crate::color::{ColorScheme, ColorTransform, Rgba, SchemeSlot};
use serde::{Deserialize, Serialize};

use crate::style::{ColorRef, Effects, Fill, Gradient, GradientShape, GradientStop, Line, Shadow};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontSet {
    pub latin: String,
    pub ea: String,
    pub cs: String,
    /// Per-script typefaces (`a:font script="Arab" typeface="…"`), in document order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptFont>,
}

/// A theme font for one script (ISO 15924 tag, e.g. `Arab`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScriptFont {
    pub script: String,
    pub typeface: String,
}

impl FontSet {
    /// The typeface the theme gives `script` (`"Arab"`), if any.
    pub fn script(&self, script: &str) -> Option<&str> {
        self.scripts.iter().find(|f| f.script.eq_ignore_ascii_case(script) && !f.typeface.is_empty()).map(|f| f.typeface.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontScheme {
    pub name: String,
    /// Headings.
    pub major: FontSet,
    /// Body.
    pub minor: FontSet,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FormatScheme {
    pub name: String,
    /// Subtle, moderate, intense (style index 1–3).
    pub fills: Vec<Fill>,
    pub lines: Vec<Line>,
    pub effects: Vec<Effects>,
    /// Background fills (index 1001–1003).
    pub bg_fills: Vec<Fill>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Theme {
    pub name: String,
    pub colors: ColorScheme,
    pub fonts: FontScheme,
    pub format: FormatScheme,
    /// Raw theme XML extras kept for round-trip (object defaults, extra colour lists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_extra: Option<String>,
}

impl Default for Theme {
    /// A plain theme: black and white, a slate and a warm grey, six accents, no font names (the
    /// writer puts `Arial` where a Latin face is required) and [`default_format`].
    fn default() -> Self {
        let hex = |s: &str| Rgba::from_hex(s).unwrap_or(Rgba::BLACK);
        let colors = ["000000", "FFFFFF", "1E2B3C", "EDEBE7", "2E6FD8", "F26B1D", "2EA36F", "8A5CD8", "D83F6B", "E3B21C", "2E6FD8", "7A5BA6"];
        Theme {
            name: "Theme".into(),
            colors: ColorScheme::new("Default", colors.map(hex)),
            fonts: FontScheme::default(),
            format: default_format(),
            raw_extra: None,
        }
    }
}

impl Theme {
    /// Resolve a theme font reference (`+mj-lt`, `+mn-ea`…) to a family name.
    pub fn font(&self, name: &str) -> String {
        match name {
            "+mj-lt" => self.fonts.major.latin.clone(),
            "+mn-lt" => self.fonts.minor.latin.clone(),
            "+mj-ea" => self.fonts.major.ea.clone(),
            "+mn-ea" => self.fonts.minor.ea.clone(),
            "+mj-cs" => self.fonts.major.cs.clone(),
            "+mn-cs" => self.fonts.minor.cs.clone(),
            other => other.to_string(),
        }
    }
}

fn ph() -> ColorRef {
    ColorRef::scheme(SchemeSlot::PhClr)
}

/// The format scheme used where a theme has none, and to fill a short style list: flat solid fills, a soft gradient, and a gentle shadow for "intense".
pub fn default_format() -> FormatScheme {
    let grad = |a: i32, b: i32| {
        Fill::Gradient(Gradient {
            tile: [0.0; 4],
            stops: vec![
                GradientStop { pos: 0.0, color: ph().with(ColorTransform::LumMod(a)).with(ColorTransform::SatMod(103000)) },
                GradientStop { pos: 1.0, color: ph().with(ColorTransform::LumMod(b)) },
            ],
            shape: Some(GradientShape::Linear { angle: 90.0, scaled: false }),
            rotate_with_shape: Some(true),
            flip: None,
        })
    };
    FormatScheme {
        name: "Default".into(),
        fills: vec![Fill::solid(ph()), grad(110000, 92000), grad(105000, 80000)],
        lines: vec![Line::solid(ph(), 0.75), Line::solid(ph(), 1.0), Line::solid(ph(), 1.5)],
        effects: vec![
            Effects::default(),
            Effects::default(),
            Effects {
                outer_shadow: Some(Shadow {
                    color: ColorRef::rgb(Rgba::BLACK).with(ColorTransform::Alpha(40000)),
                    blur: 6.0,
                    dist: 3.0,
                    dir: 90.0,
                    inner: false,
                    sx: 1.0,
                    sy: 1.0,
                    kx: 0.0,
                    ky: 0.0,
                    align: "t".into(),
                    rotate_with_shape: false,
                }),
                ..Default::default()
            },
        ],
        bg_fills: vec![
            Fill::solid(ph()),
            Fill::solid(ph().with(ColorTransform::Tint(95000)).with(ColorTransform::SatMod(170000))),
            grad(102000, 85000),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_has_a_full_style_matrix_and_no_font_names() {
        let t = Theme::default();
        assert_eq!((t.format.fills.len(), t.format.lines.len(), t.format.effects.len(), t.format.bg_fills.len()), (3, 3, 3, 3));
        assert_eq!(t.colors.get(SchemeSlot::Lt1), Rgba::WHITE);
        assert_eq!(t.fonts, FontScheme::default());
    }

    #[test]
    fn theme_font_references_resolve_and_other_names_pass_through() {
        let mut t = Theme::default();
        t.fonts.major.latin = "Head".into();
        t.fonts.minor.cs = "Body CS".into();
        t.fonts.minor.scripts.push(ScriptFont { script: "Arab".into(), typeface: "Naskh".into() });
        assert_eq!(t.font("+mj-lt"), "Head");
        assert_eq!(t.font("+mn-cs"), "Body CS");
        assert_eq!(t.font("Georgia"), "Georgia");
        assert_eq!(t.fonts.minor.script("arab"), Some("Naskh"));
        assert_eq!(t.fonts.minor.script("Hebr"), None);
    }

    #[test]
    fn a_theme_survives_json_and_missing_fields_take_the_default() {
        let t = Theme::default();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Theme>(&json).unwrap(), t);
        let partial: Theme = serde_json::from_str(r#"{"name":"X"}"#).unwrap();
        assert_eq!(partial.colors, t.colors);
        assert_eq!(partial.name, "X");
    }
}

/// The `a:ext` of a colour scheme that holds its extra named colours and explicit tint rows
/// ([`ColorScheme::extra`], [`ColorScheme::tints`], [`ColorScheme::computed_tints`]). The URI and
/// namespace are those of the files already written with it.
pub const EXT_EXTRA_COLORS: &str = "{D3C7C0DE-0C0A-4E57-9A1E-5C0E0C0DEC01}";
/// The namespace of the elements inside [`EXT_EXTRA_COLORS`].
pub const NS_EXTRA_COLORS: &str = "urn:deckcraft:theme:extra-colors:1";

mod read;
mod write;
pub use read::read_theme;
pub use write::theme_xml;

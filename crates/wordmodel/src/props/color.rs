use super::{Deserialize, Serialize};

/// An sRGB colour: craft-math's, which equations and text share.
pub use craft_math::Rgb;

/// A text colour: automatic (black on light backgrounds, white on dark) or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Default)]
pub enum TextColor {
    #[default]
    Auto,
    Rgb(Rgb),
}

impl TextColor {
    /// `auto` or `automatic` (any case), or a colour [`Rgb::parse`] reads.
    pub fn parse(s: &str) -> Option<TextColor> {
        if s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("automatic") { Some(TextColor::Auto) } else { Rgb::parse(s).map(TextColor::Rgb) }
    }
}

impl<'de> Deserialize<'de> for TextColor {
    /// Reads what it writes (`"Auto"`, `{"Rgb": [r, g, b]}`) and also a string [`TextColor::parse`]
    /// reads, so `styles.modify` and `format.set` take the colours `format.color` takes.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, Unexpected};
        #[derive(Deserialize)]
        enum Written {
            Auto,
            Rgb(Rgb),
        }
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => {
                TextColor::parse(&s).ok_or_else(|| D::Error::invalid_value(Unexpected::Str(&s), &"`auto`, `RRGGBB` or `#RRGGBB`"))
            }
            v => Ok(match serde_json::from_value(v).map_err(D::Error::custom)? {
                Written::Auto => TextColor::Auto,
                Written::Rgb(c) => TextColor::Rgb(c),
            }),
        }
    }
}

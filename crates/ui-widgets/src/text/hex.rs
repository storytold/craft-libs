//! `HexField`: a text field over a colour's `#rrggbb` form, and `HexCodec`, the codec behind it.

#[cfg(feature = "testing")]
mod stories;

use super::TextField;
use crate::contract::Bind;
use crate::runtime::env::Label;
use craft_behavior::{Caret, Codec, NotAValue};
use craft_theme::Rgba;
use std::borrow::Cow;

/// The widest text a colour takes, for the field's ideal width.
const WIDEST: &str = "#000000";

/// A colour's hexadecimal text: `#rrggbb`, or `#rrggbbaa` when the colour is not opaque.
///
/// Typed text reads with or without the `#`, in either case. Three digits stand for the doubled digits (`#f80` is
/// `#ff8800`), six digits are the colour, and eight digits add an alpha. A colour typed without an alpha keeps the
/// alpha it had when editing began.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HexCodec;

impl Codec for HexCodec {
    type Value = Rgba;

    fn format(&self, v: &Rgba) -> String {
        v.to_hex()
    }

    fn parse(&self, text: &str, start: &Rgba) -> Result<Rgba, NotAValue> {
        let digits = text.trim().trim_start_matches('#');
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(NotAValue);
        }
        let pair = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
        let doubled = |at: usize| u8::from_str_radix(digits.get(at..=at)?, 16).ok().map(|digit| digit * 17);
        let [.., alpha] = start.0;
        let [r, g, b, a] = match digits.len() {
            3 => [doubled(0), doubled(1), doubled(2), Some(alpha)],
            6 => [pair(0), pair(2), pair(4), Some(alpha)],
            8 => [pair(0), pair(2), pair(4), pair(6)],
            _ => return Err(NotAValue),
        };
        match (r, g, b, a) {
            (Some(r), Some(g), Some(b), Some(a)) => Ok(Rgba([r, g, b, a])),
            _ => Err(NotAValue),
        }
    }

    fn filter(&self, c: char) -> bool {
        c == '#' || c.is_ascii_hexdigit()
    }

    fn normalise(&self, _text: &str, v: &Rgba) -> String {
        self.format(v)
    }

    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::All
    }

    fn sample(&self, _current: &Rgba) -> Cow<'_, str> {
        Cow::Borrowed(WIDEST)
    }
}

/// A text field over a colour's hexadecimal text.
pub type HexField<'a> = TextField<'a, HexCodec>;

impl<'a> TextField<'a, HexCodec> {
    /// A hex field named `label` that shows and edits `value`.
    pub fn hex(label: impl Into<Label>, value: Bind<'a, Rgba>) -> Self {
        let mut field = TextField::with_codec(label, value, HexCodec);
        field.empty = Some(Rgba::rgb(0, 0, 0));
        field
    }
}

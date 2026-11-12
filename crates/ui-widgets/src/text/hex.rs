//! `HexField`: a text field over a colour's hex digits behind a fixed `#`, and `HexCodec`, the codec behind it.

#[cfg(feature = "testing")]
mod stories;

use super::TextField;
use crate::contract::Bind;
use crate::runtime::env::Label;
use craft_behavior::{Caret, Codec, NotAValue};
use craft_i18n::Word;
use craft_theme::Rgba;
use std::borrow::Cow;

/// A colour's hexadecimal digits, uppercase and without the `#`, which the field shows before them: `RRGGBB`, or
/// `RGB` with one digit per channel, each channel on its nearest doubled digit (a web colour). With `alpha` the
/// alpha digits follow (`RRGGBBAA`, `RGBA`).
///
/// Typed text reads with or without a leading `#`, in either case: three digits per channel count (`F80` is #FF8800
/// with one digit), and with `alpha` four. A colour typed without alpha digits keeps the alpha it had when editing
/// began. A draft applies as soon as it is a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HexCodec {
    /// Digits per channel: 1 or 2.
    digits: usize,
    /// Whether the alpha digits are shown and read.
    alpha: bool,
}

impl Default for HexCodec {
    fn default() -> Self {
        HexCodec { digits: 2, alpha: false }
    }
}

impl HexCodec {
    /// Digits per channel: 2 (`RRGGBB`, the default) or 1 (`RGB`); other counts take the nearer of those.
    pub fn digits(mut self, per_channel: usize) -> Self {
        self.digits = per_channel.clamp(1, 2);
        self
    }

    /// Whether the alpha digits are shown and read; off by default.
    pub fn alpha(mut self, on: bool) -> Self {
        self.alpha = on;
        self
    }

    /// The channels the text has: three, or four with the alpha.
    fn channels(&self) -> usize {
        if self.alpha { 4 } else { 3 }
    }
}

impl Codec for HexCodec {
    type Value = Rgba;

    fn format(&self, v: &Rgba) -> String {
        let channels = &v.0[..self.channels()];
        match self.digits {
            1 => channels.iter().map(|value| format!("{:X}", (u16::from(*value) + 8) / 17)).collect(),
            _ => channels.iter().map(|value| format!("{value:02X}")).collect(),
        }
    }

    fn parse(&self, text: &str, start: &Rgba) -> Result<Rgba, NotAValue> {
        let digits = text.trim();
        let digits = digits.strip_prefix('#').unwrap_or(digits);
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(NotAValue);
        }
        let width = self.digits;
        let channel = |at: usize| {
            let value = u8::from_str_radix(digits.get(at * width..(at + 1) * width)?, 16).ok()?;
            Some(if width == 1 { value * 17 } else { value })
        };
        let [.., alpha] = start.0;
        let [r, g, b, a] = match digits.len() {
            n if n == 3 * width => [channel(0), channel(1), channel(2), Some(alpha)],
            n if self.alpha && n == 4 * width => [channel(0), channel(1), channel(2), channel(3)],
            _ => return Err(NotAValue),
        };
        match (r, g, b, a) {
            (Some(r), Some(g), Some(b), Some(a)) => Ok(Rgba([r, g, b, a])),
            _ => Err(NotAValue),
        }
    }

    fn applies_as_typed(&self, _text: &str) -> bool {
        true
    }

    fn filter(&self, c: char) -> bool {
        c == '#' || c.is_ascii_hexdigit()
    }

    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::All
    }

    fn sample(&self, _current: &Rgba) -> Cow<'_, str> {
        Cow::Owned("0".repeat(self.digits * self.channels()))
    }
}

/// A text field over a colour's hexadecimal digits.
pub type HexField<'a> = TextField<'a, HexCodec>;

/// Why a hex field's draft is no colour.
const NOT_HEX: Word = Word::keyed("hexField.unparsed", "Not a hex color");

impl<'a> TextField<'a, HexCodec> {
    /// A hex field named `label` that shows and edits `value`: monospace digits behind a fixed `#`. A draft that is no
    /// colour says so ("Not a hex color", `TextField::unparsed`).
    pub fn hex(label: impl Into<Label>, value: Bind<'a, Rgba>) -> Self {
        let mut field = TextField::with_codec(label, value, HexCodec::default()).mono().prefix("#").unparsed(NOT_HEX);
        field.empty = Some(Rgba::rgb(0, 0, 0));
        field
    }

    /// Digits per channel: 2 (`RRGGBB`, the default) or 1 (`RGB`, a web colour).
    #[must_use]
    pub fn digits(mut self, per_channel: usize) -> Self {
        self.codec = self.codec.digits(per_channel);
        self
    }

    /// Whether the field also shows and takes the alpha digits; off by default.
    #[must_use]
    pub fn alpha(mut self, on: bool) -> Self {
        self.codec = self.codec.alpha(on);
        self
    }
}

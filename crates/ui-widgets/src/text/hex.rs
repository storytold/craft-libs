//! `HexField`: a text field over a colour's hex digits, the `#` side label its row shows before it, and `HexCodec`,
//! the codec behind it.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::TextField;
use crate::contract::{Bind, GlyphLabel, LabelKind, SideLabel};
use crate::runtime::env::Label;
use craft_behavior::{Caret, Codec, NotAValue};
use craft_i18n::Word;
use craft_theme::Rgba;
use std::borrow::Cow;

/// A colour's hexadecimal digits, uppercase and without the `#`, which the field's row shows before them: `RRGGBB`, or
/// `RGB` with one digit per channel, each channel on its nearest doubled digit (a web colour). With `alpha` the
/// alpha digits follow (`RRGGBBAA`, `RGBA`).
///
/// Typed text reads with or without a leading `#`, in either case: three digits per channel count (`F80` is #FF8800
/// with one digit), and with `alpha` four. A colour typed without alpha digits keeps the alpha it had when editing
/// began. A draft applies as soon as it is a colour. With [`notations`](Self::notations) it also reads the other common
/// ways of writing a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HexCodec {
    /// Digits per channel: 1 or 2.
    digits: usize,
    /// Whether the alpha digits are shown and read.
    alpha: bool,
    /// Whether text may also be `0x` digits, `rgb(r, g, b)` or three channel values.
    notations: bool,
}

impl Default for HexCodec {
    fn default() -> Self {
        HexCodec { digits: 2, alpha: false, notations: false }
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

    /// Whether typed text may also be a colour written another common way: hex digits after `0x`, `rgb(r, g, b)`, or three
    /// channel values 0 to 255 separated by commas or spaces (`12, 34, 56`). Off by default. A colour written with channel
    /// values keeps the alpha it had and applies on commit, as a shorthand does.
    pub fn notations(mut self, on: bool) -> Self {
        self.notations = on;
        self
    }

    /// The channels the text has: three, or four with the alpha.
    fn channels(&self) -> usize {
        if self.alpha { 4 } else { 3 }
    }

    /// The digits of `text`: trimmed and less one `#`, and with `notations` less one `0x`.
    fn digits_of<'t>(&self, text: &'t str) -> &'t str {
        let text = text.trim();
        let text = text.strip_prefix('#').unwrap_or(text);
        match self.notations {
            true => text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text),
            false => text,
        }
    }

    /// With `notations`, the channels of a colour written as `rgb(r, g, b)` or as three values 0 to 255 separated by
    /// commas or spaces.
    fn written(&self, text: &str) -> Option<[u8; 3]> {
        if !self.notations {
            return None;
        }
        let lower = text.trim().to_ascii_lowercase();
        let inner = lower.strip_prefix("rgb").map_or(lower.as_str(), |rest| rest.trim().trim_start_matches('(').trim_end_matches(')'));
        let parts: Vec<&str> = inner.split(|c: char| c == ',' || c.is_whitespace()).filter(|part| !part.is_empty()).collect();
        let [r, g, b] = parts.as_slice() else { return None };
        Some([r.parse().ok()?, g.parse().ok()?, b.parse().ok()?])
    }

    /// Whether `digits` are the shorthand of a two-digit codec: one digit a channel, each read doubled.
    fn is_shorthand(&self, digits: &str) -> bool {
        self.digits == 2 && digits.len() == self.channels()
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
        let digits = self.digits_of(text);
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let [.., alpha] = start.0;
            return self.written(text).map(|[r, g, b]| Rgba([r, g, b, alpha])).ok_or(NotAValue);
        }
        let doubled: String;
        let digits = if self.is_shorthand(digits) {
            doubled = digits.chars().flat_map(|digit| [digit, digit]).collect();
            doubled.as_str()
        } else {
            digits
        };
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

    /// A shorthand's digits are the start of a full colour (`336` of `336699`): they apply on commit only, as channel
    /// values do (`12, 34, 5` of `12, 34, 56`).
    fn applies_as_typed(&self, text: &str) -> bool {
        let digits = self.digits_of(text);
        digits.bytes().all(|byte| byte.is_ascii_hexdigit()) && !self.is_shorthand(digits)
    }

    fn filter(&self, c: char) -> bool {
        c == '#'
            || c.is_ascii_hexdigit()
            || (self.notations && matches!(c, 'x' | 'X' | 'r' | 'R' | 'g' | 'G' | '(' | ')' | ',' | ' ' | '.' | '-' | '+'))
    }

    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::All
    }

    fn sample(&self, _current: &Rgba) -> Cow<'_, str> {
        Cow::Owned("0".repeat(self.digits * self.channels()))
    }

    fn whole_paste(&self, pasted: &str) -> bool {
        self.parse(pasted, &Rgba([0, 0, 0, 255])).is_ok()
    }
}

/// A text field over a colour's hexadecimal digits.
pub type HexField<'a> = TextField<'a, HexCodec>;

/// Why a hex field's draft is no colour.
const NOT_HEX: Word = Word::keyed("hexField.unparsed", "Not a hex color");

impl<'a> TextField<'a, HexCodec> {
    /// A hex field named `label` that shows and edits `value` as monospace digits. A draft that is no colour says so
    /// ("Not a hex color", `TextField::unparsed`). Its row names the digits with [`hash_label`](Self::hash_label).
    pub fn hex(label: impl Into<Label>, value: Bind<'a, Rgba>) -> Self {
        let mut field = TextField::with_codec(label, value, HexCodec::default()).mono().unparsed(NOT_HEX);
        field.empty = Some(Rgba::rgb(0, 0, 0));
        field
    }

    /// The text before a hex field.
    pub const HASH: &'static str = "#";

    /// The `#` before a hex field: the glyph side label of the `Field` that holds it, in every place a hex field shows.
    pub fn hash_label() -> SideLabel {
        SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(Self::HASH)))
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

    /// Whether the field also takes a colour written as `0x` digits, `rgb(r, g, b)` or three channel values; off by
    /// default ([`HexCodec::notations`]).
    #[must_use]
    pub fn notations(mut self, on: bool) -> Self {
        self.codec = self.codec.notations(on);
        self
    }
}

//! `HexField`: a text field over a colour's hex digits, the `#` side label its row shows before it, and `HexCodec`,
//! the codec behind it.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::TextField;
use crate::contract::{Bind, GlyphLabel, LabelKind, SideLabel};
use crate::runtime::env::Label;
use craft_behavior::{Caret, Codec, NotAValue, ParseNote};
use craft_i18n::Word;
use craft_theme::Rgba;
use std::borrow::Cow;

/// A colour's hexadecimal digits, uppercase and without the `#`, which the field's row shows before them: `RRGGBB`, or
/// `RGB` with one digit per channel, each channel on its nearest doubled digit (a web colour). With `alpha` the
/// alpha digits follow (`RRGGBBAA`, `RGBA`).
///
/// Typed text reads with or without a leading `#`, in either case: three digits per channel count (`F80` is #FF8800
/// with one digit), and with `alpha` four. A colour typed without alpha digits keeps the alpha it had when editing
/// began. A draft applies as soon as it is a colour.
///
/// With [`short_input`](Self::short_input) a six-digit field also reads other counts, as the reference image editor's
/// Color Picker does: fewer digits pad on the left with zeros (`12` is `000012`), and more read as the closest value,
/// `FFFFFF`, with the parse's note saying the typed number was held ([`ParseNote::held`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HexCodec {
    /// Digits per channel: 1 or 2.
    digits: usize,
    /// Whether the alpha digits are shown and read.
    alpha: bool,
    /// Whether a six-digit field pads short input and holds long input to `FFFFFF`.
    short_input: bool,
    /// Whether the digits show in lower case.
    lower_case: bool,
}

impl Default for HexCodec {
    fn default() -> Self {
        HexCodec { digits: 2, alpha: false, short_input: false, lower_case: false }
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

    /// Whether a six-digit field (two digits per channel, no alpha) reads any count of digits: fewer than six pad on
    /// the left with zeros, three still read as the shorthand, and more than six read as `FFFFFF`, the closest value,
    /// with the typed number noted as held. Such input applies on commit only. Off by default; other forms ignore it.
    pub fn short_input(mut self, on: bool) -> Self {
        self.short_input = on;
        self
    }

    /// Whether the digits show in lower case (`ff8000`, the reference image editor's Color Picker); upper case by
    /// default. Typed digits read in either case.
    pub fn lower_case(mut self, on: bool) -> Self {
        self.lower_case = on;
        self
    }

    /// Whether the short-input reading applies: on, and a six-digit field.
    fn reads_short(&self) -> bool {
        self.short_input && self.digits == 2 && !self.alpha
    }

    /// The channels the text has: three, or four with the alpha.
    fn channels(&self) -> usize {
        if self.alpha { 4 } else { 3 }
    }

    /// The digits of `text`: trimmed and less one `#`.
    fn digits_of(text: &str) -> &str {
        let text = text.trim();
        text.strip_prefix('#').unwrap_or(text)
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
        let text: String = match self.digits {
            1 => channels.iter().map(|value| format!("{:X}", (u16::from(*value) + 8) / 17)).collect(),
            _ => channels.iter().map(|value| format!("{value:02X}")).collect(),
        };
        if self.lower_case { text.to_ascii_lowercase() } else { text }
    }

    fn parse(&self, text: &str, start: &Rgba) -> Result<Rgba, NotAValue> {
        self.parse_tagged(text, start).map(|(value, _)| value)
    }

    /// The colour of `text`; with short input, a number longer than six digits is held to `FFFFFF` and the note
    /// carries the number typed.
    fn parse_tagged(&self, text: &str, start: &Rgba) -> Result<(Rgba, ParseNote), NotAValue> {
        let digits = Self::digits_of(text);
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(NotAValue);
        }
        let [.., alpha] = start.0;
        let full = 3 * self.digits;
        if self.reads_short() && digits.len() > full {
            // The number typed, for the note: each digit's value, saturating to infinity for absurd lengths.
            let typed = digits.chars().filter_map(|digit| digit.to_digit(16)).fold(0.0, |number: f64, digit| number * 16.0 + f64::from(digit));
            let mut note = ParseNote::default();
            note.held = Some(typed);
            return Ok((Rgba([255, 255, 255, alpha]), note));
        }
        let doubled: String;
        let padded: String;
        let digits = if self.is_shorthand(digits) {
            doubled = digits.chars().flat_map(|digit| [digit, digit]).collect();
            doubled.as_str()
        } else if self.reads_short() && !digits.is_empty() && digits.len() < full {
            padded = format!("{digits:0>full$}");
            padded.as_str()
        } else {
            digits
        };
        let width = self.digits;
        let channel = |at: usize| {
            let value = u8::from_str_radix(digits.get(at * width..(at + 1) * width)?, 16).ok()?;
            Some(if width == 1 { value * 17 } else { value })
        };
        let [r, g, b, a] = match digits.len() {
            n if n == 3 * width => [channel(0), channel(1), channel(2), Some(alpha)],
            n if self.alpha && n == 4 * width => [channel(0), channel(1), channel(2), channel(3)],
            _ => return Err(NotAValue),
        };
        match (r, g, b, a) {
            (Some(r), Some(g), Some(b), Some(a)) => Ok((Rgba([r, g, b, a]), ParseNote::default())),
            _ => Err(NotAValue),
        }
    }

    /// A shorthand's digits are the start of a full colour (`336` of `336699`): they apply on commit only. With short
    /// input, so does every count but six.
    fn applies_as_typed(&self, text: &str) -> bool {
        let digits = Self::digits_of(text);
        if self.reads_short() {
            return digits.len() == 3 * self.digits;
        }
        !self.is_shorthand(digits)
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

    /// Whether the digits show in lower case ([`HexCodec::lower_case`]); upper case by default.
    #[must_use]
    pub fn lower_case(mut self, on: bool) -> Self {
        self.codec = self.codec.lower_case(on);
        self
    }

    /// Whether a six-digit field pads short input with zeros and reads input past six digits as `FFFFFF`
    /// ([`HexCodec::short_input`]); off by default. The committed number a field held is reported through
    /// [`held_into`](TextField::held_into).
    #[must_use]
    pub fn short_input(mut self, on: bool) -> Self {
        self.codec = self.codec.short_input(on);
        self
    }
}

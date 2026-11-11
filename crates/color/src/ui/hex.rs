//! `PairCodec` and `HexPairField`: a byte as two hex digits, the field of a Web Color slider.
//!
//! The colour's own hex field is `craft_ui::text::HexField`.

use craft_behavior::{Caret, Codec, NotAValue};
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Label;
use craft_ui::text::TextField;
use std::borrow::Cow;

/// The widest text a byte takes, for the field's ideal width.
const WIDEST: &str = "FF";

/// A byte as two hex digits, uppercase as Photoshop's Web Color Sliders show it. Typed text is exactly two hex
/// digits; anything else is not a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PairCodec;

impl Codec for PairCodec {
    type Value = u8;

    fn format(&self, value: &u8) -> String {
        format!("{value:02X}")
    }

    fn parse(&self, text: &str, _start: &u8) -> Result<u8, NotAValue> {
        // Two hex digits only: `from_str_radix` would also take a sign.
        let text = text.trim();
        match u8::from_str_radix(text, 16) {
            Ok(value) if text.len() == 2 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) => Ok(value),
            _ => Err(NotAValue),
        }
    }

    fn applies_as_typed(&self, _text: &str) -> bool {
        true
    }

    fn filter(&self, c: char) -> bool {
        c.is_ascii_hexdigit()
    }

    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::All
    }

    fn sample(&self, _current: &u8) -> Cow<'_, str> {
        Cow::Borrowed(WIDEST)
    }
}

/// A text field over a byte's two hex digits.
pub type HexPairField<'a> = TextField<'a, PairCodec>;

/// The constructor of [`HexPairField`].
pub trait HexPair<'a> {
    /// A pair field named `label` that shows and edits `value`.
    fn hex_pair(label: impl Into<Label>, value: Bind<'a, u8>) -> Self;
}

impl<'a> HexPair<'a> for TextField<'a, PairCodec> {
    fn hex_pair(label: impl Into<Label>, value: Bind<'a, u8>) -> Self {
        TextField::with_codec(label, value, PairCodec)
    }
}

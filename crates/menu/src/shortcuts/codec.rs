//! `ChordCodec`: the text form of a binding, for the typed shortcut field.

use craft_behavior::{Codec, NotAValue};
use craft_keymap::{Binding, Platform};

/// A binding's text: `Cmd+K`, `Cmd+K Cmd+K`, or `Timeline: Space` with a scope. Empty text is no binding.
///
/// The value is the whole binding, not the bare chord, because the typed field takes a scope prefix and a double
/// press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChordCodec {
    platform: Platform,
}

impl ChordCodec {
    /// A codec for `platform`.
    pub const fn new(platform: Platform) -> Self {
        ChordCodec { platform }
    }

    /// The platform the field belongs to.
    pub const fn platform(&self) -> Platform {
        self.platform
    }
}

impl Codec for ChordCodec {
    type Value = Option<Binding>;

    fn format(&self, v: &Option<Binding>) -> String {
        v.as_ref().map_or_else(String::new, Binding::to_string)
    }

    fn parse(&self, text: &str, _start: &Option<Binding>) -> Result<Option<Binding>, NotAValue> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        Binding::parse(text).map(Some).map_err(|_| NotAValue)
    }
}

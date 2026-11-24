//! The custom-connection start of the form: the "Other (OpenAI compatible)" preset as its own
//! entry point, and the check on the name of the header that carries the key.

use super::{ConnectionDraft, DraftError, Field, PRESETS};
use crate::connection::check_key_header;

impl ConnectionDraft {
    /// The form for a server that is not on the list: OpenAI-shaped, address typed in, key needed,
    /// billing unknown until the person says otherwise. Same as picking the `custom` preset.
    pub fn custom() -> Self {
        match PRESETS.iter().find(|p| p.key == "custom") {
            Some(preset) => Self::from_preset(preset),
            None => Self::default(),
        }
    }

    /// Whether this is a custom connection, whose models come from the person and not from a
    /// guess at the server's model names.
    pub fn is_custom(&self) -> bool {
        self.provider.trim() == "openai-compatible"
    }

    /// The key header name, checked: `None` means the bearer default.
    pub(super) fn checked_key_header(&self) -> Result<Option<String>, DraftError> {
        let name = self.key_header.trim();
        if name.is_empty() {
            return Ok(None);
        }
        if !self.uses_key {
            return Err(DraftError::new(Field::KeyHeader, "A header name only applies when the connection uses a key."));
        }
        check_key_header(name).map_err(|why| DraftError::new(Field::KeyHeader, format!("The header name: {why}.")))?;
        Ok(Some(name.to_owned()))
    }
}

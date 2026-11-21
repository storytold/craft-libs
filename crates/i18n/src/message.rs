//! Matching a filled-in English message against `@msg` and `@msgplural` templates.

/// What a [`crate::I18n::message_with`] hook sees of the matched template.
#[derive(Clone, Copy, Debug)]
pub struct MessageMatch<'a> {
    /// The English template that matched.
    pub english: &'a str,
    /// The captured values, as (key, value) pairs.
    pub captures: &'a [(&'a str, &'a str)],
}

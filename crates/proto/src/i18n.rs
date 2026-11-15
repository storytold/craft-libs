// SPDX-FileCopyrightText: Craft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Error message handling for protocol errors.
//!
//! Provides the `said!` macro and related types for creating user-facing error messages.

pub use crate::said;
use std::fmt;

/// A value that can be converted to an argument for error messages.
pub trait ToArg {
    fn to_arg(&self) -> Arg;
}

impl<T: fmt::Display> ToArg for T {
    fn to_arg(&self) -> Arg {
        Arg::Plain(format!("{}", self))
    }
}

/// An argument for error messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Plain(String),
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Plain(s) => f.write_str(s),
        }
    }
}

impl Arg {
    /// Create an argument from a string.
    pub fn said(t: Text) -> Self {
        Arg::Plain(format!("{}", t))
    }
}

/// A text with values: `said!("no folder named {name}", name)`, or
/// `said!("{name} is larger than {mb} MB", name, mb = MAX >> 20)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    template: String,
}

impl Text {
    /// Preserve text supplied by a server or host without interpreting it.
    pub fn verbatim(text: String) -> Self {
        Self { template: text }
    }

    /// Create a new text with the given template and arguments.
    pub fn new(template: &str, args: Vec<(&str, Arg)>) -> Self {
        // Substitute only placeholders in the template, never text inserted by a server.
        let mut filled = String::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            filled.push_str(rest.get(..open).unwrap_or_default());
            let after = rest.get(open.saturating_add(1)..).unwrap_or_default();
            let Some(close) = after.find('}') else {
                filled.push_str(rest.get(open..).unwrap_or_default());
                rest = "";
                break;
            };
            let key = after.get(..close).unwrap_or_default();
            if let Some((_, arg)) = args.iter().find(|(name, _)| *name == key) {
                filled.push_str(&arg.to_string());
            } else {
                filled.push('{');
                filled.push_str(key);
                filled.push('}');
            }
            rest = after.get(close.saturating_add(1)..).unwrap_or_default();
        }
        filled.push_str(rest);
        Text { template: filled }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.template)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text { template: s.to_string() }
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::verbatim(s)
    }
}

/// The `said!` macro for creating error messages.
#[macro_export]
macro_rules! said {
    (@arg $k:ident) => { $crate::i18n::ToArg::to_arg(&$k) };
    (@arg $k:ident = $v:expr) => { $crate::i18n::ToArg::to_arg(&$v) };
    ($t:literal $(, $k:ident $(= $v:expr)?)* $(,)?) => {
        $crate::i18n::Text::new($t, vec![$((stringify!($k), $crate::said!(@arg $k $(= $v)?))),*])
    };
}

/// Tidy text for error messages: remove control characters.
pub fn tidy(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// Escape control characters and bound diagnostic output.
pub fn for_trace(text: &str) -> String {
    text.chars().take(1024).flat_map(char::escape_debug).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn inserted_arguments_are_not_interpreted() {
        let msg = Text::new("{a} {b}", vec![("a", Arg::Plain("{b}".into())), ("b", Arg::Plain("secret".into()))]);
        assert_eq!(msg.to_string(), "{b} secret");
    }

    #[test]
    fn simple_message() {
        let msg = said!("the server closed the connection");
        assert_eq!(msg.to_string(), "the server closed the connection");
    }

    #[test]
    fn message_with_args() {
        let msg = said!("the server sent {n} bytes", n = 123);
        assert_eq!(msg.to_string(), "the server sent 123 bytes");
    }

    #[test]
    fn tidy_removes_control_chars() {
        assert_eq!(tidy("hello\x01world"), "helloworld");
        assert_eq!(tidy("hello\nworld"), "hello\nworld");
    }
}

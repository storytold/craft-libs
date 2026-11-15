// SPDX-FileCopyrightText: Craft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Error message handling for protocol errors.
//! 
//! Provides the `said!` macro and related types for creating user-facing error messages.

use std::fmt;

/// A value that can be converted to an argument for error messages.
pub trait ToArg {
    fn to_arg(&self) -> Arg;
}

impl<T: fmt::Display> ToArg for T {
    fn to_arg(&self) -> Arg {
        Arg::String(format!("{}", self))
    }
}

/// An argument for error messages.
pub enum Arg {
    String(String),
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::String(s) => f.write_str(s),
        }
    }
}

impl Arg {
    /// Create an argument from a string.
    pub fn said<T: fmt::Display>(t: T) -> Self {
        Arg::String(format!("{}", t))
    }
}

/// A text with values: `said!("no folder named {name}", name)`, or
/// `said!("{name} is larger than {mb} MB", name, mb = MAX >> 20)`.
#[derive(Clone)]
pub struct Text {
    template: String,
    args: Vec<(String, Arg)>,
}

impl Text {
    /// Create a new text with the given template and arguments.
    pub fn new(template: &str, args: Vec<(&str, Arg)>) -> Self {
        let mut filled = template.to_string();
        for (key, arg) in args.into_iter().rev() {
            filled = filled.replace(&format!("{{{key}}}"), &arg.to_string());
        }
        Text { template: filled, args: Vec::new() }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.template)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text { template: s.to_string(), args: Vec::new() }
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
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// For trace logging: format a command or response.
#[macro_export]
macro_rules! for_trace {
    ($($t:tt)*) => { log::trace!($($t)*) };
}

#[cfg(test)]
mod tests {
    use super::*;

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

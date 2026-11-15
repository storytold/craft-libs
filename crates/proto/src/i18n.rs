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

/// Trace only recognized protocol structure, never arbitrary reply text or command arguments.
/// Uppercase text can be private too; capitalization must not determine what is safe to log.
pub fn for_trace(line: &str) -> String {
    let line = line.trim_end();
    let mut words = line.split_ascii_whitespace();
    let first = words.next().unwrap_or_default();
    let known = |word: &str| {
        matches!(
            word,
            "OK" | "NO"
                | "BAD"
                | "BYE"
                | "PREAUTH"
                | "CAPABILITY"
                | "LOGIN"
                | "LOGOUT"
                | "AUTHENTICATE"
                | "SELECT"
                | "EXAMINE"
                | "CREATE"
                | "DELETE"
                | "RENAME"
                | "SUBSCRIBE"
                | "UNSUBSCRIBE"
                | "LIST"
                | "LSUB"
                | "STATUS"
                | "APPEND"
                | "CHECK"
                | "CLOSE"
                | "EXPUNGE"
                | "SEARCH"
                | "FETCH"
                | "STORE"
                | "COPY"
                | "MOVE"
                | "UID"
                | "IDLE"
                | "DONE"
                | "NOOP"
                | "EHLO"
                | "HELO"
                | "STARTTLS"
                | "AUTH"
                | "MAIL"
                | "RCPT"
                | "DATA"
                | "RSET"
                | "QUIT"
                | "EXISTS"
                | "RECENT"
                | "FLAGS"
        )
    };
    let smtp_code = first.get(..3).is_some_and(|code| code.bytes().all(|b| b.is_ascii_digit()))
        && first.as_bytes().first().is_some_and(|b| matches!(b, b'1'..=b'5'));
    let tag = first.strip_prefix('A').is_some_and(|digits| !digits.is_empty() && digits.len() <= 16 && digits.bytes().all(|b| b.is_ascii_digit()));
    let end = if smtp_code && (first.len() == 3 || first.get(3..4) == Some("-")) {
        if first.len() > 3 { 4 } else { 3 }
    } else if known(first) {
        first.len()
    } else if first == "*" || tag {
        let second = words.next().unwrap_or_default();
        let last = if first == "*" && !second.is_empty() && second.bytes().all(|b| b.is_ascii_digit()) {
            words.next().filter(|word| known(word))
        } else {
            known(second).then_some(second)
        };
        last.and_then(|word| line.find(word)?.checked_add(word.len())).unwrap_or(first.len())
    } else if first == "+" {
        1
    } else {
        0
    };
    let kept = line.get(..end).unwrap_or_default();
    let left = line.get(end..).unwrap_or_default().trim_start().len();
    match (kept.is_empty(), left) {
        (_, 0) => kept.to_owned(),
        (true, _) => format!("({left} bytes)"),
        (false, _) => format!("{kept} … ({left} bytes)"),
    }
}

/// Server text made fit for an error message: one line, no control characters, not too long.
pub fn tidy(text: &str) -> String {
    const MAX_CHARS: usize = 300;
    let mut out = String::new();
    let mut space = false;
    for (n, c) in text.trim().chars().enumerate() {
        if n >= MAX_CHARS {
            out.push('…');
            break;
        }
        if c.is_control() || c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    out
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
        assert_eq!(tidy("hello\x01world"), "hello world");
        assert_eq!(tidy("hello\nworld"), "hello world");
    }
}

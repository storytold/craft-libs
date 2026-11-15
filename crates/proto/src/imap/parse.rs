// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading IMAP responses: the pieces a response arrives in, the values in it, and status lines.

use crate::ProtoError;
use crate::i18n::said;

/// Deepest that parenthesised lists may nest.
const MAX_DEPTH: usize = 32;

/// A response as it came off the wire: lines of text with the literals between them.
#[derive(Debug)]
pub(crate) enum Piece {
    Text(Vec<u8>),
    Literal(Vec<u8>),
}

/// One value of a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    /// A bare word: a number, a flag, `NIL`, `BODY[]`.
    Atom(String),
    /// A quoted string or a literal.
    Str(Vec<u8>),
    List(Vec<Value>),
}

impl Value {
    pub(crate) fn atom(&self) -> Option<&str> {
        match self {
            Value::Atom(a) => Some(a),
            _ => None,
        }
    }

    pub(crate) fn number(&self) -> Option<u32> {
        self.atom()?.parse().ok()
    }

    /// The text of a string or a bare word (what the grammar calls an `astring`).
    pub(crate) fn into_text(self) -> Option<String> {
        match self {
            Value::Atom(a) => Some(a),
            Value::Str(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Value::List(_) => None,
        }
    }
}

/// Where a line ends in a literal marker (`{123}`, also `{123+}`) outside any quoted string: the
/// marker's position and the size it names.
pub(crate) fn literal_marker(line: &[u8]) -> Option<(usize, u64)> {
    let inner_end = line.len().checked_sub(1).filter(|last| line.get(*last) == Some(&b'}'))?;
    let open = line.iter().rposition(|b| *b == b'{')?;
    let inner = line.get(open + 1..inner_end)?;
    let digits = inner.strip_suffix(b"+").unwrap_or(inner);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    // A brace inside a quoted string is just text.
    let (mut quoted, mut escaped) = (false, false);
    for b in line.get(..open)? {
        if escaped {
            escaped = false;
        } else if quoted && *b == b'\\' {
            escaped = true;
        } else if *b == b'"' {
            quoted = !quoted;
        }
    }
    if quoted {
        return None;
    }
    // A size beyond u64 is as large as can be, and refused like any other that is too large.
    let size = digits.iter().try_fold(0u64, |n, d| n.checked_mul(10)?.checked_add(u64::from(d.wrapping_sub(b'0')))).unwrap_or(u64::MAX);
    Some((open, size))
}

/// The values of a response. Unbalanced parentheses are forgiven; lists nested too deeply are not.
pub(crate) fn values(pieces: Vec<Piece>) -> Result<Vec<Value>, ProtoError> {
    // The list being filled is on top; the response itself is at the bottom.
    let mut stack: Vec<Vec<Value>> = vec![Vec::new()];
    let mut pieces = pieces.into_iter().peekable();
    while let Some(piece) = pieces.next() {
        match piece {
            Piece::Literal(bytes) => push(&mut stack, Value::Str(bytes)),
            Piece::Text(text) => {
                // The marker of the literal that follows is not a value.
                let end = match (pieces.peek(), literal_marker(&text)) {
                    (Some(Piece::Literal(_)), Some((at, _))) => at,
                    _ => text.len(),
                };
                scan(text.get(..end).unwrap_or(&[]), &mut stack)?;
            }
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    Ok(stack.pop().unwrap_or_default())
}

fn push(stack: &mut [Vec<Value>], value: Value) {
    if let Some(top) = stack.last_mut() {
        top.push(value);
    }
}

fn close(stack: &mut Vec<Vec<Value>>) {
    if stack.len() > 1
        && let Some(list) = stack.pop()
    {
        push(stack, Value::List(list));
    }
}

fn scan(text: &[u8], stack: &mut Vec<Vec<Value>>) -> Result<(), ProtoError> {
    // A `[` only opens a section (`BODY[HEADER.FIELDS (TO)]`) when a `]` follows somewhere.
    let last_close = text.iter().rposition(|b| *b == b']');
    let mut i = 0;
    // Every arm moves `i` forward.
    while let Some(&b) = text.get(i) {
        match b {
            b' ' | b'\t' => i += 1,
            b'(' => {
                if stack.len() > MAX_DEPTH {
                    return Err(ProtoError::Protocol(said!("the server's response is nested too deeply")));
                }
                stack.push(Vec::new());
                i += 1;
            }
            b')' => {
                close(stack);
                i += 1;
            }
            b'"' => {
                let (string, next) = quoted(text, i + 1);
                push(stack, Value::Str(string));
                i = next;
            }
            _ => {
                let fetch_attribute = stack.len() == 2
                    && stack.first().and_then(|root| root.get(1)).and_then(Value::atom).is_some_and(|word| word.eq_ignore_ascii_case("FETCH"));
                let end = atom_end(text, i, if fetch_attribute { last_close } else { None });
                push(stack, Value::Atom(String::from_utf8_lossy(text.get(i..end).unwrap_or(&[])).into_owned()));
                i = end;
            }
        }
    }
    Ok(())
}

/// The contents of the quoted string that starts at `start` (after its opening quote), and where
/// the text goes on. A string the line ends in the middle of is taken as far as it goes.
fn quoted(text: &[u8], start: usize) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while let Some(&b) = text.get(i) {
        match b {
            b'"' => return (out, i + 1),
            b'\\' => {
                out.extend(text.get(i + 1));
                i += 2;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    (out, i)
}

/// Where the bare word starting at `start` ends: at a space or parenthesis outside `[…]`.
fn atom_end(text: &[u8], start: usize, last_close: Option<usize>) -> usize {
    let mut i = start;
    while let Some(&b) = text.get(i) {
        match b {
            b' ' | b'\t' | b'(' | b')' if i > start => break,
            b'[' if last_close.is_some_and(|close| close > i)
                && text.get(start..i).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"BODY") || prefix.eq_ignore_ascii_case(b"BODY.PEEK")) =>
            {
                let close = text.get(i..).and_then(|rest| rest.iter().position(|b| *b == b']')).unwrap_or(0);
                i += close + 1;
            }
            _ => i += 1,
        }
    }
    i
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Ok,
    No,
    Bad,
    Bye,
    Preauth,
}

impl Status {
    pub(crate) fn parse(word: &str) -> Option<Status> {
        match word.to_ascii_uppercase().as_str() {
            "OK" => Some(Status::Ok),
            "NO" => Some(Status::No),
            "BAD" => Some(Status::Bad),
            "BYE" => Some(Status::Bye),
            "PREAUTH" => Some(Status::Preauth),
            _ => None,
        }
    }
}

/// `OK [UIDVALIDITY 3857529045] UIDs valid`, tagged or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusLine {
    pub(crate) status: Status,
    /// The name of the response code, upper-cased (`UIDVALIDITY`); empty when there is none.
    pub(crate) code: String,
    /// What follows the name inside the brackets.
    pub(crate) arg: String,
    /// Everything after the status word, code included.
    pub(crate) text: String,
}

impl StatusLine {
    pub(crate) fn new(status: Status, rest: &str) -> StatusLine {
        let text = rest.trim();
        let inner = text.strip_prefix('[').and_then(|r| r.split_once(']')).map_or("", |(inner, _)| inner.trim());
        let (code, arg) = inner.split_once(' ').unwrap_or((inner, ""));
        StatusLine { status, code: code.to_ascii_uppercase(), arg: arg.trim().to_string(), text: text.to_string() }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn text(s: &str) -> Piece {
        Piece::Text(s.as_bytes().to_vec())
    }

    fn atom(s: &str) -> Value {
        Value::Atom(s.into())
    }

    fn string(s: &str) -> Value {
        Value::Str(s.as_bytes().to_vec())
    }

    fn parse(s: &str) -> Vec<Value> {
        values(vec![text(s)]).unwrap()
    }

    #[test]
    fn atoms_strings_and_lists() {
        assert_eq!(
            parse(r#"LIST (\HasNoChildren \Sent) "/" "Sent \"old\" \\ mail""#),
            vec![atom("LIST"), Value::List(vec![atom("\\HasNoChildren"), atom("\\Sent")]), string("/"), string("Sent \"old\" \\ mail")]
        );
        assert_eq!(
            parse("12 FETCH (UID 7 FLAGS ())"),
            vec![atom("12"), atom("FETCH"), Value::List(vec![atom("UID"), atom("7"), atom("FLAGS"), Value::List(vec![])])]
        );
        assert_eq!(
            parse("a ((b) (c (d))) NIL"),
            vec![atom("a"), Value::List(vec![Value::List(vec![atom("b")]), Value::List(vec![atom("c"), Value::List(vec![atom("d")])])]), atom("NIL")]
        );
        assert_eq!(parse(""), vec![]);
        assert_eq!(parse("   "), vec![]);
    }

    #[test]
    fn sections_are_part_of_the_word() {
        assert_eq!(
            parse("12 FETCH (BODY[HEADER.FIELDS (FROM TO)] x BODY[]<0> y)"),
            vec![atom("12"), atom("FETCH"), Value::List(vec![atom("BODY[HEADER.FIELDS (FROM TO)]"), atom("x"), atom("BODY[]<0>"), atom("y")])]
        );
        assert_eq!(parse(r#"LIST () "/" [Gmail]/Trash"#), vec![atom("LIST"), Value::List(vec![]), string("/"), atom("[Gmail]/Trash")]);
        // A bracket that never closes is an ordinary character.
        assert_eq!(parse("(a[b c)"), vec![Value::List(vec![atom("a[b"), atom("c")])]);
        assert_eq!(parse("[[[[ x"), vec![atom("[[[["), atom("x")]);
    }

    #[test]
    fn broken_text_is_read_as_far_as_it_goes() {
        assert_eq!(parse("(a (b"), vec![Value::List(vec![atom("a"), Value::List(vec![atom("b")])])]);
        assert_eq!(parse("a) b))"), vec![atom("a"), atom("b")]);
        assert_eq!(parse(r#""never closed"#), vec![string("never closed")]);
        assert_eq!(parse(r#""ends in a backslash\"#), vec![string("ends in a backslash")]);
        assert_eq!(parse("a\"b c"), vec![atom("a\"b"), atom("c")]);
        let deep = "(".repeat(MAX_DEPTH + 5);
        assert!(matches!(values(vec![text(&deep)]), Err(ProtoError::Protocol(_))));
        assert!(values(vec![text(&"(".repeat(MAX_DEPTH))]).is_ok());
    }

    #[test]
    fn literals_take_the_place_of_their_marker() {
        let pieces =
            vec![text("1 FETCH (BODY[] {5}"), Piece::Literal(b"a (\"b".to_vec()), text(" UID 3 X {0}"), Piece::Literal(Vec::new()), text(")")];
        assert_eq!(
            values(pieces).unwrap(),
            vec![atom("1"), atom("FETCH"), Value::List(vec![atom("BODY[]"), string("a (\"b"), atom("UID"), atom("3"), atom("X"), string("")])]
        );
        // Without a literal after it, a marker is only a word.
        assert_eq!(parse("a {5}"), vec![atom("a"), atom("{5}")]);
    }

    #[test]
    fn markers() {
        assert_eq!(literal_marker(b"* 1 FETCH (BODY[] {123}"), Some((18, 123)));
        assert_eq!(literal_marker(b"{0}"), Some((0, 0)));
        assert_eq!(literal_marker(b"x {7+}"), Some((2, 7)));
        assert_eq!(literal_marker(b"\"quoted\" {7}"), Some((9, 7)));
        assert_eq!(literal_marker(b"x {99999999999999999999999}"), Some((2, u64::MAX)));
        for not in ["", "}", "{}", "{+}", "{5} ", "{-5}", "{5 }", "\"in a string {5}", "\"a \\\" {5}", "{5}x", "{ 5}", "5}"] {
            assert_eq!(literal_marker(not.as_bytes()), None, "{not:?}");
        }
    }

    #[test]
    fn status_lines() {
        let s = StatusLine::new(Status::Ok, " [UIDVALIDITY 3857529045] UIDs valid ");
        assert_eq!((s.code.as_str(), s.arg.as_str(), s.text.as_str()), ("UIDVALIDITY", "3857529045", "[UIDVALIDITY 3857529045] UIDs valid"));
        let s = StatusLine::new(Status::No, "[alert] disk full");
        assert_eq!((s.code.as_str(), s.arg.as_str()), ("ALERT", ""));
        let s = StatusLine::new(Status::Ok, "done [not a code]");
        assert_eq!((s.code.as_str(), s.text.as_str()), ("", "done [not a code]"));
        assert_eq!(StatusLine::new(Status::Bad, "[unclosed").code, "");
        assert_eq!(StatusLine::new(Status::Bad, "").text, "");
        assert_eq!(Status::parse("preauth"), Some(Status::Preauth));
        assert_eq!(Status::parse("FETCH"), None);
    }

    #[test]
    fn values_as_numbers_and_text() {
        assert_eq!(atom("42").number(), Some(42));
        assert_eq!(atom("4294967296").number(), None);
        assert_eq!(atom("-1").number(), None);
        assert_eq!(string("42").number(), None);
        assert_eq!(string("Entw&APw-rfe").into_text().as_deref(), Some("Entw&APw-rfe"));
        assert_eq!(Value::Str(vec![0xFF, b'a']).into_text().as_deref(), Some("\u{FFFD}a"));
        assert_eq!(Value::List(vec![]).into_text(), None);
    }
}

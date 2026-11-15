//! Streaming helpers for the native JSON envelope.
//!
//! Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors.
//! Extracted from VectorCraft, licensed under MIT OR Apache-2.0.

use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, Write};

/// Indents JSON text written through it one level (two spaces after each newline), as a member
/// of the file's top-level object; newlines only occur between tokens, inside strings they are
/// escaped.
pub struct Indent<W: Write> {
    pub inner: W,
    pub on: bool,
}

impl<W: Write> Write for Indent<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if !self.on {
            return self.inner.write_all(buf);
        }
        let mut lines = buf.split(|&b| b == b'\n');
        self.inner.write_all(lines.next().unwrap_or_default())?;
        for line in lines {
            self.inner.write_all(b"\n  ")?;
            self.inner.write_all(line)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The member that names an image.
const TAG: &[u8] = b"\"key\":";

/// Passes JSON text through, collecting the string values of every `"key": "…"` member at any
/// depth (what the images a document uses are named by). Quotes inside JSON strings are escaped,
/// so every match is a real member.
pub struct KeyScan<W: Write> {
    pub inner: W,
    pub keys: HashSet<String>,
    /// Looking for keys (else it only passes the text on).
    pub active: bool,
    /// How much of [`TAG`] the last bytes matched.
    matched: usize,
    /// Right after a tag: skipping whitespace to its value.
    after_tag: bool,
    /// The value string so far, quotes included.
    value: Option<Vec<u8>>,
    escaped: bool,
}

impl<W: Write> KeyScan<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, keys: HashSet::new(), active: true, matched: 0, after_tag: false, value: None, escaped: false }
    }

    fn scan(&mut self, buf: &[u8]) {
        if !self.active {
            return;
        }
        let mut i = 0;
        while let Some(&b) = buf.get(i) {
            // Nothing in progress: skip to the next quote.
            if self.matched == 0 && !self.after_tag && self.value.is_none() {
                match buf.get(i..).and_then(|r| r.iter().position(|&c| c == b'"')) {
                    Some(at) => i += at,
                    None => return,
                }
                self.matched = 1;
                i += 1;
                continue;
            }
            self.step(b);
            i += 1;
        }
    }

    fn step(&mut self, b: u8) {
        if let Some(v) = self.value.as_mut() {
            v.push(b);
            if self.escaped {
                self.escaped = false;
            } else if b == b'\\' {
                self.escaped = true;
            } else if b == b'"'
                && let Some(v) = self.value.take()
                && let Ok(s) = serde_json::from_slice::<String>(&v)
            {
                self.keys.insert(s);
            }
        } else if self.after_tag && !b.is_ascii_whitespace() {
            self.after_tag = false;
            if b == b'"' {
                self.value = Some(vec![b]);
            }
        }
        if TAG.get(self.matched) == Some(&b) {
            self.matched += 1;
            if self.matched == TAG.len() {
                self.matched = 0;
                self.after_tag = true;
                self.value = None;
                self.escaped = false;
            }
        } else {
            self.matched = usize::from(b == b'"');
        }
    }
}

impl<W: Write> Write for KeyScan<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.scan(buf);
        self.inner.write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The file's top-level object, written member by member (the document is streamed in).
pub struct Envelope {
    pretty: bool,
    first: bool,
}

impl Envelope {
    /// Start an object with optional indentation.
    pub fn new(pretty: bool) -> Self {
        Self { pretty, first: true }
    }

    /// Serialize a complete member before writing it to the envelope.
    pub fn field(&mut self, out: &mut impl Write, key: &str, value: &impl Serialize) -> io::Result<()> {
        let json = if self.pretty { serde_json::to_vec_pretty(value) } else { serde_json::to_vec(value) }.map_err(io::Error::other)?;
        self.key(out, key)?;
        Indent { inner: out, on: self.pretty }.write_all(&json)
    }

    /// The start of member `key` (a plain name): its value follows, indented one level when
    /// pretty (as serde_json would: see [`Indent`]).
    pub fn key(&mut self, out: &mut impl Write, key: &str) -> io::Result<()> {
        let sep: &[u8] = if std::mem::take(&mut self.first) { b"{" } else { b"," };
        let indent: &[u8] = if self.pretty { b"\n  " } else { b"" };
        out.write_all(sep)?;
        out.write_all(indent)?;
        serde_json::to_writer(&mut *out, key).map_err(io::Error::other)?;
        out.write_all(if self.pretty { b": " } else { b":" })
    }

    /// Close the object.
    pub fn finish(self, out: &mut impl Write) -> io::Result<()> {
        if self.first {
            return out.write_all(b"{}");
        }
        out.write_all(if self.pretty { b"\n}" } else { b"}" })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// How the keys were found before saves were streamed: in the whole JSON text at once.
    fn string_members(json: &[u8]) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut rest = json;
        while let Some(at) = rest.windows(TAG.len()).position(|w| w == TAG) {
            rest = rest.get(at + TAG.len()..).unwrap_or_default();
            let value = rest.trim_ascii_start();
            if value.first() == Some(&b'"')
                && let Some(Ok(s)) = serde_json::Deserializer::from_slice(value).into_iter::<String>().next()
            {
                out.insert(s);
            }
        }
        out
    }

    /// The keys `KeyScan` finds in `json` written in pieces of `chunk` bytes.
    fn scanned(json: &[u8], chunk: usize) -> HashSet<String> {
        let mut scan = KeyScan::new(Vec::new());
        for piece in json.chunks(chunk.max(1)) {
            scan.write_all(piece).unwrap();
        }
        assert_eq!(scan.inner, json, "the text passes through");
        scan.keys
    }

    #[test]
    fn finds_the_keys_the_whole_text_search_found() {
        let v = serde_json::json!({"key": "a", "x": [{"key": "b \"q\" \\ é"}, {"monkey": "no"}, {"k": "key"}], "s": "\"key\": \"no\"", "key2": "n", "o": {"key": 3}});
        for json in [serde_json::to_vec(&v).unwrap(), serde_json::to_vec_pretty(&v).unwrap()] {
            let want = string_members(&json);
            assert_eq!(want, HashSet::from(["a".to_string(), "b \"q\" \\ é".to_string()]));
            for chunk in [1, 2, 3, 7, 64, json.len()] {
                assert_eq!(scanned(&json, chunk), want, "chunk {chunk}");
            }
        }
    }

    fn arb_json() -> impl Strategy<Value = serde_json::Value> {
        let key = prop::sample::select(vec!["key", "k", "kkey", "key ", "x"]).prop_map(str::to_string);
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<i32>().prop_map(serde_json::Value::from),
            "[a-z\"\\\\: ké\n]{0,8}".prop_map(serde_json::Value::from),
        ];
        leaf.prop_recursive(4, 32, 6, move |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..5).prop_map(serde_json::Value::from),
                prop::collection::vec((key.clone(), inner), 0..5).prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn scanning_in_pieces_finds_what_the_whole_text_search_found(v in arb_json(), chunk in 1usize..40, pretty in any::<bool>()) {
            let json = if pretty { serde_json::to_vec_pretty(&v) } else { serde_json::to_vec(&v) }.unwrap();
            prop_assert_eq!(scanned(&json, chunk), string_members(&json));
        }
    }
}

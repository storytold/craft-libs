//! "Import as copy": a free id for a theme whose id is taken, and the file with only its id
//! changed.

use craft_theme::{IdError, ThemeId};
use serde_json::Value;

/// The id an "import as copy" gives a theme whose id is taken: `<id>-<n>`, shortening `<id>` so
/// the result keeps the id length limit. The result is checked against the id rules again.
pub(crate) fn copy_id(id: &ThemeId, n: u32) -> Result<ThemeId, IdError> {
    const MAX_ID: usize = 64;
    let suffix = format!("-{n}");
    let keep = MAX_ID.saturating_sub(suffix.len()).min(id.as_str().len());
    // Ids are ASCII, so any byte index is a char boundary.
    let stem = id.as_str().get(..keep).unwrap_or_default();
    ThemeId::parse_user(&format!("{stem}{suffix}"))
}

/// `bytes` with every top-level `"id"` member's value replaced by `id`, everything else byte for
/// byte. `bytes` must be a JSON object that [`craft_theme::format::parse`] accepted.
pub(crate) fn replace_top_level_id(bytes: &[u8], id: &ThemeId) -> Option<Vec<u8>> {
    let mut scan = Scanner { bytes, pos: bytes.len() - strip_bom(bytes).len() };
    let mut spans = Vec::new();
    scan.ws();
    scan.expect(b'{')?;
    scan.ws();
    if scan.peek()? != b'}' {
        loop {
            let key_start = scan.pos;
            scan.string()?;
            let key: String = serde_json::from_slice(bytes.get(key_start..scan.pos)?).ok()?;
            scan.ws();
            scan.expect(b':')?;
            scan.ws();
            let value_start = scan.pos;
            scan.value()?;
            if key == "id" {
                spans.push((value_start, scan.pos));
            }
            scan.ws();
            match scan.next()? {
                b',' => scan.ws(),
                b'}' => break,
                _ => return None,
            }
        }
    }
    let replacement = json_string(id.as_str());
    let mut out = Vec::with_capacity(bytes.len() + replacement.len());
    let mut at = 0;
    for (start, end) in spans {
        out.extend_from_slice(bytes.get(at..start)?);
        out.extend_from_slice(replacement.as_bytes());
        at = end;
    }
    out.extend_from_slice(bytes.get(at..)?);
    Some(out)
}

/// Just enough of a JSON scanner to find value spans in text serde_json has already accepted.
struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, b: u8) -> Option<()> {
        (self.next()? == b).then_some(())
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn string(&mut self) -> Option<()> {
        self.expect(b'"')?;
        loop {
            match self.next()? {
                b'\\' => {
                    self.next()?;
                }
                b'"' => return Some(()),
                _ => {}
            }
        }
    }

    fn value(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.string(),
            b'{' | b'[' => self.container(),
            _ => {
                while self.peek().is_some_and(|b| !matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')) {
                    self.pos += 1;
                }
                Some(())
            }
        }
    }

    /// Skips an object or array; strings inside may contain brackets.
    fn container(&mut self) -> Option<()> {
        let mut depth = 0usize;
        loop {
            match self.peek()? {
                b'"' => self.string()?,
                b'{' | b'[' => {
                    depth += 1;
                    self.pos += 1;
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                _ => self.pos += 1,
            }
        }
    }
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

fn json_string(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

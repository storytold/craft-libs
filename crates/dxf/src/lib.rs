//! A standalone DXF tag reader and writer.
//!
//! DXF is a sequence of (group code, value) pairs. This crate parses ASCII and binary DXF into a
//! flat list of [`Tag`]s, splits them into sections and entities, and writes tags back. [`model`]
//! reads the tags as a drawing (header variables, tables, blocks, entities, objects); mapping
//! that onto an application's own document is left to the caller. Written from Autodesk's
//! published DXF Reference; depends on no other Craft crate. Moved from CADCraft's `cadcraft-dxf`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::io::Write as _;

pub mod model;
mod version;
pub use version::Version;

#[derive(Debug, thiserror::Error)]
pub enum DxfError {
    #[error("not a DXF file")]
    NotDxf,
    #[error("line {line}: bad group code `{text}`")]
    BadCode { line: usize, text: String },
    #[error("unexpected end of file")]
    Eof,
    /// More group codes than [`MAX_TAGS`]: `tags` is the count found (or the count read when the
    /// limit was reached), `limit` the maximum.
    #[error("drawing too large: {tags} DXF group codes, more than the {limit} CADCraft reads (set CADCRAFT_MAX_DXF_TAGS to read more)")]
    TooLarge { tags: usize, limit: usize },
    /// The memory for `tags` group codes couldn't be had: an error, not the process ending.
    #[error("not enough memory to read this drawing's {tags} DXF group codes")]
    OutOfMemory { tags: usize },
}

pub type Result<T> = std::result::Result<T, DxfError>;

/// A typed group value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Real(f64),
    Int(i64),
    Bool(bool),
    /// Binary chunk (310–319, 1004) as hex text.
    Hex(String),
}

impl Value {
    pub fn as_str(&self) -> String {
        match self {
            Value::Str(s) | Value::Hex(s) => s.clone(),
            Value::Real(r) => format_real(*r),
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => i64::from(*b).to_string(),
        }
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "DXF numeric accessors intentionally convert integer values to f64, accepting precision loss beyond 2^53."
    )]
    pub fn as_f64(&self) -> f64 {
        match self {
            Value::Real(r) if r.is_finite() => *r,
            Value::Int(i) => *i as f64,
            Value::Str(s) => s.trim().parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0),
            Value::Bool(b) => f64::from(u8::from(*b)),
            _ => 0.0,
        }
    }
    #[expect(clippy::cast_possible_truncation, reason = "DXF numeric accessors intentionally truncate finite real values to i64.")]
    pub fn as_i64(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Real(r) if r.is_finite() => *r as i64,
            Value::Str(s) => {
                s.trim().parse::<i64>().unwrap_or_else(|_| s.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map_or(0, |f| f as i64))
            }
            Value::Bool(b) => i64::from(*b),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub code: i32,
    pub value: Value,
}

impl Tag {
    pub fn new(code: i32, value: Value) -> Self {
        Tag { code, value }
    }
    pub fn s(code: i32, v: impl Into<String>) -> Self {
        Tag { code, value: Value::Str(v.into()) }
    }
    pub fn f(code: i32, v: f64) -> Self {
        Tag { code, value: Value::Real(if v.is_finite() { v } else { 0.0 }) }
    }
    pub fn i(code: i32, v: i64) -> Self {
        Tag { code, value: Value::Int(v) }
    }
    pub fn str(&self) -> String {
        self.value.as_str()
    }
    /// The value when it is text, without a copy.
    pub fn str_ref(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) | Value::Hex(s) => Some(s),
            _ => None,
        }
    }
    pub fn f64(&self) -> f64 {
        self.value.as_f64()
    }
    pub fn i64(&self) -> i64 {
        self.value.as_i64()
    }
}

/// The value type for a group code (DXF Reference, "Group Code Value Types").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Str,
    Real,
    Int16,
    Int32,
    Int64,
    Bool,
    Hex,
}

pub fn kind_of(code: i32) -> Kind {
    match code {
        10..=59 | 110..=149 | 210..=239 | 460..=469 | 1010..=1059 => Kind::Real,
        60..=79 | 170..=179 | 270..=289 | 370..=389 | 400..=409 | 1060..=1070 => Kind::Int16,
        90..=99 | 420..=429 | 440..=449 | 450..=459 | 1071 => Kind::Int32,
        160..=169 => Kind::Int64,
        290..=299 => Kind::Bool,
        310..=319 | 1004 => Kind::Hex,
        _ => Kind::Str,
    }
}

#[expect(clippy::cast_possible_truncation, reason = "DXF numeric parsing intentionally truncates finite real values to i64.")]
fn typed(code: i32, text: &str) -> Value {
    let t = text.trim();
    match kind_of(code) {
        Kind::Real => t.parse::<f64>().ok().filter(|v| v.is_finite()).map_or(Value::Real(0.0), Value::Real),
        Kind::Int16 | Kind::Int32 | Kind::Int64 => {
            // A float spelling ("1e3", "12.0") is accepted when finite, clamped to what the
            // group's width holds, signed or unsigned (files write 16-bit flags up to 65535, as
            // the integer spelling reads them); NaN, inf and text become 0.
            let (lo, hi) = match kind_of(code) {
                Kind::Int16 => (i64::from(i16::MIN), i64::from(u16::MAX)),
                Kind::Int32 => (i64::from(i32::MIN), i64::from(u32::MAX)),
                _ => (i64::MIN, i64::MAX),
            };
            t.parse::<i64>()
                .map_or_else(|_| Value::Int(t.parse::<f64>().ok().filter(|f| f.is_finite()).map_or(0, |f| (f as i64).clamp(lo, hi))), Value::Int)
        }
        Kind::Bool => Value::Bool(t.parse::<i64>().unwrap_or(0) != 0),
        Kind::Hex => Value::Hex(normalize_hex(t)),
        Kind::Str => Value::Str(text.trim_end_matches(['\r', '\n']).to_string()),
    }
}

const BINARY_SENTINEL: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\0";
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
/// Most group codes read from one file by default. A `Tag` takes 40 bytes plus its text, so this
/// bounds the tag list to about 4 GB plus text. 50 million rejected large DWGs that converted fine
/// (a 172 MB drawing has 56 million, issue #1948). The file-size limits bound memory too: 512 MB
/// for a DXF opened directly, 1 GB for a DWG's DXF rendition. Set `CADCRAFT_MAX_DXF_TAGS` to
/// raise or lower it.
pub const MAX_TAGS: usize = 100_000_000;

/// Environment variable overriding [`MAX_TAGS`] (a positive integer).
pub const MAX_TAGS_ENV: &str = "CADCRAFT_MAX_DXF_TAGS";

/// The group-code limit in effect: [`MAX_TAGS_ENV`] when set to a positive integer, else
/// [`MAX_TAGS`].
pub fn max_tags() -> usize {
    max_tags_from(std::env::var(MAX_TAGS_ENV).ok().as_deref())
}

fn max_tags_from(v: Option<&str>) -> usize {
    v.and_then(|s| s.trim().replace('_', "").parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(MAX_TAGS)
}

mod codepage;
pub use codepage::decode_code_page;

/// Parse a DXF file (ASCII or binary).
///
/// # Errors
///
/// Returns an error for empty input, an invalid group code, an incomplete group-code/value pair,
/// truncated binary data, or more than [`MAX_TAGS`] group codes. For ASCII input, invalid numeric
/// text becomes `0.0` for real values or `0` for integer values, and an EOF tag is not required.
pub fn parse(bytes: &[u8]) -> Result<Vec<Tag>> {
    parse_with_limit(bytes, max_tags())
}

/// [`parse`] for a file that may be cut short or damaged: an ASCII file is read up to the first
/// line that is no group code (or a last group code with no value), and what came before is
/// returned with the 1-based number of that line, so a caller can open what the file still holds
/// and say where it stops. `None` for a file read to its end. A truncated binary file, an empty
/// one, one damaged before its first group, and one over the limit are still errors.
///
/// # Errors
///
/// As [`parse`], except for damage after the first group of an ASCII file.
pub fn parse_recovering(bytes: &[u8]) -> Result<(Vec<Tag>, Option<usize>)> {
    if let Some(body) = bytes.strip_prefix(BINARY_SENTINEL) {
        return parse_binary_with_limit(body, max_tags()).map(|tags| (tags, None));
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let (tags, limit) = (count_ascii_pairs(bytes), max_tags());
    if tags > limit {
        return Err(DxfError::TooLarge { tags, limit });
    }
    ascii_tags(&codepage::decode_ascii_file(bytes), limit, tags, true)
}

/// The nonblank group-code/value pairs of an ASCII file (LF, CRLF and a lone CR each end a
/// line), up to EOF just as the parser stops there. Blank lines do not count.
fn count_ascii_pairs(bytes: &[u8]) -> usize {
    let mut lines = byte_lines(bytes);
    let mut tags = 0;
    while let Some(code) = lines.next() {
        let code = code.trim_ascii();
        if code.is_empty() {
            continue;
        }
        let Some(value) = lines.next() else { break };
        tags += 1;
        if code == b"0" && value.trim_ascii() == b"EOF" {
            break;
        }
    }
    tags
}

/// [`parse`] with an explicit group-code limit. For ASCII DXF the limit is checked up front from
/// the line count, before the text is decoded or any tag allocated, so an oversized file fails in
/// milliseconds instead of after it has filled memory.
///
/// # Errors
///
/// Returns an error for empty input, an invalid group code, an incomplete group-code/value pair,
/// truncated binary data, or more than `max_tags` group codes. For ASCII input, invalid numeric
/// text becomes `0.0` for real values or `0` for integer values, and an EOF tag is not required.
pub fn parse_with_limit(bytes: &[u8], max_tags: usize) -> Result<Vec<Tag>> {
    if let Some(body) = bytes.strip_prefix(BINARY_SENTINEL) {
        return parse_binary_with_limit(body, max_tags);
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    // Counted first, before the text is decoded or any tag allocated.
    let tags = count_ascii_pairs(bytes);
    if tags > max_tags {
        return Err(DxfError::TooLarge { tags, limit: max_tags });
    }
    parse_ascii_counted(&codepage::decode_ascii_file(bytes), max_tags, tags)
}

/// The four hex digits of a `\U+XXXX` escape starting at `chars[i]`, if there is one.
fn escape_at(chars: &[char], i: usize) -> Option<u32> {
    if chars.get(i) != Some(&'\\') || chars.get(i + 1) != Some(&'U') || chars.get(i + 2) != Some(&'+') {
        return None;
    }
    let hex = chars.get(i + 3..i + 7)?;
    if !hex.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    hex.iter().try_fold(0u32, |acc, c| Some(acc * 16 + c.to_digit(16)?))
}

/// The character of a `\M+nXXXX` escape starting at `chars[i]`, if there is one: `n` names a
/// double-byte code page and `XXXX` is the character's two bytes in hex.
pub fn mif_escape_at(chars: &[char], i: usize) -> Option<char> {
    if chars.get(i) != Some(&'\\') || chars.get(i + 1) != Some(&'M') || chars.get(i + 2) != Some(&'+') {
        return None;
    }
    let n = *chars.get(i + 3)?;
    let hex = chars.get(i + 4..i + 8)?;
    if !hex.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let code = hex.iter().try_fold(0u16, |acc, c| Some(acc * 16 + u16::try_from(c.to_digit(16)?).ok()?))?;
    codepage::mif_char(n, code.to_be_bytes())
}

/// Unescape the `\U+XXXX` sequences pre-R2007 DXF uses for characters outside the file's code
/// page (four hex digits, one UTF-16 code unit; characters above U+FFFF are a surrogate pair of
/// escapes), and the `\M+nXXXX` sequences for double-byte East Asian characters. Malformed
/// escapes, unpaired surrogates and unknown `\M+` code pages are kept as literal text.
fn unescape_unicode(s: &str) -> String {
    if !has_escape(s) {
        return s.to_string();
    }
    unescape_found(s)
}

/// [`unescape_unicode`] for a string the caller owns: without escapes (nearly always) it is
/// returned as it is, with no copy.
fn unescape_owned(s: String) -> String {
    if has_escape(&s) { unescape_found(&s) } else { s }
}

fn has_escape(s: &str) -> bool {
    s.contains("\\U+") || s.contains("\\M+")
}

/// The longest escape: a surrogate pair of two `\U+XXXX` (14 characters).
const MAX_ESCAPE_CHARS: usize = 14;

fn unescape_found(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    // Everything up to the next backslash is copied as it is; an escape is looked for in a window
    // of the next few characters only, so no copy of the whole string (4 bytes a character) is made.
    while let Some(at) = rest.find('\\') {
        let (plain, tail) = rest.split_at(at);
        out.push_str(plain);
        let mut buf = ['\0'; MAX_ESCAPE_CHARS];
        let mut n = 0;
        for (slot, c) in buf.iter_mut().zip(tail.chars()) {
            *slot = c;
            n += 1;
        }
        let window = buf.get(..n).unwrap_or_default();
        let (ch, used) = if let Some(ch) = mif_escape_at(window, 0) {
            (Some(ch), 8)
        } else if let Some(unit) = escape_at(window, 0) {
            if let Some(ch) = char::from_u32(unit) {
                (Some(ch), 7)
            } else if (0xD800..0xDC00).contains(&unit)
                && let Some(low) = escape_at(window, 7).filter(|u| (0xDC00..0xE000).contains(u))
            {
                (char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)), 14)
            } else {
                (None, 0)
            }
        } else {
            (None, 0)
        };
        match ch {
            Some(ch) => {
                out.push(ch);
                // Every escape character is ASCII, so `used` characters are `used` bytes.
                rest = tail.get(used..).unwrap_or_default();
            }
            None => {
                out.push('\\');
                rest = tail.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// The lines of an ASCII DXF text: LF, CRLF and a lone CR (old Mac files) all end a line, like
/// [`str::lines`] otherwise (no empty line after a final terminator).
fn ascii_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let line = rest.get(..end).unwrap_or_default();
        let after = rest.get(end..).unwrap_or_default();
        rest = after.strip_prefix("\r\n").or_else(|| after.strip_prefix(['\r', '\n'])).unwrap_or(after);
        Some(line)
    })
}

/// Byte-level twin of [`ascii_lines`] for the up-front limit check, so the count matches the parser.
fn byte_lines(mut rest: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.iter().position(|&b| b == b'\r' || b == b'\n').unwrap_or(rest.len());
        let line = &rest[..end];
        let after = &rest[end..];
        rest = match after {
            [b'\r', b'\n', tail @ ..] | [b'\r' | b'\n', tail @ ..] => tail,
            _ => after,
        };
        Some(line)
    })
}

/// Parse ASCII DXF text into typed tags.
///
/// # Errors
///
/// Returns an error for empty input, an invalid group code, an incomplete group-code/value pair,
/// or too many tags. Invalid numeric text becomes `0.0` for real values or `0` for integer values.
/// A complete tag stream does not require an EOF tag.
pub fn parse_ascii(text: &str) -> Result<Vec<Tag>> {
    parse_ascii_counted(text, max_tags(), 0)
}

/// [`parse_ascii`] of a file known to hold `expected` group codes. Their list is reserved first,
/// with an allocation that can fail: where the system refuses that much memory, opening the file
/// is an error instead of the end of the app. Growing the list instead could also briefly take
/// half as much again.
fn parse_ascii_counted(text: &str, max_tags: usize, expected: usize) -> Result<Vec<Tag>> {
    ascii_tags(text, max_tags, expected, false).map(|(tags, _)| tags)
}

/// The tags of ASCII DXF text. With `recover`, a line that is no group code, or a group code
/// with no value after it, ends the reading instead of failing it: the tags before it are
/// returned with the 1-based number of that line.
#[allow(clippy::while_let_loop)]
fn ascii_tags(text: &str, max_tags: usize, expected: usize, recover: bool) -> Result<(Vec<Tag>, Option<usize>)> {
    let mut tags = Vec::new();
    tags.try_reserve_exact(expected.min(max_tags)).map_err(|_| DxfError::OutOfMemory { tags: expected })?;
    let mut lines = ascii_lines(text.strip_prefix('\u{feff}').unwrap_or(text)).enumerate();
    let mut damaged = None;
    loop {
        let Some((ln, code_line)) = lines.next() else { break };
        let ct = code_line.trim();
        if ct.is_empty() {
            continue;
        }
        let Ok(code) = ct.parse::<i32>() else {
            if recover && !tags.is_empty() {
                damaged = Some(ln + 1);
                break;
            }
            return Err(DxfError::BadCode { line: ln + 1, text: ct.chars().take(40).collect() });
        };
        let Some((_, val)) = lines.next() else {
            if recover && !tags.is_empty() {
                damaged = Some(ln + 1);
                break;
            }
            return Err(DxfError::Eof);
        };
        let v = match typed(code, val) {
            // Only text with a backslash can hold an escape; the rest moves in as it is.
            Value::Str(s) => Value::Str(unescape_owned(s)),
            o => o,
        };
        let eof = code == 0 && matches!(&v, Value::Str(s) if s.trim_end() == "EOF");
        tags.push(Tag { code, value: v });
        if tags.len() > max_tags {
            return Err(DxfError::TooLarge { tags: tags.len(), limit: max_tags });
        }
        if eof {
            break;
        }
    }
    if tags.is_empty() {
        return Err(DxfError::NotDxf);
    }
    Ok((tags, damaged))
}

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.p.checked_add(n).ok_or(DxfError::Eof)?;
        let s = self.b.get(self.p..end).ok_or(DxfError::Eof)?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(*self.take(1)?.first().ok_or(DxfError::Eof)?)
    }
    fn i16(&mut self) -> Result<i16> {
        let s = self.take(2)?;
        Ok(i16::from_le_bytes([s[0], s[1]]))
    }
    fn i32(&mut self) -> Result<i32> {
        let s = self.take(4)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i64(&mut self) -> Result<i64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(i64::from_le_bytes(a))
    }
    fn f64(&mut self) -> Result<f64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(f64::from_le_bytes(a))
    }
    /// The bytes of a zero-terminated string (without the zero), decoded by the caller.
    fn cstr(&mut self) -> Result<&'a [u8]> {
        let rest = self.b.get(self.p..).unwrap_or(&[]);
        let len = rest.iter().position(|&b| b == 0).ok_or(DxfError::Eof)?;
        self.p += len + 1;
        Ok(rest.get(..len).unwrap_or(&[]))
    }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Binary DXF, the body after the sentinel: R13+ (2-byte group codes) or R12 and earlier (1-byte
/// group codes, where 255 is followed by a 2-byte code).
///
/// # Errors
///
/// Returns an error for an empty or malformed file, an unexpected end of file, or too many tags.
pub fn parse_binary(b: &[u8]) -> Result<Vec<Tag>> {
    parse_binary_with_limit(b, max_tags())
}

/// Whether a binary DXF body uses the pre-R13 layout with 1-byte group codes, told from its first
/// tag: R13+ starts with a 2-byte code (`00 00` for the usual `0 SECTION`, `E7 03` for a 999
/// comment), the old layout with a 1-byte `00` followed by the string's first character, or with
/// the 255 escape and a code that needs it (255 to 1071; an R13+ file whose first group is code 255
/// has the string's first character there instead).
fn legacy_binary_layout(b: &[u8]) -> bool {
    match b {
        [0, next, ..] => *next != 0,
        [255, lo, hi, ..] => (255..=1071).contains(&u16::from_le_bytes([*lo, *hi])),
        _ => false,
    }
}

fn parse_binary_with_limit(b: &[u8], max_tags: usize) -> Result<Vec<Tag>> {
    let mut text = codepage::BinaryText::default();
    let tags = parse_binary_pass(b, max_tags, &mut text)?;
    if !text.needs_code_page_pass() {
        return Ok(tags);
    }
    // The first pass's tags go before the second pass runs: both held at once would double
    // the peak near the tag limit.
    drop(tags);
    parse_binary_pass(b, max_tags, &mut codepage::BinaryText::code_page_pass())
}

fn parse_binary_pass(b: &[u8], max_tags: usize, text: &mut codepage::BinaryText) -> Result<Vec<Tag>> {
    let mut c = Cur { b, p: 0 };
    let mut tags = Vec::new();
    let legacy = legacy_binary_layout(b);
    while c.p < b.len() {
        // 255 escapes a 2-byte code only in the 1-byte layout; in R13+ it is an ordinary code.
        let code = if legacy {
            match c.u8()? {
                255 => i32::from(c.i16()?),
                x => i32::from(x),
            }
        } else {
            i32::from(c.i16()?)
        };
        let v = match kind_of(code) {
            Kind::Str => Value::Str(unescape_unicode(&text.string(code, c.cstr()?))),
            Kind::Real => Value::Real(Some(c.f64()?).filter(|v| v.is_finite()).unwrap_or(0.0)),
            Kind::Int16 => Value::Int(i64::from(c.i16()?)),
            Kind::Int32 => Value::Int(i64::from(c.i32()?)),
            Kind::Int64 => Value::Int(c.i64()?),
            Kind::Bool => Value::Bool(c.u8()? != 0),
            Kind::Hex => {
                let n = usize::from(c.u8()?);
                let bytes = c.take(n)?;
                let mut s = String::with_capacity(n * 2);
                for &x in bytes {
                    s.push(char::from(HEX_DIGITS[usize::from(x >> 4)]));
                    s.push(char::from(HEX_DIGITS[usize::from(x & 15)]));
                }
                Value::Hex(s)
            }
        };
        let eof = code == 0 && matches!(&v, Value::Str(s) if s.trim_end() == "EOF");
        tags.push(Tag { code, value: v });
        if tags.len() > max_tags {
            return Err(DxfError::TooLarge { tags: tags.len(), limit: max_tags });
        }
        if eof {
            break;
        }
    }
    if tags.is_empty() {
        return Err(DxfError::NotDxf);
    }
    Ok(tags)
}

/// Format a real the way DXF writers usually do (shortest round-trip, with a decimal point).
pub fn format_real(v: f64) -> String {
    let mut out = Vec::new();
    push_real(&mut out, v);
    // What `push_real` writes is ASCII.
    String::from_utf8(out).unwrap_or_default()
}

/// [`format_real`] appended to `out`, without a string of its own (a large drawing writes
/// millions of reals).
fn push_real(out: &mut Vec<u8>, v: f64) {
    if !v.is_finite() {
        out.extend_from_slice(b"0.0");
        return;
    }
    // Very large and very small values in exponent form, as AutoCAD writes them (`1.0E+20`):
    // plain digits would run to hundreds of characters.
    let a = v.abs();
    if a != 0.0 && !(1e-10..1e16).contains(&a) {
        let e = format!("{v:E}");
        let (m, x) = e.split_once('E').unwrap_or((&e, "0"));
        let x: i32 = x.parse().unwrap_or(0);
        let point = if m.contains('.') { "" } else { ".0" };
        let _ = write!(out, "{m}{point}E{}{:02}", if x < 0 { '-' } else { '+' }, x.abs());
        return;
    }
    let start = out.len();
    let _ = write!(out, "{v}");
    // A finite real in this range prints as an optional sign, digits and an optional point.
    if !out.get(start..).is_some_and(|s| s.contains(&b'.')) {
        out.extend_from_slice(b".0");
    }
}

/// A group code right-aligned to three characters and its line end, as `write!("{:>3}\r\n")`
/// writes it, without the formatting machinery for the codes DXF has (0 to 9999).
fn push_code(out: &mut Vec<u8>, code: i32) {
    match u16::try_from(code) {
        Ok(c) if c < 10_000 => {
            let digits = [c / 1000, c / 100 % 10, c / 10 % 10, c % 10].map(|d| b'0' + u8::try_from(d % 10).unwrap_or(0));
            // The significant digits (at least one), padded to three.
            let first = digits.iter().position(|&d| d != b'0').unwrap_or(3);
            for _ in 1..first {
                out.push(b' ');
            }
            out.extend_from_slice(digits.get(first..).unwrap_or_default());
            out.extend_from_slice(b"\r\n");
        }
        _ => {
            let _ = write!(out, "{code:>3}\r\n");
        }
    }
}

/// Most characters one string group holds: longer text is split over several groups (MTEXT
/// contents, field codes).
pub const MAX_STRING_CHARS: usize = 250;

/// `s` split into strings of at most [`MAX_STRING_CHARS`] characters, in order; none for an
/// empty string.
pub fn string_groups(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut n = 0;
    for c in s.chars() {
        if n == MAX_STRING_CHARS {
            out.push(std::mem::take(&mut cur));
            n = 0;
        }
        cur.push(c);
        n += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Write tags as ASCII DXF (CRLF line ends, codes right-aligned to 3 as is conventional).
///
/// The output is pure ASCII: non-ASCII characters in strings are written as `\U+XXXX` escapes,
/// the pre-R2007 convention, so it is correct whatever `$DWGCODEPAGE` the header declares.
pub fn write_ascii(tags: &[Tag]) -> String {
    write_ascii_as(tags, false)
}

/// Write tags as ASCII DXF with text in UTF-8 (`utf8`, the R2007+ convention: non-ASCII
/// characters as they are) or escaped as `\U+XXXX` (older versions, see [`write_ascii`]). Line
/// breaks inside strings become spaces either way.
pub fn write_ascii_as(tags: &[Tag], utf8: bool) -> String {
    let bytes = ascii(tags, if utf8 { Strings::Utf8 } else { Strings::Escaped(None) });
    // Both forms are UTF-8 (the escaped one is ASCII).
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Write tags as pre-R2007 ASCII DXF with text in the code page `code_page` names (a
/// `$DWGCODEPAGE` value such as `ANSI_1252`), as the DXF Reference describes for those versions:
/// a character the code page has is written as its bytes, any other as a `\U+XXXX` escape. A
/// code page this crate doesn't know, `ASCII`, and `UTF8`/`UNICODE` (not a pre-2007 code page)
/// give [`write_ascii`]'s pure ASCII output. So does a file whose code page bytes happen to be
/// valid UTF-8 (`СІ` in ANSI_1251 is D1 B2, U+0472), since readers, ours included, take an ASCII
/// DXF that is valid UTF-8 as UTF-8.
pub fn write_ascii_in(tags: &[Tag], code_page: &str) -> Vec<u8> {
    let bytes = write_ascii_in_page(tags, code_page);
    if !bytes.is_ascii() && std::str::from_utf8(&bytes).is_ok() {
        return ascii(tags, Strings::Escaped(None));
    }
    bytes
}

/// [`write_ascii_in`] without its fallback to pure ASCII: the text is in the code page even when
/// its bytes happen to be valid UTF-8. For a reader that decodes a pre-2007 DXF by the
/// `$DWGCODEPAGE` it declares and never guesses UTF-8, as the DWG bridge's acadrust does. A code
/// page this crate doesn't know, `ASCII`, and `UTF8`/`UNICODE` still give pure ASCII.
pub fn write_ascii_in_page(tags: &[Tag], code_page: &str) -> Vec<u8> {
    ascii(tags, Strings::Escaped(codepage::encoder(code_page)))
}

/// How a writer stores string values.
#[derive(Clone, Copy)]
enum Strings {
    /// As they are (R2007+).
    Utf8,
    /// ASCII as it is, other characters in this code page when it has them, otherwise as
    /// `\U+XXXX` escapes (before R2007).
    Escaped(Option<codepage::Decoder>),
}

fn ascii(tags: &[Tag], strings: Strings) -> Vec<u8> {
    let mut out = Vec::with_capacity(tags.len() * 16);
    for t in tags {
        push_code(&mut out, t.code);
        match &t.value {
            // Most text has no line breaks or backslashes and is ASCII (or goes out as UTF-8):
            // copied as it is. A code page may write an ASCII character another way, so text
            // for one takes the long path.
            Value::Str(s)
                if !s.contains(['\r', '\n', '\\'])
                    && (matches!(strings, Strings::Utf8) || (matches!(strings, Strings::Escaped(None)) && s.is_ascii())) =>
            {
                out.extend_from_slice(s.as_bytes());
            }
            Value::Str(s) => push_string(s, strings, &mut out),
            Value::Real(r) => push_real(&mut out, *r),
            Value::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Value::Bool(b) => out.push(if *b { b'1' } else { b'0' }),
            Value::Hex(h) => {
                // The DXF Reference limits a binary chunk line to 254 hex digits (127 bytes);
                // longer data continues in further groups of the same code. Only hex digits are
                // written, so a hostile value can't inject tags.
                let digits = normalize_hex(h);
                let mut chunks: Vec<&str> = digits.as_bytes().chunks(254).map(|c| std::str::from_utf8(c).unwrap_or_default()).collect();
                if chunks.is_empty() {
                    chunks.push("");
                }
                for (n, chunk) in chunks.into_iter().enumerate() {
                    if n > 0 {
                        let _ = write!(out, "\r\n{:>3}\r\n", t.code);
                    }
                    out.extend_from_slice(chunk.as_bytes());
                }
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Write tags as binary DXF (DXF Reference, "Binary DXF Files", R13+ layout): the sentinel,
/// then per tag a 2-byte little-endian group code and the value in the binary form of the code's
/// [`Kind`]: a zero-terminated string, an 8-byte real, a 2-, 4- or 8-byte integer, a 1-byte
/// boolean, or a binary chunk (one length byte, then up to 255 bytes; longer data is split into
/// several groups of the same code). Strings are UTF-8 when `utf8` (R2007+), otherwise escaped as
/// in [`write_ascii`]. A tag's value is converted to its code's kind, so a number written as text
/// is stored as a number; a code outside the 16-bit range is skipped.
pub fn write_binary(tags: &[Tag], utf8: bool) -> Vec<u8> {
    binary(tags, if utf8 { Strings::Utf8 } else { Strings::Escaped(None) })
}

/// Write tags as pre-R2007 binary DXF with text in the code page `code_page` names, as
/// [`write_ascii_in`] does.
pub fn write_binary_in(tags: &[Tag], code_page: &str) -> Vec<u8> {
    binary(tags, Strings::Escaped(codepage::encoder(code_page)))
}

fn binary(tags: &[Tag], strings: Strings) -> Vec<u8> {
    let mut out = Vec::with_capacity(BINARY_SENTINEL.len() + tags.len() * 12);
    out.extend_from_slice(BINARY_SENTINEL);
    for t in tags {
        let Ok(code) = i16::try_from(t.code) else { continue };
        let code_bytes = code.to_le_bytes();
        match kind_of(t.code) {
            Kind::Str => {
                out.extend_from_slice(&code_bytes);
                let s = match &t.value {
                    Value::Str(s) => s.clone(),
                    v => v.as_str(),
                };
                let mut text = Vec::with_capacity(s.len());
                push_string(&s, strings, &mut text);
                out.extend(text.into_iter().filter(|&b| b != 0));
                out.push(0);
            }
            Kind::Real => {
                out.extend_from_slice(&code_bytes);
                out.extend_from_slice(&t.value.as_f64().to_le_bytes());
            }
            Kind::Int16 => {
                out.extend_from_slice(&code_bytes);
                out.extend_from_slice(&wrap_i16(t.value.as_i64()));
            }
            Kind::Int32 => {
                out.extend_from_slice(&code_bytes);
                out.extend_from_slice(&wrap_i32(t.value.as_i64()));
            }
            Kind::Int64 => {
                out.extend_from_slice(&code_bytes);
                out.extend_from_slice(&t.value.as_i64().to_le_bytes());
            }
            Kind::Bool => {
                out.extend_from_slice(&code_bytes);
                out.push(u8::from(t.value.as_i64() != 0));
            }
            Kind::Hex => {
                let bytes = hex_bytes(&t.value.as_str());
                // An empty chunk is still one group, as in ASCII.
                let mut chunks: Vec<&[u8]> = bytes.chunks(255).collect();
                if chunks.is_empty() {
                    chunks.push(&[]);
                }
                for chunk in chunks {
                    out.extend_from_slice(&code_bytes);
                    out.push(u8::try_from(chunk.len()).unwrap_or(u8::MAX));
                    out.extend_from_slice(chunk);
                }
            }
        }
    }
    out
}

/// Hex text cleaned up: only the hex digits are kept, and an odd trailing digit is padded with
/// `0` (so the bytes before it are never shifted). Used by the reader and both writers.
fn normalize_hex(s: &str) -> String {
    let mut out: String = s.chars().filter(char::is_ascii_hexdigit).collect();
    if out.len() % 2 == 1 {
        out.push('0');
    }
    out
}

/// The bytes of hex text, two digits each (see [`normalize_hex`]).
fn hex_bytes(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = normalize_hex(s).bytes().filter_map(|b| char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok())).collect();
    digits.as_chunks::<2>().0.iter().map(|[hi, lo]| (hi << 4) | lo).collect()
}

/// A 16- or 32-bit integer for binary DXF: values that fit the unsigned width wrap into the
/// signed one (65535 is -1, as writers that store unsigned flags expect); anything else clamps.
fn wrap_i16(v: i64) -> [u8; 2] {
    match u16::try_from(v) {
        Ok(u) => u.to_le_bytes(),
        Err(_) => i16::try_from(v.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or_default().to_le_bytes(),
    }
}

fn wrap_i32(v: i64) -> [u8; 4] {
    match u32::try_from(v) {
        Ok(u) => u.to_le_bytes(),
        Err(_) => i32::try_from(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or_default().to_le_bytes(),
    }
}

/// Whether `bytes` are a binary DXF file (they start with the binary sentinel).
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(BINARY_SENTINEL)
}

/// A string value as `strings` stores it. Line breaks become spaces. Before R2007 the text is
/// in the `$DWGCODEPAGE` code page: ASCII stays as it is, a character the code page has becomes
/// its bytes, and every other one `\U+XXXX` (a surrogate pair of escapes above U+FFFF). Without
/// a code page the result is ASCII, so it is valid in any code page. In every form a backslash
/// that would start a `\U+` or `\M+` sequence is written as `\U+005C`, so the reader gives back
/// the literal text (#2050). So is a double-byte character whose trail byte is 0x5C (`\`) in that
/// place, such as Shift-JIS `表` or Big5 `許`: CADCraft decodes the code page before it looks for
/// escapes, but a reader that looks first would take that byte and the `U+` after it for one.
fn push_string(s: &str, strings: Strings, out: &mut Vec<u8>) {
    let chars: Vec<char> = s.chars().collect();
    let escape_at = |j: usize| matches!(chars.get(j), Some('U' | 'M')) && chars.get(j + 1) == Some(&'+');
    for (i, &c) in chars.iter().enumerate() {
        match (c, strings) {
            ('\r' | '\n', _) => out.push(b' '),
            ('\\', _) if escape_at(i + 1) => {
                out.extend_from_slice(b"\\U+005C");
            }
            (c, Strings::Utf8) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            (c, Strings::Escaped(Some(d))) if d.shadows_ascii(c) => {
                let _ = write!(out, "\\U+{:04X}", u32::from(c));
            }
            (c, _) if c.is_ascii() => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            (c, Strings::Escaped(Some(d))) if code_page_bytes(c, d, escape_at(i + 1), out) => {}
            (c, Strings::Escaped(_)) => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\U+{u:04X}");
                }
            }
        }
    }
}

/// `c` in the code page `d`, if it has it, unless it is a double-byte character whose trail byte
/// is 0x5C and an escape's `U+` or `M+` follows (`before_escape`): see [`push_string`].
fn code_page_bytes(c: char, d: codepage::Decoder, before_escape: bool, out: &mut Vec<u8>) -> bool {
    let start = out.len();
    if !codepage::encode_char(c, d, out) {
        return false;
    }
    if before_escape && out.len() - start > 1 && out.last() == Some(&b'\\') {
        out.truncate(start);
        return false;
    }
    true
}

/// A section: `0 SECTION / 2 NAME ... 0 ENDSEC`.
#[derive(Clone, Debug, Default)]
pub struct Section {
    pub name: String,
    pub tags: Vec<Tag>,
}

/// Split a tag stream into sections.
pub fn sections(tags: &[Tag]) -> Vec<Section> {
    section_slices(tags).into_iter().map(|s| Section { name: s.name.to_string(), tags: s.tags.to_vec() }).collect()
}

/// A section borrowed from the tag stream it was split from: [`Section`] without the copy.
#[derive(Clone, Copy, Debug)]
pub struct SectionRef<'a> {
    pub name: &'a str,
    pub tags: &'a [Tag],
}

/// [`sections`] without copying the tags: each section is a slice of `tags`. A large file's tags
/// take gigabytes (40 bytes a tag, 25 million tags in a 0.5 GB DXF), so a reader that holds the
/// stream while it reads the sections should take them this way.
pub fn section_slices(tags: &[Tag]) -> Vec<SectionRef<'_>> {
    let is = |t: &Tag, word: &str| t.code == 0 && t.str_ref().is_some_and(|s| s.trim_end() == word);
    let mut out = Vec::new();
    let mut i = 0;
    while i < tags.len() {
        if tags.get(i).is_some_and(|t| is(t, "SECTION")) {
            let name = tags.get(i + 1).filter(|t| t.code == 2).and_then(Tag::str_ref).unwrap_or_default().trim_end();
            let start = i + 2;
            let mut j = start;
            // A section cut short has no ENDSEC: the EOF is not one of its records.
            while j < tags.len() && !tags.get(j).is_some_and(|t| is(t, "ENDSEC") || is(t, "EOF")) {
                j += 1;
            }
            out.push(SectionRef { name, tags: tags.get(start..j).unwrap_or(&[]) });
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Split a run of tags into records starting at each code-0 tag: (type, tags after the 0).
pub fn records(tags: &[Tag]) -> Vec<(String, Vec<Tag>)> {
    record_slices(tags).into_iter().map(|(kind, tags)| (kind, tags.to_vec())).collect()
}

/// [`records`] without copying the tags: each record's tags are a slice of `tags`.
pub fn record_slices(tags: &[Tag]) -> Vec<(String, &[Tag])> {
    let starts: Vec<usize> = tags.iter().enumerate().filter(|(_, t)| t.code == 0).map(|(i, _)| i).collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &i)| {
            let end = starts.get(n + 1).copied().unwrap_or(tags.len());
            (tags.get(i).map(Tag::str).unwrap_or_default(), tags.get(i + 1..end).unwrap_or(&[]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writer's own code and real formatting gives the text `format!` gave.
    #[test]
    fn fast_code_and_real_formatting_match_format() {
        for code in [-1_000_000, -10, -1, 0, 1, 9, 10, 42, 99, 100, 330, 999, 1000, 1071, 9999, 10_000, 65_535, 1_000_000] {
            let mut s = Vec::new();
            push_code(&mut s, code);
            assert_eq!(String::from_utf8(s).unwrap(), format!("{code:>3}\r\n"), "{code}");
        }
        let old = |v: f64| {
            if !v.is_finite() {
                return "0.0".to_string();
            }
            let a = v.abs();
            if a != 0.0 && !(1e-10..1e16).contains(&a) {
                let e = format!("{v:E}");
                let (m, x) = e.split_once('E').unwrap_or((&e, "0"));
                let x: i32 = x.parse().unwrap_or(0);
                let m = if m.contains('.') { m.to_string() } else { format!("{m}.0") };
                return format!("{m}E{}{:02}", if x < 0 { '-' } else { '+' }, x.abs());
            }
            let s = format!("{v}");
            if s.contains('.') { s } else { format!("{s}.0") }
        };
        for v in [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.1,
            1.5e-7,
            123456789.0,
            1e15,
            1e16,
            1e21,
            1e300,
            -1e-300,
            1e-10,
            9e-11,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::NAN,
            f64::INFINITY,
        ] {
            assert_eq!(format_real(v), old(v), "{v}");
        }
        let tags = [
            Tag::f(10, 2.0),
            Tag::i(70, -3),
            Tag::new(290, Value::Bool(true)),
            Tag::s(1, "a\r\nb"),
            Tag::s(1, "é"),
            Tag::s(1, "plain"),
            Tag::s(1, "a\\U+0041"),
        ];
        assert_eq!(
            write_ascii_as(&tags, true),
            " 10\r\n2.0\r\n 70\r\n-3\r\n290\r\n1\r\n  1\r\na  b\r\n  1\r\né\r\n  1\r\nplain\r\n  1\r\na\\U+005CU+0041\r\n"
        );
        assert_eq!(write_ascii_as(&tags, false).lines().nth(9), Some("\\U+00E9"));
        // A code page that writes an ASCII character another way still does.
        let yen = [Tag::s(1, "a\\b~")];
        assert_eq!(write_ascii_in_page(&yen, "ANSI_932"), ascii_slow(&yen, "ANSI_932"));
    }

    /// What the writer gave before the fast path: every string through `push_string`.
    fn ascii_slow(tags: &[Tag], code_page: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for t in tags {
            let _ = write!(out, "{:>3}\r\n", t.code);
            push_string(&t.str(), Strings::Escaped(codepage::encoder(code_page)), &mut out);
            out.extend_from_slice(b"\r\n");
        }
        out
    }

    #[test]
    fn literal_escape_text_roundtrips() {
        for utf8 in [false, true] {
            for text in ["\\U+00E9", "\\M+183A4F", "a\\nb \\P", "\\\\U+00E9", "\\U+005C"] {
                let tags = vec![Tag::s(8, text)];
                let back = parse_ascii(&write_ascii_as(&tags, utf8)).unwrap();
                assert_eq!(back, tags, "utf8={utf8} {text}");
                let back = parse_binary(&write_binary(&tags, utf8)).unwrap();
                assert_eq!(back[1..], tags[..], "binary utf8={utf8} {text}");
            }
        }
    }

    #[test]
    fn blank_lines_do_not_consume_group_code_limit() {
        let tags = parse_with_limit(b"0\nEOF\n\n\n", 1).expect("one group code should fit a limit of one");
        assert_eq!(tags, vec![Tag::s(0, "EOF")]);
    }

    #[test]
    fn ascii_roundtrip() {
        let tags = vec![
            Tag::s(0, "SECTION"),
            Tag::s(2, "ENTITIES"),
            Tag::s(0, "LINE"),
            Tag::s(8, "0"),
            Tag::f(10, 1.5),
            Tag::f(20, 2.0),
            Tag::i(62, 1),
            Tag::s(0, "ENDSEC"),
            Tag::s(0, "EOF"),
        ];
        let text = write_ascii(&tags);
        let back = parse(text.as_bytes()).unwrap();
        assert_eq!(back, tags);
        let secs = sections(&back);
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].name, "ENTITIES");
        let recs = records(&secs[0].tags);
        assert_eq!(recs[0].0, "LINE");
        assert_eq!(recs[0].1.len(), 4);
    }

    #[test]
    fn binary_parse() {
        let mut b = BINARY_SENTINEL.to_vec();
        let push_str = |b: &mut Vec<u8>, code: i16, s: &str| {
            b.extend_from_slice(&code.to_le_bytes());
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        };
        push_str(&mut b, 0, "LINE");
        b.extend_from_slice(&10i16.to_le_bytes());
        b.extend_from_slice(&3.25f64.to_le_bytes());
        b.extend_from_slice(&62i16.to_le_bytes());
        b.extend_from_slice(&5i16.to_le_bytes());
        push_str(&mut b, 0, "EOF");
        let tags = parse(&b).unwrap();
        assert_eq!(tags[1], Tag::f(10, 3.25));
        assert_eq!(tags[2], Tag::i(62, 5));
    }

    /// A pre-R13 binary body: 1-byte group codes, 255 escaping a 2-byte code (1001 here).
    fn r12_binary() -> Vec<u8> {
        let mut b = BINARY_SENTINEL.to_vec();
        let push_str = |b: &mut Vec<u8>, code: u8, s: &str| {
            b.push(code);
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        };
        push_str(&mut b, 0, "SECTION");
        push_str(&mut b, 2, "ENTITIES");
        push_str(&mut b, 0, "LINE");
        push_str(&mut b, 8, "RAILROAD");
        b.push(10);
        b.extend_from_slice(&3.25f64.to_le_bytes());
        b.push(62);
        b.extend_from_slice(&5i16.to_le_bytes());
        b.push(255);
        b.extend_from_slice(&1001i16.to_le_bytes());
        b.extend_from_slice(b"APP\0");
        push_str(&mut b, 0, "ENDSEC");
        push_str(&mut b, 0, "EOF");
        b
    }

    #[test]
    fn r12_binary_parse() {
        let tags = parse(&r12_binary()).unwrap();
        assert_eq!(tags[0], Tag::s(0, "SECTION"));
        assert_eq!(tags[3], Tag::s(8, "RAILROAD"));
        assert_eq!(tags[4], Tag::f(10, 3.25));
        assert_eq!(tags[5], Tag::i(62, 5));
        assert_eq!(tags[6], Tag::s(1001, "APP"));
        assert_eq!(tags.last(), Some(&Tag::s(0, "EOF")));
    }

    #[test]
    fn r13_binary_code_255_is_ordinary() {
        let mut b = BINARY_SENTINEL.to_vec();
        for (code, s) in [(0i16, "LINE"), (255, "x"), (0, "EOF")] {
            b.extend_from_slice(&code.to_le_bytes());
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        }
        let tags = parse(&b).unwrap();
        assert_eq!(tags.get(1).map(|t| t.code), Some(255));
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn binary_layout_detection() {
        assert!(legacy_binary_layout(b"\0SECTION\0"));
        assert!(legacy_binary_layout(&[255, 0xE7, 0x03]));
        assert!(!legacy_binary_layout(b"\0\0SECTION\0"));
        assert!(!legacy_binary_layout(&[0xE7, 0x03, b'x', 0]));
        assert!(!legacy_binary_layout(b""));
        assert!(!legacy_binary_layout(&[0]));
        // R13+ code 255 with the string "x": not the escape
        assert!(!legacy_binary_layout(&[255, 0, b'x', 0]));
        assert!(!legacy_binary_layout(&[255, 0xE7]));
    }

    #[test]
    fn truncated_r12_binary_never_panics() {
        let b = r12_binary();
        for n in BINARY_SENTINEL.len()..b.len() {
            let _ = parse(b.get(..n).unwrap_or(&[]));
        }
        let mut cut = BINARY_SENTINEL.to_vec();
        cut.extend_from_slice(&[0, b'S', 0, 255, 0x10]);
        assert!(parse(&cut).is_err());
    }

    #[test]
    fn borrowed_sections_and_records_match_the_copies() {
        let text = "999\nstray\n  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1015\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n  5\n1A\n  0\nLINE\n  8\n0\n 10\n1.5\n  0\nCIRCLE\n 40\n2.0\n  0\nENDSEC\n  0\nEOF\n";
        let tags = parse(text.as_bytes()).unwrap();
        let (copies, slices) = (sections(&tags), section_slices(&tags));
        assert_eq!(slices.iter().map(|s| s.name).collect::<Vec<_>>(), ["HEADER", "ENTITIES"]);
        assert_eq!(copies.len(), slices.len());
        for (c, s) in copies.iter().zip(&slices) {
            assert_eq!(c.name, s.name);
            assert_eq!(c.tags.as_slice(), s.tags);
            let (rc, rs) = (records(&c.tags), record_slices(s.tags));
            assert_eq!(rc.len(), rs.len());
            for ((kc, tc), (ks, ts)) in rc.iter().zip(&rs) {
                assert_eq!((kc, tc.as_slice()), (ks, *ts));
            }
        }
        // Tags before the first record start none, as in `records`.
        assert_eq!(record_slices(slices[1].tags).iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["LINE", "CIRCLE"]);
        assert_eq!(slices[1].tags[0].str_ref(), Some("1A"));
        assert_eq!(Tag::f(10, 1.5).str_ref(), None);
    }

    #[test]
    fn huge_and_tiny_reals_in_exponent_form() {
        // Plain digits would run to hundreds of characters; the value reads back exactly.
        assert_eq!(format_real(1e20), "1.0E+20");
        assert_eq!(format_real(-2.5e-12), "-2.5E-12");
        assert_eq!(format_real(0.5), "0.5");
        assert_eq!(format_real(12.0), "12.0");
        for v in [f64::MAX, -f64::MAX, 1e308, f64::MIN_POSITIVE, 5e-324, 1.2345678901234567e16] {
            let s = format_real(v);
            assert!(s.len() < 32, "{s}");
            assert_eq!(s.parse::<f64>().ok(), Some(v), "{s}");
            // and through the writer and the reader
            let back = parse(write_ascii(&[Tag::f(10, v), Tag::s(0, "EOF")]).as_bytes()).unwrap();
            assert_eq!(back[0], Tag::f(10, v));
        }
    }

    #[test]
    fn long_strings_split_into_250_character_groups() {
        assert!(string_groups("").is_empty());
        assert_eq!(string_groups("abc"), ["abc"]);
        let s = "é".repeat(251);
        let g = string_groups(&s);
        assert_eq!(g.iter().map(|x| x.chars().count()).collect::<Vec<_>>(), [250, 1]);
        assert_eq!(g.concat(), s);
        assert_eq!(string_groups(&"x".repeat(500)).len(), 2);
    }

    #[test]
    fn malformed_inputs() {
        assert!(parse(b"").is_err());
        assert!(parse(b"abc\nxyz\n").is_err());
        assert!(parse(b"0\n").is_err());
        let mut b = BINARY_SENTINEL.to_vec();
        b.extend_from_slice(&10i16.to_le_bytes());
        b.extend_from_slice(&[1, 2, 3]);
        assert!(parse(&b).is_err());
        // Bad numbers become 0 rather than failing.
        let t = parse(b"10\nnotanumber\n0\nEOF\n").unwrap();
        assert!(t[0].f64().abs() < f64::EPSILON);
    }

    #[test]
    fn unicode_escapes_and_latin1() {
        let t = parse(b"1\nCaf\\U+00E9\n0\nEOF\n").unwrap();
        assert_eq!(t[0].str(), "Café");
        let t = parse(b"1\nCaf\xe9\n0\nEOF\n").unwrap();
        assert_eq!(t[0].str(), "Café");
    }

    #[test]
    fn surrogate_pair_escapes() {
        let t = parse(b"1\n\\U+D83D\\U+DE00 \\u+56fe\\U+53f7\n0\nEOF\n").unwrap();
        assert_eq!(t[0].str(), "😀 \\u+56fe号");
    }

    #[test]
    fn hostile_escapes_stay_literal() {
        for s in [
            "\\U+",
            "\\U",
            "\\",
            "a\\U+ZZZZ",
            "\\U+12",
            "\\U+D800",
            "\\U+DC00",
            "\\U+D800\\U+0041",
            "\\U+D83D\\U+DE",
            "\\U++123",
            "\\U+-123",
            "x\\U+12\u{e9}4",
        ] {
            let t = parse(format!("1\n{s}\n0\nEOF\n").as_bytes()).unwrap();
            let expect = if s == "\\U+D800\\U+0041" { "\\U+D800A" } else { s };
            assert_eq!(t[0].str(), expect, "{s:?}");
        }
    }

    #[test]
    fn writes_non_ascii_as_escapes() {
        let tags = vec![Tag::s(1, "图号 Ä°ø 😀\r\nend"), Tag::s(1000, "Ø"), Tag::s(0, "EOF")];
        let text = write_ascii(&tags);
        assert!(text.is_ascii());
        assert!(text.contains("\\U+56FE\\U+53F7 \\U+00C4\\U+00B0\\U+00F8 \\U+D83D\\U+DE00  end\r\n"));
        let back = parse(text.as_bytes()).unwrap();
        assert_eq!(back[0].str(), "图号 Ä°ø 😀  end");
        assert_eq!(back[1].str(), "Ø");
    }

    #[test]
    fn binary_strings_unescape() {
        let mut b = BINARY_SENTINEL.to_vec();
        b.extend_from_slice(&1i16.to_le_bytes());
        b.extend_from_slice(b"\\U+56FE\\U+53F7\0");
        b.extend_from_slice(&0i16.to_le_bytes());
        b.extend_from_slice(b"EOF\0");
        assert_eq!(parse(&b).unwrap()[0].str(), "图号");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn tag_limit_defaults_and_env_override() {
        assert_eq!(max_tags_from(None), MAX_TAGS);
        assert_eq!(max_tags_from(Some("")), MAX_TAGS);
        assert_eq!(max_tags_from(Some("0")), MAX_TAGS);
        assert_eq!(max_tags_from(Some("abc")), MAX_TAGS);
        assert_eq!(max_tags_from(Some(" 400_000_000 ")), 400_000_000);
        // Issue #1948: the default must admit a DXF rendition well above 50 million group codes.
        const { assert!(MAX_TAGS >= 100_000_000) };
    }

    #[test]
    fn the_tag_list_is_reserved_once_and_a_refusal_is_an_error() {
        let tags = parse_ascii_counted("0\nSECTION\n0\nENDSEC\n0\nEOF\n", MAX_TAGS, 3).unwrap();
        assert_eq!((tags.len(), tags.capacity()), (3, 3));
        let e = parse_ascii_counted("0\nEOF\n", usize::MAX, usize::MAX).unwrap_err();
        assert!(matches!(e, DxfError::OutOfMemory { .. }), "{e}");
    }
}

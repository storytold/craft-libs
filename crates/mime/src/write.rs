//! MIME entities to bytes: text and file parts, multiparts and their boundaries, and the
//! header values that need encoding.

use crate::header::{clean, is_field_name, write_field};
use crate::words::{WORD_BYTES, encode_b, is_plain};
use crate::{charset, transfer};

/// Longest physical line a body may have without a transfer encoding (RFC 5322 §2.1.1).
const MAX_LINE: usize = 998;

/// Longest piece of a file name parameter, so that its header line folds within 78 characters.
const NAME_PIECE: usize = 52;

/// Longest boundary prefix kept (a boundary is at most 70 characters, RFC 2046 §5.1.1).
const MAX_PREFIX: usize = 32;

/// Free text for a header field called `name` (`Subject`, `Comments`): as it is when it is
/// plain, else UTF-8 encoded-words, the first one short enough to fit on the line after the
/// name. Text that begins or ends with a space is encoded too, because readers trim header
/// values, and so is a word too long to fold. CR, LF and NUL are dropped.
pub fn encode_unstructured(name: &str, value: &str) -> String {
    let value = clean(value);
    if is_plain(&value) && !value.starts_with(' ') && !value.ends_with(' ') && !unfoldable(&value) {
        return value;
    }
    // Leave room for the name on the first line.
    let room = 76usize.saturating_sub(name.len().saturating_add(2 + 12)) / 4 * 3;
    encode_b(&value, if room < 9 { WORD_BYTES } else { room })
}

/// A value that is written as it stands when it is printable ASCII (and tabs), `=?` and all,
/// for a field whose readers don't decode encoded-words (a structured or a trace field kept
/// from a message read); anything else as [`encode_unstructured`] text, so that what is
/// written stays seven-bit.
pub fn seven_bit_value(name: &str, value: &str) -> String {
    let value = clean(value);
    let edge = |c: char| c == ' ' || c == '\t';
    if value.bytes().all(|b| b == b'\t' || (0x20..=0x7E).contains(&b)) && !value.starts_with(edge) && !value.ends_with(edge) && !unfoldable(&value) {
        return value;
    }
    encode_unstructured(name, &value)
}

/// Whether a word of `value` is too long for any line it could be folded onto.
fn unfoldable(value: &str) -> bool {
    value.split(' ').any(|word| word.len() > MAX_LINE - 100)
}

/// A message id or content id without anything that could end it early: no control characters,
/// no white space, no angle brackets. Written between `<` and `>`.
pub fn clean_id(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control() && !c.is_whitespace() && !matches!(c, '<' | '>')).collect()
}

/// A media type that is safe to write for a leaf part: `type/subtype` in lower case. Anything
/// malformed, and `multipart/*` (which would make readers look for parts), becomes
/// `application/octet-stream`.
pub fn safe_media_type(media_type: &str) -> String {
    let t = media_type.trim().to_ascii_lowercase();
    match t.split_once('/') {
        Some((kind, sub)) if is_type_token(kind) && is_type_token(sub) && kind != "multipart" => t,
        _ => "application/octet-stream".to_string(),
    }
}

/// Whether `s` can be a media type's type or subtype (RFC 6838 §4.2).
fn is_type_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
}

/// A quoted-string: only `"` and `\` need a backslash. CR, LF and NUL are dropped.
pub fn quoted(s: &str) -> String {
    format!("\"{}\"", clean(s).replace('\\', "\\\\").replace('"', "\\\""))
}

/// The `filename` parameters of `Content-Disposition`, each with its `; ` before it: a quoted
/// name when it is plain ASCII, else the RFC 2231 form (`filename*=utf-8''…`, in numbered pieces
/// when long) after an ASCII stand-in for readers that don't know it.
pub fn file_name_params(name: &str) -> String {
    let name = clean(name);
    let name = name.as_str();
    if is_plain(name) && name.len() <= 200 {
        return format!("; filename={}", quoted(name));
    }
    let stand_in: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).take(NAME_PIECE).collect();
    let mut out = format!("; filename=\"{stand_in}\"");
    let mut pieces: Vec<String> = vec![String::new()];
    for b in name.bytes() {
        let token = if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") };
        match pieces.last_mut() {
            Some(piece) if piece.len().saturating_add(token.len()) <= NAME_PIECE => piece.push_str(&token),
            _ => pieces.push(token),
        }
    }
    match pieces.as_slice() {
        [only] => out.push_str(&format!("; filename*=utf-8''{only}")),
        many => {
            for (i, piece) in many.iter().enumerate() {
                out.push_str(&format!("; filename*{i}*={}{piece}", if i == 0 { "utf-8''" } else { "" }));
            }
        }
    }
    out
}

/// A file to write as a part ([`Entity::file`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct File<'a> {
    /// Its media type; made safe with [`safe_media_type`].
    pub media_type: &'a str,
    /// Its name; no name parameters are written when it is empty. (A `text/plain` or
    /// `text/html` part without a name is read as body text by mail readers: give it one.)
    pub name: &'a str,
    /// `Content-Disposition: inline`, not `attachment`.
    pub inline: bool,
    /// Its `Content-ID`, without angle brackets; none is written when it is empty.
    pub content_id: &'a str,
    pub data: &'a [u8],
}

/// A MIME entity ready to be written: its content fields and its encoded body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entity {
    /// The fields that say what the body is (`Content-Type`, `Content-Transfer-Encoding`…),
    /// names and values. More can be pushed (`Content-Location`); every value is written
    /// through [`write_field`], which no value can break out of.
    pub fields: Vec<(String, String)>,
    /// The body as it goes on the wire, without a line end after it.
    pub content: Vec<u8>,
}

impl Entity {
    /// `text/<subtype>; charset=utf-8`: `7bit` when the text is ASCII in lines short enough,
    /// `quoted-printable` otherwise. Every line end is written as CRLF. A `subtype` that is no
    /// token becomes `plain`.
    pub fn text(subtype: &str, text: &str) -> Entity {
        let subtype = if is_type_token(subtype) { subtype.to_ascii_lowercase() } else { "plain".to_string() };
        let text = charset::unix_lines(text);
        let seven_bit = text.bytes().all(|b| matches!(b, b'\n' | b'\t' | 0x20..=0x7E)) && text.split('\n').all(|line| line.len() <= MAX_LINE);
        let (encoding, content) =
            if seven_bit { ("7bit", text.replace('\n', "\r\n").into_bytes()) } else { ("quoted-printable", transfer::qp_encode(text.as_bytes())) };
        Entity {
            fields: vec![("Content-Type".into(), format!("text/{subtype}; charset=utf-8")), ("Content-Transfer-Encoding".into(), encoding.into())],
            content,
        }
    }

    /// A file: base64 in 76-column lines, with its name in both `Content-Type` (`name`, as
    /// encoded-words, which is all old readers know) and `Content-Disposition` (`filename`,
    /// [`file_name_params`]). An attached message (`message/rfc822`) that is clean 7-bit text
    /// goes as it is, as RFC 2046 §5.2.1 asks.
    pub fn file(file: &File<'_>) -> Entity {
        let mime = safe_media_type(file.media_type);
        let name = clean(file.name);
        let mut content_type = mime.clone();
        let mut disposition = String::from(if file.inline { "inline" } else { "attachment" });
        if !name.is_empty() {
            content_type.push_str("; name=");
            // The first word is shorter, to fit on a line after ` name="`.
            content_type.push_str(&quoted(&if is_plain(&name) { name.clone() } else { encode_b(&name, 42) }));
            disposition.push_str(&file_name_params(&name));
        }
        let (encoding, content) = if mime == "message/rfc822" && is_seven_bit_message(file.data) {
            ("7bit", file.data.to_vec())
        } else {
            ("base64", transfer::base64_lines(file.data))
        };
        let mut fields = vec![
            ("Content-Type".to_string(), content_type),
            ("Content-Transfer-Encoding".to_string(), encoding.to_string()),
            ("Content-Disposition".to_string(), disposition),
        ];
        let content_id = clean_id(file.content_id);
        if !content_id.is_empty() {
            fields.push(("Content-ID".to_string(), format!("<{content_id}>")));
        }
        Entity { fields, content }
    }

    /// `multipart/<subtype>` holding `parts`, with a boundary from `boundaries` that occurs in
    /// none of them. `params` (`("type", "text/html")`) go between the type and the boundary,
    /// each value as a quoted-string; a parameter whose name is no token is left out, and a
    /// `subtype` that is no token becomes `mixed`.
    pub fn multipart(subtype: &str, params: &[(&str, &str)], parts: &[Entity], boundaries: &mut Boundaries) -> Entity {
        let subtype = if is_type_token(subtype) { subtype.to_ascii_lowercase() } else { "mixed".to_string() };
        let children: Vec<Vec<u8>> = parts.iter().map(Entity::to_bytes).collect();
        let boundary = boundaries.next(&children);
        let size = children
            .iter()
            .map(|c| c.len().saturating_add(boundary.len()).saturating_add(8))
            .fold(boundary.len().saturating_add(8), usize::saturating_add);
        let mut content = Vec::with_capacity(size);
        for child in &children {
            content.extend_from_slice(b"--");
            content.extend_from_slice(boundary.as_bytes());
            content.extend_from_slice(b"\r\n");
            content.extend_from_slice(child);
            content.extend_from_slice(b"\r\n");
        }
        content.extend_from_slice(b"--");
        content.extend_from_slice(boundary.as_bytes());
        content.extend_from_slice(b"--");
        let mut content_type = format!("multipart/{subtype};");
        for (name, value) in params.iter().filter(|(name, _)| is_type_token(name) && !name.eq_ignore_ascii_case("boundary")) {
            content_type.push_str(&format!(" {name}={};", quoted(value)));
        }
        content_type.push_str(&format!(" boundary=\"{boundary}\""));
        Entity { fields: vec![("Content-Type".to_string(), content_type)], content }
    }

    /// The entity as it stands inside a multipart: its fields, an empty line, its body.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.content.len().saturating_add(256));
        for (name, value) in &self.fields {
            write_field(&mut out, name, value);
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.content);
        out
    }
}

/// Write a whole message with CRLF line ends: `headers` in order, `MIME-Version: 1.0`, the
/// content fields of `body`, an empty line, and the body.
///
/// Every value goes through [`write_field`]: CR, LF and NUL are dropped and long lines are
/// folded, so no value can add a header field; a header whose name isn't a field name is left
/// out, and so are a `MIME-Version` and any `Content-…` field among `headers` (the body says
/// those). Values are otherwise written as given: the caller encodes text
/// ([`encode_unstructured`]), addresses ([`crate::format_address_list`]) and ids ([`clean_id`]).
pub fn write_message(headers: &[(&str, &str)], body: &Entity) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if is_field_name(name.as_bytes()) && lower != "mime-version" && !lower.starts_with("content-") {
            write_field(&mut out, name, value);
        }
    }
    write_field(&mut out, "MIME-Version", "1.0");
    out.extend_from_slice(&body.to_bytes());
    out.extend_from_slice(b"\r\n");
    out
}

/// Whether bytes can travel as they are: ASCII without NUL, CRLF line ends only, no long lines.
fn is_seven_bit_message(data: &[u8]) -> bool {
    let mut line = 0usize;
    let mut prev = 0u8;
    for (i, &b) in data.iter().enumerate() {
        match b {
            0 | 0x80..=0xFF => return false,
            b'\n' if prev != b'\r' => return false,
            b'\n' => line = 0,
            b'\r' if data.get(i.saturating_add(1)) != Some(&b'\n') => return false,
            b'\r' => {}
            _ => {
                line = line.saturating_add(1);
                if line > MAX_LINE {
                    return false;
                }
            }
        }
        prev = b;
    }
    true
}

/// Where the boundaries of one message's multiparts come from.
///
/// A boundary is a prefix, an FNV-1a hash of the parts and a counter (which sets the multiparts
/// of one message apart, and moves on when a part happens to hold the boundary), so the same
/// content always gets the same boundary and the same bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boundaries {
    prefix: String,
    counter: u32,
}

impl Default for Boundaries {
    fn default() -> Self {
        Boundaries::new("=_craft_")
    }
}

impl Boundaries {
    /// Boundaries that start with `prefix`, of which only the characters a boundary may hold
    /// without quoting trouble are kept (letters, digits, `=`, `_`, `-`, `.`), at most 32.
    pub fn new(prefix: &str) -> Self {
        let prefix = prefix.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '=' | '_' | '-' | '.')).take(MAX_PREFIX).collect();
        Boundaries { prefix, counter: 0 }
    }

    /// A boundary that occurs in none of `children`.
    fn next(&mut self, children: &[Vec<u8>]) -> String {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for child in children {
            for b in child {
                hash = (hash ^ u64::from(*b)).wrapping_mul(PRIME);
            }
            // The edge between two parts counts too.
            hash = (hash ^ 0x1ff).wrapping_mul(PRIME);
        }
        self.free(hash, children)
    }

    /// The first boundary for `hash`, counting up, that none of the parts holds.
    fn free(&mut self, hash: u64, children: &[Vec<u8>]) -> String {
        loop {
            let candidate = format!("{}{hash:016x}_{}", self.prefix, self.counter);
            self.counter = self.counter.wrapping_add(1);
            if !children.iter().any(|c| contains(c, candidate.as_bytes())) {
                return candidate;
            }
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    let Some((first, rest)) = needle.split_first() else { return true };
    let mut from = 0usize;
    while let Some(i) = haystack.get(from..).and_then(|h| h.iter().position(|b| b == first)) {
        let at = from.saturating_add(i).saturating_add(1);
        if haystack.get(at..at.saturating_add(rest.len())) == Some(rest) {
            return true;
        }
        from = at;
    }
    false
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;

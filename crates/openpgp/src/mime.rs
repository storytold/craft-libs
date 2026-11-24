//! Just enough MIME to find what OpenPGP signed, written from RFC 2045, RFC 2046 and RFC 3156:
//! a part's header fields and parameters, the parts of a multipart with their bytes exactly as
//! they came, and the two transfer encodings that carry text. Everything works on borrowed bytes
//! of the original message and is linear in them.

/// How many header fields of one part are read; the rest are ignored.
const MAX_FIELDS: usize = 1_000;

/// A part's header fields, names in lower case, folded lines joined.
#[derive(Debug, Default)]
pub(crate) struct Headers {
    fields: Vec<(String, String)>,
}

impl Headers {
    /// The first field called `name` (lower case).
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// Every field called `name` (lower case), in order.
    pub(crate) fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.fields.iter().filter(move |(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The part's media type and its parameters (`text/plain` when it says none, RFC 2045 §5.2).
    pub(crate) fn content_type(&self) -> Params {
        let params = self.get("content-type").map(Params::parse).unwrap_or_default();
        if params.value.is_empty() { Params { value: "text/plain".into(), ..params } } else { params }
    }

    /// The part's transfer encoding, in lower case.
    pub(crate) fn encoding(&self) -> String {
        self.get("content-transfer-encoding").map(|e| e.trim().to_ascii_lowercase()).unwrap_or_default()
    }
}

/// Splits a part into its header fields and its body: the body starts after the first empty
/// line (or is empty when there is none).
pub(crate) fn split(part: &[u8]) -> (Headers, &[u8]) {
    let (head, body) = match blank_line(part) {
        Some((head_end, body_start)) => (part.get(..head_end).unwrap_or_default(), part.get(body_start..).unwrap_or_default()),
        None => (part, &[][..]),
    };
    (parse_fields(head), body)
}

/// Where the header of `part` ends and where its body starts.
fn blank_line(part: &[u8]) -> Option<(usize, usize)> {
    // A part that starts with an empty line has no header fields at all.
    if part.starts_with(b"\r\n") {
        return Some((0, 2));
    }
    if part.starts_with(b"\n") {
        return Some((0, 1));
    }
    let mut i = 0usize;
    while let Some(off) = part.get(i..).and_then(|rest| rest.iter().position(|b| *b == b'\n')) {
        let nl = i.saturating_add(off);
        let next = nl.saturating_add(1);
        match part.get(next..) {
            Some(rest) if rest.starts_with(b"\r\n") => return Some((next, next.saturating_add(2))),
            Some(rest) if rest.starts_with(b"\n") => return Some((next, next.saturating_add(1))),
            Some(_) => i = next,
            None => break,
        }
    }
    None
}

fn parse_fields(head: &[u8]) -> Headers {
    let mut fields: Vec<(String, String)> = Vec::new();
    for raw in head.split(|b| *b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        if matches!(line.first(), Some(b' ' | b'\t')) {
            if let Some((_, value)) = fields.last_mut() {
                value.push(' ');
                value.push_str(String::from_utf8_lossy(line).trim());
            }
            continue;
        }
        if fields.len() >= MAX_FIELDS {
            continue;
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else { continue };
        let name = String::from_utf8_lossy(line.get(..colon).unwrap_or_default()).trim().to_ascii_lowercase();
        let value = String::from_utf8_lossy(line.get(colon.saturating_add(1)..).unwrap_or_default()).trim().to_string();
        if !name.is_empty() {
            fields.push((name, value));
        }
    }
    Headers { fields }
}

/// A header value with `;` parameters: `multipart/signed; protocol="application/pgp-signature"`.
#[derive(Debug, Default, Clone)]
pub(crate) struct Params {
    /// The value before the first `;`, in lower case.
    pub(crate) value: String,
    params: Vec<(String, String)>,
}

impl Params {
    pub(crate) fn parse(text: &str) -> Params {
        let mut pieces = Pieces { text, at: 0 };
        let value = pieces.next().unwrap_or_default().trim().to_ascii_lowercase();
        let mut params = Vec::new();
        for piece in pieces {
            let Some((name, raw)) = piece.split_once('=') else { continue };
            let name = name.trim().to_ascii_lowercase();
            if !name.is_empty() && params.len() < MAX_FIELDS {
                params.push((name, unquote(raw.trim())));
            }
        }
        Params { value, params }
    }

    /// The parameter called `name` (lower case).
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// The `;`-separated pieces of a header value, a `;` inside quotes not counting.
struct Pieces<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Iterator for Pieces<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = self.text.get(self.at..)?;
        if rest.is_empty() && self.at > 0 {
            return None;
        }
        let (mut quoted, mut escaped) = (false, false);
        for (i, c) in rest.char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                ';' if !quoted => {
                    self.at = self.at.saturating_add(i).saturating_add(1);
                    return rest.get(..i);
                }
                _ => {}
            }
        }
        self.at = self.text.len().saturating_add(1);
        Some(rest)
    }
}

fn unquote(raw: &str) -> String {
    let Some(inner) = raw.strip_prefix('"') else { return raw.to_string() };
    let mut out = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            _ if escaped => {
                out.push(c);
                escaped = false;
            }
            '\\' => escaped = true,
            '"' => break,
            _ => out.push(c),
        }
    }
    out
}

/// The parts of the multipart `body` with `boundary` (RFC 2046 §5.1.1), each exactly as it came:
/// from after its delimiter line to before the line break that comes ahead of the next one,
/// which belongs to the delimiter. At most `max` parts; a part after the last delimiter that
/// isn't closed still counts, as mail programs read it.
pub(crate) fn parts<'a>(body: &'a [u8], boundary: &str, max: usize) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    if boundary.is_empty() {
        return out;
    }
    let mut dash = Vec::with_capacity(boundary.len().saturating_add(2));
    dash.extend_from_slice(b"--");
    dash.extend_from_slice(boundary.as_bytes());
    // The start of the current part, once its opening delimiter has been seen.
    let mut open: Option<usize> = None;
    let mut line_start = 0usize;
    while line_start <= body.len() {
        let rest = body.get(line_start..).unwrap_or_default();
        let line_len = rest.iter().position(|b| *b == b'\n');
        let next = line_len.map_or(body.len().saturating_add(1), |n| line_start.saturating_add(n).saturating_add(1));
        if let Some(after) = rest.strip_prefix(dash.as_slice()) {
            let line = after.get(..line_len.map_or(after.len(), |n| n.saturating_sub(dash.len()))).unwrap_or_default();
            let close = line.starts_with(b"--");
            let tail = if close { line.get(2..).unwrap_or_default() } else { line };
            if tail.iter().all(|b| matches!(b, b' ' | b'\t' | b'\r')) {
                if let Some(start) = open.take() {
                    // The line break before the delimiter is the delimiter's.
                    let mut end = line_start.saturating_sub(1);
                    if end > start && body.get(end.saturating_sub(1)) == Some(&b'\r') {
                        end = end.saturating_sub(1);
                    }
                    let end = end.max(start);
                    out.push(body.get(start..end).unwrap_or_default());
                    if out.len() >= max {
                        return out;
                    }
                }
                if close {
                    return out;
                }
                open = Some(next.min(body.len()));
            }
        }
        line_start = next;
    }
    if let Some(start) = open {
        out.push(body.get(start..).unwrap_or_default());
    }
    out.truncate(max);
    out
}

/// `bytes` with every line break as CR LF (RFC 3156 §5: what was signed is the part's
/// canonical form). A lone LF gets its CR; nothing else changes.
pub(crate) fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len().saturating_add(bytes.len() / 32));
    let mut previous = 0u8;
    for &b in bytes {
        if b == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        previous = b;
    }
    out
}

/// A leaf's body without its transfer encoding: base64 and quoted-printable are undone, every
/// other encoding is the bytes themselves.
pub(crate) fn decode(encoding: &str, body: &[u8]) -> Vec<u8> {
    match encoding {
        "base64" => base64(body),
        "quoted-printable" => quoted_printable(body),
        _ => body.to_vec(),
    }
}

/// Base64 (RFC 2045 §6.8, RFC 4648), tolerant: characters outside the alphabet are skipped and
/// decoding stops at the first `=`.
pub(crate) fn base64(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits = bits.saturating_add(6);
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    out
}

/// Quoted-printable (RFC 2045 §6.7): `=XX` is a byte, `=` at a line's end joins it to the next.
fn quoted_printable(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0usize;
    while let Some(&b) = text.get(i) {
        if b != b'=' {
            out.push(b);
            i = i.saturating_add(1);
            continue;
        }
        let hex = |o: usize| text.get(i.saturating_add(o)).and_then(|c| (*c as char).to_digit(16));
        if let (Some(h), Some(l)) = (hex(1), hex(2)) {
            out.push(((h << 4) | l) as u8);
            i = i.saturating_add(3);
            continue;
        }
        // A soft line break, with any white space the line ends in.
        let mut j = i.saturating_add(1);
        while matches!(text.get(j), Some(b' ' | b'\t')) {
            j = j.saturating_add(1);
        }
        match (text.get(j), text.get(j.saturating_add(1))) {
            (Some(b'\r'), Some(b'\n')) => i = j.saturating_add(2),
            (Some(b'\n'), _) => i = j.saturating_add(1),
            (None, _) => i = j,
            _ => {
                out.push(b'=');
                i = i.saturating_add(1);
            }
        }
    }
    out
}

/// The address of the one mailbox in a From field (or an address given as it is): what is in
/// its `<…>` outside quoted strings and comments, else the value without its comments, in
/// lower case. Empty when there is no one address in it.
pub(crate) fn address(value: &str) -> String {
    // RFC 5322 §3.4: a mailbox is `name <addr>` or a bare addr-spec. Quoted strings and comments
    // are read past, so that nothing in them is taken for the address; a field with more than
    // one mailbox (§3.6.2: a comma between them, or more than one `<…>`) has no one address.
    let (mut quoted, mut escaped, mut comment) = (false, false, 0usize);
    let (mut angle, mut angles, mut closed) = (false, 0usize, false);
    let (mut inside, mut outside) = (String::new(), String::new());
    for c in value.chars() {
        let text = comment == 0;
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted || comment > 0 => {
                escaped = true;
                continue;
            }
            '"' if comment == 0 => quoted = !quoted,
            _ if quoted => {}
            '(' => comment = comment.saturating_add(1),
            ')' => comment = comment.saturating_sub(1),
            _ if comment > 0 => {}
            '<' if angle => return String::new(),
            '<' => {
                angle = true;
                angles = angles.saturating_add(1);
                continue;
            }
            '>' if angle => {
                angle = false;
                closed = true;
                continue;
            }
            ',' if !angle => return String::new(),
            _ => {}
        }
        // What is in a comment, and the parentheses around it, is neither.
        if text && comment == 0 && c != ')' {
            if angle { inside.push(c) } else { outside.push(c) }
        }
    }
    let spec = match angles {
        0 => outside,
        1 if closed && !angle => inside,
        _ => return String::new(),
    };
    let spec = spec.trim();
    // A control or format character (a zero-width space, a direction mark) would make an
    // address that looks like another one: none is taken.
    let clean = spec.matches('@').count() == 1
        && !spec.chars().any(|c| c.is_whitespace() || c.is_control() || is_format(c) || matches!(c, '<' | '>' | '"' | '(' | ')'));
    if clean { spec.to_lowercase() } else { String::new() }
}

/// Whether `c` is one of Unicode's format characters (general category Cf, Unicode 17.0): the
/// soft hyphen, the Arabic number signs and end-of-ayah marks, the Syriac abbreviation mark, the
/// Arabic pound and piastre marks above, the Mongolian vowel separator, zero-width marks and
/// joiners, bidirectional marks, embeddings, overrides and isolates, the word joiner and
/// invisible operators, the deprecated shaping controls, the byte-order mark, interlinear
/// annotations, the Kaithi number signs, the Egyptian hieroglyph, shorthand and musical format
/// controls, and tags.
pub(crate) fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

//! A message as a tree of MIME parts (RFC 2045, RFC 2046).

use crate::header::{self, Headers, Params, parse_params};
use crate::{Limits, charset, transfer};

/// A message read by [`parse`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    /// The message itself: its header fields and its body, which is the tree of its parts.
    pub root: Part,
    /// Whether a [`Limits`] left something out: input past the size cap, header fields, parts
    /// past the count or the depth, a body past the decoded size.
    pub truncated: bool,
}

/// One MIME entity: a message, or a part of a multipart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    /// Its header fields, in order, as written (unfolded; encoded-words not decoded).
    pub headers: Headers,
    /// Its `Content-Type`, parsed; empty when it has none.
    pub content_type: Params,
    pub body: Body,
}

/// What a [`Part`] holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// The bytes of a leaf part, its `Content-Transfer-Encoding` undone. Also the raw body of a
    /// `multipart/*` that names no boundary, which can't be split.
    Data(Vec<u8>),
    /// The parts of a `multipart/*`, in order. The text before the first boundary and after
    /// the closing one is not kept.
    Parts(Vec<Part>),
}

impl Default for Body {
    fn default() -> Self {
        Body::Data(Vec::new())
    }
}

/// `type/subtype`, both there.
fn is_media_type(t: &str) -> bool {
    t.split_once('/').is_some_and(|(kind, sub)| !kind.is_empty() && !sub.is_empty() && !sub.contains('/'))
}

/// A `Content-ID` or `Content-Location` value without the angle brackets and space around it.
fn bare(value: &str) -> &str {
    value.trim().trim_start_matches('<').trim_end_matches('>').trim()
}

impl Part {
    /// The media type the part says it is (`text/html`, lower case), when it says one that has
    /// both a type and a subtype.
    pub fn declared_media_type(&self) -> Option<&str> {
        Some(self.content_type.value.as_str()).filter(|t| is_media_type(t))
    }

    /// The media type of the part. RFC 2045: a part that doesn't say what it is, is plain text.
    pub fn media_type(&self) -> &str {
        self.declared_media_type().unwrap_or("text/plain")
    }

    /// Whether the part is a `multipart/*`.
    pub fn is_multipart(&self) -> bool {
        self.media_type().starts_with("multipart/")
    }

    /// The `Content-Disposition`, parsed: its value is `attachment`, `inline`, or empty.
    pub fn disposition(&self) -> Params {
        self.headers.get("content-disposition").map(parse_params).unwrap_or_default()
    }

    /// The file name the part gives itself: the `filename` of its `Content-Disposition`, else
    /// the `name` of its `Content-Type`, RFC 2231 and encoded-words decoded. As the sender wrote
    /// it: whoever saves a file under it must not trust it as a path.
    pub fn file_name(&self) -> Option<String> {
        self.disposition().text("filename").or_else(|| self.content_type.text("name")).filter(|n| !n.is_empty())
    }

    /// The `Content-ID`, without angle brackets; `None` when there is none or it is empty.
    pub fn content_id(&self) -> Option<&str> {
        self.headers.get("content-id").map(bare).filter(|v| !v.is_empty())
    }

    /// The `Content-Location` (RFC 2557); `None` when there is none or it is empty.
    pub fn content_location(&self) -> Option<&str> {
        self.headers.get("content-location").map(bare).filter(|v| !v.is_empty())
    }

    /// The `Content-Transfer-Encoding`, lower case; empty when there is none.
    pub fn transfer_encoding(&self) -> String {
        self.headers.get("content-transfer-encoding").map(|e| e.trim().to_ascii_lowercase()).unwrap_or_default()
    }

    /// The decoded bytes of a leaf part; empty for a multipart that was split.
    pub fn data(&self) -> &[u8] {
        match &self.body {
            Body::Data(data) => data,
            Body::Parts(_) => &[],
        }
    }

    /// The parts of a multipart; none for a leaf.
    pub fn parts(&self) -> &[Part] {
        match &self.body {
            Body::Data(_) => &[],
            Body::Parts(parts) => parts,
        }
    }

    /// The bytes of a leaf part as text, in the `charset` its `Content-Type` names
    /// ([`charset::decode`]; guessed when it names none), without a byte order mark at its
    /// start. Line ends are as they came ([`charset::unix_lines`] makes them `\n`).
    pub fn text(&self) -> String {
        let text = charset::decode(self.data(), self.content_type.get("charset").unwrap_or(""));
        match text.strip_prefix('\u{feff}') {
            Some(rest) => rest.trim_start_matches('\u{feff}').to_string(),
            None => text,
        }
    }

    /// Every part that isn't a split multipart, this one included, in the order of the message.
    pub fn leaves(&self) -> Vec<&Part> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(part) = stack.pop() {
            match &part.body {
                Body::Data(_) => out.push(part),
                Body::Parts(parts) => stack.extend(parts.iter().rev()),
            }
        }
        out
    }
}

/// Parse an Internet message: an `.eml` file, an MHT web archive, what a server hands over.
///
/// Never fails: parts that can't be read are skipped, and garbage gives a message with no
/// header fields and the bytes as its body. A leading mbox `From ` line is skipped.
///
/// A `multipart/*` with a boundary is split into its parts, down to [`Limits::max_depth`]; one
/// without a boundary keeps its raw body as [`Body::Data`]. Every other part is a leaf, its
/// transfer encoding undone; a `message/rfc822` part is a leaf too (its bytes are a message for
/// another call).
pub fn parse(bytes: &[u8], limits: &Limits) -> Message {
    parse_at(bytes, limits, true)
}

/// [`parse`] for an entity that isn't the top of a file (the content of a signed or wrapped
/// part): a first line that starts with `From ` is not skipped.
pub fn parse_entity(bytes: &[u8], limits: &Limits) -> Message {
    parse_at(bytes, limits, false)
}

fn parse_at(bytes: &[u8], limits: &Limits, top: bool) -> Message {
    let mut walk =
        Walk { limits, max_depth: limits.max_depth.min(Limits::HARD_MAX_DEPTH), parts: 0, decoded: 0, truncated: bytes.len() > limits.max_input };
    let root = walk.part(bytes.get(..limits.max_input).unwrap_or(bytes), top, 0);
    Message { root, truncated: walk.truncated }
}

struct Walk<'l> {
    limits: &'l Limits,
    max_depth: usize,
    /// Parts read so far.
    parts: usize,
    /// Decoded bytes kept so far.
    decoded: usize,
    truncated: bool,
}

impl Walk<'_> {
    fn part(&mut self, bytes: &[u8], top: bool, depth: usize) -> Part {
        self.parts = self.parts.saturating_add(1);
        let (headers, body, cut) = header::split_counted(bytes, top, self.limits);
        self.truncated |= cut;
        let content_type = headers.get("content-type").map(parse_params).unwrap_or_default();
        let multipart = is_media_type(&content_type.value) && content_type.value.starts_with("multipart/");
        let boundary = content_type.get("boundary").filter(|b| multipart && !b.is_empty());
        let body = match boundary {
            Some(_) if depth >= self.max_depth => {
                self.truncated = true;
                Body::Parts(Vec::new())
            }
            Some(boundary) => {
                let room = self.limits.max_parts.saturating_sub(self.parts);
                // One more than there is room for, to know whether any were left out.
                let pieces = split_multipart(body, boundary, room.saturating_add(1));
                self.truncated |= pieces.len() > room;
                let mut parts = Vec::new();
                for piece in pieces {
                    if self.parts >= self.limits.max_parts {
                        self.truncated = true;
                        break;
                    }
                    parts.push(self.part(piece, false, depth.saturating_add(1)));
                }
                Body::Parts(parts)
            }
            // Nothing to split on: the raw body, so that a caller can show what is there.
            None if multipart => Body::Data(self.keep(body.to_vec())),
            None => {
                let encoding = headers.get("content-transfer-encoding").unwrap_or("");
                Body::Data(self.keep(transfer::decode(body, encoding)))
            }
        };
        Part { headers, content_type, body }
    }

    /// `data` when it fits in what is left of [`Limits::max_decoded`], else nothing.
    fn keep(&mut self, data: Vec<u8>) -> Vec<u8> {
        match self.decoded.checked_add(data.len()).filter(|total| *total <= self.limits.max_decoded) {
            Some(total) => {
                self.decoded = total;
                data
            }
            None => {
                self.truncated = true;
                Vec::new()
            }
        }
    }
}

/// The parts of a multipart body: what lies between `--boundary` lines, without the line break
/// that belongs to the delimiter. The text before the first delimiter and after the closing
/// `--boundary--` is ignored. A body that is cut short still gives its last part. At most `max`
/// parts are given.
///
/// For callers that need a part's bytes as they came (a signature is made over them); [`parse`]
/// gives the parts read.
pub fn split_multipart<'a>(body: &'a [u8], boundary: &str, max: usize) -> Vec<&'a [u8]> {
    let delimiter = [b"--", boundary.as_bytes()].concat();
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    let mut pos = 0;
    while pos < body.len() {
        let (line, next) = header::line_at(body, pos);
        if let Some(rest) = line.strip_prefix(delimiter.as_slice()) {
            let (closing, tail) = match rest.strip_prefix(b"--") {
                Some(tail) => (true, tail),
                None => (false, rest),
            };
            if tail.iter().all(|b| matches!(b, b' ' | b'\t')) {
                if let Some(from) = start.take()
                    && parts.len() < max
                {
                    let piece = body.get(from..pos).unwrap_or_default();
                    parts.push(piece.strip_suffix(b"\r\n").or_else(|| piece.strip_suffix(b"\n")).unwrap_or(piece));
                }
                if closing || parts.len() >= max {
                    return parts;
                }
                start = Some(next);
            }
        }
        pos = next;
    }
    if let Some(from) = start
        && parts.len() < max
    {
        parts.push(body.get(from..).unwrap_or_default());
    }
    parts
}

#[cfg(test)]
#[path = "part_tests.rs"]
mod tests;

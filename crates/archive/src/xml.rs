//! XML nesting and DTD checks that run before a recursive parser does.
//!
//! Recursive XML parsers — roxmltree among them — consume stack per element, so deeply nested
//! input aborts the process instead of returning an error. [`check`] measures the nesting in one
//! iterative, linear pass over the raw bytes, so the parser is never handed a document it could
//! die on. The same pass finds the `<!DOCTYPE`, whose internal subset gives the parser entity
//! expansion and content-model recursion the application has to admit deliberately.
//!
//! The scan is deliberately pragmatic rather than a validating parser: comments, CDATA sections,
//! processing instructions, quoted attribute values and entity references cannot fake a tag, and
//! anything malformed simply ends the scan (the parser reports the malformed document afterwards).
//! It never recurses, never allocates and never panics.
//!
//! **The input must be UTF-8.** The scan reads raw bytes; a document in another encoding (a UTF-16
//! BOM is the tell) is refused with [`Error::UnsupportedEncoding`] rather than silently admitted.
//! Decode first and check the same bytes the downstream parser reads.
//!
//! Markup inside an entity declaration is *not* counted against the nesting limit, so an
//! application that admits an internal subset should bound its size ([`Dtd::InternalSubset`]) and
//! still set its parser's own node limit: expansion happens where the entity is referenced.

use std::fmt;

/// What to do about a document type declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtd {
    /// Any `<!DOCTYPE` is an error.
    Refuse,
    /// A `<!DOCTYPE` without an internal subset is allowed; one with a subset is an error.
    DeclarationOnly,
    /// A `<!DOCTYPE` with an internal subset is allowed while the subset stays within `max_bytes`
    /// bytes.
    InternalSubset {
        /// The most bytes the internal subset may occupy.
        max_bytes: u64,
    },
}

/// Application-owned limits for [`check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The deepest element nesting the document may have.
    ///
    /// A document nested exactly this deep is accepted; one level more is refused. The Craft apps
    /// set their numbers so that a document at the limit still leaves the parser room to run.
    pub max_depth: usize,
    /// The document type declaration policy.
    pub dtd: Dtd,
}

/// Why [`check`] refused a document, or why a [`Depth`] refused another level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Element nesting reached `depth`, one level past the limit.
    TooDeep {
        /// The nesting depth that was attempted.
        depth: usize,
        /// The caller's limit.
        limit: usize,
    },
    /// A document type declaration was present and the policy refuses any.
    DtdRefused,
    /// The declaration had an internal subset and the policy allows only a declaration.
    InternalSubsetNotAllowed,
    /// The internal subset was larger than the policy admits.
    InternalSubsetTooLarge {
        /// The size of the internal subset in bytes.
        actual: u64,
        /// The caller's limit.
        limit: u64,
    },
    /// The document is not UTF-8: the scan reads raw bytes, so it cannot check a document whose
    /// markup is not contiguous ASCII (a UTF-16 BOM is the tell). The caller must decode to
    /// UTF-8 first and check the decoded bytes — the same bytes the downstream parser reads.
    UnsupportedEncoding,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooDeep { depth, limit } => write!(f, "elements nested {depth} deep; the limit is {limit}"),
            Self::DtdRefused => f.write_str("a document type declaration is not allowed"),
            Self::InternalSubsetNotAllowed => f.write_str("a document type declaration with an internal subset is not allowed"),
            Self::InternalSubsetTooLarge { actual, limit } => write!(f, "the internal subset is {actual} bytes; the limit is {limit}"),
            Self::UnsupportedEncoding => f.write_str("the document is not UTF-8; decode it to UTF-8 before checking it"),
        }
    }
}

impl std::error::Error for Error {}

/// A nesting counter for parsers that count while they parse.
///
/// A reader calls [`enter`](Self::enter) for each element it opens and [`leave`](Self::leave) for
/// each it closes, so the limit applies to the same nesting [`check`] measures on the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depth {
    depth: usize,
    limit: usize,
}

impl Depth {
    /// A counter that admits at most `limit` elements open at once.
    pub const fn new(limit: usize) -> Self {
        Self { depth: 0, limit }
    }

    /// Open one element, refusing when that would exceed the limit.
    pub fn enter(&mut self) -> Result<(), Error> {
        if self.depth >= self.limit {
            return Err(Error::TooDeep { depth: self.depth.saturating_add(1), limit: self.limit });
        }
        self.depth = self.depth.saturating_add(1);
        Ok(())
    }

    /// Close one element. Closing more elements than were opened leaves the counter at zero.
    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// The elements open right now.
    pub const fn current(&self) -> usize {
        self.depth
    }

    /// The limit this counter was created with.
    pub const fn limit(&self) -> usize {
        self.limit
    }
}

/// Check element nesting and the document type declaration in one pass over `text`.
///
/// The nesting is counted from start tags that are not self-closing and from end tags; comments,
/// CDATA sections, processing instructions, other declarations and quoted attribute values are
/// skipped whole, so a `<` inside them cannot change the count. A document nested exactly
/// [`Limits::max_depth`] deep is accepted; one level more is [`Error::TooDeep`].
///
/// A `<!DOCTYPE` is handed to [`Limits::dtd`]: refused, refused when it has an internal subset, or
/// admitted while its internal subset stays within a byte cap. An unterminated construct ends the
/// scan; the parser that runs next reports the malformed document.
pub fn check(text: &[u8], limits: &Limits) -> Result<(), Error> {
    // The scan reads raw bytes and recognises only contiguous ASCII markup. A UTF-16 document
    // (BOM: FF FE / FE FF) interleaves zero bytes, so the scan would see no tags at all and
    // admit a DTD the policy refuses. Refuse rather than silently admit: the caller must
    // decode to UTF-8 and check those bytes. The UTF-8 BOM (EF BB BF) is ASCII-compatible and
    // checked normally.
    if text.starts_with(&[0xFF, 0xFE]) || text.starts_with(&[0xFE, 0xFF]) {
        return Err(Error::UnsupportedEncoding);
    }
    let mut depth = Depth::new(limits.max_depth);
    let mut at = 0usize;
    while let Some(&byte) = text.get(at) {
        if byte != b'<' {
            at = at.saturating_add(1);
            continue;
        }
        let rest = text.get(at..).unwrap_or_default();
        if rest.starts_with(b"<!--") {
            at = skip_to(text, at.saturating_add(4), b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            at = skip_to(text, at.saturating_add(9), b"]]>");
        } else if rest.starts_with(b"<?") {
            at = skip_to(text, at.saturating_add(2), b"?>");
        } else if opens_doctype(rest) {
            at = check_doctype(text, at, limits.dtd)?;
        } else if rest.starts_with(b"<!") {
            // Another declaration (a notation, element or attribute declaration outside a subset,
            // which is malformed anyway): skip it without counting markup.
            at = skip_to(text, at.saturating_add(2), b">");
        } else if rest.starts_with(b"</") {
            depth.leave();
            at = skip_to(text, at.saturating_add(2), b">");
        } else {
            let end = tag_end(text, at.saturating_add(1));
            // A self-closing tag opens and closes at once, so it is counted exactly as a parser
            // sees it: one level, immediately released. That keeps this scan and [`Depth`] in
            // agreement, including on an empty element at the deepest point.
            depth.enter()?;
            if is_self_closing(text, at.saturating_add(1), end) {
                depth.leave();
            }
            at = end;
        }
    }
    Ok(())
}

/// Whether the markup starting at `rest` opens a document type declaration.
///
/// The `<!` prefix is required, not implied: an element whose *name* contains the keyword
/// (`<aDOCTYPE>`) is an element, and treating it as a declaration would both bypass the depth
/// cap under `DeclarationOnly` and falsely refuse the document under `Refuse`. Case is
/// ignored so that a lower-case declaration cannot slip past the policy into a parser that
/// recognises it.
fn opens_doctype(rest: &[u8]) -> bool {
    const KEYWORD: &[u8] = b"DOCTYPE";
    if !rest.starts_with(b"<!") {
        return false;
    }
    let Some(after_bang) = rest.get(2..) else { return false };
    let Some(keyword) = after_bang.get(..KEYWORD.len()) else { return false };
    keyword.eq_ignore_ascii_case(KEYWORD)
}

/// Apply the DTD policy to the declaration starting at `at`; return the offset just past it.
///
/// The declaration ends at the first `>` outside a quoted value, outside a comment and outside the
/// internal subset, since a `>` or `]` inside a quoted entity value must not end it. An
/// unterminated declaration simply runs to the end of the document.
fn check_doctype(text: &[u8], at: usize, dtd: Dtd) -> Result<usize, Error> {
    let mut quote: Option<u8> = None;
    let mut brackets = 0usize;
    let mut subset_start = None;
    let mut subset_end = None;
    let mut closed = None;
    let mut index = at.saturating_add(2);
    while let Some(&byte) = text.get(index) {
        if quote.is_none() && text.get(index..).is_some_and(|rest| rest.starts_with(b"<!--")) {
            index = skip_to(text, index.saturating_add(4), b"-->");
            continue;
        }
        // A processing instruction is legal inside an internal subset, so its bytes are part
        // of the subset: skip it atomically through `?>` so a `]` or `>` inside it cannot end
        // the apparent subset or declaration early, leaving subset bytes uncharged.
        if quote.is_none() && text.get(index..).is_some_and(|rest| rest.starts_with(b"<?")) {
            index = skip_to(text, index.saturating_add(2), b"?>");
            continue;
        }
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'[' => {
                    if brackets == 0 {
                        subset_start = Some(index.saturating_add(1));
                    }
                    brackets = brackets.saturating_add(1);
                }
                b']' if brackets > 0 => {
                    brackets = brackets.saturating_sub(1);
                    if brackets == 0 {
                        subset_end = Some(index);
                    }
                }
                b'>' if brackets == 0 => {
                    closed = Some(index.saturating_add(1));
                    break;
                }
                _ => {}
            },
        }
        index = index.saturating_add(1);
    }

    let declaration_end = closed.unwrap_or(text.len());
    if let Some(start) = subset_start {
        let end = subset_end.unwrap_or(declaration_end);
        let actual = u64::try_from(end.saturating_sub(start)).unwrap_or(u64::MAX);
        match dtd {
            Dtd::Refuse => return Err(Error::DtdRefused),
            Dtd::DeclarationOnly => return Err(Error::InternalSubsetNotAllowed),
            Dtd::InternalSubset { max_bytes } if actual > max_bytes => {
                return Err(Error::InternalSubsetTooLarge { actual, limit: max_bytes });
            }
            Dtd::InternalSubset { .. } => {}
        }
    } else if dtd == Dtd::Refuse {
        return Err(Error::DtdRefused);
    }
    Ok(declaration_end)
}

/// Index just past `needle` from `from`, or the end of `text` when it is not found.
fn skip_to(text: &[u8], from: usize, needle: &[u8]) -> usize {
    let mut index = from;
    while index < text.len() {
        if text.get(index..).is_some_and(|rest| rest.starts_with(needle)) {
            return index.saturating_add(needle.len());
        }
        index = index.saturating_add(1);
    }
    text.len()
}

/// Index just past the `>` that ends the tag whose name starts at `from`, or the end of `text`.
///
/// A `>` inside a quoted attribute value does not end the tag.
fn tag_end(text: &[u8], from: usize) -> usize {
    let mut quote: Option<u8> = None;
    let mut index = from;
    while let Some(&byte) = text.get(index) {
        index = index.saturating_add(1);
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => return index,
                _ => {}
            },
        }
    }
    text.len()
}

/// Whether the tag that ends at `end` (just past its `>`) closes itself with `/>`.
fn is_self_closing(text: &[u8], from: usize, end: usize) -> bool {
    let mut index = end.saturating_sub(1);
    while index > from {
        match text.get(index.saturating_sub(1)) {
            Some(&byte) if byte.is_ascii_whitespace() => index = index.saturating_sub(1),
            Some(&b'/') => return true,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests;

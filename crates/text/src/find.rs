//! Find and replace (N3.03): the matcher behind a find bar or a Find and Replace dialog, and the
//! matches of a search across a paged document.
//!
//! - [`FindPattern`] reads the Find what text with [`FindOptions`]: Acrobat's two options (Match
//!   case, Whole words only), Word's search options (Match prefix, Match suffix, Ignore
//!   punctuation, Ignore white-space), Word's `^` codes (`^p` paragraph mark, `^t` tab, `^#` any
//!   digit, `^?` any character, …), Word's wildcards (`?`, `*`, `[a-z]`, `{n,m}`, `@`, `<`, `>`,
//!   `( )`) and regular expressions. Wildcards and expressions compile to a linear-time engine,
//!   so no pattern can backtrack catastrophically.
//! - [`parse_replacement`] and [`expand_replacement`] read and expand Replace with text: `^&`
//!   (the text found), `^c` (the clipboard), `\1`–`\9` with wildcards, `$n` and `${name}` with an
//!   expression.
//! - [`FindMatches`] holds the matches of a document, page by page, in document order, with the
//!   current match and Acrobat's rule for the first match a search shows (the first at or after
//!   the page the search started from, wrapping round).
//!
//! The app reads its text into one string per story or page (a paragraph mark,
//! [`PARAGRAPH_MARK`], between paragraphs) and maps the char ranges back to its own positions.

mod matches;
mod pattern;
mod replace;

pub use matches::{FindMatch, FindMatches, PageMatches};
pub use pattern::{FindError, FindOptions, FindPattern, MAX_COUNT, MAX_PATTERN, PARAGRAPH_MARK, RegexError, SECTION_MARK, WildcardError};
pub use replace::{ReplacePiece, expand_replacement, parse_replacement, resolve_group_names};

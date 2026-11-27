//! The caps every catalog, message and language list is read under, so that no catalog, however large or hostile, makes a
//! lookup run without bound or allocate without bound. Work is linear in a catalog's bytes and a message's parts; choosing a
//! variant is at most `MAX_SELECTORS` times `MAX_VARIANTS` comparisons. Past a cap the row, the rest of the file, the file or the
//! excess is skipped and reported (`I18n::check_all`, a `log::warn!` on first use), and the English shows.

/// A catalog file larger than this many bytes is not used.
pub const MAX_CATALOG_BYTES: usize = 2 << 20;
/// Rows read from one catalog file; the rest are skipped.
pub const MAX_ROWS: usize = 20_000;
/// A row (a line, with its tabs) longer than this many bytes is skipped.
pub const MAX_ROW_BYTES: usize = 16 << 10;
/// A selector message (a translation that begins with `.input`, `.local` or `.match`) longer than this many bytes is skipped.
pub const MAX_MESSAGE_BYTES: usize = 4 << 10;
/// Selector variables in one `.match`.
pub const MAX_SELECTORS: usize = 3;
/// Variants in one `.match`.
pub const MAX_VARIANTS: usize = 64;
/// `.input` and `.local` declarations in one message.
pub const MAX_DECLARATIONS: usize = 8;
/// Text runs and placeholders in one pattern.
pub const MAX_PARTS: usize = 64;
/// `@noun` rows read for one locale; the rest are skipped.
pub const MAX_NOUNS: usize = 2_000;
/// Named forms of one noun, and names in one `@grammar` row.
pub const MAX_FORMS: usize = 16;
/// Characters of a person's text (`Arg::Name`, `Arg::Text`) inserted into a message; more are cut and end with `…`.
pub const MAX_ARG_CHARS: usize = 1_024;
/// Characters of a filled message; more are cut and end with `…`.
pub const MAX_OUTPUT_CHARS: usize = 4_096;
/// Locales taken from the embedded or owned catalogs; the rest are ignored.
pub const MAX_LOCALES: usize = 64;
/// A language tag longer than this many bytes is no tag (the longest BCP 47 tag the standard recommends supporting).
pub const MAX_TAG_BYTES: usize = 35;
/// Preferred languages read from the system's list; the rest are ignored.
pub const MAX_PREFERRED: usize = 16;

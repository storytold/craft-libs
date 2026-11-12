#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Internet messages and MIME, read and written.
//!
//! [`parse()`] turns the bytes of a message (an `.eml` file, an MHT web archive, what a server
//! hands over) into a [`Message`]: a tree of [`Part`]s, each with its header fields, its content
//! type and parameters, and its body with the transfer encoding undone. [`Entity`] and
//! [`write_message`] write one. Beside them are the pieces both are made of, for callers that
//! work below the tree: header fields ([`split_headers`], [`write_field`]), parameters
//! ([`parse_params`]), encoded-words ([`decode_words`], [`encode_words`]), transfer encodings
//! ([`transfer`]), character sets ([`charset`]; with the `detect` feature, `detect` for bytes
//! that name none), address lists ([`parse_address_list`],
//! [`format_address_list`]), dates ([`parse_date`], [`format_date`]), mbox files
//! ([`mbox_messages`], [`write_mbox_message`]) and the lookups of `multipart/related`
//! ([`Resources`]). Beside MIME: `mailto:` links ([`parse_mailto`], RFC 6068) and strict
//! base64 ([`base64`], RFC 4648).
//!
//! Written from the public RFCs: 5322 (messages), 2045–2049 (MIME), 2047 (encoded-words), 2231
//! (parameter values), 2387 and 2557 (`multipart/related`, MHTML) and 4155 (mbox).
//!
//! Input is hostile. Nothing here fails or panics on any bytes: what can't be read is skipped,
//! sizes, counts and nesting are capped ([`Limits`]), and the work is linear in the input.
//! Nothing written can be bent into extra header fields by the values it is given: every header
//! value written loses its CR, LF and NUL.
//!
//! What a message means (which part is the body, which parts are files, what a flag header
//! says) is the caller's: this crate has no notion of a mailbox or a document.

pub mod base64;
pub mod charset;
#[cfg(feature = "detect")]
pub mod detect;
pub mod transfer;

mod address;
mod date;
mod header;
mod mailto;
mod mbox;
mod part;
mod related;
mod words;
mod write;

pub use address::{MAX_ADDRESSES, Mailbox, format_address, format_address_list, parse_address_list};
pub use date::{DateTime, format_date, parse_date};
pub use header::{Field, Headers, MAX_PARAMS, Params, clean, is_field_name, parse_params, split_headers, write_field};
pub use mailto::{Mailto, is_mailto, parse_mailto};
pub use mbox::{MboxMessages, mbox_messages, write_mbox_message};
pub use part::{Body, Message, Part, parse, parse_entity, split_multipart};
pub use related::{Resources, resolve_location, start_part};
pub use words::{WORD_BYTES, decode_words, encode_b, encode_words, is_plain};
pub use write::{Boundaries, Entity, File, clean_id, encode_unstructured, file_name_params, quoted, safe_media_type, seven_bit_value, write_message};

/// How much of a message is read. Whatever a limit leaves out is skipped, never an error, and
/// [`Message::truncated`] says that something was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most bytes of input looked at; what lies past them is not read.
    pub max_input: usize,
    /// Most header fields kept from one header block; the rest are read past.
    pub max_header_fields: usize,
    /// Longest header value kept, in bytes; continuation lines past it are dropped.
    pub max_header_value: usize,
    /// Most header bytes kept from one header block in all (names and values together).
    pub max_header_block: usize,
    /// Deepest nesting of multiparts that is opened; a multipart deeper than this has no parts.
    /// Never more than [`Limits::HARD_MAX_DEPTH`], whatever is asked for.
    pub max_depth: usize,
    /// Most MIME parts read from one message, multiparts themselves included.
    pub max_parts: usize,
    /// Most bytes of decoded part bodies kept from one message in all; a part that would pass
    /// it has an empty body.
    pub max_decoded: usize,
    /// Header fields (names, any letter case) that are kept past `max_header_fields` and
    /// `max_header_block`, so that a sender's padding can't push them out: at most
    /// `max_reserved_each` of each, each value cut at `max_reserved_value` bytes with a `…`
    /// after it when it was longer.
    pub reserved_fields: &'static [&'static str],
    /// See `reserved_fields`.
    pub max_reserved_each: usize,
    /// See `reserved_fields`.
    pub max_reserved_value: usize,
}

impl Limits {
    /// The deepest nesting any [`Limits`] can ask for: the reader recurses once per level.
    pub const HARD_MAX_DEPTH: usize = 100;

    /// What [`Limits::default`] gives.
    pub const DEFAULT: Limits = Limits {
        max_input: 1 << 30,
        max_header_fields: 2000,
        max_header_value: 256 * 1024,
        max_header_block: 4 * 1024 * 1024,
        max_depth: 20,
        max_parts: 1000,
        max_decoded: 1 << 30,
        reserved_fields: &[],
        max_reserved_each: 4,
        max_reserved_value: 64,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

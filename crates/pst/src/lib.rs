#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! A reader of Outlook personal folders files (`.pst`), written from Microsoft's published
//! specification, [MS-PST], and nothing else.
//!
//! [`Pst::open`] reads over any `Read + Seek` and never holds the file in memory: a page or a
//! block is read when it is needed. ANSI and Unicode files are read, with data blocks encoded
//! either way the format allows (or not at all). A file with a password is read only once
//! [`Pst::unlock`] has been given it.
//!
//! The layers are the specification's: the node database (`ndb`: B-trees of nodes and blocks,
//! data trees, subnode trees), lists, tables and properties (`ltp`: the heap on a node, its
//! B-tree, property and table contexts), and the messaging layer ([`Pst`]: the store, folders,
//! items, recipients, attachments, attached messages, named properties).
//!
//! A .pst file is hostile input. Nothing here panics on any bytes: every offset and length is
//! checked against the file, every CRC, signature and block id against what it guards, trees
//! are bounded in depth and read each part once, and the caps of [`Limits`] are checked before
//! anything is read or allocated. Past a cap, a part is left out and named ([`Error::Cap`]).
#![cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]

mod bth;
mod bytes;
mod crc;
mod crypt;
mod error;
mod header;
mod heap;
mod list_stream;
mod ltp;
mod named;
mod ndb;
mod prop;
mod pst;
pub mod recur;
mod source;
/// The test writer: synthetic .pst files from invented folders and messages, for this crate's
/// tests and, through the `test-writer` feature (a dev-dependency only, never in a build that
/// ships), for the tests of the import that reads them.
#[cfg(any(test, feature = "test-writer"))]
#[doc(hidden)]
#[path = "tests/writer.rs"]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::new_without_default,
    clippy::too_many_arguments,
    dead_code
)]
pub mod test_writer;
#[cfg(test)]
mod tests;

pub use crc::crc32;
pub use crypt::Encryption;
pub use error::{Cap, Damage, Error, Limits, MAX_DAMAGE, Result};
pub use header::Format;
pub use list_stream::{ListMember, ListStream, MAX_LIST_ENTRY, list_stream};
pub use named::{NamedMap, NamedProperty, PS_MAPI, PS_PUBLIC_STRINGS, PropName};
pub use prop::{PropValue, Properties, Property, types};
pub use pst::{
    Attachment, AttachmentContent, AttachmentReader, AttachmentRef, Folder, FolderEntry, FolderKind, FolderTree, ItemList, ItemSummary, LeftOut,
    LeftOutPart, Message, MessageRef, Nid, NodeInfo, Part, Pst, Recipient, RecipientKind, Store, node_types,
};
pub use recur::{Recur, RecurException, appointment_recurrence, time_zone_key};

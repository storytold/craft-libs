//! What can go wrong reading a .pst file, and the caps that keep a hostile one bounded.

use std::fmt;

/// A failure to read (part of) a .pst file. Nothing in this crate panics: everything a file can
/// get wrong ends here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The reader under the file failed.
    Io(std::io::ErrorKind),
    /// Not a .pst file (its first bytes are not `!BDN` … `SM`).
    NotPst,
    /// A .pst file of a version this reader doesn't know (`wVer`).
    UnsupportedVersion(u16),
    /// Data encoded in a way this reader can't undo (`bCryptMethod`, for example Windows
    /// Information Protection).
    UnsupportedEncryption(u8),
    /// A checksum, signature or block id that doesn't match what it guards.
    BadChecksum(&'static str),
    /// A structure that makes no sense.
    Corrupt(&'static str),
    /// An offset or length that points past the end of the file.
    OutOfBounds,
    /// A tree that leads back to where it was (or reads the same part twice).
    Cycle(&'static str),
    /// Something the file should have and hasn't.
    NotFound(&'static str),
    /// Past one of the caps in [`Limits`]: left out, not read.
    Cap(Cap),
    /// The file has a password and it has not been given ([`crate::Pst::unlock`]).
    PasswordRequired,
    /// The password given is not the file's.
    WrongPassword,
    /// The operation does not apply to this object (a search folder's items, for example).
    Unsupported(&'static str),
}

/// Which cap was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Cap {
    FileSize,
    Items,
    Folders,
    FolderDepth,
    ItemSize,
    AttachmentSize,
    Recipients,
    Attachments,
    EmbeddedDepth,
    BTreePages,
    Subnodes,
    DataBlocks,
    NamedProperties,
    /// [`Limits::max_listing_bytes`]: what one folder walk or one listing may read.
    ListingSize,
    /// [`Limits::max_embedded_messages`]: attached messages read under one item.
    EmbeddedMessages,
    /// [`Limits::read_multiple`]: more read from the file, in all, than its size allows for.
    TotalRead,
}

impl fmt::Display for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self {
            Cap::FileSize => "the file is larger than the cap",
            Cap::Items => "more items than the cap",
            Cap::Folders => "more folders than the cap",
            Cap::FolderDepth => "folders nested deeper than the cap",
            Cap::ItemSize => "an item larger than the cap",
            Cap::AttachmentSize => "an attachment larger than the cap",
            Cap::Recipients => "more recipients than the cap",
            Cap::Attachments => "more attachments than the cap",
            Cap::EmbeddedDepth => "messages attached deeper than the cap",
            Cap::BTreePages => "a tree with more pages than the cap",
            Cap::Subnodes => "an item with more parts than the cap",
            Cap::DataBlocks => "data spread over more blocks than its size needs",
            Cap::NamedProperties => "a named property's name, or all of them, longer than the cap",
            Cap::ListingSize => "a folder list or a list of items larger than the cap",
            Cap::EmbeddedMessages => "more attached messages under one item than the cap",
            Cap::TotalRead => "more read from the file than its size allows for",
        };
        f.write_str(what)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(kind) => write!(f, "the file could not be read ({kind})"),
            Error::NotPst => f.write_str("not a .pst file"),
            Error::UnsupportedVersion(36) => f.write_str("an offline folders file (.ost) with 4 KB pages, not supported (version 36)"),
            Error::UnsupportedVersion(37) => f.write_str("a file protected by Windows Information Protection, not supported (version 37)"),
            Error::UnsupportedVersion(v) => write!(f, "a .pst file of a version not supported ({v})"),
            Error::UnsupportedEncryption(m) => write!(f, "the file's data is encoded in a way not supported ({m:#04x})"),
            Error::BadChecksum(what) => write!(f, "damaged file: {what} doesn't match its checksum"),
            Error::Corrupt(what) => write!(f, "damaged file: {what}"),
            Error::OutOfBounds => f.write_str("damaged file: a part points past its end"),
            Error::Cycle(what) => write!(f, "damaged file: {what} leads back to itself"),
            Error::NotFound(what) => write!(f, "damaged file: {what} is missing"),
            Error::Cap(cap) => write!(f, "left out: {cap}"),
            Error::PasswordRequired => f.write_str("the file has a password"),
            Error::WrongPassword => f.write_str("the password is not the file's"),
            Error::Unsupported(what) => write!(f, "not supported: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::UnexpectedEof => Error::OutOfBounds,
            kind => Error::Io(kind),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A part of the file's own structure that could not be read: a page of the node tree (the
/// folders and items under it can't be found) or of the block tree (the data of the nodes that
/// use its blocks can't be found). The page's own count of what it held can't be trusted, so
/// what was lost with it is not known; each folder or item that needed it is named in the
/// `left_out` of the walk, list or message that met it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Damage {
    /// Where the page is in the file.
    pub offset: u64,
    /// "a page of the node tree" or "a page of the block tree".
    pub what: &'static str,
    pub reason: Error,
    /// 1; or, for the last entry once [`MAX_DAMAGE`] pages have been named, how many pages
    /// from it on could not be read (at least: past 65,536 they are no longer counted).
    pub pages: u64,
}

/// Damaged pages named one by one; past them they are counted in one entry.
pub const MAX_DAMAGE: usize = 1000;

/// The caps, checked before anything is read or allocated. [`Limits::default`] is the import's
/// (the file at most 100 GB, 2,000,000 items, 10,000 folders nested at most 64 deep, 256 MB an
/// item, 150 MB an attachment, 1,000 recipients and 1,000 attachments an item, attached
/// messages at most 8 deep, 10,000 of them under one item; a folder walk or a listing at most
/// 1 GB; at most 64 times the file's size plus 256 MB read in all); lower ones serve tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_file_size: u64,
    /// Items listed across the file's folders ([`crate::Pst::items`]).
    pub max_items: usize,
    pub max_folders: usize,
    pub max_folder_depth: usize,
    /// One node's data (a message's properties, a table), and all of an item's property values
    /// read at once.
    pub max_item_bytes: u64,
    pub max_attachment_bytes: u64,
    pub max_recipients: usize,
    pub max_attachments: usize,
    pub max_embedded_depth: usize,
    /// Pages a walk of the whole node tree may visit.
    pub max_btree_pages: usize,
    /// Entries in one node's subnode tree.
    pub max_subnodes: usize,
    /// A property value larger than this is not read with the rest ([`crate::PropValue::Large`])
    /// and is read on its own ([`crate::Pst::read_property`]).
    pub max_inline_value: u64,
    /// All that one [`crate::Pst::walk_folders`] or one listing of a folder's items may read
    /// and keep (tables and values together).
    pub max_listing_bytes: u64,
    /// A value larger than this in a folder walk or an item list is [`crate::PropValue::Large`].
    pub max_summary_value: u64,
    /// Attached messages read under one item, at every depth together.
    pub max_embedded_messages: usize,
    /// All that may be read from the file while it is open: this many times its size, plus
    /// [`Limits::read_floor`], so that the work stays in proportion to the file. It is
    /// counted for each [`crate::Pst`] from its opening: an import that reads a file more than
    /// once (again after a cancel, say) opens it again.
    pub read_multiple: u64,
    pub read_floor: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_size: 100 * 1024 * 1024 * 1024,
            max_items: 2_000_000,
            max_folders: 10_000,
            max_folder_depth: 64,
            max_item_bytes: 256 * 1024 * 1024,
            max_attachment_bytes: 150 * 1024 * 1024,
            max_recipients: 1_000,
            max_attachments: 1_000,
            max_embedded_depth: 8,
            max_btree_pages: 4_000_000,
            max_subnodes: 200_000,
            max_inline_value: 4 * 1024 * 1024,
            max_listing_bytes: 1024 * 1024 * 1024,
            max_summary_value: 64 * 1024,
            max_embedded_messages: 10_000,
            read_multiple: 64,
            read_floor: 256 * 1024 * 1024,
        }
    }
}

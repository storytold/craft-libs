//! The messaging layer ([MS-PST] 2.4): the message store, folders, items, recipients,
//! attachments and attached messages, over the node database and its lists, tables and
//! properties.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek};
use std::mem::size_of;

use crate::crc::crc32;
use crate::crypt::Encryption;
use crate::error::{Cap, Error, Limits, Result};
use crate::header::{Format, HEADER_READ, Header};
use crate::ltp::{Budget, NodeData, Table, pc_properties, pc_records, pc_value};
use crate::named::{self, NamedMap};
use crate::ndb::{Ndb, NodeEntry, is_internal};
use crate::prop::{PropValue, Properties, types};
use crate::source::Source;

const NID_MESSAGE_STORE: u32 = 0x21;
const NID_NAME_TO_ID_MAP: u32 = 0x61;
const NID_ROOT_FOLDER: u32 = 0x122;
const NID_ATTACHMENT_TABLE: u32 = 0x671;
const NID_RECIPIENT_TABLE: u32 = 0x692;
/// Rows of a recipient or attachment table read at most: the caps of each are applied to what
/// is read, so a table may be larger than they are, but not without bound.
const MAX_PART_ROWS: usize = 1 << 16;

/// Node types ([MS-PST] 2.2.2.1), the low five bits of a node id.
pub mod node_types {
    pub const NORMAL_FOLDER: u8 = 0x02;
    pub const SEARCH_FOLDER: u8 = 0x03;
    pub const NORMAL_MESSAGE: u8 = 0x04;
    pub const ATTACHMENT: u8 = 0x05;
    pub const ASSOC_MESSAGE: u8 = 0x08;
    pub const HIERARCHY_TABLE: u8 = 0x0D;
    pub const CONTENTS_TABLE: u8 = 0x0E;
    pub const ASSOC_CONTENTS_TABLE: u8 = 0x0F;
    pub const ATTACHMENT_TABLE: u8 = 0x11;
    pub const RECIPIENT_TABLE: u8 = 0x12;
}

use node_types::{
    ASSOC_CONTENTS_TABLE, ASSOC_MESSAGE, ATTACHMENT_TABLE, CONTENTS_TABLE, HIERARCHY_TABLE, NORMAL_FOLDER, NORMAL_MESSAGE, RECIPIENT_TABLE,
    SEARCH_FOLDER,
};

/// A node id: unique in the file for a folder or an item, unique within its parent for an
/// attachment or an attached message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Nid(pub u32);

impl Nid {
    /// The node's type ([`node_types`]).
    pub fn kind(self) -> u8 {
        (self.0 & 0x1F) as u8
    }

    fn with_kind(self, kind: u8) -> Nid {
        Nid((self.0 & !0x1F) | u32::from(kind))
    }
}

/// The message store's own properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Store {
    pub display_name: Option<String>,
    /// The file's provider UID (`PidTagRecordKey`), which its entry ids carry.
    pub record_key: Option<Vec<u8>>,
    /// The top of the folders the owner sees ("Top of Personal Folders").
    pub ipm_subtree: Option<Nid>,
    pub deleted_items: Option<Nid>,
    /// The root of the search folders.
    pub search_root: Option<Nid>,
    pub properties: Properties,
}

/// What a folder holds, from its container class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderKind {
    Mail,
    Calendar,
    Contacts,
    Tasks,
    Notes,
    Journal,
    /// A container class this reader doesn't name.
    Other(String),
    /// A search folder: its items live in other folders.
    Search,
}

impl FolderKind {
    fn of(class: Option<&str>, nid: Nid) -> FolderKind {
        if nid.kind() == SEARCH_FOLDER {
            return FolderKind::Search;
        }
        let Some(class) = class else { return FolderKind::Mail };
        let family = |prefix: &str| class == prefix || class.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('.'));
        if family("IPF.Note") || family("IPF.Imap") || class == "IPF" {
            FolderKind::Mail
        } else if family("IPF.Appointment") {
            FolderKind::Calendar
        } else if family("IPF.Contact") {
            FolderKind::Contacts
        } else if family("IPF.Task") {
            FolderKind::Tasks
        } else if family("IPF.StickyNote") {
            FolderKind::Notes
        } else if family("IPF.Journal") {
            FolderKind::Journal
        } else {
            FolderKind::Other(class.to_string())
        }
    }
}

/// A folder's own properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub id: Nid,
    /// None for the root (whose parent is itself).
    pub parent: Option<Nid>,
    pub name: String,
    /// What the folder says it holds (`PidTagContentCount`); [`Pst::items`] says what it does.
    pub content_count: u32,
    pub unread_count: u32,
    pub has_subfolders: bool,
    pub container_class: Option<String>,
    pub kind: FolderKind,
    pub properties: Properties,
}

/// A folder in [`FolderTree`].
#[derive(Debug, Clone, PartialEq)]
pub struct FolderEntry {
    pub folder: Folder,
    /// 0 for the root.
    pub depth: usize,
    /// Whether it is the top of the owner's folders or under it; the rest (the root, the search
    /// root, views) are the file's own.
    pub in_ipm_subtree: bool,
}

/// What was left out of a walk or a read, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct LeftOut {
    pub id: Nid,
    pub parent: Option<Nid>,
    /// The one property left out of the folder or item, when that is what was.
    pub property: Option<u16>,
    pub reason: Error,
}

impl LeftOut {
    fn new(id: Nid, parent: Option<Nid>, reason: Error) -> LeftOut {
        LeftOut { id, parent, property: None, reason }
    }

    /// One for each damaged property of `properties`.
    fn damaged(id: Nid, parent: Option<Nid>, properties: &Properties) -> impl Iterator<Item = LeftOut> + '_ {
        properties.damaged().map(move |(p, reason)| LeftOut { id, parent, property: Some(p), reason: reason.clone() })
    }
}

/// Every folder of the file, parents before children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderTree {
    pub folders: Vec<FolderEntry>,
    /// Folders that could not be read (with their subfolders), or past a cap, and folders'
    /// properties that could not be read. A folder whose subfolders were left out for a cap is
    /// named with that cap.
    pub left_out: Vec<LeftOut>,
}

/// An item as its folder's contents table lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemSummary {
    pub id: Nid,
    pub class: Option<String>,
    /// The table's columns for it (subject, flags, times…); empty when the row could not be
    /// read (and then named in [`ItemList::left_out`]). A value larger than
    /// [`Limits::max_summary_value`] is [`PropValue::Large`].
    pub properties: Properties,
}

/// The items of a folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemList {
    pub items: Vec<ItemSummary>,
    /// Items whose row, or a property of it, could not be read (listed all the same); and,
    /// once each, the folder itself when items were past [`Limits::max_items`] or past the
    /// listing's budget ([`Limits::max_listing_bytes`]).
    pub left_out: Vec<LeftOut>,
    /// How many items were not listed: past [`Limits::max_items`] (counted across the
    /// folders listed so far), or past the listing's budget.
    pub past_cap: usize,
}

/// Where a message is: a folder's item, or attached to one (through its attachments).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageRef {
    pub node: Nid,
    /// For an attached message, pairs of (attachment, object) subnode ids from the item down.
    pub path: Vec<Nid>,
}

impl MessageRef {
    pub fn item(node: Nid) -> MessageRef {
        MessageRef { node, path: Vec::new() }
    }

    /// How deep it is attached (0 for an item in a folder).
    pub fn depth(&self) -> usize {
        self.path.len() / 2
    }
}

/// An attachment of a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttachmentRef {
    pub message: MessageRef,
    pub id: Nid,
}

/// A recipient: its row of the recipient table.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipient {
    pub properties: Properties,
}

/// To, Cc or Bcc (`PidTagRecipientType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientKind {
    Originator,
    To,
    Cc,
    Bcc,
    Other(i64),
}

impl Recipient {
    pub fn name(&self) -> Option<String> {
        self.properties.text(0x3001)
    }

    /// The SMTP address when there is one, else the address of its type.
    pub fn address(&self) -> Option<String> {
        self.properties.text(0x39FE).or_else(|| self.properties.text(0x3003))
    }

    pub fn address_type(&self) -> Option<String> {
        self.properties.text(0x3002)
    }

    pub fn kind(&self) -> RecipientKind {
        match self.properties.int(0x0C15).map(|v| v & 0x0FFF_FFFF) {
            Some(0) => RecipientKind::Originator,
            Some(1) | None => RecipientKind::To,
            Some(2) => RecipientKind::Cc,
            Some(3) => RecipientKind::Bcc,
            Some(other) => RecipientKind::Other(other),
        }
    }
}

/// What an attachment holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentContent {
    /// Nothing (a link to a file elsewhere, by its path, for one).
    None,
    /// Bytes, read with [`Pst::attachment_reader`] or [`Pst::attachment_bytes`].
    Data { size: u64 },
    /// An attached message, read with [`Pst::message`].
    Message(MessageRef),
    /// An attached message deeper than [`Limits::max_embedded_depth`]: left out.
    MessageTooDeep,
    /// An object other than a message (an OLE object), read as bytes.
    Object { size: u64 },
}

/// An attachment's properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub id: AttachmentRef,
    /// `PidTagAttachMethod`: 1 by value, 2–4 by reference, 5 an attached message, 6 OLE.
    pub method: Option<i64>,
    /// The long file name when there is one, else the short.
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub content_id: Option<String>,
    pub content: AttachmentContent,
    pub properties: Properties,
}

/// A part of a message that was left out, and why.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    /// Recipients past [`Limits::max_recipients`].
    Recipients { count: usize },
    /// One damaged recipient row.
    Recipient { row: u32 },
    /// The recipient table as a whole.
    RecipientTable,
    /// Attachments past [`Limits::max_attachments`].
    Attachments { count: usize },
    /// One attachment.
    Attachment { id: Nid },
    /// The attachment table as a whole.
    AttachmentTable,
    /// A property of the message.
    Property { id: u16 },
    /// A property of a recipient (by its row).
    RecipientProperty { row: u32, id: u16 },
    /// A property of an attachment.
    AttachmentProperty { attachment: Nid, id: u16 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeftOutPart {
    pub part: Part,
    pub reason: Error,
}

/// A message (or any item) with its recipients and attachments.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: MessageRef,
    pub class: Option<String>,
    pub properties: Properties,
    pub recipients: Vec<Recipient>,
    pub attachments: Vec<Attachment>,
    pub left_out: Vec<LeftOutPart>,
}

/// A node of the node B-tree, for counting what the file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeInfo {
    pub id: Nid,
    /// The folder it is in (for a folder or an item), or zero.
    pub parent: Nid,
}

/// A .pst file, read over any `Read + Seek`.
pub struct Pst<R> {
    ndb: Ndb<R>,
    store: Properties,
    password: u32,
    unlocked: bool,
    named: Option<NamedMap>,
    listed: HashMap<u32, usize>,
    listed_total: usize,
    /// The item whose attached messages are being read: what is left of its budget, and how
    /// many have been read.
    tree: ItemTree,
}

/// What one item and the messages attached to it, together, may still read.
#[derive(Debug, Clone, Copy, Default)]
struct ItemTree {
    node: u32,
    left: u64,
    embedded: usize,
}

/// `s` in Windows-1252, when every character is in it.
fn windows_1252(s: &str) -> Option<Vec<u8>> {
    // 0x80 to 0x9F; zero where the code page has no character.
    const HIGH: [u32; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C,
        0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
    ];
    s.chars()
        .map(|c| {
            let n = u32::from(c);
            if n < 0x80 || (0xA0..=0xFF).contains(&n) {
                u8::try_from(n).ok()
            } else {
                HIGH.iter().position(|&h| h != 0 && h == n).and_then(|i| u8::try_from(i).ok()).map(|i| i | 0x80)
            }
        })
        .collect()
}

/// The node id an entry id names (its last four bytes, [MS-PST] 2.4.3.2).
fn entry_id_nid(b: &[u8]) -> Option<Nid> {
    let tail = b.get(20..24)?;
    Some(Nid(u32::from_le_bytes([tail.first().copied()?, tail.get(1).copied()?, tail.get(2).copied()?, tail.get(3).copied()?])))
}

/// What ends a call rather than leaving one part of it out: the reader under the file failing,
/// the file's read allowance ([`Cap::TotalRead`]) spent, or the call's own budget (`own`:
/// [`Cap::ItemSize`] for a message; a walk names each folder past its budget and a listing
/// counts the items past it). Anything else is damage to that part, or a cap on that part
/// alone (its recipients, attachments, rows, depth, blocks or size), and is named in
/// `left_out` while the rest is read.
fn ends_call(e: &Error, own: Option<Cap>) -> bool {
    match e {
        Error::Io(_) | Error::Cap(Cap::TotalRead) => true,
        Error::Cap(c) => Some(*c) == own,
        _ => false,
    }
}

impl<R: Read + Seek> Pst<R> {
    /// Opens a file with the import's caps. The header and the message store are read; a file
    /// with a password then waits for [`Pst::unlock`].
    pub fn open(reader: R) -> Result<Self> {
        Self::open_with(reader, Limits::default())
    }

    pub fn open_with(reader: R, limits: Limits) -> Result<Self> {
        let mut src = Source::new(reader, &limits)?;
        let len = HEADER_READ.min(usize::try_from(src.size()).unwrap_or(HEADER_READ));
        let header = Header::parse(&src.read_at(0, len)?)?;
        let mut ndb = Ndb::new(src, header, limits);
        let entry = ndb.node(NID_MESSAGE_STORE)?;
        let mut budget = Budget::new(ndb.limits.max_item_bytes, ndb.limits.max_inline_value, Cap::ItemSize);
        let node = NodeData::load(&mut ndb, entry.data, entry.sub, &mut budget)?;
        let store = pc_properties(&mut ndb, &node, &mut budget, &[])?;
        let password = store.int(0x67FF).map(|v| v as u32).unwrap_or(0);
        Ok(Pst { ndb, store, password, unlocked: password == 0, named: None, listed: HashMap::new(), listed_total: 0, tree: ItemTree::default() })
    }

    pub fn format(&self) -> Format {
        self.ndb.header.format
    }

    pub fn encryption(&self) -> Encryption {
        self.ndb.encryption()
    }

    /// Whether the file has a password that has not been given yet.
    pub fn password_required(&self) -> bool {
        !self.unlocked
    }

    /// Whether `password` is the file's: its CRC ([MS-PST] 5.3) is what the message store keeps
    /// (`PidTagPstPassword`). The specification doesn't say in which encoding the text is
    /// hashed, so its Windows-1252 bytes (when it has them), its UTF-8 bytes and its UTF-16LE
    /// bytes are all tried. A file without a password takes any.
    pub fn check_password(&self, password: &str) -> bool {
        if self.password == 0 {
            return true;
        }
        let utf16: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let ansi = windows_1252(password).is_some_and(|b| crc32(&b) == self.password);
        ansi || crc32(password.as_bytes()) == self.password || crc32(&utf16) == self.password
    }

    /// Gives the file's password; nothing of the file is read until it has been.
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        if self.check_password(password) {
            self.unlocked = true;
            Ok(())
        } else {
            Err(Error::WrongPassword)
        }
    }

    fn gate(&self) -> Result<()> {
        if self.unlocked { Ok(()) } else { Err(Error::PasswordRequired) }
    }

    /// What one item or folder may read.
    fn budget(&self) -> Budget {
        Budget::new(self.ndb.limits.max_item_bytes, self.ndb.limits.max_inline_value, Cap::ItemSize)
    }

    /// What one folder walk or one listing may read, all of it.
    fn listing_budget(&self) -> Budget {
        let l = &self.ndb.limits;
        Budget::new(l.max_listing_bytes, l.max_summary_value.min(l.max_inline_value), Cap::ListingSize)
    }

    pub fn store(&self) -> Result<Store> {
        self.gate()?;
        let p = &self.store;
        Ok(Store {
            display_name: p.text(0x3001),
            record_key: p.binary(0x0FF9).map(<[u8]>::to_vec),
            ipm_subtree: p.binary(0x35E0).and_then(entry_id_nid),
            deleted_items: p.binary(0x35E3).and_then(entry_id_nid),
            search_root: p.binary(0x35E7).and_then(entry_id_nid),
            properties: p.clone(),
        })
    }

    /// The root folder, which holds the top of the owner's folders and the file's own.
    pub fn root_folder(&self) -> Nid {
        Nid(NID_ROOT_FOLDER)
    }

    fn load(&mut self, entry: &NodeEntry, budget: &mut Budget) -> Result<NodeData> {
        NodeData::load(&mut self.ndb, entry.data, entry.sub, budget)
    }

    /// A folder's properties; a property that could not be read is [`PropValue::Damaged`].
    pub fn folder(&mut self, id: Nid) -> Result<Folder> {
        let mut budget = self.budget();
        self.folder_in(id, &mut budget)
    }

    fn folder_in(&mut self, id: Nid, budget: &mut Budget) -> Result<Folder> {
        self.gate()?;
        if !matches!(id.kind(), NORMAL_FOLDER | SEARCH_FOLDER) {
            return Err(Error::Unsupported("a node that is not a folder"));
        }
        let entry = self.ndb.node(id.0)?;
        let node = self.load(&entry, budget)?;
        let properties = pc_properties(&mut self.ndb, &node, budget, &[])?;
        let container_class = properties.text(0x3613);
        let parent = Some(Nid(entry.parent)).filter(|p| p.0 != 0 && *p != id);
        Ok(Folder {
            id,
            parent,
            name: properties.text(0x3001).unwrap_or_default(),
            content_count: properties.int(0x3602).map(|v| v as u32).unwrap_or(0),
            unread_count: properties.int(0x3603).map(|v| v as u32).unwrap_or(0),
            has_subfolders: properties.bool(0x360A).unwrap_or(false),
            kind: FolderKind::of(container_class.as_deref(), id),
            container_class,
            properties,
        })
    }

    /// The table at `nid`, or None when the file has no such node (a folder without
    /// subfolders, say). Anything that fails after the node is found is an error.
    fn table(&mut self, nid: u32, budget: &mut Budget, max_rows: usize, cap: Cap) -> Result<Option<Table>> {
        let entry = match self.ndb.node(nid) {
            Ok(e) => e,
            Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let node = self.load(&entry, budget)?;
        Table::open(&mut self.ndb, node, max_rows, cap).map(Some)
    }

    /// A folder's subfolders, in the order of its hierarchy table.
    pub fn subfolders(&mut self, id: Nid) -> Result<Vec<Nid>> {
        let mut budget = self.budget();
        self.subfolders_in(id, &mut budget)
    }

    fn subfolders_in(&mut self, id: Nid, budget: &mut Budget) -> Result<Vec<Nid>> {
        self.gate()?;
        let max = self.ndb.limits.max_folders;
        let Some(table) = self.table(id.with_kind(HIERARCHY_TABLE).0, budget, max, Cap::Folders)? else {
            return Ok(Vec::new());
        };
        let mut rows = table.rows;
        rows.sort_by_key(|&(_, index)| index);
        Ok(rows.into_iter().map(|(rowid, _)| Nid(rowid)).collect())
    }

    /// Every folder from the root down, within the caps: at most [`Limits::max_folders`],
    /// nested at most [`Limits::max_folder_depth`], each once (a folder met again is a cycle),
    /// all of it within [`Limits::max_listing_bytes`] and each value within
    /// [`Limits::max_summary_value`] (larger ones are [`PropValue::Large`]; [`Pst::folder`]
    /// reads a folder whole).
    pub fn walk_folders(&mut self) -> Result<FolderTree> {
        self.gate()?;
        // The store's own properties, read for the one entry id (not a copy of them all).
        let ipm = self.store.binary(0x35E0).and_then(entry_id_nid);
        let limits = self.ndb.limits.clone();
        let mut budget = self.listing_budget();
        let mut tree = FolderTree::default();
        let mut seen = HashSet::new();
        let mut scheduled = 1usize;
        let mut stack = vec![(Nid(NID_ROOT_FOLDER), None::<Nid>, 0usize, false)];
        while let Some((id, parent, depth, under_ipm)) = stack.pop() {
            // What this folder adds to what is kept: its notes of what was left out too.
            let noted = tree.left_out.len();
            self.walk_one(&mut tree, &mut stack, &mut seen, &mut scheduled, &mut budget, (id, parent, depth, under_ipm), ipm, &limits)?;
            budget.note(tree.left_out.len().saturating_sub(noted).saturating_mul(size_of::<LeftOut>()));
        }
        Ok(tree)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_one(
        &mut self,
        tree: &mut FolderTree,
        stack: &mut Vec<(Nid, Option<Nid>, usize, bool)>,
        seen: &mut HashSet<Nid>,
        scheduled: &mut usize,
        budget: &mut Budget,
        (id, parent, depth, under_ipm): (Nid, Option<Nid>, usize, bool),
        ipm: Option<Nid>,
        limits: &Limits,
    ) -> Result<()> {
        {
            if !seen.insert(id) {
                tree.left_out.push(LeftOut::new(id, parent, Error::Cycle("the folder hierarchy")));
                return Ok(());
            }
            if depth > limits.max_folder_depth {
                tree.left_out.push(LeftOut::new(id, parent, Error::Cap(Cap::FolderDepth)));
                return Ok(());
            }
            let folder = match self.folder_in(id, budget) {
                Ok(f) => f,
                Err(reason) if ends_call(&reason, None) => return Err(reason),
                Err(reason) => {
                    tree.left_out.push(LeftOut::new(id, parent, reason));
                    return Ok(());
                }
            };
            let kept = size_of::<FolderEntry>()
                .saturating_add(folder.name.capacity())
                .saturating_add(folder.container_class.as_ref().map_or(0, String::capacity));
            if let Err(reason) = budget.keep(kept) {
                tree.left_out.push(LeftOut::new(id, parent, reason));
                return Ok(());
            }
            tree.left_out.extend(LeftOut::damaged(id, parent, &folder.properties));
            let in_ipm = under_ipm || Some(id) == ipm;
            let children = if id.kind() == SEARCH_FOLDER { Ok(Vec::new()) } else { self.subfolders_in(id, budget) };
            tree.folders.push(FolderEntry { folder, depth, in_ipm_subtree: in_ipm });
            match children {
                Ok(children) => {
                    let room = limits.max_folders.saturating_sub(*scheduled);
                    if children.len() > room {
                        tree.left_out.push(LeftOut::new(id, parent, Error::Cap(Cap::Folders)));
                    }
                    let take: Vec<Nid> = children.into_iter().take(room).collect();
                    *scheduled = scheduled.saturating_add(take.len());
                    for child in take.into_iter().rev() {
                        stack.push((child, Some(id), depth.saturating_add(1), in_ipm));
                    }
                }
                Err(reason) if ends_call(&reason, None) => return Err(reason),
                Err(reason) => tree.left_out.push(LeftOut::new(id, parent, reason)),
            }
        }
        Ok(())
    }

    /// The items of a folder (its contents table), in the table's order: all of the listing
    /// within [`Limits::max_listing_bytes`], each value within [`Limits::max_summary_value`].
    pub fn items(&mut self, folder: Nid) -> Result<ItemList> {
        self.list(folder, CONTENTS_TABLE)
    }

    /// The folder's associated items (views, rules, forms: the file's own, not the owner's).
    pub fn associated_items(&mut self, folder: Nid) -> Result<ItemList> {
        self.list(folder, ASSOC_CONTENTS_TABLE)
    }

    fn list(&mut self, folder: Nid, kind: u8) -> Result<ItemList> {
        self.gate()?;
        match folder.kind() {
            NORMAL_FOLDER => {}
            SEARCH_FOLDER => return Err(Error::Unsupported("a search folder's items")),
            _ => return Err(Error::Unsupported("a node that is not a folder")),
        }
        let nid = folder.with_kind(kind).0;
        let mut budget = self.listing_budget();
        let max = self.ndb.limits.max_items;
        let Some(mut table) = self.table(nid, &mut budget, max, Cap::Items)? else {
            return Ok(ItemList::default());
        };
        let mut rows = std::mem::take(&mut table.rows);
        rows.sort_by_key(|&(_, index)| index);
        let room = match self.listed.get(&nid) {
            Some(&n) => n,
            None => self.ndb.limits.max_items.saturating_sub(self.listed_total),
        };
        let mut past_cap = rows.len().saturating_sub(room);
        let mut left_out = Vec::new();
        if past_cap > 0 {
            left_out.push(LeftOut::new(folder, None, Error::Cap(Cap::Items)));
        }
        rows.truncate(room);
        if let std::collections::hash_map::Entry::Vacant(e) = self.listed.entry(nid) {
            e.insert(rows.len());
            self.listed_total = self.listed_total.saturating_add(rows.len());
        }
        let total = rows.len();
        let mut items = Vec::new();
        for (n, (rowid, index)) in rows.into_iter().enumerate() {
            let id = Nid(rowid);
            let noted = left_out.len();
            let properties = match table.row(&mut self.ndb, rowid, index, &mut budget) {
                Ok(p) => Some(p),
                Err(Error::Cap(Cap::ListingSize)) => None,
                Err(reason) if ends_call(&reason, None) => return Err(reason),
                Err(reason) => {
                    left_out.push(LeftOut::new(id, Some(folder), reason));
                    Some(Properties::default())
                }
            };
            let item = properties.map(|properties| {
                left_out.extend(LeftOut::damaged(id, Some(folder), &properties));
                ItemSummary { id, class: properties.text(0x001A), properties }
            });
            budget.note(left_out.len().saturating_sub(noted).saturating_mul(size_of::<LeftOut>()));
            let kept = item.as_ref().map(|i| size_of::<ItemSummary>().saturating_add(i.class.as_ref().map_or(0, String::capacity)));
            match (item, kept.map(|k| budget.keep(k))) {
                (Some(item), Some(Ok(()))) => items.push(item),
                // The budget is spent: the rest are counted, and the folder named once.
                _ => {
                    left_out.push(LeftOut::new(folder, None, Error::Cap(Cap::ListingSize)));
                    past_cap = past_cap.saturating_add(total.saturating_sub(n));
                    break;
                }
            }
        }
        Ok(ItemList { items, left_out, past_cap })
    }

    /// The data and subnodes of the object at `path` under the node `node`.
    fn locate(&mut self, node: Nid, path: &[Nid]) -> Result<(u64, u64)> {
        let chain = self.chain(node, path)?;
        chain.last().copied().ok_or(Error::NotFound("a node"))
    }

    /// The data and subnodes of each object from `node` down `path`; one met again on the way
    /// (an attached message that holds itself) is a cycle.
    fn chain(&mut self, node: Nid, path: &[Nid]) -> Result<Vec<(u64, u64)>> {
        let entry = self.ndb.node(node.0)?;
        let mut chain = vec![(entry.data, entry.sub)];
        let mut sub = entry.sub;
        for step in path {
            let subs = self.ndb.subnodes(sub)?;
            let s = subs.get(&step.0).ok_or(Error::NotFound("an attachment"))?;
            if chain.contains(&(s.data, s.sub)) {
                return Err(Error::Cycle("an attached message"));
            }
            chain.push((s.data, s.sub));
            sub = s.sub;
        }
        Ok(chain)
    }

    /// A message (or any item) with its recipients and attachments. Property values larger
    /// than [`Limits::max_inline_value`] come back as [`PropValue::Large`]; what could not be
    /// read is named in `left_out`.
    ///
    /// An item and the messages attached to it share one budget, for what is read and what
    /// is kept: reading the item (a call with an empty path) starts it at
    /// [`Limits::max_item_bytes`], and the messages attached to it, read straight after it,
    /// take from what is left, as many as [`Limits::max_embedded_messages`]. So the whole of
    /// an item's tree is within the cap. Reading another item in between starts the budget
    /// for that one: an import reads an item and its attached messages together.
    pub fn message(&mut self, id: &MessageRef) -> Result<Message> {
        self.gate()?;
        let limits = self.ndb.limits.clone();
        if id.depth() > limits.max_embedded_depth || !id.path.len().is_multiple_of(2) {
            return Err(Error::Cap(Cap::EmbeddedDepth));
        }
        if id.path.is_empty() && !matches!(id.node.kind(), NORMAL_MESSAGE | ASSOC_MESSAGE) {
            return Err(Error::Unsupported("a node that is not an item"));
        }
        if id.path.is_empty() || self.tree.node != id.node.0 {
            self.tree = ItemTree { node: id.node.0, left: limits.max_item_bytes, embedded: 0 };
        }
        if !id.path.is_empty() {
            self.tree.embedded = self.tree.embedded.saturating_add(1);
            if self.tree.embedded > limits.max_embedded_messages {
                return Err(Error::Cap(Cap::EmbeddedMessages));
            }
        }
        let mut budget = Budget::new(self.tree.left, limits.max_inline_value, Cap::ItemSize);
        let read = self.message_in(id, &limits, &mut budget);
        // What a failed read took counts too.
        self.tree.left = budget.left;
        read
    }

    fn message_in(&mut self, id: &MessageRef, limits: &Limits, budget: &mut Budget) -> Result<Message> {
        let chain = self.chain(id.node, &id.path)?;
        let (data, sub) = chain.last().copied().ok_or(Error::NotFound("a node"))?;
        let node = NodeData::load(&mut self.ndb, data, sub, budget)?;
        let properties = pc_properties(&mut self.ndb, &node, budget, &[])?;
        let mut left_out: Vec<LeftOutPart> =
            properties.damaged().map(|(p, reason)| LeftOutPart { part: Part::Property { id: p }, reason: reason.clone() }).collect();

        let mut recipients = Vec::new();
        let recipient_table = node
            .subnodes
            .get(&NID_RECIPIENT_TABLE)
            .copied()
            .or_else(|| node.subnodes.iter().find(|(nid, _)| Nid(**nid).kind() == RECIPIENT_TABLE).map(|(_, e)| *e));
        if let Some(e) = recipient_table {
            match NodeData::load(&mut self.ndb, e.data, e.sub, budget).and_then(|n| Table::open(&mut self.ndb, n, MAX_PART_ROWS, Cap::Recipients)) {
                Ok(mut table) => {
                    let mut rows = table.rows.clone();
                    rows.sort_by_key(|&(_, index)| index);
                    if rows.len() > limits.max_recipients {
                        left_out.push(LeftOutPart {
                            part: Part::Recipients { count: rows.len().saturating_sub(limits.max_recipients) },
                            reason: Error::Cap(Cap::Recipients),
                        });
                        rows.truncate(limits.max_recipients);
                    }
                    for (rowid, index) in rows {
                        match table.row(&mut self.ndb, rowid, index, budget) {
                            Ok(properties) => {
                                budget.keep(size_of::<Recipient>())?;
                                let row = index;
                                left_out.extend(
                                    properties
                                        .damaged()
                                        .map(|(p, reason)| LeftOutPart { part: Part::RecipientProperty { row, id: p }, reason: reason.clone() }),
                                );
                                recipients.push(Recipient { properties });
                            }
                            Err(reason) if ends_call(&reason, Some(Cap::ItemSize)) => return Err(reason),
                            Err(reason) => left_out.push(LeftOutPart { part: Part::Recipient { row: index }, reason }),
                        }
                    }
                }
                Err(reason) if ends_call(&reason, Some(Cap::ItemSize)) => return Err(reason),
                Err(reason) => left_out.push(LeftOutPart { part: Part::RecipientTable, reason }),
            }
        }

        let mut attachments = Vec::new();
        let attachment_table = node
            .subnodes
            .get(&NID_ATTACHMENT_TABLE)
            .copied()
            .or_else(|| node.subnodes.iter().find(|(nid, _)| Nid(**nid).kind() == ATTACHMENT_TABLE).map(|(_, e)| *e));
        if let Some(e) = attachment_table {
            match NodeData::load(&mut self.ndb, e.data, e.sub, budget).and_then(|n| Table::open(&mut self.ndb, n, MAX_PART_ROWS, Cap::Attachments)) {
                Ok(table) => {
                    let mut rows = table.rows.clone();
                    rows.sort_by_key(|&(_, index)| index);
                    if rows.len() > limits.max_attachments {
                        left_out.push(LeftOutPart {
                            part: Part::Attachments { count: rows.len().saturating_sub(limits.max_attachments) },
                            reason: Error::Cap(Cap::Attachments),
                        });
                        rows.truncate(limits.max_attachments);
                    }
                    // Row ids are unique: the row index's keys strictly increase (bth::read).
                    for (rowid, _) in rows {
                        let att = Nid(rowid);
                        match self.attachment_from(&node, id, &chain, att, budget) {
                            Ok(a) => {
                                let strings =
                                    [&a.file_name, &a.mime_type, &a.content_id].iter().map(|s| s.as_ref().map_or(0, String::capacity)).sum::<usize>();
                                budget.keep(size_of::<Attachment>().saturating_add(strings))?;
                                left_out.extend(a.properties.damaged().map(|(p, reason)| LeftOutPart {
                                    part: Part::AttachmentProperty { attachment: att, id: p },
                                    reason: reason.clone(),
                                }));
                                attachments.push(a);
                            }
                            Err(reason) if ends_call(&reason, Some(Cap::ItemSize)) => return Err(reason),
                            Err(reason) => left_out.push(LeftOutPart { part: Part::Attachment { id: att }, reason }),
                        }
                    }
                }
                Err(reason) if ends_call(&reason, Some(Cap::ItemSize)) => return Err(reason),
                Err(reason) => left_out.push(LeftOutPart { part: Part::AttachmentTable, reason }),
            }
        }

        budget.note(left_out.len().saturating_mul(size_of::<LeftOutPart>()));
        let class = properties.text(0x001A);
        budget.keep(size_of::<Message>().saturating_add(class.as_ref().map_or(0, String::capacity)))?;
        Ok(Message { id: id.clone(), class, properties, recipients, attachments, left_out })
    }

    fn attachment_from(&mut self, message: &NodeData, id: &MessageRef, chain: &[(u64, u64)], att: Nid, budget: &mut Budget) -> Result<Attachment> {
        let e = message.subnodes.get(&att.0).copied().ok_or(Error::NotFound("an attachment"))?;
        let node = NodeData::load(&mut self.ndb, e.data, e.sub, budget)?;
        let properties = pc_properties(&mut self.ndb, &node, budget, &[0x3701])?;
        let method = properties.int(0x3705);
        let records = pc_records(&node)?;
        let data = records.iter().find(|(pid, _, _)| *pid == 0x3701).copied();
        let content = match data {
            None => AttachmentContent::None,
            Some((_, types::BINARY, raw)) => AttachmentContent::Data { size: node.hnid_len(&mut self.ndb, raw)? },
            Some((_, types::OBJECT, raw)) => {
                let object = pc_value(&mut self.ndb, &node, types::OBJECT, raw, budget)?;
                let PropValue::Object { nid, size } = object else {
                    return Err(Error::Corrupt("an attachment's object"));
                };
                if method == Some(5) {
                    // An attached message that is its attachment, or the message it is
                    // attached to (or one above), would be read for ever.
                    let again = |o: &crate::ndb::SubEntry| (o.data == e.data && o.sub == e.sub) || chain.contains(&(o.data, o.sub));
                    if node.subnodes.get(&nid).is_some_and(again) {
                        return Err(Error::Cycle("an attached message"));
                    }
                    let mut path = id.path.clone();
                    path.extend([att, Nid(nid)]);
                    let child = MessageRef { node: id.node, path };
                    if child.depth() > self.ndb.limits.max_embedded_depth {
                        AttachmentContent::MessageTooDeep
                    } else {
                        AttachmentContent::Message(child)
                    }
                } else {
                    AttachmentContent::Object { size: u64::from(size) }
                }
            }
            Some(_) => AttachmentContent::None,
        };
        Ok(Attachment {
            id: AttachmentRef { message: id.clone(), id: att },
            method,
            file_name: properties.text(0x3707).or_else(|| properties.text(0x3704)),
            mime_type: properties.text(0x370E),
            content_id: properties.text(0x3712),
            content,
            properties,
        })
    }

    /// An attachment's data, streamed a block at a time; refused past
    /// [`Limits::max_attachment_bytes`] before anything is read.
    pub fn attachment_reader(&mut self, att: &AttachmentRef) -> Result<AttachmentReader<'_, R>> {
        self.gate()?;
        let max = self.ndb.limits.max_attachment_bytes;
        let mut path = att.message.path.clone();
        path.push(att.id);
        let (data, sub) = self.locate(att.message.node, &path)?;
        let mut budget = self.budget();
        let node = NodeData::load(&mut self.ndb, data, sub, &mut budget)?;
        let (_, ty, raw) = pc_records(&node)?.into_iter().find(|(id, _, _)| *id == 0x3701).ok_or(Error::NotFound("an attachment's data"))?;
        let hnid = match ty {
            types::BINARY => raw,
            types::OBJECT => {
                let b = node.heap.item(raw)?;
                crate::bytes::u32_at(b, 0)?
            }
            _ => return Err(Error::Corrupt("an attachment's data type")),
        };
        if let Some(e) = node.subnode(hnid) {
            let (blocks, total) = self.ndb.data_blocks(e.data, max, Cap::AttachmentSize)?;
            Ok(AttachmentReader { ndb: &mut self.ndb, blocks, next: 0, buf: Vec::new(), pos: 0, total, read: 0 })
        } else {
            let buf = node.hnid_bytes(&mut self.ndb, hnid, max, Cap::AttachmentSize)?;
            let total = buf.len() as u64;
            Ok(AttachmentReader { ndb: &mut self.ndb, blocks: Vec::new(), next: 0, buf, pos: 0, total, read: total })
        }
    }

    /// An attachment's data, whole (within [`Limits::max_attachment_bytes`]).
    pub fn attachment_bytes(&mut self, att: &AttachmentRef) -> Result<Vec<u8>> {
        let mut r = self.attachment_reader(att)?;
        let mut out = Vec::with_capacity(usize::try_from(r.total.min(1 << 20)).unwrap_or(0));
        r.read_to_end(&mut out).map_err(|e| match e.get_ref().and_then(|i| i.downcast_ref::<Error>()) {
            Some(inner) => inner.clone(),
            None => Error::from(e),
        })?;
        Ok(out)
    }

    /// One property of a message, read on its own (for a [`PropValue::Large`]), within `max`
    /// bytes and [`Limits::max_item_bytes`].
    pub fn read_property(&mut self, message: &MessageRef, id: u16, max: u64) -> Result<Option<PropValue>> {
        self.read_property_at(message.node, &message.path, id, max)
    }

    /// One property of an attachment, read on its own.
    pub fn read_attachment_property(&mut self, att: &AttachmentRef, id: u16, max: u64) -> Result<Option<PropValue>> {
        let mut path = att.message.path.clone();
        path.push(att.id);
        self.read_property_at(att.message.node, &path, id, max)
    }

    fn read_property_at(&mut self, node: Nid, path: &[Nid], id: u16, max: u64) -> Result<Option<PropValue>> {
        self.gate()?;
        let (data, sub) = self.locate(node, path)?;
        let mut budget = self.budget();
        let n = NodeData::load(&mut self.ndb, data, sub, &mut budget)?;
        let Some((_, ty, raw)) = pc_records(&n)?.into_iter().find(|(pid, _, _)| *pid == id) else {
            return Ok(None);
        };
        let max = max.min(self.ndb.limits.max_item_bytes);
        let mut budget = Budget::new(max, max, Cap::ItemSize);
        let value = pc_value(&mut self.ndb, &n, ty, raw, &mut budget)?;
        // What the value holds beyond the bytes it was made from is within `max` too.
        let read = max.saturating_sub(budget.left) as usize;
        budget.keep(value.held().saturating_sub(read))?;
        Ok(Some(value))
    }

    /// The file's named properties (what the ids from 0x8000 stand for).
    pub fn named_properties(&mut self) -> Result<&NamedMap> {
        self.gate()?;
        if self.named.is_none() {
            let map = match self.ndb.node(NID_NAME_TO_ID_MAP) {
                Ok(entry) => {
                    let mut budget = self.budget();
                    let node = self.load(&entry, &mut budget)?;
                    named::read(&mut self.ndb, &node)?
                }
                Err(Error::NotFound(_)) => NamedMap::default(),
                Err(e) => return Err(e),
            };
            self.named = Some(map);
        }
        self.named.as_ref().ok_or(Error::NotFound("the named property map"))
    }

    /// Every node of the file's node tree (folders, items and the file's own), in id order,
    /// until `f` returns false; for counting what the file holds against what was read.
    pub fn for_each_node(&mut self, mut f: impl FnMut(NodeInfo) -> bool) -> Result<()> {
        self.gate()?;
        self.ndb.for_each_node(&mut |e| f(NodeInfo { id: Nid(e.nid), parent: Nid(e.parent) }))
    }

    /// The pages of the file's own trees met so far that could not be read, and why: what was
    /// under them is lost, how much is not known. A walk of every node
    /// ([`Pst::for_each_node`]) meets every page of the node tree; the block tree's are met
    /// as the data that uses them is read.
    pub fn damage(&self) -> &[crate::Damage] {
        &self.ndb.damage
    }

    /// The file's size as the header says it (`ibFileEof`) and as it is.
    pub fn sizes(&self) -> (u64, u64) {
        (self.ndb.header.file_eof, self.ndb.file_size())
    }
}

/// An attachment's data, a block at a time ([`Pst::attachment_reader`]).
pub struct AttachmentReader<'a, R> {
    ndb: &'a mut Ndb<R>,
    blocks: Vec<u64>,
    next: usize,
    buf: Vec<u8>,
    pos: usize,
    total: u64,
    read: u64,
}

impl<R> AttachmentReader<'_, R> {
    /// The data's size.
    pub fn size(&self) -> u64 {
        self.total
    }
}

impl<R: Read + Seek> Read for AttachmentReader<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let invalid = |e: Error| io::Error::new(io::ErrorKind::InvalidData, e);
        while self.pos >= self.buf.len() {
            let Some(&bid) = self.blocks.get(self.next) else {
                if self.read != self.total {
                    return Err(invalid(Error::Corrupt("attachment data shorter than it says")));
                }
                return Ok(0);
            };
            self.next = self.next.saturating_add(1);
            if is_internal(bid) {
                return Err(invalid(Error::Corrupt("attachment data in an internal block")));
            }
            self.buf = self.ndb.block(bid).map_err(invalid)?;
            self.pos = 0;
            self.read = self.read.saturating_add(self.buf.len() as u64);
            if self.read > self.total {
                return Err(invalid(Error::Corrupt("attachment data longer than it says")));
            }
        }
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        let n = rest.len().min(out.len());
        if let (Some(dst), Some(src)) = (out.get_mut(..n), rest.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.pos = self.pos.saturating_add(n);
        Ok(n)
    }
}

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Godot packs (`.pck`): the file table of a pack on its own, or of one carried at the end of the
//! game's program (the last 12 bytes are the pack's size and `GDPC`).
//!
//! Layouts read, all little-endian, starting with `GDPC`, the format, and the engine's
//! major.minor.patch:
//! - format 1 (Godot 3): 16 reserved words, then the file count and the table straight after.
//!   Offsets count from the start of the whole file.
//! - format 2 (Godot 4 up to 4.4): pack flags, file base, 16 reserved words, then the file count
//!   and the table. Offsets count from the file base (itself from the pack's start when the flags
//!   say so).
//! - format 3 (Godot 4.5 on): pack flags, file base, the table's offset from the pack's start. The
//!   file count and table are there, and the file base always counts from the pack's start.
//!
//! A later format, or pack flags we don't know, gives the header with a note: its table isn't
//! guessed at. Nothing is ever decrypted: an encrypted table leaves no names to list, and an
//! encrypted member is listed as unreadable.

use serde::Serialize;

/// Maximum number of file-table members retained.
pub const MAX_ENTRIES: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("the file ends early ({0})")]
    Truncated(String),
    #[error("the file is malformed ({0})")]
    Malformed(String),
}

struct Reader<'a> { data: &'a [u8] }
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data } }
    fn slice(&self, at: usize, len: usize) -> Option<&'a [u8]> { self.data.get(at..at.checked_add(len)?) }
    fn u32_le(&self, at: usize) -> Option<u32> { Some(u32::from_le_bytes(self.slice(at, 4)?.try_into().ok()?)) }
    fn u64_le(&self, at: usize) -> Option<u64> { Some(u64::from_le_bytes(self.slice(at, 8)?.try_into().ok()?)) }
}

pub const MAGIC: &[u8; 4] = b"GDPC";
/// Bytes at the end of a program that say a pack is carried: its size, then [`MAGIC`].
pub const TRAILER_SIZE: u64 = 12;
/// Enough bytes at a pack's start for [`header`] to read any format's header.
pub const HEADER_READ: usize = 104;

const PACK_DIR_ENCRYPTED: u32 = 1;
const PACK_RELATIVE_BASE: u32 = 1 << 1;
const PACK_SPARSE: u32 = 1 << 2;
const PACK_KNOWN: u32 = PACK_DIR_ENCRYPTED | PACK_RELATIVE_BASE | PACK_SPARSE;
const FILE_ENCRYPTED: u32 = 1;
const FILE_REMOVAL: u32 = 1 << 1;
/// A longer path than any file system takes: past this, the table is damaged.
const MAX_PATH: u32 = 1 << 16;

/// The header of a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Header {
    /// Pack format (Godot 3 wrote 1; Godot 4 wrote 2, then 3).
    pub format: u32,
    /// The Godot version that wrote it.
    pub engine_version: String,
    /// Raw pack flags (0 for format 1, which has none).
    pub flags: u32,
    /// Raw file base from the header (0 for format 1).
    pub file_base: u64,
    /// Where the file count and table start, from the pack's start. `None` for a format not read.
    pub directory_offset: Option<u64>,
}

impl Header {
    pub fn directory_encrypted(&self) -> bool {
        self.flags & PACK_DIR_ENCRYPTED != 0
    }

    /// Where offset 0 of a member is, in the whole file, for a pack starting at `start`.
    fn member_base(&self, start: u64) -> Option<u64> {
        let relative = self.format >= 3 || self.flags & PACK_RELATIVE_BASE != 0;
        if relative { start.checked_add(self.file_base) } else { Some(self.file_base) }
    }
}

/// One line of a pack's file table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    /// The path without Godot's `res://`.
    pub path: String,
    /// Where its bytes start in the whole file. `None` when the table's numbers overflow.
    pub offset: Option<u64>,
    pub size: u64,
    /// The MD5 the table declares, as hex; `None` when it's all zeros (not set).
    pub md5: Option<String>,
    /// Raw file flags (0 for format 1).
    pub flags: u32,
}

impl Member {
    pub fn encrypted(&self) -> bool {
        self.flags & FILE_ENCRYPTED != 0
    }

    /// A patch's mark that this path is removed: it has no bytes.
    pub fn removal(&self) -> bool {
        self.flags & FILE_REMOVAL != 0
    }
}

/// A pack's header and what could be read of its table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pack {
    pub header: Header,
    /// Where the pack starts in the file (above 0 when it's carried by a program).
    pub start: u64,
    /// Whether a program carries it.
    pub embedded: bool,
    /// The file count the pack declares, when it could be read.
    pub declared_files: Option<u32>,
    /// Whether the table was read (not when it's encrypted or its layout isn't known).
    pub table_read: bool,
    pub members: Vec<Member>,
    pub truncated: bool,
    pub notes: Vec<String>,
}

/// Where a program's trailer says its pack starts, from the file's last bytes (`tail`, at least
/// [`TRAILER_SIZE`]) and the file's length. The caller still checks for [`MAGIC`] there.
pub fn trailer_start(tail: &[u8], file_len: u64) -> Option<u64> {
    let at = tail.len().checked_sub(TRAILER_SIZE as usize)?;
    let r = Reader::new(tail);
    if r.slice(at.checked_add(8)?, 4)? != MAGIC {
        return None;
    }
    file_len.checked_sub(TRAILER_SIZE)?.checked_sub(r.u64_le(at)?)
}

/// Where a pack starts in `data` and whether it's carried by a program; `None` when there's none.
pub fn find(data: &[u8]) -> Option<(usize, bool)> {
    if data.starts_with(MAGIC) {
        return Some((0, false));
    }
    let start = usize::try_from(trailer_start(data, data.len() as u64)?).ok()?;
    data.get(start..)?.starts_with(MAGIC).then_some((start, true))
}

/// The header at the start of `pack` (the bytes from the pack's start; [`HEADER_READ`] is enough).
pub fn header(pack: &[u8]) -> Result<Header, FormatError> {
    if !pack.starts_with(MAGIC) {
        return Err(FormatError::Malformed("no GDPC mark at the pack's start".into()));
    }
    let r = Reader::new(pack);
    let short = |what: &str| FormatError::Truncated(format!("the pack's header stops before {what}"));
    let word = |at: usize, what: &str| r.u32_le(at).ok_or_else(|| short(what));
    let format = word(4, "its format")?;
    let engine_version = format!("{}.{}.{}", word(8, "its version")?, word(12, "its version")?, word(16, "its version")?);
    let (flags, file_base, directory_offset) = match format {
        1 => (0, 0, Some(84)),
        2 => (word(20, "its flags")?, r.u64_le(24).ok_or_else(|| short("its file base"))?, Some(96)),
        3 => {
            let directory = r.u64_le(32).ok_or_else(|| short("its table's offset"))?;
            (word(20, "its flags")?, r.u64_le(24).ok_or_else(|| short("its file base"))?, Some(directory))
        }
        _ => (0, 0, None),
    };
    Ok(Header { format, engine_version, flags, file_base, directory_offset })
}

/// End of the pack payload, excluding an embedded trailer.
pub fn pack_end(data_len: usize, embedded: bool) -> usize {
    if embedded { data_len.saturating_sub(TRAILER_SIZE as usize) } else { data_len }
}

/// Read a pack from a whole file's bytes.
pub fn parse(data: &[u8]) -> Result<Pack, FormatError> {
    let (start, embedded) = find(data).ok_or_else(|| FormatError::Malformed("this file has no Godot pack".into()))?;
    let bytes = data.get(start..).unwrap_or_default();
    let header = header(bytes)?;
    let mut pack =
        Pack { header, start: start as u64, embedded, declared_files: None, table_read: false, members: Vec::new(), truncated: false, notes: Vec::new() };
    let Some(directory) = pack.header.directory_offset else {
        pack.notes.push(format!(
            "This pack uses format {}, whose file table isn't known here, so only its header was read (formats 1 to 3 are).",
            pack.header.format
        ));
        return Ok(pack);
    };
    let at = usize::try_from(directory).ok();
    pack.declared_files = at.and_then(|a| Reader::new(bytes).u32_le(a));
    let Some(count) = pack.declared_files else {
        return Err(FormatError::Truncated("the pack's file count lies outside the file".into()));
    };
    let unknown = pack.header.flags & !PACK_KNOWN;
    if unknown != 0 {
        pack.notes.push(format!("This pack sets flags not known here ({unknown:#x}), so its file table wasn't read; it may not mean what it seems to."));
        return Ok(pack);
    }
    if pack.header.flags & PACK_SPARSE != 0 {
        pack.notes.push(format!(
            "This pack is a sparse bundle declaring {count} file(s): only its header was read, because its members aren't stored like a plain pack's."
        ));
        return Ok(pack);
    }
    if pack.header.directory_encrypted() {
        pack.notes.push(format!(
            "This pack's file table is encrypted with the game's own key, so its {count} declared file(s) can't be named. Keys aren't searched for or guessed."
        ));
        return Ok(pack);
    }
    pack.table_read = true;
    read_table(bytes, data.len() as u64, at.unwrap_or_default().saturating_add(4), count, &mut pack);
    Ok(pack)
}

/// Read the table's lines from `at` until the count, a limit, or the first line that doesn't fit.
fn read_table(bytes: &[u8], file_len: u64, mut at: usize, count: u32, pack: &mut Pack) {
    let r = Reader::new(bytes);
    let base = pack.header.member_base(pack.start);
    let with_flags = pack.header.format >= 2;
    for i in 0..count {
        if pack.members.len() >= MAX_ENTRIES {
            pack.truncated = true;
            pack.notes.push(format!("Stopped at {MAX_ENTRIES} files; the pack declares {count}."));
            return;
        }
        let Some((member, next)) = table_line(&r, at, base, with_flags) else {
            pack.truncated = true;
            pack.notes.push(format!("The file table stops being readable at line {} of the {count} declared.", u64::from(i) + 1));
            return;
        };
        pack.members.push(member);
        at = next;
    }
    let outside = pack.members.iter().filter(|m| m.offset.and_then(|o| o.checked_add(m.size)).is_none_or(|end| end > file_len)).count();
    if outside > 0 {
        pack.notes.push(format!("{outside} file(s) in the table lie outside this file."));
    }
}

/// One table line and where the next starts.
fn table_line(r: &Reader<'_>, at: usize, base: Option<u64>, with_flags: bool) -> Option<(Member, usize)> {
    let len = r.u32_le(at)?;
    if len > MAX_PATH {
        return None;
    }
    let path_at = at.checked_add(4)?;
    let raw = r.slice(path_at, len as usize)?;
    let text = String::from_utf8_lossy(raw.split(|b| *b == 0).next().unwrap_or_default());
    let after = path_at.checked_add(len as usize)?;
    let offset = r.u64_le(after)?;
    let size = r.u64_le(after.checked_add(8)?)?;
    let md5 = r.slice(after.checked_add(16)?, 16)?;
    let mut next = after.checked_add(32)?;
    let mut flags = 0;
    if with_flags {
        flags = r.u32_le(next)?;
        next = next.checked_add(4)?;
    }
    let path = text.strip_prefix("res://").unwrap_or(&text).to_string();
    let md5 = md5.iter().any(|b| *b != 0).then(|| md5.iter().map(|b| format!("{b:02x}")).collect());
    Some((Member { path, offset: base.and_then(|b| b.checked_add(offset)), size, md5, flags }, next))
}


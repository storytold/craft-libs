//! Test fonts made from font bytes the caller supplies.
//!
//! [`rename_family`] gives one standalone face a family name of the same length, and [`collection`]
//! builds a TrueType collection (TTC) from standalone faces. Both read the structures they change as
//! the OpenType specification lays them out
//! (<https://learn.microsoft.com/en-us/typography/opentype/spec/otff> and
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/name>), and return a [`FontError`]
//! for input that does not fit them. Other tables are copied as they are and are not checked.
//!
//! The module reads no file and no environment variable, prints nothing, and builds for
//! `wasm32-unknown-unknown`.

use std::fmt;
use std::ops::Range;

/// Length of the sfnt header: version, `numTables`, `searchRange`, `entrySelector`, `rangeShift`.
const SFNT_HEADER: usize = 12;
/// Length of one table record: tag, checksum, offset, length.
const TABLE_RECORD: usize = 16;
/// Length of the `name` table header: version, count, `storageOffset`.
const NAME_HEADER: usize = 6;
/// Length of one name record.
const NAME_RECORD: usize = 12;
/// Length of one language-tag record of a version 1 `name` table.
const LANG_TAG_RECORD: usize = 4;
/// Offset of `checkSumAdjustment` in the `head` table.
const HEAD_ADJUSTMENT: usize = 8;
/// The value a standalone font's checksum and its `checkSumAdjustment` add up to.
const CHECKSUM_TARGET: u32 = 0xB1B0_AFBA;
/// Name IDs that hold a family name: legacy (1), typographic (16) and WWS (21).
const FAMILY_IDS: [u16; 3] = [1, 16, 21];
/// Name ID of the full font name, which starts with the family name.
const FULL_NAME_ID: u16 = 4;

/// Why a face could not be renamed or collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// `from` and `to` have different lengths in UTF-16 code units.
    UnequalLength,
    /// `from` or `to` is empty.
    InvalidName,
    /// No family record of the face holds `from`.
    NameNotFound,
    /// The face has no `name` table.
    MissingNameTable,
    /// A structure the function reads is truncated, out of range or of an unknown version.
    MalformedFont,
    /// A matching Macintosh Roman record would need a name that is not ASCII.
    UnsupportedEncoding,
    /// A string the rename changes shares bytes with a string it keeps, or with a different replacement.
    ConflictingNameStorage,
    /// [`collection`] was given no faces.
    EmptyCollection,
    /// The input is a collection, or an sfnt version other than TrueType or CFF outlines.
    UnsupportedFace,
    /// The output would not fit in memory or in the 32-bit offsets of a collection.
    OutputTooLarge,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnequalLength => "the old and new family names have different lengths",
            Self::InvalidName => "a family name is empty",
            Self::NameNotFound => "no family name record holds the old name",
            Self::MissingNameTable => "the font has no name table",
            Self::MalformedFont => "the font's table directory or name table is truncated or out of range",
            Self::UnsupportedEncoding => "a Macintosh Roman name record can only take an ASCII name",
            Self::ConflictingNameStorage => "a renamed string shares its bytes with a string that must not change",
            Self::EmptyCollection => "a collection needs at least one face",
            Self::UnsupportedFace => "the input is not a standalone TrueType or CFF face",
            Self::OutputTooLarge => "the output is too large",
        })
    }
}

impl std::error::Error for FontError {}

/// Returns a copy of `bytes`, one standalone face, with the family name `from` replaced by `to`.
///
/// The records changed are the legacy, typographic and WWS family names (name IDs 1, 16 and 21)
/// whose whole value is `from`, and full names (name ID 4) that are `from` or start with `from` and a
/// space; at least one family record must hold `from`. Copyright, style, PostScript (ID 6) and every
/// other name keep their bytes. Records in UTF-16BE (platform 0, and platform 3 encodings 0, 1 and
/// 10) and Macintosh Roman (platform 1 encoding 0) are read, and only ASCII names are written into
/// Macintosh Roman records; records in other encodings are left as they are. The function then writes the
/// `name` table's checksum and the `head` table's `checkSumAdjustment`.
///
/// # Errors
///
/// [`FontError::InvalidName`] when a name is empty, [`FontError::UnequalLength`] when the names have
/// different UTF-16 lengths, [`FontError::NameNotFound`] when no family record holds `from`,
/// [`FontError::UnsupportedEncoding`] when a matching Macintosh Roman record would need non-ASCII
/// bytes, [`FontError::ConflictingNameStorage`] when a changed string shares bytes with a kept one,
/// [`FontError::MissingNameTable`], [`FontError::UnsupportedFace`] for a collection, and
/// [`FontError::MalformedFont`] when the directory, the `name` table or the `head` table is out of
/// range or of an unknown version, or when the `name` or `head` table shares bytes with the directory
/// or another table.
pub fn rename_family(bytes: &[u8], from: &str, to: &str) -> Result<Vec<u8>, FontError> {
    if from.is_empty() || to.is_empty() {
        return Err(FontError::InvalidName);
    }
    if from.encode_utf16().count() != to.encode_utf16().count() {
        return Err(FontError::UnequalLength);
    }
    let tables = directory(bytes)?;
    let name = tables.iter().find(|t| &t.tag == b"name").ok_or(FontError::MissingNameTable)?;
    let head = tables.iter().find(|t| &t.tag == b"head");
    if shares_bytes(&tables, name) || head.is_some_and(|head| shares_bytes(&tables, head)) {
        return Err(FontError::MalformedFont);
    }
    if head.is_some_and(|head| head.length < HEAD_ADJUSTMENT + 4) {
        return Err(FontError::MalformedFont);
    }
    let strings = name_strings(bytes, name)?;
    let (edits, kept) = plan_edits(bytes, &strings, from, to)?;
    check_kept(&edits, kept)?;

    let mut out = copy_of(bytes)?;
    apply_edits(&mut out, name, &edits)?;
    let name_sum = checksum(bytes_at(&out, name.offset, name.length)?);
    put_u32(&mut out, name.record + 4, name_sum)?;
    if let Some(head) = head {
        let at = head.offset + HEAD_ADJUSTMENT;
        put_u32(&mut out, at, 0)?;
        let sum = checksum(&out);
        put_u32(&mut out, at, CHECKSUM_TARGET.wrapping_sub(sum))?;
    }
    Ok(out)
}

/// Returns a TrueType collection (TTC version 1.0) holding `faces`, each a standalone face, in order.
///
/// Every face keeps its sfnt header and table order and gets its own copy of each table. Tables
/// start on four-byte boundaries with zero padding between them, offsets count from the start of the
/// collection, and each record keeps the table's unpadded length. Each `head` table's
/// `checkSumAdjustment` is set to zero, as a collection does not use it, and every table checksum is
/// computed from the bytes in the collection.
///
/// # Errors
///
/// [`FontError::EmptyCollection`] for no faces, [`FontError::UnsupportedFace`] for a face that is
/// itself a collection, [`FontError::MalformedFont`] for a face shorter than its table directory or
/// with a table out of range, and [`FontError::OutputTooLarge`] when the collection would pass the
/// 4 GiB that its 32-bit offsets reach or cannot be allocated.
pub fn collection(faces: &[&[u8]]) -> Result<Vec<u8>, FontError> {
    if faces.is_empty() {
        return Err(FontError::EmptyCollection);
    }
    let directories = faces.iter().map(|face| directory(face)).collect::<Result<Vec<_>, _>>()?;
    let count = u32::try_from(faces.len()).map_err(|_| FontError::OutputTooLarge)?;
    let size = collection_size(&directories)?;
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| FontError::OutputTooLarge)?;
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.resize(SFNT_HEADER + faces.len() * 4, 0);
    for (index, (face, tables)) in faces.iter().zip(&directories).enumerate() {
        let directory = offset32(out.len())?;
        put_u32(&mut out, SFNT_HEADER + index * 4, directory)?;
        write_face(&mut out, face, tables)?;
    }
    Ok(out)
}

/// One table record of a face: where the record is and where its table is.
struct Table {
    tag: [u8; 4],
    /// Offset of the table record in the face.
    record: usize,
    offset: usize,
    length: usize,
}

/// Reads a standalone face's table directory, checking that every record and table lies in `face`.
fn directory(face: &[u8]) -> Result<Vec<Table>, FontError> {
    let version = bytes_at(face, 0, 4)?;
    if !matches!(version, [0, 1, 0, 0] | [b'O', b'T', b'T', b'O'] | [b't', b'r', b'u', b'e']) {
        return Err(FontError::UnsupportedFace);
    }
    let count = usize::from(u16_at(face, 4)?);
    bytes_at(face, 0, SFNT_HEADER + count * TABLE_RECORD)?;
    let mut tables = Vec::with_capacity(count);
    for index in 0..count {
        let record = SFNT_HEADER + index * TABLE_RECORD;
        let tag = <[u8; 4]>::try_from(bytes_at(face, record, 4)?).map_err(|_| FontError::MalformedFont)?;
        let offset = usize_from(u32_at(face, record + 8)?)?;
        let length = usize_from(u32_at(face, record + 12)?)?;
        bytes_at(face, offset, length)?;
        tables.push(Table { tag, record, offset, length });
    }
    Ok(tables)
}

/// Whether `table` shares bytes with the table directory or with another table of the face.
fn shares_bytes(tables: &[Table], table: &Table) -> bool {
    let end = table.offset + table.length;
    let overlaps = |start: usize, other_end: usize| start < end && table.offset < other_end;
    overlaps(0, SFNT_HEADER + tables.len() * TABLE_RECORD)
        || tables.iter().any(|other| other.record != table.record && overlaps(other.offset, other.offset + other.length))
}

/// Bytes a collection of these faces takes, refused past what a 32-bit offset reaches.
fn collection_size(directories: &[Vec<Table>]) -> Result<usize, FontError> {
    let mut size = u64::try_from(SFNT_HEADER + directories.len() * 4).map_err(|_| FontError::OutputTooLarge)?;
    for tables in directories {
        let directory = SFNT_HEADER + tables.len() * TABLE_RECORD;
        size = size.checked_add(u64::try_from(directory).map_err(|_| FontError::OutputTooLarge)?).ok_or(FontError::OutputTooLarge)?;
        for table in tables {
            let padded = u64::try_from(table.length).map_err(|_| FontError::OutputTooLarge)?.div_ceil(4) * 4;
            size = size.checked_add(padded).ok_or(FontError::OutputTooLarge)?;
        }
    }
    if size > u64::from(u32::MAX) {
        return Err(FontError::OutputTooLarge);
    }
    usize::try_from(size).map_err(|_| FontError::OutputTooLarge)
}

/// Appends one face's directory and tables to a collection being built.
fn write_face(out: &mut Vec<u8>, face: &[u8], tables: &[Table]) -> Result<(), FontError> {
    let directory = out.len();
    out.extend_from_slice(bytes_at(face, 0, SFNT_HEADER)?);
    out.resize(directory + SFNT_HEADER + tables.len() * TABLE_RECORD, 0);
    for (index, table) in tables.iter().enumerate() {
        let start = out.len();
        out.extend_from_slice(bytes_at(face, table.offset, table.length)?);
        if &table.tag == b"head" {
            if table.length < HEAD_ADJUSTMENT + 4 {
                return Err(FontError::MalformedFont);
            }
            put_u32(out, start + HEAD_ADJUSTMENT, 0)?;
        }
        let sum = checksum(bytes_at(out, start, table.length)?);
        out.resize(out.len().div_ceil(4) * 4, 0);
        let record = directory + SFNT_HEADER + index * TABLE_RECORD;
        put_bytes(out, record, &table.tag)?;
        put_u32(out, record + 4, sum)?;
        put_u32(out, record + 8, offset32(start)?)?;
        put_u32(out, record + 12, offset32(table.length)?)?;
    }
    Ok(())
}

/// How the bytes of a name record are encoded, as far as renaming reads them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf16,
    MacRoman,
    Other,
}

impl Encoding {
    fn of(platform: u16, encoding: u16) -> Self {
        match (platform, encoding) {
            (0, _) | (3, 0 | 1 | 10) => Self::Utf16,
            (1, 0) => Self::MacRoman,
            _ => Self::Other,
        }
    }
}

/// One string of the `name` table, with its range in the face.
struct NameString {
    encoding: Encoding,
    name_id: u16,
    range: Range<usize>,
}

/// The strings of a face's `name` table: the name records, then the language tags of version 1.
struct NameStrings {
    names: Vec<NameString>,
    lang_tags: Vec<Range<usize>>,
}

/// Reads the name and language-tag records of `table`, checking every string lies in the table.
fn name_strings(face: &[u8], table: &Table) -> Result<NameStrings, FontError> {
    let data = bytes_at(face, table.offset, table.length)?;
    let version = u16_at(data, 0)?;
    if version > 1 {
        return Err(FontError::MalformedFont);
    }
    let count = usize::from(u16_at(data, 2)?);
    let storage = usize::from(u16_at(data, 4)?);
    let records_end = NAME_HEADER + count * NAME_RECORD;
    bytes_at(data, 0, records_end)?;
    let mut header_end = records_end;
    let mut lang_tag_records = 0;
    if version == 1 {
        lang_tag_records = usize::from(u16_at(data, records_end)?);
        header_end = records_end + 2 + lang_tag_records * LANG_TAG_RECORD;
    }
    if storage < header_end || storage > data.len() {
        return Err(FontError::MalformedFont);
    }
    let string = |at: usize| -> Result<Range<usize>, FontError> {
        let length = usize::from(u16_at(data, at)?);
        let start = storage + usize::from(u16_at(data, at + 2)?);
        bytes_at(data, start, length)?;
        Ok(table.offset + start..table.offset + start + length)
    };
    let mut names = Vec::with_capacity(count);
    for index in 0..count {
        let record = NAME_HEADER + index * NAME_RECORD;
        let encoding = Encoding::of(u16_at(data, record)?, u16_at(data, record + 2)?);
        let name_id = u16_at(data, record + 6)?;
        names.push(NameString { encoding, name_id, range: string(record + 8)? });
    }
    let mut lang_tags = Vec::with_capacity(lang_tag_records);
    for index in 0..lang_tag_records {
        lang_tags.push(string(records_end + 2 + index * LANG_TAG_RECORD)?);
    }
    Ok(NameStrings { names, lang_tags })
}

/// A string the rename writes: its range in the face and its new bytes.
struct Edit {
    range: Range<usize>,
    bytes: Vec<u8>,
}

/// Works out the strings to rewrite and the ranges of the strings that must keep their bytes.
/// Refuses with [`FontError::NameNotFound`] when no family record (IDs 1, 16 and 21) holds `from`.
fn plan_edits(face: &[u8], strings: &NameStrings, from: &str, to: &str) -> Result<(Vec<Edit>, Vec<Range<usize>>), FontError> {
    let mut edits = Vec::new();
    let mut kept = strings.lang_tags.clone();
    let mut family_found = false;
    for string in &strings.names {
        let family = FAMILY_IDS.contains(&string.name_id);
        let bytes = face.get(string.range.clone()).ok_or(FontError::MalformedFont)?;
        let new = if family || string.name_id == FULL_NAME_ID { renamed(string, bytes, from, to)? } else { None };
        match new {
            Some(new) => {
                family_found |= family;
                edits.push(Edit { range: string.range.clone(), bytes: new });
            }
            None => kept.push(string.range.clone()),
        }
    }
    if !family_found {
        return Err(FontError::NameNotFound);
    }
    Ok((edits, kept))
}

/// The new bytes of a family or full-name string, or `None` when it does not hold `from`.
fn renamed(string: &NameString, bytes: &[u8], from: &str, to: &str) -> Result<Option<Vec<u8>>, FontError> {
    let family = FAMILY_IDS.contains(&string.name_id);
    let encoded = match string.encoding {
        Encoding::Utf16 => renamed_utf16(family, bytes, from, to)?,
        Encoding::MacRoman => renamed_mac_roman(family, bytes, from, to)?,
        Encoding::Other => None,
    };
    if encoded.as_ref().is_some_and(|encoded| encoded.len() != bytes.len()) {
        return Err(FontError::UnequalLength);
    }
    Ok(encoded)
}

/// Renames a UTF-16BE family string whose value is `from`, or a full name that starts with `from` and a space.
fn renamed_utf16(family: bool, bytes: &[u8], from: &str, to: &str) -> Result<Option<Vec<u8>>, FontError> {
    let value = utf16_value(bytes)?;
    let new = if family {
        (value == from).then(|| to.to_owned())
    } else {
        value.strip_prefix(from).filter(|rest| rest.is_empty() || rest.starts_with(' ')).map(|rest| format!("{to}{rest}"))
    };
    Ok(new.map(|new| new.encode_utf16().flat_map(u16::to_be_bytes).collect()))
}

/// Renames a Macintosh Roman string the way [`renamed_utf16`] does, byte by byte; only ASCII names are written.
fn renamed_mac_roman(family: bool, bytes: &[u8], from: &str, to: &str) -> Result<Option<Vec<u8>>, FontError> {
    if !from.is_ascii() {
        return if mac_roman_may_hold(family, bytes, from) { Err(FontError::UnsupportedEncoding) } else { Ok(None) };
    }
    let Some(rest) = bytes.strip_prefix(from.as_bytes()) else {
        return Ok(None);
    };
    if !(rest.is_empty() || (!family && rest.first() == Some(&b' '))) {
        return Ok(None);
    }
    if !to.is_ascii() {
        return Err(FontError::UnsupportedEncoding);
    }
    Ok(Some([to.as_bytes(), rest].concat()))
}

/// Whether a Macintosh Roman string could hold a non-ASCII `from`. The encoding has one byte per
/// character and shares its ASCII range with Unicode, so a string that differs from `from` in length
/// or in an ASCII position cannot hold it.
fn mac_roman_may_hold(family: bool, bytes: &[u8], from: &str) -> bool {
    let count = from.chars().count();
    let fits = bytes.len() == count || (!family && bytes.get(count) == Some(&b' '));
    fits && bytes.iter().zip(from.chars()).all(|(&byte, c)| !(byte.is_ascii() || c.is_ascii()) || char::from(byte) == c)
}

/// Decodes a UTF-16BE string, refusing an odd length or an unpaired surrogate.
fn utf16_value(bytes: &[u8]) -> Result<String, FontError> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return Err(FontError::MalformedFont);
    }
    let units = pairs.iter().map(|&pair| u16::from_be_bytes(pair));
    char::decode_utf16(units).collect::<Result<String, _>>().map_err(|_| FontError::MalformedFont)
}

/// Refuses a rename whose strings overlap a string that keeps its bytes.
fn check_kept(edits: &[Edit], mut kept: Vec<Range<usize>>) -> Result<(), FontError> {
    kept.retain(|range| !range.is_empty());
    kept.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(kept.len());
    for range in kept {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    for edit in edits {
        let next = merged.partition_point(|range| range.end <= edit.range.start);
        if merged.get(next).is_some_and(|range| range.start < edit.range.end) {
            return Err(FontError::ConflictingNameStorage);
        }
    }
    Ok(())
}

/// Writes the edits into `out`, refusing two edits that write different bytes to the same place.
fn apply_edits(out: &mut [u8], name: &Table, edits: &[Edit]) -> Result<(), FontError> {
    let mut ranges: Vec<&Edit> = edits.iter().collect();
    ranges.sort_by_key(|edit| (edit.range.start, edit.range.end));
    ranges.dedup_by(|a, b| a.range == b.range && a.bytes == b.bytes);
    let mut written = vec![false; name.length];
    for edit in ranges {
        let target = out.get_mut(edit.range.clone()).ok_or(FontError::MalformedFont)?;
        let start = edit.range.start.checked_sub(name.offset).ok_or(FontError::MalformedFont)?;
        let marks = written.get_mut(start..start + edit.bytes.len()).ok_or(FontError::MalformedFont)?;
        for ((byte, mark), &new) in target.iter_mut().zip(marks.iter_mut()).zip(&edit.bytes) {
            if *mark && *byte != new {
                return Err(FontError::ConflictingNameStorage);
            }
            *byte = new;
            *mark = true;
        }
    }
    Ok(())
}

/// The OpenType table checksum: the sum of big-endian 32-bit words, the last one padded with zeros.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        for (to, &from) in word.iter_mut().zip(chunk) {
            *to = from;
        }
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn copy_of(bytes: &[u8]) -> Result<Vec<u8>, FontError> {
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len()).map_err(|_| FontError::OutputTooLarge)?;
    out.extend_from_slice(bytes);
    Ok(out)
}

fn bytes_at(data: &[u8], at: usize, length: usize) -> Result<&[u8], FontError> {
    let end = at.checked_add(length).ok_or(FontError::MalformedFont)?;
    data.get(at..end).ok_or(FontError::MalformedFont)
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, FontError> {
    let bytes = <[u8; 2]>::try_from(bytes_at(data, at, 2)?).map_err(|_| FontError::MalformedFont)?;
    Ok(u16::from_be_bytes(bytes))
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, FontError> {
    let bytes = <[u8; 4]>::try_from(bytes_at(data, at, 4)?).map_err(|_| FontError::MalformedFont)?;
    Ok(u32::from_be_bytes(bytes))
}

fn put_bytes(data: &mut [u8], at: usize, bytes: &[u8]) -> Result<(), FontError> {
    let end = at.checked_add(bytes.len()).ok_or(FontError::MalformedFont)?;
    data.get_mut(at..end).ok_or(FontError::MalformedFont)?.copy_from_slice(bytes);
    Ok(())
}

fn put_u32(data: &mut [u8], at: usize, value: u32) -> Result<(), FontError> {
    put_bytes(data, at, &value.to_be_bytes())
}

fn usize_from(value: u32) -> Result<usize, FontError> {
    usize::try_from(value).map_err(|_| FontError::MalformedFont)
}

fn offset32(value: usize) -> Result<u32, FontError> {
    u32::try_from(value).map_err(|_| FontError::OutputTooLarge)
}

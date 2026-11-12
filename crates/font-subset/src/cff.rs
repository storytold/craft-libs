//! Renames the font in a CFF (version 1) table: the Name INDEX entry, the
//! FullName and FamilyName strings, and the old names inside the strings
//! that name the font (BaseFontName, and the FDArray FontNames in which CID
//! fonts repeat the font name). Other strings stay as they are; if one still
//! names the upstream font, renaming fails.
//!
//! The rewritten strings change the length of the data before the Global
//! Subr INDEX, so every absolute offset moves by the same amount. Top DICT
//! offsets are rewritten in place as 5-byte integers. The FDArray holds
//! absolute Private offsets too; it is written again at the end of the table
//! and its old copy stays unreferenced.

use std::ops::RangeInclusive;

use super::Error;
use super::rename::Banned;

/// A CFF table that does not parse; the text names the part.
#[derive(Debug, thiserror::Error)]
#[error("malformed CFF table: {0}")]
pub struct CffError(&'static str);

pub(crate) struct CffNames<'a> {
    pub ps: &'a str,
    pub full: &'a str,
    pub family: &'a str,
    /// What no string outside the protected ones and the glyph names may
    /// still contain after renaming.
    pub banned: &'a Banned,
}

const OP_VERSION: u16 = 0;
const OP_NOTICE: u16 = 1;
const OP_FULL_NAME: u16 = 2;
const OP_FAMILY_NAME: u16 = 3;
const OP_CHARSET: u16 = 15;
const OP_ENCODING: u16 = 16;
const OP_CHAR_STRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_COPYRIGHT: u16 = 0x0c00;
const OP_BASE_FONT_NAME: u16 = 0x0c16;
const OP_ROS: u16 = 0x0c1e;
const OP_FD_ARRAY: u16 = 0x0c24;
const OP_FD_SELECT: u16 = 0x0c25;
const OP_FONT_NAME: u16 = 0x0c26;
/// SIDs below this are the standard strings; the String INDEX starts here.
const STD_STRINGS: usize = 391;

struct Operand {
    raw: Vec<u8>,
    value: Option<i32>,
}

struct Entry {
    operands: Vec<Operand>,
    op: u16,
}

/// The parts of a CFF table before the Global Subr INDEX.
struct Head<'a> {
    hdr_size: usize,
    name: &'a [u8],
    top: Vec<Entry>,
    strings: Vec<&'a [u8]>,
    gsubrs_at: usize,
}

impl<'a> Head<'a> {
    fn parse(cff: &'a [u8]) -> Result<Self, CffError> {
        let hdr_size = usize::from(*cff.get(2).ok_or(CffError("header"))?);
        let (name_index, p1) = read_index(cff, hdr_size)?;
        let (top_index, p2) = read_index(cff, p1)?;
        let (strings, gsubrs_at) = read_index(cff, p2)?;
        let ([name], [top]) = (name_index.as_slice(), top_index.as_slice()) else {
            return Err(CffError("expected one font"));
        };
        Ok(Self { hdr_size, name, top: parse_dict(top)?, strings, gsubrs_at })
    }

    fn string(&self, sid: Option<i32>) -> Option<String> {
        let i = usize::try_from(sid?).ok()?.checked_sub(STD_STRINGS)?;
        self.strings.get(i).map(|s| lossy(s))
    }
}

fn lossy(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

/// The first operand of `op` in `dict`.
fn operand(dict: &[Entry], op: u16) -> Option<i32> {
    dict.iter().find(|e| e.op == op).and_then(|e| e.operands.first()?.value)
}

/// The family names the table holds: the family part of the Name INDEX
/// entry (before its first `-`) and the FamilyName string.
pub(crate) fn family_names(cff: &[u8]) -> Result<Vec<String>, CffError> {
    let head = Head::parse(cff)?;
    let ps = lossy(head.name);
    let ps_family = ps.split('-').next().map(str::to_owned);
    Ok([ps_family, head.string(operand(&head.top, OP_FAMILY_NAME))].into_iter().flatten().filter(|s| !s.is_empty()).collect())
}

pub(crate) fn rename(cff: &[u8], names: &CffNames) -> Result<Vec<u8>, Error> {
    let head = Head::parse(cff)?;
    let top = &head.top;
    let fd_array = match operand(top, OP_FD_ARRAY) {
        Some(at) => {
            let at = usize::try_from(at).map_err(|_| CffError("FDArray"))?;
            read_index(cff, at)?.0.into_iter().map(parse_dict).collect::<Result<Vec<_>, _>>()?
        }
        None => Vec::new(),
    };

    let mut replacements: Vec<(String, &str)> = [
        (Some(lossy(head.name)), names.ps),
        (head.string(operand(top, OP_FULL_NAME)), names.full),
        (head.string(operand(top, OP_FAMILY_NAME)), names.family),
    ]
    .into_iter()
    .filter_map(|(old, new)| Some((old.filter(|o| !o.is_empty())?, new)))
    .collect();
    replacements.sort_by_key(|(old, _)| std::cmp::Reverse(old.len()));

    let font_names: Vec<i32> =
        operand(top, OP_BASE_FONT_NAME).into_iter().chain(fd_array.iter().filter_map(|fd| operand(fd, OP_FONT_NAME))).collect();
    let protected: Vec<i32> = [OP_VERSION, OP_NOTICE, OP_COPYRIGHT].into_iter().filter_map(|op| operand(top, op)).collect();
    let glyph_names = glyph_name_sids(cff, top)?;
    let sid_of = |i: usize| i32::try_from(STD_STRINGS + i).unwrap_or(i32::MAX);
    let strings: Vec<Vec<u8>> = head
        .strings
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let this = Some(sid_of(i));
            if this == operand(top, OP_FULL_NAME) {
                names.full.as_bytes().to_vec()
            } else if this == operand(top, OP_FAMILY_NAME) {
                names.family.as_bytes().to_vec()
            } else if font_names.contains(&sid_of(i)) {
                replace_all(s, &replacements)
            } else {
                s.to_vec()
            }
        })
        .collect();

    let mut new_names = [names.full, names.family, names.ps];
    new_names.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for (i, s) in strings.iter().enumerate() {
        let this = sid_of(i);
        if protected.contains(&this) || glyph_names.iter().any(|r| r.contains(&this)) {
            continue;
        }
        let rest = new_names.iter().fold(lossy(s), |text, new| text.replace(new, ""));
        if names.banned.found_in(&rest) {
            return Err(Error::CffOldName(lossy(s)));
        }
    }

    let mut out = cff[..head.hdr_size].to_vec();
    out.extend(write_index(&[names.ps.as_bytes().to_vec()]));
    let top_len = write_index(&[encode_dict(top, 0, 0)?]).len();
    let strings = write_index(&strings);
    let prefix_len = out.len() + top_len + strings.len();
    let delta = prefix_len as i64 - head.gsubrs_at as i64;
    let tail = &cff[head.gsubrs_at..];
    let fd_array_at = (prefix_len + tail.len()) as i64;

    out.extend(write_index(&[encode_dict(top, delta, fd_array_at)?]));
    out.extend(strings);
    out.extend_from_slice(tail);
    if !fd_array.is_empty() {
        let fonts = fd_array.iter().map(|fd| encode_dict(fd, delta, 0)).collect::<Result<Vec<_>, _>>()?;
        out.extend(write_index(&fonts));
    }
    Ok(out)
}

/// The SIDs a custom charset gives as glyph names. CID-keyed fonts hold CIDs
/// there instead, and the predefined charsets use standard strings only.
fn glyph_name_sids(cff: &[u8], top: &[Entry]) -> Result<Vec<RangeInclusive<i32>>, CffError> {
    let Some(at) = operand(top, OP_CHARSET).filter(|&at| at > 2) else {
        return Ok(Vec::new());
    };
    if top.iter().any(|e| e.op == OP_ROS) {
        return Ok(Vec::new());
    }
    let to_usize = |v: i32| usize::try_from(v).map_err(|_| CffError("charset"));
    let char_strings = operand(top, OP_CHAR_STRINGS).ok_or(CffError("CharStrings"))?;
    let glyphs = usize::from(u16_at(cff, to_usize(char_strings)?)?);
    let mut p = to_usize(at)?;
    let format = *cff.get(p).ok_or(CffError("charset"))?;
    p += 1;
    let mut ranges = Vec::new();
    // Glyph 0 is .notdef and has no entry.
    let mut covered = 1;
    while covered < glyphs {
        let first = i32::from(u16_at(cff, p)?);
        let left = match format {
            0 => {
                p += 2;
                0
            }
            1 => {
                let left = *cff.get(p + 2).ok_or(CffError("charset"))?;
                p += 3;
                u16::from(left)
            }
            2 => {
                let left = u16_at(cff, p + 2)?;
                p += 4;
                left
            }
            _ => return Err(CffError("charset format")),
        };
        ranges.push(first..=first + i32::from(left));
        covered += usize::from(left) + 1;
    }
    Ok(ranges)
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, CffError> {
    match data.get(at..at.checked_add(2).ok_or(CffError("offset"))?) {
        Some(&[a, b]) => Ok(u16::from_be_bytes([a, b])),
        _ => Err(CffError("offset")),
    }
}

fn replace_all(s: &[u8], replacements: &[(String, &str)]) -> Vec<u8> {
    let mut text = lossy(s);
    for (old, new) in replacements {
        text = text.replace(old.as_str(), new);
    }
    text.into_bytes()
}

/// Writes a DICT with every absolute offset moved by `delta` and the FDArray
/// offset set to `fd_array_at`. Offsets are written as 5-byte integers so
/// the DICT's length does not depend on their values.
fn encode_dict(entries: &[Entry], delta: i64, fd_array_at: i64) -> Result<Vec<u8>, CffError> {
    let mut out = Vec::new();
    for entry in entries {
        let last = entry.operands.len().saturating_sub(1);
        for (i, operand) in entry.operands.iter().enumerate() {
            let new = match (entry.op, operand.value) {
                (OP_CHARSET, Some(v)) if v > 2 => Some(i64::from(v) + delta),
                (OP_ENCODING, Some(v)) if v > 1 => Some(i64::from(v) + delta),
                (OP_CHAR_STRINGS | OP_FD_SELECT, Some(v)) => Some(i64::from(v) + delta),
                (OP_PRIVATE, Some(v)) if i == last => Some(i64::from(v) + delta),
                (OP_FD_ARRAY, Some(_)) => Some(fd_array_at),
                (OP_CHARSET | OP_ENCODING | OP_PRIVATE, Some(v)) => Some(i64::from(v)),
                _ => None,
            };
            match new {
                Some(v) => {
                    let v = i32::try_from(v).map_err(|_| CffError("offset out of range"))?;
                    out.push(29);
                    out.extend(v.to_be_bytes());
                }
                None => out.extend(&operand.raw),
            }
        }
        if entry.op > 0xff {
            out.push(12);
        }
        out.push(entry.op as u8);
    }
    Ok(out)
}

fn parse_dict(data: &[u8]) -> Result<Vec<Entry>, CffError> {
    let mut entries = Vec::new();
    let mut operands = Vec::new();
    let mut i = 0;
    while let Some(&b0) = data.get(i) {
        let len = match b0 {
            12 | 247..=254 => 2,
            28 => 3,
            29 => 5,
            30 => real_len(&data[i..])?,
            0..=21 | 32..=246 => 1,
            _ => return Err(CffError("DICT operand")),
        };
        let raw = data.get(i..i + len).ok_or(CffError("DICT"))?;
        let at = |k: usize| i32::from(raw[k]);
        let value = match b0 {
            28 => Some(i32::from(i16::from_be_bytes([raw[1], raw[2]]))),
            29 => Some(i32::from_be_bytes([raw[1], raw[2], raw[3], raw[4]])),
            32..=246 => Some(at(0) - 139),
            247..=250 => Some((at(0) - 247) * 256 + at(1) + 108),
            251..=254 => Some(-(at(0) - 251) * 256 - at(1) - 108),
            _ => None,
        };
        if b0 <= 21 {
            let op = if b0 == 12 { 0x0c00 | u16::from(raw[1]) } else { u16::from(b0) };
            entries.push(Entry { operands: std::mem::take(&mut operands), op });
        } else {
            operands.push(Operand { raw: raw.to_vec(), value });
        }
        i += len;
    }
    Ok(entries)
}

/// Length of a real-number operand: the 30 byte, then nibbles up to 0xf.
fn real_len(data: &[u8]) -> Result<usize, CffError> {
    let end = data[1..].iter().position(|b| b & 0x0f == 0x0f || b >> 4 == 0x0f).ok_or(CffError("real"))?;
    Ok(end + 2)
}

fn read_index(data: &[u8], at: usize) -> Result<(Vec<&[u8]>, usize), CffError> {
    let count = usize::from(u16_at(data, at).map_err(|_| CffError("INDEX"))?);
    if count == 0 {
        return Ok((Vec::new(), at + 2));
    }
    let off_size = usize::from(*data.get(at + 2).ok_or(CffError("INDEX"))?);
    if !(1..=4).contains(&off_size) {
        return Err(CffError("INDEX offset size"));
    }
    let offsets_at = at + 3;
    let offsets_end = offsets_at.checked_add((count + 1) * off_size).ok_or(CffError("INDEX"))?;
    let offsets = data.get(offsets_at..offsets_end).ok_or(CffError("INDEX"))?;
    let offset = |i: usize| offsets[i * off_size..(i + 1) * off_size].iter().fold(0usize, |acc, &b| acc << 8 | usize::from(b));
    if offset(0) != 1 {
        return Err(CffError("INDEX first offset"));
    }
    // Offsets count from the byte before the data.
    let data_at = offsets_end - 1;
    let at_offset = |o: usize| data_at.checked_add(o).ok_or(CffError("INDEX"));
    let mut items = Vec::with_capacity(count);
    let mut start = 1;
    for i in 1..=count {
        let end = offset(i);
        if end < start {
            return Err(CffError("INDEX offsets decrease"));
        }
        items.push(data.get(at_offset(start)?..at_offset(end)?).ok_or(CffError("INDEX"))?);
        start = end;
    }
    Ok((items, at_offset(start)?))
}

fn write_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    let total: usize = items.iter().map(Vec::len).sum::<usize>() + 1;
    let off_size = match total {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x10000..=0xff_ffff => 3,
        _ => 4,
    };
    out.push(off_size as u8);
    let mut offset = 1usize;
    let push_offset = |out: &mut Vec<u8>, v: usize| out.extend(&(v as u32).to_be_bytes()[4 - off_size..]);
    push_offset(&mut out, offset);
    for item in items {
        offset += item.len();
        push_offset(&mut out, offset);
    }
    for item in items {
        out.extend(item);
    }
    out
}

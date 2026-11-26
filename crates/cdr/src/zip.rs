//! The ZIP archives of `.cdr` drawings from version 14 on (R24): the central directory, then stored
//! and deflated members read on request. Archives that span disks, ZIP64 archives and encrypted
//! members are refused; so are directories whose members overlap or repeat a name.

use std::borrow::Cow;
use std::io::Read as _;

use crate::Error;
use crate::bytes::Rd;

/// End of central directory record: signature and fixed length.
const END: u32 = 0x0605_4b50;
const END_LEN: usize = 22;
/// The longest archive comment, which the end record may be followed by.
const MAX_COMMENT: usize = 0xFFFF;
/// ZIP64 end of central directory locator, 20 bytes before the end record when present.
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
/// Central directory header: signature and fixed length.
const CENTRAL: u32 = 0x0201_4b50;
const CENTRAL_LEN: usize = 46;
/// Local file header: signature and fixed length.
const LOCAL: u32 = 0x0403_4b50;
const LOCAL_LEN: usize = 30;
/// General purpose flag bit 0: the member is encrypted.
const ENCRYPTED: u16 = 1;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
/// What members packed by another method are refused as.
const OTHER_METHODS: &str = "package compression methods other than storing and deflate";

/// One member, as the central directory and its local header give it.
#[derive(Clone, Copy, Debug)]
struct Entry<'a> {
    name: &'a [u8],
    flags: u16,
    method: u16,
    crc: u32,
    packed: usize,
    size: usize,
    /// Where its local header starts and where its packed bytes start.
    header: usize,
    data: usize,
}

/// An archive's directory over its bytes.
#[derive(Debug)]
pub(crate) struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry<'a>>,
    /// The entries in name order, for lookups.
    by_name: Vec<usize>,
}

/// Does `bytes` start with a local file header?
pub(crate) fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

impl<'a> Archive<'a> {
    /// The directory of `bytes`, holding at most `max_entries` members.
    pub(crate) fn open(bytes: &'a [u8], max_entries: usize) -> Result<Self, Error> {
        let end = find_end(bytes).ok_or(Error::Damaged("package directory"))?;
        if end >= 20 && Rd::at(bytes, end - 20).u32() == Some(ZIP64_LOCATOR) {
            return Err(Error::Unsupported("ZIP64 packages"));
        }
        let mut r = Rd::at(bytes, end + 4);
        let dir = || Error::Damaged("package directory");
        let (disk, dir_disk) = (r.u16().ok_or_else(dir)?, r.u16().ok_or_else(dir)?);
        let (on_disk, total) = (r.u16().ok_or_else(dir)?, r.u16().ok_or_else(dir)?);
        let (dir_len, dir_at) = (r.u32().ok_or_else(dir)?, r.u32().ok_or_else(dir)?);
        if total == u16::MAX || dir_len == u32::MAX || dir_at == u32::MAX {
            return Err(Error::Unsupported("ZIP64 packages"));
        }
        if disk != 0 || dir_disk != 0 || on_disk != total {
            return Err(Error::Unsupported("packages split across files"));
        }
        let total = usize::from(total);
        if total > max_entries {
            return Err(Error::Limit("package member count"));
        }
        let (dir_at, dir_len) = (dir_at as usize, dir_len as usize);
        let dir_end = dir_at.checked_add(dir_len).filter(|&e| e <= end).ok_or_else(dir)?;
        let mut r = Rd::at(bytes, dir_at);
        let mut entries = Vec::with_capacity(total);
        for _ in 0..total {
            let e = central_entry(bytes, &mut r, dir_end)?;
            entries.push(e);
        }
        // Each member's bytes end before the next member's header and before the directory.
        let mut order: Vec<&Entry> = entries.iter().collect();
        order.sort_unstable_by_key(|e| e.header);
        for (i, e) in order.iter().enumerate() {
            let stop = order.get(i + 1).map_or(dir_at, |n| n.header);
            if e.data.checked_add(e.packed).is_none_or(|x| x > stop) {
                return Err(Error::Damaged("package members overlap"));
            }
        }
        let mut by_name: Vec<usize> = (0..entries.len()).collect();
        by_name.sort_unstable_by_key(|&i| entries.get(i).map(|e| e.name));
        let name = |i: &usize| entries.get(*i).map(|e| e.name);
        if by_name.windows(2).any(|w| name(&w[0]) == name(&w[1])) {
            return Err(Error::Damaged("package member named twice"));
        }
        Ok(Archive { bytes, entries, by_name })
    }

    /// The member named `name`.
    pub(crate) fn find(&self, name: &[u8]) -> Option<usize> {
        let at = self.by_name.binary_search_by(|&i| self.entries.get(i).map(|e| e.name).cmp(&Some(name))).ok()?;
        self.by_name.get(at).copied()
    }

    /// The size member `i`'s directory entry gives.
    pub(crate) fn size(&self, i: usize) -> usize {
        self.entries.get(i).map_or(0, |e| e.size)
    }

    /// How many members the archive holds.
    pub(crate) fn count(&self) -> usize {
        self.entries.len()
    }

    /// The bytes reading member `i` may inflate: its stated size when it is deflated, else 0.
    pub(crate) fn inflate_cost(&self, i: usize) -> usize {
        self.entries.get(i).filter(|e| e.method == DEFLATED).map_or(0, |e| e.size)
    }

    /// An error when member `i` is encrypted or packed by a method other than storing or deflate.
    pub(crate) fn check(&self, i: usize) -> Result<(), Error> {
        let e = self.entries.get(i).ok_or(Error::Damaged("package member"))?;
        if e.flags & ENCRYPTED != 0 {
            return Err(Error::Unsupported("encrypted packages"));
        }
        if e.method != STORED && e.method != DEFLATED {
            return Err(Error::Unsupported(OTHER_METHODS));
        }
        Ok(())
    }

    /// Member `i`'s bytes when they come to at most `limit`, and a deflated member's when they
    /// also come to at most `inflate_left`. A stored member is borrowed; a deflated one inflates to
    /// at most its stated size plus one byte, so a member that inflates past what it states stops
    /// there. The length and CRC-32 must match the directory's.
    pub(crate) fn read(&self, i: usize, limit: usize, inflate_left: usize) -> Result<Cow<'a, [u8]>, Error> {
        self.check(i)?;
        let e = self.entries.get(i).ok_or(Error::Damaged("package member"))?;
        if e.size > limit {
            return Err(Error::Limit("package member size"));
        }
        if e.method == DEFLATED && e.size > inflate_left {
            return Err(Error::Limit("inflated size"));
        }
        let packed = self.bytes.get(e.data..e.data + e.packed).ok_or(Error::Damaged("package member"))?;
        let out: Cow<'a, [u8]> = match e.method {
            STORED if e.packed == e.size => Cow::Borrowed(packed),
            STORED => return Err(Error::Damaged("package member")),
            DEFLATED => {
                let mut out = Vec::new();
                let got = flate2::read::DeflateDecoder::new(packed)
                    .take(e.size as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|_| Error::Damaged("package member"))?;
                if got != e.size {
                    return Err(Error::Damaged("package member"));
                }
                Cow::Owned(out)
            }
            _ => return Err(Error::Unsupported(OTHER_METHODS)),
        };
        if crc32fast::hash(&out) != e.crc {
            return Err(Error::Damaged("package member"));
        }
        Ok(out)
    }
}

/// Where the end of central directory record starts: searched back from the end over at most the
/// longest comment, among the records whose comment reaches exactly to the end the first that
/// names members in a directory before it, else the first. A comment or a member may hold bytes
/// that look like an empty archive's end record.
fn find_end(bytes: &[u8]) -> Option<usize> {
    let last = bytes.len().checked_sub(END_LEN)?;
    let first = last.saturating_sub(MAX_COMMENT);
    let mut found = None;
    for at in (first..=last).rev() {
        let mut r = Rd::at(bytes, at);
        if r.u32() != Some(END) || r.skip(6).is_none() {
            continue;
        }
        let (total, dir_len, dir_at, comment) = (r.u16(), r.u32(), r.u32(), r.u16());
        if !comment.is_some_and(|c| at + END_LEN + usize::from(c) == bytes.len()) {
            continue;
        }
        let names_members = total.is_some_and(|t| t > 0)
            && dir_len.zip(dir_at).is_some_and(|(len, dir)| {
                (dir as usize).checked_add(len as usize).is_some_and(|e| e <= at) && Rd::at(bytes, dir as usize).u32() == Some(CENTRAL)
            });
        if names_members {
            return Some(at);
        }
        found = found.or(Some(at));
    }
    found
}

/// The central directory entry at `r` (before `dir_end`) and its local header.
fn central_entry<'a>(bytes: &'a [u8], r: &mut Rd<'a>, dir_end: usize) -> Result<Entry<'a>, Error> {
    let dir = || Error::Damaged("package directory");
    if r.pos().checked_add(CENTRAL_LEN).is_none_or(|e| e > dir_end) || r.u32() != Some(CENTRAL) {
        return Err(dir());
    }
    r.skip(4).ok_or_else(dir)?;
    let flags = r.u16().ok_or_else(dir)?;
    let method = r.u16().ok_or_else(dir)?;
    r.skip(4).ok_or_else(dir)?;
    let crc = r.u32().ok_or_else(dir)?;
    let (packed, size) = (r.u32().ok_or_else(dir)?, r.u32().ok_or_else(dir)?);
    let name_len = usize::from(r.u16().ok_or_else(dir)?);
    let extra_len = usize::from(r.u16().ok_or_else(dir)?);
    let comment_len = usize::from(r.u16().ok_or_else(dir)?);
    let disk = r.u16().ok_or_else(dir)?;
    r.skip(6).ok_or_else(dir)?;
    let header = r.u32().ok_or_else(dir)?;
    if packed == u32::MAX || size == u32::MAX || header == u32::MAX {
        return Err(Error::Unsupported("ZIP64 packages"));
    }
    if disk != 0 {
        return Err(Error::Unsupported("packages split across files"));
    }
    let name = r.bytes(name_len).ok_or_else(dir)?;
    r.skip(extra_len).and_then(|()| r.skip(comment_len)).ok_or_else(dir)?;
    if r.pos() > dir_end {
        return Err(dir());
    }
    // The local header repeats the name; its own extra field may differ in length.
    let header = header as usize;
    let mut l = Rd::at(bytes, header);
    let local = || Error::Damaged("package member header");
    if l.u32() != Some(LOCAL) {
        return Err(local());
    }
    // The signature, 22 bytes, then the name and extra field lengths.
    l.skip(LOCAL_LEN - 8).ok_or_else(local)?;
    let (local_name, local_extra) = (usize::from(l.u16().ok_or_else(local)?), usize::from(l.u16().ok_or_else(local)?));
    if l.bytes(local_name) != Some(name) {
        return Err(local());
    }
    l.skip(local_extra).ok_or_else(local)?;
    Ok(Entry { name, flags, method, crc, packed: packed as usize, size: size as usize, header, data: l.pos() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{Member, zip};

    fn stored(name: &str, data: &[u8]) -> Member {
        Member { name: name.into(), data: data.to_vec(), deflate: false }
    }

    #[test]
    fn stored_and_deflated_members_read_back() {
        let bytes = zip(&[stored("a", b"hello"), Member { name: "b".into(), data: vec![7; 5000], deflate: true }], b"");
        let a = Archive::open(&bytes, 10).unwrap();
        assert!(matches!(a.read(a.find(b"a").unwrap(), 100, 100).unwrap(), Cow::Borrowed(b"hello")));
        assert_eq!(a.read(a.find(b"b").unwrap(), 5000, 5000).unwrap().as_ref(), &[7; 5000][..]);
        assert_eq!(a.read(a.find(b"b").unwrap(), 4999, 5000), Err(Error::Limit("package member size")));
        assert_eq!(a.find(b"c"), None);
        // What is left to inflate holds back deflated members only.
        assert_eq!(a.read(a.find(b"b").unwrap(), 5000, 4999), Err(Error::Limit("inflated size")));
        assert!(a.read(a.find(b"a").unwrap(), 100, 0).is_ok());
    }

    #[test]
    fn the_end_record_is_found_behind_a_comment() {
        let bytes = zip(&[stored("a", b"x")], &[b'c'; 300]);
        assert!(Archive::open(&bytes, 10).is_ok());
        // A comment longer than the record says leaves no end record to find.
        let mut cut = bytes.clone();
        cut.push(0);
        assert_eq!(Archive::open(&cut, 10).unwrap_err(), Error::Damaged("package directory"));
        // A comment that ends in bytes like an empty archive's end record: the record that names
        // members in a directory is the archive's.
        let mut fake = END.to_le_bytes().to_vec();
        fake.resize(END_LEN, 0);
        let bytes = zip(&[stored("a", b"x")], &fake);
        let a = Archive::open(&bytes, 10).unwrap();
        assert_eq!(a.count(), 1);
    }

    #[test]
    fn a_member_whose_crc_or_length_is_wrong_is_damage() {
        let bytes = zip(&[Member { name: "a".into(), data: vec![1; 64], deflate: true }], b"");
        let a = Archive::open(&bytes, 10).unwrap();
        // The directory's CRC-32 sits 16 bytes into its entry.
        let dir_at = bytes.windows(4).position(|w| w == CENTRAL.to_le_bytes()).unwrap();
        let mut bad = bytes.clone();
        bad[dir_at + 16] ^= 1;
        let b = Archive::open(&bad, 10).unwrap();
        assert_eq!(b.read(0, 1000, 1000).unwrap_err(), Error::Damaged("package member"));
        // A stated size one short: the inflate runs past it.
        let mut short = bytes.clone();
        short[dir_at + 24] -= 1;
        let s = Archive::open(&short, 10).unwrap();
        assert_eq!(s.read(0, 1000, 1000).unwrap_err(), Error::Damaged("package member"));
        assert!(a.read(0, 1000, 1000).is_ok());
    }
}

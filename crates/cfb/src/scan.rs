//! A lenient look at the directory, for telling what a file is when it may be too damaged to
//! open: only the header, the DIFAT and the directory chain are followed. No FAT is built and
//! nothing is allocated beyond the names themselves.

use crate::read::{Limits, entry_name, span, u16_at, u32_at};
use crate::{ENDOFCHAIN, MAXREGSECT, compare_names, sniff};

/// The names of the directory entries of a compound file, in directory order (the root entry
/// first; unallocated entries, which have no name, are left out), or `None` when `bytes` is not a
/// compound file or its directory cannot be followed.
///
/// More forgiving than [`crate::CompoundFile::open`]: the byte order mark, the FAT sector count,
/// the mini FAT, the mini stream and the sibling trees are not looked at, so entries no storage
/// links to are listed too. A directory chain that loops, or that holds more than
/// [`Limits::max_dir_entries`] entries, gives the names read up to that point.
pub fn directory_names(bytes: &[u8], limits: &Limits) -> Option<Vec<String>> {
    if bytes.len() < 512 || !sniff(bytes) {
        return None;
    }
    let shift = u16_at(bytes, 30);
    if shift != 9 && shift != 12 {
        return None;
    }
    let size = 1usize << shift;
    let per = size / 4;
    // No chain is longer than the file has sectors: the bound on both walks below.
    let max_sectors = bytes.len() / size + 1;
    let sector = |id: u32| -> Option<&[u8]> {
        let (start, end) = span(usize::try_from(id).ok()?, 1, size)?;
        bytes.get(start..end)
    };
    // The DIFAT: the FAT's sector ids, 109 in the header then chained DIFAT sectors.
    let mut difat: Vec<u32> = (0..109).map(|i| u32_at(bytes, 76 + i * 4)).filter(|&s| s <= MAXREGSECT).collect();
    let mut next = u32_at(bytes, 68);
    for _ in 0..max_sectors {
        if next > MAXREGSECT {
            break;
        }
        let s = sector(next)?;
        difat.extend((0..per - 1).map(|i| u32_at(s, i * 4)).filter(|&x| x <= MAXREGSECT));
        next = u32_at(s, (per - 1) * 4);
    }
    let fat_next = |id: u32| -> Option<u32> {
        let idx = usize::try_from(id).ok()?;
        Some(u32_at(sector(*difat.get(idx / per)?)?, (idx % per) * 4))
    };
    let mut names = Vec::new();
    let mut id = u32_at(bytes, 48);
    for _ in 0..max_sectors {
        if id == ENDOFCHAIN {
            return Some(names);
        }
        if id > MAXREGSECT {
            return None;
        }
        for e in sector(id)?.as_chunks::<128>().0 {
            // an allocated entry's name length counts its terminating NUL: 2 to 64 bytes
            if !(2..=64).contains(&u16_at(e, 64)) {
                continue;
            }
            if names.len() >= limits.max_dir_entries {
                return Some(names);
            }
            names.push(entry_name(e));
        }
        id = fat_next(id)?;
    }
    // A cyclic chain: report what was read.
    Some(names)
}

/// Whether the compound file `bytes` holds a directory entry called `name` (compared as sibling
/// names are, so case-insensitively), at any depth. `false` when `bytes` does not start with the
/// compound file signature.
///
/// When the directory cannot be followed (a truncated or damaged file), falls back to searching
/// the bytes for the name in UTF-16LE, so a damaged file still answers. That fallback is
/// case-sensitive and can match stream contents: use it to choose an error message, not to make a
/// security decision.
pub fn has_entry_named(bytes: &[u8], name: &str, limits: &Limits) -> bool {
    if !sniff(bytes) {
        return false;
    }
    match directory_names(bytes, limits) {
        Some(names) => names.iter().any(|n| compare_names(n, name).is_eq()),
        None => {
            let needle: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            !needle.is_empty() && bytes.windows(needle.len()).any(|w| w == needle.as_slice())
        }
    }
}

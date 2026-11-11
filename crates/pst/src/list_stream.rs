//! The stream a personal distribution list keeps its members in once they pass about 15,000
//! bytes (PidLidDistributionListStream, [MS-OXOCNTC] 2.2.2.2.4): a header, then one
//! DistListMemberInfo (2.2.2.2.4.1) for each member, each an entry id and a one-off entry id,
//! then two terminators.
//!
//! The bytes are hostile: the count of members is checked against the bytes there are, every
//! length against what is left, nothing is allocated by what the stream says, and at most
//! `max_members` members are kept.

use crate::bytes::{slice, u16_at, u32_at};
use crate::error::{Error, Result};

/// The header: StreamVersion, Reserved, BuildVersion, DistListStreamFlags, CountOfEntries,
/// TotalEntryIDSize, TotalOneOffSize, TotalExtraSize.
const HEADER: usize = 28;
/// The least a member takes: its three sizes.
const LEAST_MEMBER: usize = 12;
/// Longest an entry id of a member may be (one-off and wrapped entry ids are small).
pub const MAX_LIST_ENTRY: usize = 16 * 1024;

/// One member of the stream: its entry id (a one-off entry id or a WrappedEntryId) and its
/// one-off entry id (empty when it has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMember<'a> {
    pub entry_id: &'a [u8],
    pub one_off: &'a [u8],
}

/// The members a stream holds, as far as they could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListStream<'a> {
    /// The members read, at most the cap given.
    pub members: Vec<ListMember<'a>>,
    /// Members read past the cap, counted and not kept.
    pub more: usize,
    /// Members the stream says it has and that couldn't be read: it ends, or a length runs past
    /// it, before them.
    pub damaged: usize,
    /// Whether the totals of the header disagree with the members read, or the members carry
    /// extra information, which they are not to.
    pub totals_differ: bool,
}

const SHORT: Error = Error::Corrupt("a distribution list's stream is shorter than its header");
const VERSION: Error = Error::Corrupt("a distribution list's stream of a version this reader doesn't know");
const COUNT: Error = Error::Corrupt("a distribution list's stream counts more members than it has room for");

/// The members of a distribution list's stream. Fails when the header can't be believed (too
/// short, another version, more members than the bytes could hold); a member that runs past the
/// end stops the reading there, and the members before it are kept.
pub fn list_stream(b: &[u8], max_members: usize) -> Result<ListStream<'_>> {
    if b.len() < HEADER {
        return Err(SHORT);
    }
    if u16_at(b, 0)? != 1 {
        return Err(VERSION);
    }
    let count = usize::try_from(u32_at(b, 12)?).map_err(|_| COUNT)?;
    let total_ids = u32_at(b, 16)?;
    let total_one_offs = u32_at(b, 20)?;
    let total_extra = u32_at(b, 24)?;
    let room = b.len().saturating_sub(HEADER) / LEAST_MEMBER;
    if count > room {
        return Err(COUNT);
    }
    let mut out = ListStream { members: Vec::new(), more: 0, damaged: 0, totals_differ: false };
    let (mut ids, mut one_offs, mut extra) = (0u64, 0u64, 0u64);
    let mut at = HEADER;
    for read in 0..count {
        let Some((member, extra_len, next)) = member(b, at) else {
            out.damaged = count.saturating_sub(read);
            break;
        };
        at = next;
        ids = ids.saturating_add(member.entry_id.len() as u64);
        one_offs = one_offs.saturating_add(member.one_off.len() as u64);
        extra = extra.saturating_add(extra_len as u64);
        if out.members.len() < max_members {
            out.members.push(member);
        } else {
            out.more = out.more.saturating_add(1);
        }
    }
    out.totals_differ = out.damaged == 0 && (ids != u64::from(total_ids) || one_offs != u64::from(total_one_offs) || extra != 0 || total_extra != 0);
    Ok(out)
}

/// The member at `at`, how long its extra information is, and where the next begins, when it
/// fits in `b`. ExtraMemberInfoSize is to be 0; what it names is stepped over when it fits.
fn member(b: &[u8], at: usize) -> Option<(ListMember<'_>, usize, usize)> {
    let (entry_id, at) = sized(b, at)?;
    let (one_off, at) = sized(b, at)?;
    let (extra, at) = sized(b, at)?;
    Some((ListMember { entry_id, one_off }, extra.len(), at))
}

/// The bytes a 4-byte size at `at` names, at most [`MAX_LIST_ENTRY`], and where they end.
fn sized(b: &[u8], at: usize) -> Option<(&[u8], usize)> {
    let len = usize::try_from(u32_at(b, at).ok()?).ok()?;
    if len > MAX_LIST_ENTRY {
        return None;
    }
    let start = at.checked_add(4)?;
    let bytes = slice(b, start, len).ok()?;
    Some((bytes, start.checked_add(len)?))
}

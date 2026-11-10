//! Node records: a tag word (opcode, variant, flags) followed by the node's fields and the data its fields
//! point to (expressions, keys). A record's extent runs to the next record, or to the defaults block for
//! the last one. Typed layouts per opcode are decoded on top of these raw records.

use super::section::Section;
use crate::decode::DecodeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecord {
    pub index: usize,
    /// Section offset of the tag word.
    pub offset: u32,
    /// Section offset where the record's extent ends.
    pub end: u32,
    /// Low byte of the tag: what the node does.
    pub opcode: u8,
    /// Second byte of the tag (0x88 for most nodes; other values are format/channel variants).
    pub variant: u8,
    /// High half of the tag. Its upper byte marks which optional parameters are present.
    pub flags: u16,
    /// The words after the tag, up to `end`.
    pub body: Vec<u32>,
}

impl NodeRecord {
    /// Section offset of body word `i`.
    pub fn word_offset(&self, i: usize) -> u32 {
        self.offset + 4 + 4 * u32::try_from(i).unwrap_or(u32::MAX / 8)
    }

    pub fn word(&self, i: usize) -> Option<u32> {
        self.body.get(i).copied()
    }

    /// Whether `v` points inside this record (fields store such pointers to their expressions).
    pub fn points_inside(&self, v: u32) -> bool {
        v > self.offset && v < self.end && v.is_multiple_of(4)
    }
}

/// Cuts the records out of the section, given each record's start (in node-table order).
pub(super) fn split(s: &Section<'_>, offsets: &[u32], defaults: u32) -> Result<Vec<NodeRecord>, DecodeError> {
    let mut sorted: Vec<u32> = offsets.to_vec();
    sorted.push(defaults);
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = Vec::with_capacity(offsets.len());
    for (index, &offset) in offsets.iter().enumerate() {
        let end = sorted.iter().copied().find(|&o| o > offset).unwrap_or_else(|| s.len());
        let tag = s.u32(offset)?;
        let n = (end.saturating_sub(offset) / 4).saturating_sub(1);
        out.push(NodeRecord {
            index,
            offset,
            end,
            opcode: (tag & 0xff) as u8,
            variant: ((tag >> 8) & 0xff) as u8,
            flags: (tag >> 16) as u16,
            body: s.words(offset + 4, n)?,
        });
    }
    Ok(out)
}

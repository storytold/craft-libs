//! The heap-on-node ([MS-PST] 2.3.1): a node's data blocks, each with its page map, and the
//! items allocated in them, found by their HID.

use crate::bytes::{slice, u8_at, u16_at, u32_at};
use crate::error::{Error, Result};

const HN_SIGNATURE: u8 = 0xEC;
pub(crate) const CLIENT_TC: u8 = 0x7C;
pub(crate) const CLIENT_BTH: u8 = 0xB5;
pub(crate) const CLIENT_PC: u8 = 0xBC;

pub(crate) struct Heap {
    blocks: Vec<Vec<u8>>,
    pub client: u8,
    pub user_root: u32,
}

/// Whether an HNID names a heap item (else it names a subnode).
pub(crate) fn is_hid(hnid: u32) -> bool {
    hnid & 0x1F == 0
}

impl Heap {
    pub(crate) fn new(blocks: Vec<Vec<u8>>) -> Result<Heap> {
        let first = blocks.first().ok_or(Error::Corrupt("an empty heap"))?;
        if u8_at(first, 2)? != HN_SIGNATURE {
            return Err(Error::Corrupt("a heap's signature"));
        }
        let client = u8_at(first, 3)?;
        let user_root = u32_at(first, 4)?;
        if blocks.len() > 1 << 16 {
            return Err(Error::Corrupt("a heap of too many blocks"));
        }
        // Every page map's offsets run forward and end before the map, so that no two items
        // share a byte and the items of a heap together are no larger than it.
        for block in &blocks {
            let map = usize::from(u16_at(block, 0)?);
            let count = usize::from(u16_at(block, map)?);
            let mut last = 0usize;
            for i in 0..=count {
                let off = i.checked_mul(2).and_then(|o| o.checked_add(map)).and_then(|o| o.checked_add(4)).ok_or(Error::OutOfBounds)?;
                let at = usize::from(u16_at(block, off)?);
                if at < last || at > map {
                    return Err(Error::Corrupt("a heap's page map"));
                }
                last = at;
            }
        }
        Ok(Heap { blocks, client, user_root })
    }

    /// The bytes of all its blocks.
    pub(crate) fn size(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum()
    }

    /// The item `hid`; an HID of zero is an empty item.
    pub(crate) fn item(&self, hid: u32) -> Result<&[u8]> {
        if hid == 0 {
            return Ok(&[]);
        }
        if !is_hid(hid) {
            return Err(Error::Corrupt("a heap id of the wrong type"));
        }
        let index = usize::from(((hid >> 5) & 0x7FF) as u16);
        let block = self.blocks.get((hid >> 16) as usize).ok_or(Error::Corrupt("a heap id past the heap"))?;
        let map = usize::from(u16_at(block, 0)?);
        let count = usize::from(u16_at(block, map)?);
        if index == 0 || index > count {
            return Err(Error::Corrupt("a heap id past the page map"));
        }
        let at = |i: usize| -> Result<usize> {
            let off = i.checked_mul(2).and_then(|o| o.checked_add(map)).and_then(|o| o.checked_add(4)).ok_or(Error::OutOfBounds)?;
            Ok(usize::from(u16_at(block, off)?))
        };
        let start = at(index.saturating_sub(1))?;
        let end = at(index)?;
        if start > end || end > map {
            return Err(Error::Corrupt("a heap item outside its block"));
        }
        slice(block, start, end.saturating_sub(start))
    }
}

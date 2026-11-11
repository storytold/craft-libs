//! The node database ([MS-PST] 2.2): the node and block B-trees (pages of 512 bytes), blocks
//! (at most 8 KB, each with its CRC, signature and id checked), data trees (XBLOCK, XXBLOCK)
//! and subnode trees (SLBLOCK, SIBLOCK).
//!
//! A hostile file is kept bounded: a B-tree is at most 9 pages deep and each page must be one
//! level below the page that points to it, so a lookup ends; a walk of the whole node tree
//! visits a page at most once and at most [`Limits::max_btree_pages`] pages; a data tree is at
//! most two levels, its claimed size is checked against the cap before a block is read, and it
//! may not be spread over more blocks than its size needs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek};

use crate::bytes::{slice, u8_at, u16_at, u32_at};
use crate::crc::crc32;
use crate::crypt::{Encryption, decode};
use crate::error::{Cap, Damage, Error, Limits, MAX_DAMAGE, Result};
use crate::header::{Bref, Format, Header};
use crate::source::Source;

const PAGE: usize = 512;
const PTYPE_BBT: u8 = 0x80;
const PTYPE_NBT: u8 = 0x81;
/// "Up to 8 intermediate levels" (2.2.2.7.7).
const MAX_LEVEL: u8 = 8;
/// A data tree's average block holds at least this much, or it is refused ([`Cap::DataBlocks`]).
const MIN_AVERAGE_BLOCK: u64 = 64;
/// B-tree pages kept once read (512 bytes each), so that the upper pages, which every lookup
/// passes through, are read from the file once.
const CACHED_PAGES: usize = 256;
/// Damaged pages told apart (so that one met again is not counted again); past them the
/// count of [`Damage::pages`] stops, and is a lower bound.
const MAX_NOTED: usize = 1 << 16;

/// A leaf of the node B-tree (`NBTENTRY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeEntry {
    pub nid: u32,
    pub data: u64,
    pub sub: u64,
    pub parent: u32,
}

/// A subnode (`SLENTRY`): its data and its own subnodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SubEntry {
    pub data: u64,
    pub sub: u64,
}

pub(crate) type Subnodes = BTreeMap<u32, SubEntry>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockEntry {
    pub ib: u64,
    pub cb: u16,
}

struct Page {
    data: Vec<u8>,
    count: usize,
    entry_size: usize,
    level: u8,
}

impl Page {
    fn entry(&self, i: usize) -> Result<&[u8]> {
        slice(&self.data, i.checked_mul(self.entry_size).ok_or(Error::OutOfBounds)?, self.entry_size)
    }
}

/// The block signature (5.5).
pub(crate) fn signature(ib: u64, bid: u64) -> u16 {
    let x = ib ^ bid;
    ((x >> 16) as u16) ^ (x as u16)
}

/// Whether a block id names an internal block (an XBLOCK, XXBLOCK, SLBLOCK or SIBLOCK).
pub(crate) fn is_internal(bid: u64) -> bool {
    bid & 2 != 0
}

/// Whether an error is damage to the file (noted and skipped) rather than a cap reached or
/// the reader under the file failing (which end what is being done).
fn is_damage(e: &Error) -> bool {
    !matches!(e, Error::Cap(_) | Error::Io(_))
}

/// The id a block is looked up by: the reserved low bit is ignored (2.2.2.2).
fn key(bid: u64) -> u64 {
    bid & !1
}

pub(crate) struct Ndb<R> {
    src: Source<R>,
    pub header: Header,
    pub limits: Limits,
    /// Pages as read, by offset; checked again each time they are used.
    pages: HashMap<u64, Vec<u8>>,
    /// Pages below a tree's root that could not be read, each once.
    pub damage: Vec<Damage>,
    noted: HashSet<u64>,
}

impl<R: Read + Seek> Ndb<R> {
    pub(crate) fn new(src: Source<R>, header: Header, limits: Limits) -> Self {
        Ndb { src, header, limits, pages: HashMap::new(), damage: Vec::new(), noted: HashSet::new() }
    }

    /// Notes a page below a root that could not be read. A cap or a failure of the reader
    /// under the file is not damage to the file, and is not noted.
    pub(crate) fn note(&mut self, ib: u64, ptype: u8, e: &Error) {
        if !is_damage(e) {
            return;
        }
        if self.damage.len() >= MAX_DAMAGE {
            // Past the cap the pages are counted (once each, to the same cap as the walk's).
            if self.noted.len() < MAX_NOTED
                && self.noted.insert(ib)
                && let Some(last) = self.damage.last_mut()
            {
                last.pages = last.pages.saturating_add(1);
            }
            return;
        }
        if !self.noted.insert(ib) {
            return;
        }
        let what = if ptype == PTYPE_NBT { "a page of the node tree" } else { "a page of the block tree" };
        let pages = 1;
        self.damage.push(Damage { offset: ib, what, reason: e.clone(), pages });
        if self.damage.len() == MAX_DAMAGE {
            // The last entry is the one that counts the rest.
            if let Some(last) = self.damage.last_mut() {
                last.what = if ptype == PTYPE_NBT { "pages of the node tree, from this one" } else { "pages of the block tree, from this one" };
            }
        }
    }

    /// The page at `bref`, at the level `expect` when one is expected.
    fn page_at(&mut self, bref: Bref, ptype: u8, expect: Option<u8>) -> Result<Page> {
        let page = self.read_page(bref, ptype)?;
        if expect.is_some_and(|e| e != page.level) {
            return Err(Error::Corrupt("a B-tree page at the wrong level"));
        }
        Ok(page)
    }

    fn page_bytes(&mut self, ib: u64) -> Result<Vec<u8>> {
        if let Some(p) = self.pages.get(&ib) {
            return Ok(p.clone());
        }
        let data = self.src.read_at(ib, PAGE)?;
        // ponytail: emptied when full, not least-recently-used; the root and upper pages come
        // back at once. An LRU if lookups in very large files show it matters.
        if self.pages.len() >= CACHED_PAGES {
            self.pages.clear();
        }
        self.pages.insert(ib, data.clone());
        Ok(data)
    }

    fn format(&self) -> Format {
        self.header.format
    }

    fn read_page(&mut self, bref: Bref, ptype: u8) -> Result<Page> {
        let f = self.format();
        let data = self.page_bytes(bref.ib)?;
        let t = PAGE.saturating_sub(f.trailer_size());
        let (crc_at, bid_at) = match f {
            Format::Unicode => (t.saturating_add(4), t.saturating_add(8)),
            Format::Ansi => (t.saturating_add(8), t.saturating_add(4)),
        };
        if u8_at(&data, t)? != ptype || u8_at(&data, t.saturating_add(1))? != ptype {
            return Err(Error::Corrupt("a B-tree page of the wrong type"));
        }
        if crc32(slice(&data, 0, t)?) != u32_at(&data, crc_at)? {
            return Err(Error::BadChecksum("a B-tree page"));
        }
        let bid = f.read_id(&data, bid_at)?;
        if key(bid) != key(bref.bid) || u16_at(&data, t.saturating_add(2))? != signature(bref.ib, bid) {
            return Err(Error::BadChecksum("a B-tree page's id"));
        }
        let (area, meta) = match f {
            Format::Unicode => (488, 488),
            Format::Ansi => (496, 496),
        };
        let count = usize::from(u8_at(&data, meta)?);
        let entry_size = usize::from(u8_at(&data, meta.saturating_add(2))?);
        let level = u8_at(&data, meta.saturating_add(3))?;
        // A level the format doesn't have is damage to that page, like a bad count.
        if level > MAX_LEVEL {
            return Err(Error::Corrupt("a B-tree page's level"));
        }
        let id = f.id_size();
        let needed = match (level, ptype) {
            (1.., _) => id.saturating_mul(3),
            (0, PTYPE_NBT) => id.saturating_mul(3).saturating_add(4),
            _ => id.saturating_mul(2).saturating_add(4),
        };
        if entry_size < needed || count.saturating_mul(entry_size) > area {
            return Err(Error::Corrupt("a B-tree page's entries"));
        }
        Ok(Page { data, count, entry_size, level })
    }

    /// The leaf entry of `target` in the tree at `root`, if there is one.
    fn find(&mut self, root: Bref, ptype: u8, target: u64) -> Result<Option<Vec<u8>>> {
        let f = self.format();
        let id = f.id_size();
        let mask = |k: u64| if ptype == PTYPE_BBT { key(k) } else { k };
        let target = mask(target);
        let mut bref = root;
        let mut expect: Option<u8> = None;
        loop {
            // A page below the root that can't be read loses what is under it, not the file:
            // it is noted, and this lookup fails.
            let page = match self.page_at(bref, ptype, expect) {
                Ok(p) => p,
                Err(e) => {
                    if bref != root {
                        self.note(bref.ib, ptype, &e);
                    }
                    return Err(e);
                }
            };
            if page.level == 0 {
                for i in 0..page.count {
                    let e = page.entry(i)?;
                    if mask(f.read_id(e, 0)?) == target {
                        return Ok(Some(e.to_vec()));
                    }
                }
                return Ok(None);
            }
            let mut best: Option<(u64, Bref)> = None;
            for i in 0..page.count {
                let e = page.entry(i)?;
                let k = mask(f.read_id(e, 0)?);
                if k <= target && best.is_none_or(|(b, _)| k >= b) {
                    best = Some((k, f.read_bref(e, id)?));
                }
            }
            match best {
                Some((_, next)) => bref = next,
                None => return Ok(None),
            }
            expect = page.level.checked_sub(1);
        }
    }

    fn node_entry(&self, e: &[u8]) -> Result<NodeEntry> {
        let f = self.format();
        let id = f.id_size();
        Ok(NodeEntry {
            nid: f.read_id(e, 0)? as u32,
            data: f.read_id(e, id)?,
            sub: f.read_id(e, id.saturating_mul(2))?,
            parent: u32_at(e, id.saturating_mul(3))?,
        })
    }

    /// The node `nid` of the node B-tree.
    pub(crate) fn node(&mut self, nid: u32) -> Result<NodeEntry> {
        let root = self.header.nbt;
        match self.find(root, PTYPE_NBT, u64::from(nid))? {
            Some(e) => self.node_entry(&e),
            None => Err(Error::NotFound("a node")),
        }
    }

    pub(crate) fn block_entry(&mut self, bid: u64) -> Result<BlockEntry> {
        let root = self.header.bbt;
        let e = self.find(root, PTYPE_BBT, bid)?.ok_or(Error::NotFound("a block"))?;
        let f = self.format();
        let bref = f.read_bref(&e, 0)?;
        let cb = u16_at(&e, f.id_size().saturating_mul(2))?;
        Ok(BlockEntry { ib: bref.ib, cb })
    }

    /// The bytes of block `bid`, decoded when it holds data.
    pub(crate) fn block(&mut self, bid: u64) -> Result<Vec<u8>> {
        let f = self.format();
        let entry = self.block_entry(bid)?;
        let cb = usize::from(entry.cb);
        let ts = f.trailer_size();
        let total = cb.saturating_add(ts).div_ceil(64).saturating_mul(64);
        let mut raw = self.src.read_at(entry.ib, total)?;
        let t = total.saturating_sub(ts);
        let (crc_at, bid_at) = match f {
            Format::Unicode => (t.saturating_add(4), t.saturating_add(8)),
            Format::Ansi => (t.saturating_add(8), t.saturating_add(4)),
        };
        let stored_bid = f.read_id(&raw, bid_at)?;
        if usize::from(u16_at(&raw, t)?) != cb || key(stored_bid) != key(bid) {
            return Err(Error::BadChecksum("a block's id"));
        }
        if u16_at(&raw, t.saturating_add(2))? != signature(entry.ib, stored_bid) {
            return Err(Error::BadChecksum("a block's signature"));
        }
        if crc32(slice(&raw, 0, cb)?) != u32_at(&raw, crc_at)? {
            return Err(Error::BadChecksum("a block"));
        }
        raw.truncate(cb);
        if !is_internal(bid) {
            // The key of cyclic encoding is the block's id without the reserved bit A, which
            // a file may set on a reference or on the trailer alone.
            decode(self.header.encryption, &mut raw, key(stored_bid));
        }
        Ok(raw)
    }

    /// The size of the data stream at `bid`, from its top block alone.
    pub(crate) fn data_size(&mut self, bid: u64) -> Result<u64> {
        if bid == 0 {
            return Ok(0);
        }
        if !is_internal(bid) {
            return Ok(u64::from(self.block_entry(bid)?.cb));
        }
        let top = self.block(bid)?;
        if u8_at(&top, 0)? != 1 {
            return Err(Error::Corrupt("a data tree's block type"));
        }
        Ok(u64::from(u32_at(&top, 4)?))
    }

    /// The data blocks of the data tree at `bid`, in order, refused past `max` bytes.
    pub(crate) fn data_blocks(&mut self, bid: u64, max: u64, cap: Cap) -> Result<(Vec<u64>, u64)> {
        if bid == 0 {
            return Ok((Vec::new(), 0));
        }
        if !is_internal(bid) {
            let cb = u64::from(self.block_entry(bid)?.cb);
            if cb > max {
                return Err(Error::Cap(cap));
            }
            return Ok((vec![bid], cb));
        }
        let f = self.format();
        let id = f.id_size();
        let top = self.block(bid)?;
        if u8_at(&top, 0)? != 1 {
            return Err(Error::Corrupt("a data tree's block type"));
        }
        let level = u8_at(&top, 1)?;
        let total = u64::from(u32_at(&top, 4)?);
        if total > max {
            return Err(Error::Cap(cap));
        }
        let allowed = usize::try_from(total / MIN_AVERAGE_BLOCK).unwrap_or(usize::MAX).saturating_add(8);
        let ids = |b: &[u8]| -> Result<Vec<u64>> {
            let n = usize::from(u16_at(b, 2)?);
            if n > allowed {
                return Err(Error::Cap(Cap::DataBlocks));
            }
            (0..n).map(|i| f.read_id(b, i.saturating_mul(id).saturating_add(8))).collect()
        };
        let mut out = Vec::new();
        // A block named twice in one data tree would be read twice: refused.
        let mut seen = HashSet::new();
        let mut once = |bid: u64| if seen.insert(key(bid)) { Ok(()) } else { Err(Error::Cycle("a data tree")) };
        match level {
            1 => {
                for child in ids(&top)? {
                    once(child)?;
                    if is_internal(child) {
                        return Err(Error::Corrupt("an XBLOCK pointing at an internal block"));
                    }
                    out.push(child);
                }
            }
            2 => {
                for child in ids(&top)? {
                    once(child)?;
                    if !is_internal(child) {
                        return Err(Error::Corrupt("an XXBLOCK pointing at a data block"));
                    }
                    let x = self.block(child)?;
                    if u8_at(&x, 0)? != 1 || u8_at(&x, 1)? != 1 {
                        return Err(Error::Corrupt("an XXBLOCK pointing at something else than an XBLOCK"));
                    }
                    for grandchild in ids(&x)? {
                        once(grandchild)?;
                        if is_internal(grandchild) {
                            return Err(Error::Corrupt("an XBLOCK pointing at an internal block"));
                        }
                        if out.len() >= allowed {
                            return Err(Error::Cap(Cap::DataBlocks));
                        }
                        out.push(grandchild);
                    }
                }
            }
            _ => return Err(Error::Corrupt("a data tree's level")),
        }
        Ok((out, total))
    }

    /// The whole data stream at `bid`, one piece per data block, refused past `max` bytes.
    pub(crate) fn read_pieces(&mut self, bid: u64, max: u64, cap: Cap) -> Result<Vec<Vec<u8>>> {
        let (blocks, total) = self.data_blocks(bid, max, cap)?;
        let mut pieces = Vec::with_capacity(blocks.len());
        let mut sum = 0u64;
        for b in blocks {
            let piece = self.block(b)?;
            sum = sum.saturating_add(piece.len() as u64);
            if sum > total {
                return Err(Error::Corrupt("a data tree longer than it says"));
            }
            pieces.push(piece);
        }
        if sum != total {
            return Err(Error::Corrupt("a data tree shorter than it says"));
        }
        Ok(pieces)
    }

    /// The whole data stream at `bid` as one buffer, refused past `max` bytes.
    pub(crate) fn read_data(&mut self, bid: u64, max: u64, cap: Cap) -> Result<Vec<u8>> {
        Ok(self.read_pieces(bid, max, cap)?.concat())
    }

    /// The subnodes of the subnode tree at `bid`.
    pub(crate) fn subnodes(&mut self, bid: u64) -> Result<Subnodes> {
        let mut out = Subnodes::new();
        if bid == 0 {
            return Ok(out);
        }
        if !is_internal(bid) {
            return Err(Error::Corrupt("a subnode tree in a data block"));
        }
        let top = self.block(bid)?;
        if u8_at(&top, 0)? != 2 {
            return Err(Error::Corrupt("a subnode tree's block type"));
        }
        match u8_at(&top, 1)? {
            0 => self.leaf_entries(&top, &mut out)?,
            1 => {
                let mut seen = HashSet::new();
                for child in self.intermediate_entries(&top)? {
                    if !seen.insert(key(child)) {
                        return Err(Error::Cycle("a subnode tree"));
                    }
                    if !is_internal(child) {
                        return Err(Error::Corrupt("a subnode tree in a data block"));
                    }
                    let leaf = self.block(child)?;
                    if u8_at(&leaf, 0)? != 2 || u8_at(&leaf, 1)? != 0 {
                        return Err(Error::Corrupt("an SIBLOCK pointing at something else than an SLBLOCK"));
                    }
                    self.leaf_entries(&leaf, &mut out)?;
                }
            }
            _ => return Err(Error::Corrupt("a subnode tree's level")),
        }
        Ok(out)
    }

    fn entries_start(&self) -> usize {
        match self.format() {
            Format::Unicode => 8,
            Format::Ansi => 4,
        }
    }

    fn leaf_entries(&self, b: &[u8], out: &mut Subnodes) -> Result<()> {
        let f = self.format();
        let id = f.id_size();
        let size = id.saturating_mul(3);
        let n = usize::from(u16_at(b, 2)?);
        for i in 0..n {
            let at = i.saturating_mul(size).saturating_add(self.entries_start());
            let e = slice(b, at, size)?;
            let nid = f.read_id(e, 0)? as u32;
            let entry = SubEntry { data: f.read_id(e, id)?, sub: f.read_id(e, id.saturating_mul(2))? };
            if out.len() >= self.limits.max_subnodes {
                return Err(Error::Cap(Cap::Subnodes));
            }
            out.entry(nid).or_insert(entry);
        }
        Ok(())
    }

    fn intermediate_entries(&self, b: &[u8]) -> Result<Vec<u64>> {
        let f = self.format();
        let id = f.id_size();
        let size = id.saturating_mul(2);
        let n = usize::from(u16_at(b, 2)?);
        (0..n).map(|i| f.read_id(b, i.saturating_mul(size).saturating_add(self.entries_start()).saturating_add(id))).collect()
    }

    /// Every node of the node B-tree, in key order, until `f` returns false.
    /// A page below the root that can't be read, or that is met twice, is noted and skipped
    /// with what is under it; the root must be read.
    pub(crate) fn for_each_node(&mut self, f: &mut dyn FnMut(NodeEntry) -> bool) -> Result<()> {
        let root = self.header.nbt;
        let mut visited = HashSet::new();
        let mut stack: Vec<(Bref, Option<u8>)> = vec![(root, None)];
        while let Some((bref, expect)) = stack.pop() {
            if !visited.insert(bref.ib) {
                self.note(bref.ib, PTYPE_NBT, &Error::Cycle("the node tree"));
                continue;
            }
            if visited.len() > self.limits.max_btree_pages {
                return Err(Error::Cap(Cap::BTreePages));
            }
            let page = match self.page_at(bref, PTYPE_NBT, expect) {
                Ok(p) => p,
                // Skipped only when it is damage, and named; a cap or the reader under the
                // file failing ends the walk.
                Err(e) if bref != root && is_damage(&e) => {
                    self.note(bref.ib, PTYPE_NBT, &e);
                    continue;
                }
                Err(e) => return Err(e),
            };
            if page.level == 0 {
                for i in 0..page.count {
                    let e = self.node_entry(page.entry(i)?)?;
                    if !f(e) {
                        return Ok(());
                    }
                }
                continue;
            }
            let id = self.format().id_size();
            let below = page.level.checked_sub(1);
            // Children pushed last-first so that they come off the stack in key order.
            for i in (0..page.count).rev() {
                let child = self.format().read_bref(page.entry(i)?, id)?;
                stack.push((child, below));
            }
        }
        Ok(())
    }

    pub(crate) fn encryption(&self) -> Encryption {
        self.header.encryption
    }

    pub(crate) fn file_size(&self) -> u64 {
        self.src.size()
    }
}

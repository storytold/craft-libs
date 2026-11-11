//! A small .pst writer, for tests only: it builds synthetic files from [MS-PST] (ANSI or
//! Unicode, each encoding, with or without a password) holding invented folders, messages,
//! recipients, attachments and attached messages, so that the reader has something to read.
//! It writes no allocation maps (the reader doesn't use them) and can be told to split data
//! into small blocks and pages, so that data trees, subnode trees, heaps over several blocks,
//! row matrices over several blocks and B-trees of several levels come out of small files.

use std::collections::BTreeMap;

use crate::crc::crc32;
use crate::crypt::{Encryption, encode};
use crate::header::Format;
use crate::ndb::signature;
use crate::prop::types::{BINARY, BOOLEAN, INT32, OBJECT, STRING, STRING8, TIME};

// ---------------------------------------------------------------------------------------------
// The node database.

/// A node's data: given as the blocks it is to be stored in (a heap), or as one stream that
/// the writer splits.
#[derive(Clone, Debug)]
pub enum Data {
    Blocks(Vec<Vec<u8>>),
    Stream(Vec<u8>),
    /// A block id given as it is, for nothing written (to name another node's data).
    Bid(u64),
    /// One data block named `times` times by an XBLOCK, and (with `levels` 2) that XBLOCK
    /// named `times` times by an XXBLOCK: a data tree that reads one block over and over.
    Repeated {
        piece: Vec<u8>,
        times: usize,
        levels: u8,
    },
}

#[derive(Clone, Debug)]
pub struct SubNode {
    pub nid: u32,
    pub data: Data,
    pub subnodes: Vec<SubNode>,
}

/// Marks a subnode's own subnode tree as a block id given as it is ([`SubNode::with_sub_bid`]).
const SUB_BID: u32 = u32::MAX;

impl SubNode {
    /// A subnode whose data and subnode tree are the block ids given, for nothing written.
    pub fn with_sub_bid(nid: u32, data: u64, sub: u64) -> SubNode {
        SubNode { nid, data: Data::Bid(data), subnodes: vec![SubNode { nid: SUB_BID, data: Data::Bid(sub), subnodes: Vec::new() }] }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub nid: u32,
    pub parent: u32,
    pub data: Data,
    pub subnodes: Vec<SubNode>,
}

/// How the writer lays things out; the small sizes make deep structures from little data.
#[derive(Clone, Debug)]
pub struct Layout {
    pub format: Format,
    pub encryption: Encryption,
    /// Data bytes per data block of a stream (at most a block's capacity).
    pub block_data: usize,
    /// Ids per XBLOCK / XXBLOCK.
    pub xblock_ids: usize,
    /// Entries per SLBLOCK / SIBLOCK.
    pub sl_entries: usize,
    /// Entries per B-tree page.
    pub page_entries: usize,
    /// Bytes of heap items per heap block.
    pub heap_block: usize,
    /// Records per BTH leaf item (an index level above when there are more).
    pub bth_leaf: usize,
    /// Store a table's rows in a subnode even when they would fit the heap.
    pub rows_in_subnode: bool,
    /// Set the reserved bit A (bit 0) of every block id where a block is referred to (the
    /// B-trees, data and subnode trees), or in each block's own trailer.
    pub bit_a_in_references: bool,
    pub bit_a_in_trailers: bool,
}

impl Layout {
    pub fn new(format: Format, encryption: Encryption) -> Layout {
        let cap = 8192 - format.trailer_size();
        Layout {
            format,
            encryption,
            block_data: cap,
            xblock_ids: (cap - 8) / format.id_size(),
            sl_entries: 300,
            page_entries: 15,
            heap_block: 3800,
            bth_leaf: 400,
            rows_in_subnode: false,
            bit_a_in_references: false,
            bit_a_in_trailers: false,
        }
    }

    /// Everything as small as it goes, to exercise every level of every tree.
    pub fn tiny(format: Format, encryption: Encryption) -> Layout {
        Layout {
            block_data: 100,
            xblock_ids: 3,
            sl_entries: 2,
            page_entries: 3,
            heap_block: 160,
            bth_leaf: 3,
            rows_in_subnode: true,
            ..Layout::new(format, encryption)
        }
    }
}

struct Ndb {
    layout: Layout,
    out: Vec<u8>,
    next_block: u64,
    next_page: u64,
    blocks: Vec<(u64, u64, u16)>,
}

fn put_id(out: &mut Vec<u8>, f: Format, v: u64) {
    match f {
        Format::Unicode => out.extend_from_slice(&v.to_le_bytes()),
        Format::Ansi => out.extend_from_slice(&(v as u32).to_le_bytes()),
    }
}

impl Ndb {
    fn new(layout: Layout) -> Ndb {
        Ndb { layout, out: vec![0; 0x4400], next_block: 1, next_page: 1, blocks: Vec::new() }
    }

    /// A block id as it is written where the block is referred to.
    fn r(&self, bid: u64) -> u64 {
        if self.layout.bit_a_in_references && bid != 0 { bid | 1 } else { bid }
    }

    fn align(&mut self, to: usize) {
        while !self.out.len().is_multiple_of(to) {
            self.out.push(0);
        }
    }

    fn block(&mut self, data: &[u8], internal: bool) -> u64 {
        let f = self.layout.format;
        let bid = (self.next_block << 2) | if internal { 2 } else { 0 };
        self.next_block += 1;
        let mut stored = data.to_vec();
        if !internal {
            encode(self.layout.encryption, &mut stored, bid);
        }
        self.align(64);
        let ib = self.out.len() as u64;
        let ts = f.trailer_size();
        let total = (stored.len() + ts).div_ceil(64) * 64;
        assert!(total <= 8192, "block too large");
        let own = if self.layout.bit_a_in_trailers { bid | 1 } else { bid };
        let mut b = stored.clone();
        b.resize(total - ts, 0xAB);
        b.extend_from_slice(&(stored.len() as u16).to_le_bytes());
        b.extend_from_slice(&signature(ib, own).to_le_bytes());
        let crc = crc32(&stored);
        match f {
            Format::Unicode => {
                b.extend_from_slice(&crc.to_le_bytes());
                b.extend_from_slice(&own.to_le_bytes());
            }
            Format::Ansi => {
                b.extend_from_slice(&(own as u32).to_le_bytes());
                b.extend_from_slice(&crc.to_le_bytes());
            }
        }
        self.out.extend_from_slice(&b);
        self.blocks.push((bid, ib, stored.len() as u16));
        bid
    }

    fn stream(&mut self, data: &Data) -> u64 {
        let pieces: Vec<Vec<u8>> = match data {
            Data::Bid(bid) => return *bid,
            Data::Repeated { piece, times, levels } => {
                let f = self.layout.format;
                let one = self.block(piece, false);
                let mut x = vec![1u8, 1];
                x.extend_from_slice(&(*times as u16).to_le_bytes());
                x.extend_from_slice(&((piece.len() * times) as u32).to_le_bytes());
                for _ in 0..*times {
                    put_id(&mut x, f, self.r(one));
                }
                let xb = self.block(&x, true);
                if *levels < 2 {
                    return xb;
                }
                let mut xx = vec![1u8, 2];
                xx.extend_from_slice(&(*times as u16).to_le_bytes());
                xx.extend_from_slice(&((piece.len() * times * times) as u32).to_le_bytes());
                for _ in 0..*times {
                    put_id(&mut xx, f, self.r(xb));
                }
                return self.block(&xx, true);
            }
            Data::Blocks(b) => b.clone(),
            Data::Stream(s) if s.is_empty() => vec![Vec::new()],
            Data::Stream(s) => s.chunks(self.layout.block_data).map(<[u8]>::to_vec).collect(),
        };
        if pieces.len() == 1 {
            return self.block(&pieces[0], false);
        }
        let f = self.layout.format;
        let per = self.layout.xblock_ids;
        let mut xblocks = Vec::new();
        for group in pieces.chunks(per) {
            let total: usize = group.iter().map(Vec::len).sum();
            let ids: Vec<u64> = group.iter().map(|p| self.block(p, false)).collect();
            let mut x = vec![1u8, 1];
            x.extend_from_slice(&(ids.len() as u16).to_le_bytes());
            x.extend_from_slice(&(total as u32).to_le_bytes());
            for id in ids {
                put_id(&mut x, f, self.r(id));
            }
            xblocks.push((self.block(&x, true), total));
        }
        if xblocks.len() == 1 {
            return xblocks[0].0;
        }
        assert!(xblocks.len() <= (8192 - 8 - f.trailer_size()) / f.id_size(), "data too large for an XXBLOCK");
        let total: usize = xblocks.iter().map(|x| x.1).sum();
        let mut xx = vec![1u8, 2];
        xx.extend_from_slice(&(xblocks.len() as u16).to_le_bytes());
        xx.extend_from_slice(&(total as u32).to_le_bytes());
        for (id, _) in xblocks {
            put_id(&mut xx, f, self.r(id));
        }
        self.block(&xx, true)
    }

    fn subnodes(&mut self, subs: &[SubNode]) -> u64 {
        if subs.is_empty() {
            return 0;
        }
        let f = self.layout.format;
        let mut entries: Vec<(u32, u64, u64)> = subs
            .iter()
            .map(|s| {
                let sub = match s.subnodes.as_slice() {
                    [SubNode { nid: SUB_BID, data: Data::Bid(bid), .. }] => *bid,
                    other => self.subnodes(other),
                };
                (s.nid, self.stream(&s.data), sub)
            })
            .collect();
        entries.sort_by_key(|e| e.0);
        let head = |level: u8, n: usize| -> Vec<u8> {
            let mut b = vec![2u8, level];
            b.extend_from_slice(&(n as u16).to_le_bytes());
            if f == Format::Unicode {
                b.extend_from_slice(&[0; 4]);
            }
            b
        };
        let mut leaves = Vec::new();
        for group in entries.chunks(self.layout.sl_entries) {
            let mut b = head(0, group.len());
            for &(nid, data, sub) in group {
                put_id(&mut b, f, u64::from(nid));
                put_id(&mut b, f, self.r(data));
                put_id(&mut b, f, self.r(sub));
            }
            leaves.push((group[0].0, self.block(&b, true)));
        }
        if leaves.len() == 1 {
            return leaves[0].1;
        }
        let mut b = head(1, leaves.len());
        for (nid, bid) in leaves {
            put_id(&mut b, f, u64::from(nid));
            put_id(&mut b, f, self.r(bid));
        }
        self.block(&b, true)
    }

    /// A B-tree over `leaves` (key, entry bytes), returning the root's (bid, ib).
    fn btree(&mut self, ptype: u8, leaves: Vec<(u64, Vec<u8>)>, leaf_size: usize) -> (u64, u64) {
        let f = self.layout.format;
        let mut level_entries: Vec<(u64, Vec<u8>)> = leaves;
        let mut level = 0u8;
        let mut entry_size = leaf_size;
        loop {
            let mut next = Vec::new();
            let chunks: Vec<Vec<(u64, Vec<u8>)>> =
                if level_entries.is_empty() { vec![Vec::new()] } else { level_entries.chunks(self.layout.page_entries).map(<[_]>::to_vec).collect() };
            let single = chunks.len() == 1;
            for chunk in chunks {
                let (bid, ib) = self.page(ptype, level, entry_size, &chunk);
                let key = chunk.first().map_or(0, |e| e.0);
                if single {
                    return (bid, ib);
                }
                let mut e = Vec::new();
                put_id(&mut e, f, key);
                put_id(&mut e, f, bid);
                put_id(&mut e, f, ib);
                next.push((key, e));
            }
            level_entries = next;
            level += 1;
            entry_size = 3 * f.id_size();
        }
    }

    fn page(&mut self, ptype: u8, level: u8, entry_size: usize, entries: &[(u64, Vec<u8>)]) -> (u64, u64) {
        let f = self.layout.format;
        self.align(512);
        let ib = self.out.len() as u64;
        let bid = self.next_page;
        self.next_page += 1;
        let area = if f == Format::Unicode { 488 } else { 496 };
        let mut p = Vec::new();
        for (_, e) in entries {
            let mut e = e.clone();
            e.resize(entry_size, 0);
            p.extend_from_slice(&e);
        }
        assert!(p.len() <= area);
        p.resize(area, 0);
        p.push(entries.len() as u8);
        p.push((area / entry_size) as u8);
        p.push(entry_size as u8);
        p.push(level);
        if f == Format::Unicode {
            p.extend_from_slice(&[0; 4]);
        }
        let crc = crc32(&p);
        p.push(ptype);
        p.push(ptype);
        p.extend_from_slice(&signature(ib, bid).to_le_bytes());
        match f {
            Format::Unicode => {
                p.extend_from_slice(&crc.to_le_bytes());
                p.extend_from_slice(&bid.to_le_bytes());
            }
            Format::Ansi => {
                p.extend_from_slice(&(bid as u32).to_le_bytes());
                p.extend_from_slice(&crc.to_le_bytes());
            }
        }
        assert_eq!(p.len(), 512);
        self.out.extend_from_slice(&p);
        (bid, ib)
    }

    fn finish(mut self, nodes: &[Node]) -> Vec<u8> {
        let f = self.layout.format;
        let mut nbt = Vec::new();
        for n in nodes {
            let data = self.stream(&n.data);
            let sub = self.subnodes(&n.subnodes);
            let mut e = Vec::new();
            put_id(&mut e, f, u64::from(n.nid));
            put_id(&mut e, f, self.r(data));
            put_id(&mut e, f, self.r(sub));
            e.extend_from_slice(&n.parent.to_le_bytes());
            nbt.push((u64::from(n.nid), e));
        }
        nbt.sort_by_key(|e| e.0);
        let mut bbt: Vec<(u64, Vec<u8>)> = self
            .blocks
            .iter()
            .map(|&(bid, ib, cb)| {
                let mut e = Vec::new();
                put_id(&mut e, f, self.r(bid));
                put_id(&mut e, f, ib);
                e.extend_from_slice(&cb.to_le_bytes());
                e.extend_from_slice(&2u16.to_le_bytes());
                (bid, e)
            })
            .collect();
        bbt.sort_by_key(|e| e.0);
        let (nbt_size, bbt_size) = match f {
            Format::Unicode => (32, 24),
            Format::Ansi => (16, 12),
        };
        let nbt_root = self.btree(0x81, nbt, nbt_size);
        let bbt_root = self.btree(0x80, bbt, bbt_size);
        let eof = self.out.len() as u64;
        let header = header(f, self.layout.encryption, eof, nbt_root, bbt_root, self.next_block << 2, self.next_page);
        self.out[..header.len()].copy_from_slice(&header);
        self.out
    }
}

/// The HEADER, with both CRCs.
pub fn header(f: Format, enc: Encryption, eof: u64, nbt: (u64, u64), bbt: (u64, u64), next_b: u64, next_p: u64) -> Vec<u8> {
    let crypt = match enc {
        Encryption::None => 0u8,
        Encryption::Permute => 1,
        Encryption::Cyclic => 2,
    };
    let mut h = Vec::new();
    h.extend_from_slice(b"!BDN");
    h.extend_from_slice(&[0; 4]); // dwCRCPartial
    h.extend_from_slice(b"SM");
    h.extend_from_slice(&(if f == Format::Unicode { 23u16 } else { 14 }).to_le_bytes());
    h.extend_from_slice(&19u16.to_le_bytes());
    h.extend_from_slice(&[1, 1]);
    h.extend_from_slice(&[0; 8]); // dwReserved1, dwReserved2
    let mut root = Vec::new();
    root.extend_from_slice(&[0; 4]);
    put_id(&mut root, f, eof);
    put_id(&mut root, f, 0); // ibAMapLast
    put_id(&mut root, f, 0); // cbAMapFree
    put_id(&mut root, f, 0); // cbPMapFree
    put_id(&mut root, f, nbt.0);
    put_id(&mut root, f, nbt.1);
    put_id(&mut root, f, bbt.0);
    put_id(&mut root, f, bbt.1);
    root.extend_from_slice(&[2, 0, 0, 0]);
    let mut rgnid = Vec::new();
    for _ in 0..32 {
        rgnid.extend_from_slice(&0x400u32.to_le_bytes());
    }
    match f {
        Format::Unicode => {
            h.extend_from_slice(&[0; 8]); // bidUnused
            h.extend_from_slice(&next_p.to_le_bytes());
            h.extend_from_slice(&1u32.to_le_bytes()); // dwUnique
            h.extend_from_slice(&rgnid);
            h.extend_from_slice(&[0; 8]); // qwUnused
            h.extend_from_slice(&root);
            h.extend_from_slice(&[0; 4]); // dwAlign
            h.extend_from_slice(&[0xFF; 256]);
            h.extend_from_slice(&[0x80, crypt, 0, 0]);
            h.extend_from_slice(&next_b.to_le_bytes());
            h.extend_from_slice(&[0; 4]); // dwCRCFull
            h.extend_from_slice(&[0; 36]);
            assert_eq!(h.len(), 564);
            let full = crc32(&h[8..524]);
            h[524..528].copy_from_slice(&full.to_le_bytes());
        }
        Format::Ansi => {
            h.extend_from_slice(&(next_b as u32).to_le_bytes());
            h.extend_from_slice(&(next_p as u32).to_le_bytes());
            h.extend_from_slice(&1u32.to_le_bytes());
            h.extend_from_slice(&rgnid);
            h.extend_from_slice(&root);
            h.extend_from_slice(&[0xFF; 256]);
            h.extend_from_slice(&[0x80, crypt, 0, 0]);
            h.extend_from_slice(&[0; 12]);
            h.extend_from_slice(&[0; 36]);
            assert_eq!(h.len(), 512);
        }
    }
    let partial = crc32(&h[8..479]);
    h[4..8].copy_from_slice(&partial.to_le_bytes());
    h
}

// ---------------------------------------------------------------------------------------------
// Lists, tables and properties.

/// A heap being filled: items go into blocks of at most `capacity` bytes.
pub struct HeapBuilder {
    client: u8,
    capacity: usize,
    blocks: Vec<Vec<Vec<u8>>>,
    pub user_root: u32,
}

impl HeapBuilder {
    pub fn new(client: u8, capacity: usize) -> HeapBuilder {
        HeapBuilder { client, capacity, blocks: vec![Vec::new()], user_root: 0 }
    }

    pub fn alloc(&mut self, item: Vec<u8>) -> u32 {
        let used: usize = self.blocks.last().unwrap().iter().map(Vec::len).sum();
        let count = self.blocks.last().unwrap().len();
        if count > 0 && (used + item.len() > self.capacity || count >= 2000) {
            self.blocks.push(Vec::new());
        }
        let block = self.blocks.len() - 1;
        self.blocks[block].push(item);
        let index = self.blocks[block].len();
        ((block as u32) << 16) | ((index as u32) << 5)
    }

    pub fn finish(self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for (i, items) in self.blocks.iter().enumerate() {
            let header_len = if i == 0 {
                12
            } else if i == 8 || (i > 8 && (i - 8) % 128 == 0) {
                66
            } else {
                2
            };
            let mut b = vec![0u8; header_len];
            let mut offsets = Vec::new();
            for item in items {
                offsets.push(b.len() as u16);
                b.extend_from_slice(item);
            }
            offsets.push(b.len() as u16);
            if b.len() % 2 == 1 {
                b.push(0);
            }
            let map = b.len() as u16;
            b[0..2].copy_from_slice(&map.to_le_bytes());
            if i == 0 {
                b[2] = 0xEC;
                b[3] = self.client;
                b[4..8].copy_from_slice(&self.user_root.to_le_bytes());
            }
            b.extend_from_slice(&((offsets.len() - 1) as u16).to_le_bytes());
            b.extend_from_slice(&0u16.to_le_bytes());
            for o in offsets {
                b.extend_from_slice(&o.to_le_bytes());
            }
            out.push(b);
        }
        out
    }
}

/// A BTH in `heap` over `records` (sorted by key), returning its header's HID.
pub fn bth(heap: &mut HeapBuilder, key: usize, ent: usize, records: &[Vec<u8>], leaf: usize) -> u32 {
    if records.is_empty() {
        return heap.alloc(vec![0xB5, key as u8, ent as u8, 0, 0, 0, 0, 0]);
    }
    let mut level: Vec<(Vec<u8>, u32)> = records.chunks(leaf).map(|c| (c[0][..key].to_vec(), heap.alloc(c.concat()))).collect();
    let mut levels = 0u8;
    while level.len() > 1 {
        levels += 1;
        level = level
            .chunks(leaf)
            .map(|c| {
                let mut item = Vec::new();
                for (k, hid) in c {
                    item.extend_from_slice(k);
                    item.extend_from_slice(&hid.to_le_bytes());
                }
                (c[0].0.clone(), heap.alloc(item))
            })
            .collect();
    }
    let mut h = vec![0xB5, key as u8, ent as u8, levels];
    h.extend_from_slice(&level[0].1.to_le_bytes());
    heap.alloc(h)
}

/// A property value as the writer is given it.
#[derive(Clone, Debug)]
pub struct Value {
    pub ty: u16,
    pub bytes: Vec<u8>,
}

/// What [`hnid_ref`] puts before the HNID.
const HNID_REF: &[u8; 8] = b"\xFFHNIDREF";

/// A variable-size value whose record names `hnid` as it is (an HNID another record names
/// too, or one that names nothing).
pub fn hnid_ref(ty: u16, hnid: u32) -> Value {
    let mut bytes = HNID_REF.to_vec();
    bytes.extend_from_slice(&hnid.to_le_bytes());
    Value { ty, bytes }
}

fn as_ref(v: &Value) -> Option<u32> {
    let rest = v.bytes.strip_prefix(HNID_REF)?;
    Some(u32::from_le_bytes(rest.try_into().ok()?))
}

pub fn int32(v: i32) -> Value {
    Value { ty: INT32, bytes: v.to_le_bytes().to_vec() }
}
pub fn boolean(v: bool) -> Value {
    Value { ty: BOOLEAN, bytes: vec![u8::from(v)] }
}
pub fn time(v: u64) -> Value {
    Value { ty: TIME, bytes: v.to_le_bytes().to_vec() }
}
pub fn binary(v: &[u8]) -> Value {
    Value { ty: BINARY, bytes: v.to_vec() }
}
pub fn unicode(s: &str) -> Value {
    Value { ty: STRING, bytes: s.encode_utf16().flat_map(u16::to_le_bytes).collect() }
}
pub fn string8(s: &str) -> Value {
    Value { ty: STRING8, bytes: s.chars().map(|c| c as u8).collect() }
}

/// A one-off entry id ([MS-OXCDATA] 2.2.5.1): a display name, an address type and an address,
/// in UTF-16 or in single bytes.
pub fn one_off(name: &str, kind: &str, address: &str, unicode: bool) -> Vec<u8> {
    let mut e = vec![0u8; 4];
    e.extend([0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02]);
    e.extend([0, 0]);
    e.extend(if unicode { 0x8000u16 } else { 0 }.to_le_bytes());
    for t in [name, kind, address] {
        if unicode {
            e.extend(t.encode_utf16().flat_map(u16::to_le_bytes));
            e.extend([0, 0]);
        } else {
            e.extend(t.bytes());
            e.push(0);
        }
    }
    e
}

/// A WrappedEntryId ([MS-OXOCNTC] 2.2.2.2.4.1.1): `kind` (0x00 a one-off, 0xC3 a contact's
/// first email address, 0xB4 a list…) and the entry id it wraps.
pub fn wrapped_entry_id(kind: u8, embedded: &[u8]) -> Vec<u8> {
    let mut e = vec![0u8; 4];
    e.extend([0xC0, 0x91, 0xAD, 0xD3, 0x51, 0x9D, 0xCF, 0x11, 0xA4, 0xA9, 0x00, 0xAA, 0x00, 0x47, 0xFA, 0xA4]);
    e.push(kind);
    e.extend_from_slice(embedded);
    e
}

/// One changed occurrence of a series: start, end, original start, new subject.
pub type ExceptionSpec = (u32, u32, u32, Option<String>);

/// An AppointmentRecurrencePattern ([MS-OXOCAL] 2.2.1.44.5). `specific` is the PatternTypeSpecific
/// words; dates are minutes since 1601 by the series' clock.
#[allow(clippy::too_many_arguments)]
pub fn recurrence_bytes(
    frequency: u16,
    pattern: u16,
    period: u32,
    specific: &[u32],
    end: (u32, u32, u32),
    deleted: &[u32],
    start_date: u32,
    offsets: (u32, u32),
    exceptions: &[ExceptionSpec],
) -> Vec<u8> {
    let mut b = Vec::new();
    let w16 = |b: &mut Vec<u8>, v: u16| b.extend_from_slice(&v.to_le_bytes());
    let w32 = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    w16(&mut b, 0x3004);
    w16(&mut b, 0x3004);
    w16(&mut b, frequency);
    w16(&mut b, pattern);
    w16(&mut b, 0);
    w32(&mut b, 0);
    w32(&mut b, period);
    w32(&mut b, 0);
    specific.iter().for_each(|v| w32(&mut b, *v));
    w32(&mut b, end.0);
    w32(&mut b, end.1);
    w32(&mut b, 1);
    w32(&mut b, deleted.len() as u32);
    deleted.iter().for_each(|v| w32(&mut b, *v));
    w32(&mut b, exceptions.len() as u32);
    exceptions.iter().for_each(|x| w32(&mut b, x.2));
    w32(&mut b, start_date);
    w32(&mut b, end.2);
    w32(&mut b, 0x3006);
    w32(&mut b, 0x3008);
    w32(&mut b, offsets.0);
    w32(&mut b, offsets.1);
    w16(&mut b, exceptions.len() as u16);
    for (s, e, o, subject) in exceptions {
        w32(&mut b, *s);
        w32(&mut b, *e);
        w32(&mut b, *o);
        w16(&mut b, u16::from(subject.is_some()));
        if let Some(t) = subject {
            w16(&mut b, t.len() as u16 + 1);
            w16(&mut b, t.len() as u16);
            b.extend_from_slice(t.as_bytes());
        }
    }
    w32(&mut b, 0);
    for (s, e, o, subject) in exceptions {
        w32(&mut b, 0);
        if let Some(t) = subject {
            w32(&mut b, *s);
            w32(&mut b, *e);
            w32(&mut b, *o);
            let wide: Vec<u16> = t.encode_utf16().collect();
            w16(&mut b, wide.len() as u16);
            wide.iter().for_each(|u| w16(&mut b, *u));
            w32(&mut b, 0);
        }
    }
    w32(&mut b, 0);
    b
}

/// A TZDEFINITION with a key name and no rules.
pub fn time_zone_bytes(key: &str) -> Vec<u8> {
    let mut b = vec![2, 1];
    let wide: Vec<u16> = key.encode_utf16().collect();
    b.extend_from_slice(&((6 + wide.len() * 2) as u16).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&(wide.len() as u16).to_le_bytes());
    wide.iter().for_each(|u| b.extend_from_slice(&u.to_le_bytes()));
    b.extend_from_slice(&0u16.to_le_bytes());
    b
}

/// The bytes of a distribution list's stream ([MS-OXOCNTC] 2.2.2.2.4): the header, each member's
/// entry id and one-off entry id (empty for none), and the two terminators.
pub fn list_stream_bytes(members: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(1u16.to_le_bytes());
    b.extend([0, 0]);
    b.extend(160_000u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend((members.len() as u32).to_le_bytes());
    b.extend((members.iter().map(|m| m.0.len()).sum::<usize>() as u32).to_le_bytes());
    b.extend((members.iter().map(|m| m.1.len()).sum::<usize>() as u32).to_le_bytes());
    b.extend(0u32.to_le_bytes());
    for (id, one_off) in members {
        b.extend((id.len() as u32).to_le_bytes());
        b.extend_from_slice(id);
        b.extend((one_off.len() as u32).to_le_bytes());
        b.extend_from_slice(one_off);
        b.extend(0u32.to_le_bytes());
    }
    b.extend([0; 8]);
    b
}

/// A distribution list's stream as a binary property value.
pub fn list_stream(members: &[(Vec<u8>, Vec<u8>)]) -> Value {
    binary(&list_stream_bytes(members))
}

/// Where a value too large for the heap goes: a subnode, with a fresh local id.
pub struct Spill {
    pub subnodes: Vec<SubNode>,
    next: u32,
}

impl Spill {
    pub fn new() -> Spill {
        Spill { subnodes: Vec::new(), next: 0x100 }
    }

    pub fn put(&mut self, kind: u8, data: Vec<u8>) -> u32 {
        self.next += 1;
        let nid = (self.next << 5) | u32::from(kind);
        self.subnodes.push(SubNode { nid, data: Data::Stream(data), subnodes: Vec::new() });
        nid
    }
}

const HEAP_VALUE_MAX: usize = 3580;

fn hnid(heap: &mut HeapBuilder, spill: &mut Spill, bytes: &[u8]) -> u32 {
    if bytes.len() <= HEAP_VALUE_MAX.min(heap.capacity) { heap.alloc(bytes.to_vec()) } else { spill.put(0x1F, bytes.to_vec()) }
}

fn fixed_len(ty: u16) -> Option<usize> {
    crate::prop::fixed_size(ty)
}

/// A property context: its heap blocks; values too large go to `spill`.
pub fn pc(layout: &Layout, props: &[(u16, Value)], spill: &mut Spill) -> Vec<Vec<u8>> {
    let mut heap = HeapBuilder::new(0xBC, layout.heap_block);
    let mut sorted = props.to_vec();
    sorted.sort_by_key(|p| p.0);
    let mut records = Vec::new();
    for (id, v) in &sorted {
        let raw: u32 = match fixed_len(v.ty) {
            Some(n) if n <= 4 => {
                let mut b = v.bytes.clone();
                b.resize(4, 0);
                u32::from_le_bytes(b.try_into().unwrap())
            }
            Some(_) => heap.alloc(v.bytes.clone()),
            None if v.ty == OBJECT => heap.alloc(v.bytes.clone()),
            None => as_ref(v).unwrap_or_else(|| hnid(&mut heap, spill, &v.bytes)),
        };
        let mut r = id.to_le_bytes().to_vec();
        r.extend_from_slice(&v.ty.to_le_bytes());
        r.extend_from_slice(&raw.to_le_bytes());
        records.push(r);
    }
    heap.user_root = bth(&mut heap, 2, 6, &records, layout.bth_leaf);
    heap.finish()
}

/// A table context's column: its tag and width in the row.
fn width(ty: u16) -> usize {
    match fixed_len(ty) {
        Some(n) if n <= 8 => n.max(1),
        _ => 4,
    }
}

/// A table context over `columns` (tags) and `rows` (row id, values by tag); its heap blocks
/// and its subnodes (the row matrix, when it goes there, and large values).
pub fn tc(layout: &Layout, columns: &[u32], rows: &[(u32, Vec<(u32, Value)>)]) -> (Vec<Vec<u8>>, Vec<SubNode>) {
    let f = layout.format;
    let mut cols: Vec<u32> = vec![0x67F2_0003, 0x67F3_0003];
    for c in columns {
        if !cols.contains(c) {
            cols.push(*c);
        }
    }
    // Offsets: row id, row version, then 8- and 4-byte, 2-byte and 1-byte columns.
    let mut place: BTreeMap<u32, (usize, usize, usize)> = BTreeMap::new();
    place.insert(0x67F2_0003, (0, 4, 0));
    place.insert(0x67F3_0003, (4, 4, 1));
    let mut at = 8;
    let mut ends = [0usize; 3];
    for (g, widths) in [(0, 4usize..=8), (1, 2..=2), (2, 1..=1)] {
        for (i, &c) in cols.iter().enumerate().skip(2) {
            let w = width((c & 0xFFFF) as u16);
            if widths.contains(&w) {
                place.insert(c, (at, w, i));
                at += w;
            }
        }
        ends[g] = at;
    }
    let [end4, end2, end1] = ends;
    let row_size = end1 + cols.len().div_ceil(8);

    let mut heap = HeapBuilder::new(0x7C, layout.heap_block);
    let mut spill = Spill::new();
    let mut matrix = Vec::new();
    for (rowid, values) in rows {
        let mut row = vec![0u8; row_size];
        row[0..4].copy_from_slice(&rowid.to_le_bytes());
        let mut set = vec![0usize, 1];
        for (tag, v) in values {
            let Some(&(off, w, bit)) = place.get(tag) else { panic!("no column {tag:#x}") };
            let ty = (tag & 0xFFFF) as u16;
            match fixed_len(ty) {
                Some(n) if n <= 8 => {
                    let mut b = v.bytes.clone();
                    b.resize(w, 0);
                    row[off..off + w].copy_from_slice(&b);
                }
                _ => {
                    let h = as_ref(v).unwrap_or_else(|| hnid(&mut heap, &mut spill, &v.bytes));
                    row[off..off + 4].copy_from_slice(&h.to_le_bytes());
                }
            }
            set.push(bit);
        }
        for bit in set {
            row[end1 + bit / 8] |= 0x80 >> (bit % 8);
        }
        matrix.push(row);
    }
    // The row index.
    let mut index: Vec<Vec<u8>> = rows
        .iter()
        .enumerate()
        .map(|(i, (rowid, _))| {
            let mut r = rowid.to_le_bytes().to_vec();
            match f {
                Format::Unicode => r.extend_from_slice(&(i as u32).to_le_bytes()),
                Format::Ansi => r.extend_from_slice(&(i as u16).to_le_bytes()),
            }
            r
        })
        .collect();
    index.sort_by_key(|r| u32::from_le_bytes([r[0], r[1], r[2], r[3]]));
    let ent = if f == Format::Unicode { 4 } else { 2 };
    let row_index = bth(&mut heap, 4, ent, &index, layout.bth_leaf);
    let matrix_bytes: usize = matrix.iter().map(Vec::len).sum();
    let rows_hnid = if matrix.is_empty() {
        0
    } else if !layout.rows_in_subnode && matrix_bytes <= HEAP_VALUE_MAX.min(layout.heap_block) {
        heap.alloc(matrix.concat())
    } else {
        let per_block = ((8192 - f.trailer_size()) / row_size).max(1);
        let pieces: Vec<Vec<u8>> = matrix.chunks(per_block).map(|c| c.concat()).collect();
        spill.next += 1;
        let nid = (spill.next << 5) | 0x1F;
        spill.subnodes.push(SubNode { nid, data: Data::Blocks(pieces), subnodes: Vec::new() });
        nid
    };
    let mut info = vec![0x7C, cols.len() as u8];
    for v in [end4, end2, end1, row_size] {
        info.extend_from_slice(&(v as u16).to_le_bytes());
    }
    info.extend_from_slice(&row_index.to_le_bytes());
    info.extend_from_slice(&rows_hnid.to_le_bytes());
    info.extend_from_slice(&0u32.to_le_bytes());
    let mut sorted = cols.clone();
    sorted.sort();
    for c in sorted {
        let (off, w, bit) = place[&c];
        info.extend_from_slice(&c.to_le_bytes());
        info.extend_from_slice(&(off as u16).to_le_bytes());
        info.push(w as u8);
        info.push(bit as u8);
    }
    heap.user_root = heap.alloc(info);
    (heap.finish(), spill.subnodes)
}

// ---------------------------------------------------------------------------------------------
// The messaging layer.

#[derive(Clone, Debug, Default)]
pub struct MessageSpec {
    pub class: String,
    pub subject: String,
    pub body: String,
    pub read: bool,
    pub delivered: u64,
    pub extra: Vec<(u16, Value)>,
    pub recipients: Vec<(String, String, i32)>,
    /// PidTagRecipientFlags and PidTagRecipientTrackStatus of each recipient, by position.
    pub recipient_states: Vec<(i32, i32)>,
    pub attachments: Vec<AttachSpec>,
}

#[derive(Clone, Debug)]
pub enum AttachSpec {
    File {
        name: String,
        mime: String,
        data: Vec<u8>,
    },
    Message(Box<MessageSpec>),
    /// An attached message (method 5) whose object is this subnode, as it is.
    Object(SubNode),
    /// An attachment with exactly these properties (a linked file, an inline picture with its
    /// content id, a hidden one).
    Props(Vec<(u16, Value)>),
    /// An OLE object (method 6): these bytes in a subnode of its own.
    Ole(Vec<u8>),
}

#[derive(Clone, Debug, Default)]
pub struct FolderSpec {
    pub name: String,
    pub class: Option<String>,
    pub messages: Vec<MessageSpec>,
    pub children: Vec<FolderSpec>,
}

#[derive(Clone, Debug)]
pub struct FileSpec {
    pub layout: Layout,
    pub password: Option<String>,
    /// The password's CRC taken of its Windows-1252 bytes (its characters as bytes) rather
    /// than its UTF-8.
    pub password_ansi: bool,
    pub name: String,
    pub folders: Vec<FolderSpec>,
    /// Named properties: (property set, numeric name).
    pub named: Vec<([u8; 16], u32)>,
    pub named_strings: Vec<([u8; 16], String)>,
}

pub const RECORD_KEY: [u8; 16] = [0x5A, 0x11, 0x0E, 0x93, 0x42, 0x7C, 0x4B, 0x1D, 0x8E, 0x22, 0x61, 0x0C, 0x3F, 0x77, 0x90, 0x05];

pub fn entry_id(nid: u32) -> Vec<u8> {
    let mut e = vec![0u8; 4];
    e.extend_from_slice(&RECORD_KEY);
    e.extend_from_slice(&nid.to_le_bytes());
    e
}

/// A container class written `hidden:IPF.Contact` is a hidden folder (PidTagAttributeHidden) of
/// that class.
fn hidden_class(class: &str) -> (&str, bool) {
    match class.strip_prefix("hidden:") {
        Some(rest) => (rest, true),
        None => (class, false),
    }
}

pub struct Ids {
    folder: u32,
    message: u32,
}

impl Ids {
    fn folder(&mut self) -> u32 {
        self.folder += 1;
        (self.folder << 5) | 0x02
    }
    fn message(&mut self) -> u32 {
        self.message += 1;
        (self.message << 5) | 0x04
    }
}

pub const IPM_SUBTREE: u32 = (0x401 << 5) | 0x02;
pub const SEARCH_ROOT: u32 = (0x402 << 5) | 0x02;

fn text(layout: &Layout, s: &str) -> Value {
    if layout.format == Format::Ansi { string8(s) } else { unicode(s) }
}

fn text_tag(layout: &Layout, id: u16) -> u32 {
    (u32::from(id) << 16) | u32::from(if layout.format == Format::Ansi { STRING8 } else { STRING })
}

/// The columns this writer puts in a hierarchy table.
fn hierarchy_columns(l: &Layout) -> Vec<u32> {
    vec![text_tag(l, 0x3001), 0x3602_0003, 0x3603_0003, 0x360A_000B, text_tag(l, 0x3613)]
}

fn contents_columns(l: &Layout) -> Vec<u32> {
    vec![0x0017_0003, text_tag(l, 0x001A), text_tag(l, 0x0037), 0x0E06_0040, 0x0E07_0003]
}

/// Builds the message `m`: its PC blocks and its subnodes.
fn message(l: &Layout, m: &MessageSpec) -> (Vec<Vec<u8>>, Vec<SubNode>) {
    let flags = i32::from(m.read) | if m.attachments.is_empty() { 0 } else { 0x10 };
    let mut props = vec![
        (0x001A, text(l, &m.class)),
        (0x0037, text(l, &m.subject)),
        (0x1000, text(l, &m.body)),
        (0x0E07, int32(flags)),
        (0x0E06, time(m.delivered)),
        (0x0017, int32(1)),
    ];
    // A property in `extra` takes the place of the one given above.
    props.retain(|(id, _)| !m.extra.iter().any(|(e, _)| e == id));
    props.extend(m.extra.iter().cloned());
    let mut spill = Spill::new();
    let pc_blocks = pc(l, &props, &mut spill);
    let mut subs = spill.subnodes;

    // Recipients.
    let rcols = vec![0x0C15_0003, text_tag(l, 0x3001), text_tag(l, 0x3003), text_tag(l, 0x3002), text_tag(l, 0x39FE), 0x5FFD_0003, 0x5FFF_0003];
    let rrows: Vec<(u32, Vec<(u32, Value)>)> = m
        .recipients
        .iter()
        .enumerate()
        .map(|(i, (name, email, kind))| {
            let mut row = vec![
                (0x0C15_0003, int32(*kind)),
                (text_tag(l, 0x3001), text(l, name)),
                (text_tag(l, 0x3003), text(l, email)),
                (text_tag(l, 0x3002), text(l, "SMTP")),
            ];
            if let Some((flags, track)) = m.recipient_states.get(i) {
                row.push((0x5FFD_0003, int32(*flags)));
                row.push((0x5FFF_0003, int32(*track)));
            }
            (i as u32 + 1, row)
        })
        .collect();
    let (rblocks, rsubs) = tc(l, &rcols, &rrows);
    subs.push(SubNode { nid: 0x692, data: Data::Blocks(rblocks), subnodes: rsubs });

    // Attachments.
    if !m.attachments.is_empty() {
        let acols = vec![0x0E20_0003, text_tag(l, 0x3704), 0x3705_0003, 0x370B_0003];
        let mut arows = Vec::new();
        for (i, a) in m.attachments.iter().enumerate() {
            let nid = ((0x200 + i as u32) << 5) | 0x05;
            let mut aspill = Spill::new();
            let (aprops, size, method) = match a {
                AttachSpec::File { name, mime, data } => {
                    let props = vec![
                        (0x3705, int32(1)),
                        (0x3707, text(l, name)),
                        (0x3704, text(l, name)),
                        (0x370E, text(l, mime)),
                        (0x0E20, int32(data.len() as i32)),
                        (0x370B, int32(-1)),
                        (0x3701, binary(data)),
                    ];
                    (props, data.len(), 1)
                }
                AttachSpec::Message(inner) => {
                    let (mblocks, msubs) = message(l, inner);
                    let size: usize = mblocks.iter().map(Vec::len).sum();
                    aspill.next += 1;
                    let onid = (aspill.next << 5) | 0x04;
                    aspill.subnodes.push(SubNode { nid: onid, data: Data::Blocks(mblocks), subnodes: msubs });
                    let mut obj = onid.to_le_bytes().to_vec();
                    obj.extend_from_slice(&(size as u32).to_le_bytes());
                    let props = vec![
                        (0x3705, int32(5)),
                        (0x3704, text(l, &inner.subject)),
                        (0x0E20, int32(size as i32)),
                        (0x370B, int32(-1)),
                        (0x3701, Value { ty: OBJECT, bytes: obj }),
                    ];
                    (props, size, 5)
                }
                AttachSpec::Object(object) => {
                    aspill.subnodes.push(object.clone());
                    let mut obj = object.nid.to_le_bytes().to_vec();
                    obj.extend_from_slice(&0u32.to_le_bytes());
                    let props = vec![(0x3705, int32(5)), (0x3704, text(l, "object")), (0x3701, Value { ty: OBJECT, bytes: obj })];
                    (props, 0, 5)
                }
                AttachSpec::Props(props) => {
                    let method = props.iter().find(|(id, _)| *id == 0x3705).map_or(1, |(_, v)| i32::from_le_bytes(v.bytes[..4].try_into().unwrap()));
                    (props.clone(), 0, method)
                }
                AttachSpec::Ole(data) => {
                    let onid = aspill.put(0x01, data.clone());
                    let mut obj = onid.to_le_bytes().to_vec();
                    obj.extend_from_slice(&(data.len() as u32).to_le_bytes());
                    let props = vec![(0x3705, int32(6)), (0x3704, text(l, "Picture (Metafile)")), (0x3701, Value { ty: OBJECT, bytes: obj })];
                    (props, data.len(), 6)
                }
            };
            let ablocks = pc(l, &aprops, &mut aspill);
            subs.push(SubNode { nid, data: Data::Blocks(ablocks), subnodes: aspill.subnodes });
            let name = match a {
                AttachSpec::File { name, .. } => name.clone(),
                AttachSpec::Message(inner) => inner.subject.clone(),
                AttachSpec::Object(_) => "object".into(),
                AttachSpec::Props(_) => "attachment".into(),
                AttachSpec::Ole(_) => "Picture (Metafile)".into(),
            };
            arows.push((
                nid,
                vec![
                    (0x0E20_0003, int32(size as i32)),
                    (text_tag(l, 0x3704), text(l, &name)),
                    (0x3705_0003, int32(method)),
                    (0x370B_0003, int32(-1)),
                ],
            ));
        }
        let (ablocks, asubs) = tc(l, &acols, &arows);
        subs.push(SubNode { nid: 0x671, data: Data::Blocks(ablocks), subnodes: asubs });
    }
    (pc_blocks, subs)
}

/// The four nodes of a folder (and those of its messages and subfolders, recursively).
fn folder(l: &Layout, ids: &mut Ids, nid: u32, parent: u32, spec: &FolderSpec, nodes: &mut Vec<Node>) {
    let mut child_rows = Vec::new();
    let mut child_ids = Vec::new();
    for c in &spec.children {
        let cid = ids.folder();
        child_ids.push(cid);
        let mut vals = vec![
            (text_tag(l, 0x3001), text(l, &c.name)),
            (0x3602_0003, int32(c.messages.len() as i32)),
            (0x3603_0003, int32(c.messages.iter().filter(|m| !m.read).count() as i32)),
            (0x360A_000B, boolean(!c.children.is_empty())),
        ];
        if let Some(class) = &c.class {
            vals.push((text_tag(l, 0x3613), text(l, hidden_class(class).0)));
        }
        child_rows.push((cid, vals));
    }
    let mut props = vec![
        (0x3001, text(l, &spec.name)),
        (0x3602, int32(spec.messages.len() as i32)),
        (0x3603, int32(spec.messages.iter().filter(|m| !m.read).count() as i32)),
        (0x360A, boolean(!spec.children.is_empty())),
    ];
    if let Some(class) = &spec.class {
        let (class, hidden) = hidden_class(class);
        props.push((0x3613, text(l, class)));
        if hidden {
            props.push((0x10F4, boolean(true)));
        }
    }
    let mut spill = Spill::new();
    let blocks = pc(l, &props, &mut spill);
    nodes.push(Node { nid, parent, data: Data::Blocks(blocks), subnodes: spill.subnodes });
    let (h, hs) = tc(l, &hierarchy_columns(l), &child_rows);
    nodes.push(Node { nid: (nid & !0x1F) | 0x0D, parent: 0, data: Data::Blocks(h), subnodes: hs });
    let mut rows = Vec::new();
    for m in &spec.messages {
        let mid = ids.message();
        let (blocks, subs) = message(l, m);
        nodes.push(Node { nid: mid, parent: nid, data: Data::Blocks(blocks), subnodes: subs });
        rows.push((
            mid,
            vec![
                (0x0017_0003, int32(1)),
                (text_tag(l, 0x001A), text(l, &m.class)),
                (text_tag(l, 0x0037), text(l, &m.subject)),
                (0x0E06_0040, time(m.delivered)),
                (0x0E07_0003, int32(i32::from(m.read))),
            ],
        ));
    }
    let (c, cs) = tc(l, &contents_columns(l), &rows);
    nodes.push(Node { nid: (nid & !0x1F) | 0x0E, parent: 0, data: Data::Blocks(c), subnodes: cs });
    let (a, asubs) = tc(l, &[text_tag(l, 0x001A)], &[]);
    nodes.push(Node { nid: (nid & !0x1F) | 0x0F, parent: 0, data: Data::Blocks(a), subnodes: asubs });
    for (c, cid) in spec.children.iter().zip(child_ids) {
        folder(l, ids, cid, nid, c, nodes);
    }
}

/// The name-to-id map's PC.
fn named_map(spec: &FileSpec) -> (Vec<Vec<u8>>, Vec<SubNode>) {
    let l = &spec.layout;
    let mut guids: Vec<[u8; 16]> = Vec::new();
    let mut guid_index = |g: &[u8; 16]| -> u16 {
        if *g == crate::named::PS_MAPI {
            return 1;
        }
        if *g == crate::named::PS_PUBLIC_STRINGS {
            return 2;
        }
        if let Some(i) = guids.iter().position(|x| x == g) {
            return 3 + i as u16;
        }
        guids.push(*g);
        2 + guids.len() as u16
    };
    let mut entries = Vec::new();
    let mut strings = Vec::new();
    let mut index = 0u16;
    for (g, id) in &spec.named {
        let gi = guid_index(g);
        entries.extend_from_slice(&id.to_le_bytes());
        entries.extend_from_slice(&(gi << 1).to_le_bytes());
        entries.extend_from_slice(&index.to_le_bytes());
        index += 1;
    }
    for (g, name) in &spec.named_strings {
        let gi = guid_index(g);
        let at = strings.len() as u32;
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        strings.extend_from_slice(&(units.len() as u32).to_le_bytes());
        strings.extend_from_slice(&units);
        while strings.len() % 4 != 0 {
            strings.push(0);
        }
        entries.extend_from_slice(&at.to_le_bytes());
        entries.extend_from_slice(&((gi << 1) | 1).to_le_bytes());
        entries.extend_from_slice(&index.to_le_bytes());
        index += 1;
    }
    let props = vec![(0x0001, int32(251)), (0x0002, binary(&guids.concat())), (0x0003, binary(&entries)), (0x0004, binary(&strings))];
    let mut spill = Spill::new();
    let blocks = pc(l, &props, &mut spill);
    (blocks, spill.subnodes)
}

/// The whole file.
pub fn build(spec: &FileSpec) -> Vec<u8> {
    raw(&spec.layout, &nodes(spec))
}

/// The file's nodes, for tests that change some before writing them.
pub fn nodes(spec: &FileSpec) -> Vec<Node> {
    let l = &spec.layout;
    let mut nodes = Vec::new();
    let mut store = vec![
        (0x0FF9, binary(&RECORD_KEY)),
        (0x3001, text(l, &spec.name)),
        (0x35E0, binary(&entry_id(IPM_SUBTREE))),
        (0x35E7, binary(&entry_id(SEARCH_ROOT))),
    ];
    // A top-level folder named Deleted Items is the store's (its ids run from 0x403).
    if let Some(i) = spec.folders.iter().position(|f| f.name == "Deleted Items") {
        store.push((0x35E3, binary(&entry_id(((0x403 + i as u32) << 5) | 0x02))));
    }
    if let Some(p) = &spec.password {
        let bytes: Vec<u8> = if spec.password_ansi { p.chars().map(|c| c as u8).collect() } else { p.as_bytes().to_vec() };
        store.push((0x67FF, int32(crc32(&bytes) as i32)));
    }
    let mut spill = Spill::new();
    let blocks = pc(l, &store, &mut spill);
    nodes.push(Node { nid: 0x21, parent: 0, data: Data::Blocks(blocks), subnodes: spill.subnodes });
    let (nb, ns) = named_map(spec);
    nodes.push(Node { nid: 0x61, parent: 0, data: Data::Blocks(nb), subnodes: ns });

    let mut ids = Ids { folder: 0x402, message: 0x10000 };
    let root = FolderSpec {
        name: String::new(),
        class: None,
        messages: Vec::new(),
        children: vec![
            FolderSpec { name: "Top of Personal Folders".into(), class: None, messages: Vec::new(), children: spec.folders.clone() },
            FolderSpec { name: "Search Root".into(), class: None, messages: Vec::new(), children: Vec::new() },
        ],
    };
    folder_fixed(l, &mut ids, &root, &mut nodes);
    nodes
}

/// The root (0x122, its own parent), the IPM subtree and the search root with fixed ids.
fn folder_fixed(l: &Layout, ids: &mut Ids, root: &FolderSpec, nodes: &mut Vec<Node>) {
    let rows = vec![
        (IPM_SUBTREE, vec![(text_tag(l, 0x3001), text(l, &root.children[0].name)), (0x360A_000B, boolean(!root.children[0].children.is_empty()))]),
        (SEARCH_ROOT, vec![(text_tag(l, 0x3001), text(l, &root.children[1].name)), (0x360A_000B, boolean(false))]),
    ];
    let mut spill = Spill::new();
    let blocks = pc(l, &[(0x3001, text(l, "")), (0x360A, boolean(true)), (0x3602, int32(0)), (0x3603, int32(0))], &mut spill);
    nodes.push(Node { nid: 0x122, parent: 0x122, data: Data::Blocks(blocks), subnodes: spill.subnodes });
    let (h, hs) = tc(l, &hierarchy_columns(l), &rows);
    nodes.push(Node { nid: 0x12D, parent: 0, data: Data::Blocks(h), subnodes: hs });
    let (c, cs) = tc(l, &contents_columns(l), &[]);
    nodes.push(Node { nid: 0x12E, parent: 0, data: Data::Blocks(c), subnodes: cs });
    let (a, asubs) = tc(l, &[text_tag(l, 0x001A)], &[]);
    nodes.push(Node { nid: 0x12F, parent: 0, data: Data::Blocks(a), subnodes: asubs });
    folder(l, ids, IPM_SUBTREE, 0x122, &root.children[0], nodes);
    folder(l, ids, SEARCH_ROOT, 0x122, &root.children[1], nodes);
}

/// Writes raw nodes as a file (for tests that build broken structures themselves).
pub fn raw(layout: &Layout, nodes: &[Node]) -> Vec<u8> {
    Ndb::new(layout.clone()).finish(nodes)
}

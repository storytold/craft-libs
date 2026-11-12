//! Files made to cost far more to read than their size: one value or block named many times,
//! loads that were not counted, attached messages that hold themselves or multiply. Each is
//! held to a budget for the whole call, or refused.

use std::io::Cursor;

use super::writer::{self, AttachSpec, Data, FolderSpec, Layout, Node, Spill, SubNode, binary, hnid_ref, int32, nodes, pc, raw, tc, unicode};
use super::{first_item, note, pattern, sample};
use crate::header::{HEADER_READ, Header};
use crate::ndb::Ndb;
use crate::prop::types::BINARY;
use crate::source::Source;
use crate::{AttachmentContent, Cap, Encryption, Error, Format, Limits, MessageRef, Nid, Part, PropName, PropValue, Pst};

fn unicode_layout() -> Layout {
    Layout::new(Format::Unicode, Encryption::Permute)
}

fn open(bytes: &[u8], limits: Limits) -> Pst<Cursor<&[u8]>> {
    Pst::open_with(Cursor::new(bytes), limits).unwrap()
}

/// The first spilled value of a fresh [`Spill`] (and of a table's): its subnode id.
const FIRST_SPILL: u32 = (0x101 << 5) | 0x1F;

/// Bytes kept in binary values, to measure what a walk or a listing holds.
fn binary_bytes<'a>(props: impl Iterator<Item = &'a crate::Property>) -> usize {
    props.map(|p| if let PropValue::Binary(b) = &p.value { b.len() } else { 0 }).sum()
}

fn inbox(pst: &mut Pst<Cursor<&[u8]>>) -> Nid {
    pst.walk_folders().unwrap().folders.iter().find(|f| f.folder.name == "Inbox").unwrap().folder.id
}

#[test]
fn named_property_names_are_capped_one_by_one_and_in_all() {
    // One 2,048-byte name, then 5,000 entries that all name the same 1,000-byte string: a
    // stream of 3 KB that would come out as 5 MB of names.
    let mut strings = Vec::new();
    for len in [2048usize, 1000] {
        strings.extend_from_slice(&(len as u32).to_le_bytes());
        strings.extend(std::iter::repeat_n(b'a', len).enumerate().map(|(i, c)| if i % 2 == 0 { c } else { 0 }));
    }
    let mut entries = Vec::new();
    for i in 0..5001u32 {
        let at: u32 = if i == 0 { 0 } else { 2052 };
        entries.extend_from_slice(&at.to_le_bytes());
        entries.extend_from_slice(&((2u16 << 1) | 1).to_le_bytes());
        entries.extend_from_slice(&(i as u16).to_le_bytes());
    }
    let spec = sample(unicode_layout(), None);
    let mut all = nodes(&spec);
    let mut spill = Spill::new();
    let blocks = pc(&spec.layout, &[(0x0001, int32(251)), (0x0002, binary(&[])), (0x0003, binary(&entries)), (0x0004, binary(&strings))], &mut spill);
    let map = all.iter_mut().find(|n| n.nid == 0x61).unwrap();
    map.data = Data::Blocks(blocks);
    map.subnodes = spill.subnodes;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes, Limits::default());
    let named = pst.named_properties().unwrap();
    assert_eq!(named.left_out()[0], (Some(0x8000), Error::Cap(Cap::NamedProperties)), "a name over 1 KB is left out");
    assert!(named.left_out().iter().all(|(_, e)| *e == Error::Cap(Cap::NamedProperties)));
    assert_eq!(named.len() + named.left_out().len(), 5001, "every entry is read or named as left out");
    let kept: usize = named.iter().map(|p| if let PropName::Name(n) = &p.name { n.len() * 2 } else { 0 }).sum();
    assert!(kept <= crate::named::MAX_NAMES, "{kept}");
    assert!(named.len() >= 4000, "names within the total are kept: {}", named.len());
}

/// The file's folders under Contacts, each with 64 binary properties that all name one value of
/// `size` bytes.
fn aliased_folders(size: usize, count: usize) -> (Layout, Vec<u8>) {
    let mut spec = sample(unicode_layout(), None);
    spec.folders[2].children = (0..count).map(|i| FolderSpec { name: format!("Box {i}"), ..Default::default() }).collect();
    let mut all = nodes(&spec);
    for n in all.iter_mut().filter(|n| n.nid & 0x1F == 2 && n.nid != 0x122 && n.nid != writer::IPM_SUBTREE) {
        let mut spill = Spill::new();
        let blob = spill.put(0x1F, pattern(size, 3));
        assert_eq!(blob, FIRST_SPILL);
        let mut props = vec![(0x3001, unicode("Aliased"))];
        props.extend((0..64u16).map(|i| (0x6000 + i, hnid_ref(BINARY, blob))));
        n.data = Data::Blocks(pc(&spec.layout, &props, &mut spill));
        n.subnodes = spill.subnodes;
    }
    let bytes = raw(&spec.layout, &all);
    (spec.layout, bytes)
}

#[test]
fn a_folder_walk_reads_within_one_budget() {
    // 25 folders × 64 names of one 60 KB value: 96 MB from a file of under 2 MB.
    let (_, bytes) = aliased_folders(60_000, 20);
    assert!(bytes.len() < 2_000_000);
    let limit = 4 * 1024 * 1024;
    let mut pst = open(&bytes, Limits { max_listing_bytes: limit, ..Default::default() });
    let tree = pst.walk_folders().unwrap();
    let kept: usize = tree.folders.iter().map(|f| binary_bytes(f.folder.properties.iter())).sum();
    assert!(kept as u64 <= limit, "{kept}");
    assert!(tree.left_out.iter().any(|l| l.reason == Error::Cap(Cap::ListingSize)), "{:?}", tree.left_out);
}

#[test]
fn a_folder_walk_leaves_large_values_for_later() {
    let (_, bytes) = aliased_folders(100_000, 1);
    let mut pst = open(&bytes, Limits::default());
    let tree = pst.walk_folders().unwrap();
    let boxed = tree.folders.iter().find(|f| f.folder.name == "Aliased").unwrap().folder.clone();
    assert_eq!(boxed.properties.get(0x6000), Some(&PropValue::Large { size: 100_000 }), "past the summary cap");
    let whole = pst.folder(boxed.id).unwrap();
    assert_eq!(whole.properties.get(0x6000), Some(&PropValue::Binary(pattern(100_000, 3))), "read whole on its own");
}

#[test]
fn a_listing_of_items_reads_within_one_budget() {
    // 200 rows whose binary cell names the first row's 50 KB value: 10 MB from 60 KB.
    let spec = sample(unicode_layout(), None);
    let mut all = nodes(&spec);
    let tag = 0x6100_0102u32;
    let rows: Vec<(u32, Vec<(u32, writer::Value)>)> = (0..200u32)
        .map(|i| {
            let v = if i == 0 { binary(&pattern(50_000, 9)) } else { hnid_ref(BINARY, FIRST_SPILL) };
            (((0x20000 + i) << 5) | 4, vec![(tag, v)])
        })
        .collect();
    let (blocks, subs) = tc(&spec.layout, &[tag], &rows);
    let inbox_contents = (0x403 << 5) | 0x0E;
    let table = all.iter_mut().find(|n| n.nid == inbox_contents).unwrap();
    table.data = Data::Blocks(blocks);
    table.subnodes = subs;
    let bytes = raw(&spec.layout, &all);
    assert!(bytes.len() < 200_000, "{}", bytes.len());
    let limit = 1024 * 1024;
    let mut pst = open(&bytes, Limits { max_listing_bytes: limit, ..Default::default() });
    let folder = inbox(&mut pst);
    let list = pst.items(folder).unwrap();
    // Listed until the budget is spent; the rest are counted, and the folder named once.
    assert!(!list.items.is_empty() && list.items.len() < 200, "{}", list.items.len());
    assert_eq!(list.past_cap, 200 - list.items.len());
    let kept: usize = list.items.iter().map(|i| binary_bytes(i.properties.iter())).sum();
    assert!(kept as u64 <= limit, "{kept}");
    let short: Vec<&crate::LeftOut> = list.left_out.iter().filter(|l| l.reason == Error::Cap(Cap::ListingSize)).collect();
    assert_eq!(short.len(), 1);
    assert_eq!((short[0].id, short[0].parent), (folder, None));
}

#[test]
fn a_data_tree_naming_one_block_twice_is_refused() {
    for levels in [1u8, 2] {
        let layout = Layout::new(Format::Unicode, Encryption::None);
        let store = Node { nid: 0x21, parent: 0, data: Data::Repeated { piece: vec![0xEC; 8000], times: 32, levels }, subnodes: vec![] };
        let bytes = raw(&layout, &[store]);
        assert!(bytes.len() < 30_000);
        assert_eq!(Pst::open(Cursor::new(&bytes[..])).err(), Some(Error::Cycle("a data tree")), "levels {levels}");
    }
}

#[test]
fn every_load_of_a_message_is_charged_to_its_budget() {
    // 30 attachments whose data (3,000 bytes each) is kept in their own heaps: each heap is
    // loaded, and counts, though the data itself is left for later.
    let mut spec = sample(unicode_layout(), None);
    let mut m = note("Many small files");
    m.attachments =
        (0..30).map(|i| AttachSpec::File { name: format!("f{i}.bin"), mime: "application/octet-stream".into(), data: pattern(3000, i) }).collect();
    spec.folders[0].messages = vec![m];
    let bytes = build_spec(&spec);
    let mut pst = open(&bytes, Limits::default());
    let item = first_item(&mut pst);
    assert_eq!(pst.message(&MessageRef::item(item)).unwrap().attachments.len(), 30);
    let mut pst = open(&bytes, Limits { max_item_bytes: 60_000, ..Default::default() });
    let item = first_item(&mut pst);
    assert_eq!(pst.message(&MessageRef::item(item)).err(), Some(Error::Cap(Cap::ItemSize)));
}

fn build_spec(spec: &writer::FileSpec) -> Vec<u8> {
    writer::build(spec)
}

#[test]
fn all_that_is_read_is_held_to_the_file_size() {
    let bytes = build_spec(&sample(unicode_layout(), None));
    let mut pst = open(&bytes, Limits { read_multiple: 3, read_floor: 0, ..Default::default() });
    let mut stopped = None;
    for _ in 0..100 {
        // A walk names what it could not read in its left_out, or fails as a whole.
        let reason = match pst.walk_folders() {
            Err(e) => Some(e),
            Ok(tree) => tree.left_out.first().map(|l| l.reason.clone()),
        };
        if reason.is_some() {
            stopped = reason;
            break;
        }
    }
    assert_eq!(stopped, Some(Error::Cap(Cap::TotalRead)));
    let mut pst = open(&bytes, Limits::default());
    for _ in 0..20 {
        pst.walk_folders().unwrap();
    }
}

/// A file whose first Inbox item has one attachment: a message whose object is `object`.
fn with_object(object: SubNode) -> Vec<u8> {
    let mut spec = sample(unicode_layout(), None);
    let mut m = note("Holds itself");
    m.attachments = vec![AttachSpec::Object(object)];
    spec.folders[0].messages = vec![m];
    build_spec(&spec)
}

const OBJECT: u32 = (0x300 << 5) | 0x04;
const ATTACHMENT: u32 = (0x200 << 5) | 0x05;

/// The data and subnode tree of a node, read from the file's node tree.
fn node_bids(bytes: &[u8], nid: u32) -> (u64, u64) {
    let limits = Limits::default();
    let mut src = Source::new(Cursor::new(bytes), &limits).unwrap();
    let header = Header::parse(&src.read_at(0, HEADER_READ).unwrap()).unwrap();
    let e = Ndb::new(src, header, limits).node(nid).unwrap();
    (e.data, e.sub)
}

#[test]
fn an_attached_message_that_is_the_message_it_is_attached_to_is_a_cycle() {
    // First with a stand-in for the object, to learn where the message's own data and subnode
    // tree are; then with the object naming them. Nothing else moves between the two.
    let first = with_object(SubNode::with_sub_bid(OBJECT, 4, 6));
    let mut pst = open(&first, Limits::default());
    let item = first_item(&mut pst);
    let (data, sub) = node_bids(&first, item.0);
    let bytes = with_object(SubNode::with_sub_bid(OBJECT, data, sub));
    assert_eq!(node_bids(&bytes, item.0), (data, sub));
    let mut pst = open(&bytes, Limits::default());
    let m = pst.message(&MessageRef::item(item)).unwrap();
    assert!(m.attachments.is_empty());
    assert_eq!(m.left_out[0].part, Part::Attachment { id: Nid(ATTACHMENT) });
    assert_eq!(m.left_out[0].reason, Error::Cycle("an attached message"));
    let inner = MessageRef { node: item, path: vec![Nid(ATTACHMENT), Nid(OBJECT)] };
    assert_eq!(pst.message(&inner).err(), Some(Error::Cycle("an attached message")));
}

#[test]
fn an_attachment_table_naming_a_row_twice_is_refused() {
    let mut spec = sample(unicode_layout(), None);
    let mut m = note("Twice");
    m.attachments = vec![
        AttachSpec::File { name: "a.txt".into(), mime: "text/plain".into(), data: b"one".to_vec() },
        AttachSpec::File { name: "b.txt".into(), mime: "text/plain".into(), data: b"two".to_vec() },
    ];
    spec.folders[0].messages = vec![m];
    let mut all = nodes(&spec);
    let message = all.iter_mut().find(|n| n.nid & 0x1F == 4).unwrap();
    let rows = vec![(ATTACHMENT, vec![(0x3705_0003, int32(1))]), (ATTACHMENT, vec![(0x3705_0003, int32(1))])];
    let (blocks, subs) = tc(&spec.layout, &[0x3705_0003], &rows);
    let table = message.subnodes.iter_mut().find(|s| s.nid == 0x671).unwrap();
    table.data = Data::Blocks(blocks);
    table.subnodes = subs;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes, Limits::default());
    let item = first_item(&mut pst);
    // A table's row ids are the keys of its row index, which must strictly increase.
    let m = pst.message(&MessageRef::item(item)).unwrap();
    assert!(m.attachments.is_empty());
    assert_eq!(m.left_out[0].part, Part::AttachmentTable);
    assert_eq!(m.left_out[0].reason, Error::Corrupt("a B-tree-on-heap's keys out of order"));
}

#[test]
fn attached_messages_are_counted_per_item() {
    // Three attached messages, each with three of its own: twelve under the item.
    let mut spec = sample(unicode_layout(), None);
    let leaf = note("Leaf");
    let mut middle = note("Middle");
    middle.attachments = vec![AttachSpec::Message(Box::new(leaf)); 3];
    let mut top = note("Top");
    top.attachments = vec![AttachSpec::Message(Box::new(middle)); 3];
    spec.folders[0].messages = vec![top.clone(), top];
    let bytes = build_spec(&spec);
    let mut pst = open(&bytes, Limits { max_embedded_messages: 5, ..Default::default() });
    let folder = inbox(&mut pst);
    let items = pst.items(folder).unwrap().items;
    // The first item twice: reading it again starts its count again.
    for item in [&items[0], &items[0], &items[1]] {
        let (mut read, mut refused) = (0, 0);
        let mut queue = vec![MessageRef::item(item.id)];
        while let Some(m) = queue.pop() {
            match pst.message(&m) {
                Ok(msg) => {
                    read += usize::from(!m.path.is_empty());
                    for a in msg.attachments {
                        if let AttachmentContent::Message(inner) = a.content {
                            queue.push(inner);
                        }
                    }
                }
                Err(e) => {
                    assert_eq!(e, Error::Cap(Cap::EmbeddedMessages));
                    refused += 1;
                }
            }
        }
        assert_eq!((read, refused), (5, 4), "counted again for each item");
    }
}

/// A heap block (not the first) whose page map holds `k` items that all span the same leaf of
/// `l` property records: the odd items run from its start to its end, the even ones back.
fn aliasing_block(k: usize, l: usize) -> Vec<u8> {
    let mut b = vec![0u8; 2];
    let start = b.len() as u16;
    for i in 0..l {
        b.extend_from_slice(&((0x6000 + (i % 0x1000)) as u16).to_le_bytes());
        b.extend_from_slice(&0x0003u16.to_le_bytes());
        b.extend_from_slice(&(i as u32).to_le_bytes());
    }
    let end = b.len() as u16;
    b[0..2].copy_from_slice(&end.to_le_bytes());
    b.extend_from_slice(&((2 * k - 1) as u16).to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    for i in 0..2 * k {
        b.extend_from_slice(&(if i % 2 == 0 { start } else { end }).to_le_bytes());
    }
    b
}

/// A message store whose property context's index names every aliasing item of `blocks`
/// aliasing blocks: `blocks × k × l` records from a few kilobytes.
fn aliased_store(blocks: usize, k: usize, l: usize) -> Vec<u8> {
    let mut index = Vec::new();
    for j in 1..=blocks {
        for i in 0..k {
            index.extend_from_slice(&0x6000u16.to_le_bytes());
            index.extend_from_slice(&(((j as u32) << 16) | (((2 * i + 1) as u32) << 5)).to_le_bytes());
        }
    }
    let mut b0 = vec![0u8; 12];
    b0[2] = 0xEC;
    b0[3] = 0xBC;
    b0[4..8].copy_from_slice(&(1u32 << 5).to_le_bytes());
    let s1 = b0.len() as u16;
    b0.extend_from_slice(&[0xB5, 2, 6, 1]);
    b0.extend_from_slice(&(2u32 << 5).to_le_bytes());
    let s2 = b0.len() as u16;
    b0.extend_from_slice(&index);
    let s3 = b0.len() as u16;
    b0[0..2].copy_from_slice(&s3.to_le_bytes());
    b0.extend_from_slice(&2u16.to_le_bytes());
    b0.extend_from_slice(&0u16.to_le_bytes());
    for o in [s1, s2, s3] {
        b0.extend_from_slice(&o.to_le_bytes());
    }
    let mut heap = vec![b0];
    heap.extend((0..blocks).map(|_| aliasing_block(k, l)));
    let layout = Layout::new(Format::Unicode, Encryption::None);
    let mut all = nodes(&sample(layout.clone(), None));
    let store = all.iter_mut().find(|n| n.nid == 0x21).unwrap();
    store.data = Data::Blocks(heap);
    store.subnodes = vec![];
    raw(&layout, &all)
}

#[test]
fn heap_items_that_share_bytes_are_refused() {
    // 2 blocks × 500 items × 450 records: 450,000 properties from a file of about 70 KB.
    let bytes = aliased_store(2, 500, 450);
    assert!(bytes.len() < 100_000, "{}", bytes.len());
    assert_eq!(Pst::open(Cursor::new(&bytes[..])).err(), Some(Error::Corrupt("a heap's page map")));
    // A page map whose offsets run back, alone.
    let mut block = vec![12u8, 0, 0xEC, 0xBC, 0, 0, 0, 0, 0, 0, 0, 0];
    block.extend_from_slice(&[2, 0, 0, 0, 12, 0, 8, 0, 12, 0]);
    assert!(crate::heap::Heap::new(vec![block]).is_err());
}

#[test]
fn a_property_context_naming_an_id_twice_is_refused() {
    // 65,537 records: one id is named twice, whatever the order.
    let spec = sample(unicode_layout(), None);
    let mut all = nodes(&spec);
    let props: Vec<(u16, writer::Value)> = (0..=65_536u32).map(|i| (i as u16, int32(1))).collect();
    let mut spill = Spill::new();
    let store = all.iter_mut().find(|n| n.nid == 0x21).unwrap();
    store.data = Data::Blocks(pc(&spec.layout, &props, &mut spill));
    let bytes = raw(&spec.layout, &all);
    assert_eq!(Pst::open(Cursor::new(&bytes[..])).err(), Some(Error::Corrupt("a B-tree-on-heap's keys out of order")));
}

#[test]
fn a_table_that_cannot_be_read_is_named_not_taken_for_none() {
    // The Inbox's hierarchy and contents tables name a block that isn't in the file.
    let spec = sample(unicode_layout(), None);
    let mut all = nodes(&spec);
    for kind in [0x0D, 0x0E] {
        let table = all.iter_mut().find(|n| n.nid == (0x403 << 5) | kind).unwrap();
        table.data = Data::Bid(0x9_9990);
        table.subnodes = vec![];
    }
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes, Limits::default());
    let tree = pst.walk_folders().unwrap();
    let inbox = Nid((0x403 << 5) | 2);
    assert!(tree.folders.iter().any(|f| f.folder.id == inbox));
    assert!(!tree.folders.iter().any(|f| f.folder.name == "Projects"));
    assert_eq!(tree.left_out.len(), 1);
    assert_eq!((tree.left_out[0].id, &tree.left_out[0].reason), (inbox, &Error::NotFound("a block")));
    assert_eq!(pst.items(inbox).err(), Some(Error::NotFound("a block")));
    // A folder with no tables at all has no subfolders and no items.
    let mut all = nodes(&spec);
    // The folder made last (Projects), without its tables.
    let bare = Nid(all.iter().filter(|n| n.nid & 0x1F == 2).map(|n| n.nid).max().unwrap());
    all.retain(|n| n.nid & !0x1F != bare.0 & !0x1F || n.nid == bare.0);
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes, Limits::default());
    assert_eq!(pst.subfolders(bare), Ok(vec![]));
    assert!(pst.items(bare).unwrap().items.is_empty());
}

#[test]
fn an_item_and_its_attached_messages_share_one_budget() {
    let body = "Words, and more words. ".repeat(1500);
    let mut inner = note("Inside");
    inner.body = body.clone();
    let mut top = note("Outside");
    top.body = body;
    top.attachments = vec![AttachSpec::Message(Box::new(inner))];
    let mut spec = sample(unicode_layout(), None);
    spec.folders[0].messages = vec![top];
    let bytes = build_spec(&spec);
    // Each message costs about 105 KB, read and kept: one fits in 150 KB, the two together don't.
    let mut pst = open(&bytes, Limits { max_item_bytes: 150_000, ..Default::default() });
    let item = first_item(&mut pst);
    let m = pst.message(&MessageRef::item(item)).unwrap();
    let AttachmentContent::Message(inner) = m.attachments[0].content.clone() else { panic!() };
    assert_eq!(pst.message(&inner).err(), Some(Error::Cap(Cap::ItemSize)));
    // Reading the item again starts its budget again.
    pst.message(&MessageRef::item(item)).unwrap();
    let mut pst = open(&bytes, Limits::default());
    pst.message(&MessageRef::item(item)).unwrap();
    pst.message(&inner).unwrap();
}

/// What a call's answer keeps, as the reader charges it: each structure, its strings, and
/// each property with what it holds. (A vector's spare room, at most as much again, is not.)
fn kept_props(p: &crate::Properties) -> usize {
    p.iter().map(|p| p.kept() as usize).sum()
}

fn kept_text(s: &Option<String>) -> usize {
    s.as_ref().map_or(0, String::capacity)
}

#[test]
fn what_a_call_keeps_is_within_its_budget() {
    use std::mem::size_of;
    // One byte on file becomes a whole property kept: 250 one-byte columns in 2,000 rows,
    // 8,000 one-byte properties in a message, 300 folders.
    let mut spec = sample(unicode_layout(), None);
    spec.folders[0].children = (0..300).map(|i| FolderSpec { name: format!("Folder {i}"), ..Default::default() }).collect();
    let mut all = nodes(&spec);
    let columns: Vec<u32> = (0..250u32).map(|i| ((0x6800 + i) << 16) | 0x000B).collect();
    let rows: Vec<(u32, Vec<(u32, writer::Value)>)> =
        (0..2000u32).map(|i| (((0x20000 + i) << 5) | 4, columns.iter().map(|&c| (c, writer::boolean(true))).collect())).collect();
    let (blocks, subs) = tc(&spec.layout, &columns, &rows);
    let table = all.iter_mut().find(|n| n.nid == (0x403 << 5) | 0x0E).unwrap();
    table.data = Data::Blocks(blocks);
    table.subnodes = subs;
    let props: Vec<(u16, writer::Value)> = (0..8000u16).map(|i| (0x8000 + i, writer::boolean(true))).collect();
    let mut spill = Spill::new();
    let message = all.iter_mut().find(|n| n.nid & 0x1F == 4).unwrap();
    let item = Nid(message.nid);
    message.data = Data::Blocks(pc(&spec.layout, &props, &mut spill));
    message.subnodes = spill.subnodes;
    let bytes = raw(&spec.layout, &all);
    assert!(bytes.len() < 1_500_000, "{}", bytes.len());

    // A walk of the folders.
    let budget = 64 * 1024;
    let mut pst = open(&bytes, Limits { max_listing_bytes: budget, ..Default::default() });
    let tree = pst.walk_folders().unwrap();
    let kept: usize = tree
        .folders
        .iter()
        .map(|f| size_of::<crate::FolderEntry>() + f.folder.name.capacity() + kept_text(&f.folder.container_class) + kept_props(&f.folder.properties))
        .sum::<usize>()
        + tree.left_out.len() * size_of::<crate::LeftOut>();
    assert!(kept <= budget as usize, "{kept}");
    assert!(tree.left_out.iter().any(|l| l.reason == Error::Cap(Cap::ListingSize)));

    // A listing of items.
    let inbox = Nid((0x403 << 5) | 2);
    let budget = 256 * 1024;
    let mut pst = open(&bytes, Limits { max_listing_bytes: budget, ..Default::default() });
    let list = pst.items(inbox).unwrap();
    let kept: usize = list.items.iter().map(|i| size_of::<crate::ItemSummary>() + kept_text(&i.class) + kept_props(&i.properties)).sum::<usize>()
        + list.left_out.len() * size_of::<crate::LeftOut>();
    assert!(kept <= budget as usize, "{kept}");
    assert!(list.past_cap > 0 && list.items.len() + list.past_cap == 2000);

    // A message: what it keeps is far more than what it reads, and is refused past the cap.
    let mut pst = open(&bytes, Limits::default());
    let m = pst.message(&MessageRef::item(item)).unwrap();
    let kept = size_of::<crate::Message>() + kept_text(&m.class) + kept_props(&m.properties);
    let budget = 128 * 1024;
    assert!(kept > budget as usize, "{kept}");
    let mut pst = open(&bytes, Limits { max_item_bytes: budget, ..Default::default() });
    assert_eq!(pst.message(&MessageRef::item(item)).err(), Some(Error::Cap(Cap::ItemSize)));
}

#[test]
fn a_list_value_holds_no_more_than_its_cap() {
    // 100,000 empty binaries: 400 KB in the file, each a vector of its own in memory. And
    // 100,000 two-byte numbers, which hold just their bytes.
    let count = 100_000u32;
    let mut empties = count.to_le_bytes().to_vec();
    for _ in 0..count {
        empties.extend_from_slice(&(4 + 4 * count).to_le_bytes());
    }
    let numbers: Vec<u8> = (0..count).flat_map(|i| (i as u16).to_le_bytes()).collect();
    let (binaries, int16s) = (crate::prop::types::BINARY | 0x1000, crate::prop::types::INT16 | 0x1000);
    // A list of booleans (whose items aren't read), with the same table, and with a count
    // that no table could hold.
    let booleans = crate::prop::types::BOOLEAN | 0x1000;
    let props = vec![
        (0x6900u16, writer::Value { ty: binaries, bytes: empties.clone() }),
        (0x6901, writer::Value { ty: int16s, bytes: numbers.clone() }),
        (0x6902, writer::Value { ty: booleans, bytes: empties.clone() }),
        (0x6903, writer::Value { ty: booleans, bytes: vec![0xFF; 8] }),
    ];
    let spec = sample(unicode_layout(), None);
    let mut all = nodes(&spec);
    let mut spill = Spill::new();
    let message = all.iter_mut().find(|n| n.nid & 0x1F == 4).unwrap();
    let item = MessageRef::item(Nid(message.nid));
    message.data = Data::Blocks(pc(&spec.layout, &props, &mut spill));
    message.subnodes = spill.subnodes;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes, Limits::default());
    // Within a cap of twice its bytes: refused before it is made.
    let max = 2 * empties.len() as u64;
    assert_eq!(pst.read_property(&item, 0x6900, max).err(), Some(Error::Cap(Cap::ItemSize)));
    let whole = pst.read_property(&item, 0x6900, 64 << 20).unwrap().unwrap();
    let PropValue::MultiBinary(v) = &whole else { panic!() };
    assert_eq!(v.len(), count as usize);
    assert!(whole.held() as u64 <= 64 << 20);
    // What is checked before it is made covers what making it holds.
    assert!(crate::prop::building_cost(binaries, &empties) >= whole.held(), "{}", whole.held());
    assert_eq!(crate::prop::building_cost(int16s, &numbers), 0);
    // The numbers fit a cap of exactly their bytes.
    let Some(PropValue::MultiInt16(n)) = pst.read_property(&item, 0x6901, numbers.len() as u64).unwrap() else { panic!() };
    assert_eq!((n.len(), n.capacity(), n[7]), (count as usize, count as usize, 7), "no spare room past what is charged");
    // Lists of other types are kept as their bytes, within a cap of exactly those, and
    // whatever their bytes say.
    let max = empties.len() as u64;
    assert_eq!(pst.read_property(&item, 0x6902, max).unwrap(), Some(PropValue::Unknown { ty: booleans, data: empties.clone() }));
    assert_eq!(pst.read_property(&item, 0x6903, max).unwrap(), Some(PropValue::Unknown { ty: booleans, data: vec![0xFF; 8] }));
    assert_eq!(crate::prop::building_cost(booleans, &empties), 0);
    // And in a message the list is refused within the item's cap, not made.
    let mut pst = open(&bytes, Limits { max_item_bytes: 1 << 20, max_inline_value: 1 << 20, ..Default::default() });
    let m = pst.message(&item);
    assert_eq!(m.err(), Some(Error::Cap(Cap::ItemSize)));
}

#[test]
fn a_named_entry_that_names_no_id_is_left_out_alone() {
    // Three numeric names; the middle one's index (0x8000) names no id.
    let mut entries = Vec::new();
    for (value, index) in [(0x10u32, 0u16), (0x11, 0x8000), (0x12, 1)] {
        entries.extend_from_slice(&value.to_le_bytes());
        entries.extend_from_slice(&(1u16 << 1).to_le_bytes());
        entries.extend_from_slice(&index.to_le_bytes());
    }
    let spec = sample(unicode_layout(), None);
    let mut all = nodes(&spec);
    let mut spill = Spill::new();
    let blocks = pc(&spec.layout, &[(0x0001, int32(251)), (0x0002, binary(&[])), (0x0003, binary(&entries)), (0x0004, binary(&[]))], &mut spill);
    let map = all.iter_mut().find(|n| n.nid == 0x61).unwrap();
    map.data = Data::Blocks(blocks);
    map.subnodes = spill.subnodes;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes, Limits::default());
    let named = pst.named_properties().unwrap();
    assert_eq!(named.len(), 2);
    assert_eq!(named.get(0x8001).unwrap().name, PropName::Id(0x12));
    assert_eq!(named.left_out(), &[(None, Error::Corrupt("a named property's index"))]);
}

//! Nothing that is left out vanishes: a damaged property, row or name is named with its
//! reason. And the file variants the reader is tolerant of, or refuses in words: the reserved
//! bit A of block ids, Windows-1252 passwords, versions 36 and 37.

use std::io::Cursor;

use super::writer::{self, Data, Layout, binary, build, hnid_ref, int32, nodes, raw, tc};
use super::{first_item, note, pattern, sample};
use crate::crc::crc32;
use crate::prop::types::BINARY;
use crate::{Encryption, Error, Format, MessageRef, Nid, Part, PropValue, Pst};

fn open(bytes: &[u8]) -> Pst<Cursor<&[u8]>> {
    Pst::open(Cursor::new(bytes)).unwrap()
}

/// A subnode id that names nothing.
const NOWHERE: u32 = (0x7777 << 5) | 0x1F;

fn inbox(pst: &mut Pst<Cursor<&[u8]>>) -> Nid {
    pst.walk_folders().unwrap().folders.iter().find(|f| f.folder.name == "Inbox").unwrap().folder.id
}

#[test]
fn a_damaged_property_of_a_message_is_named_in_left_out() {
    let mut spec = sample(Layout::new(Format::Unicode, Encryption::Permute), None);
    let mut m = note("A property that points nowhere");
    m.extra = vec![(0x6001, hnid_ref(BINARY, NOWHERE)), (0x6002, binary(b"fine"))];
    spec.folders[0].messages = vec![m];
    let bytes = build(&spec);
    let mut pst = open(&bytes);
    let item = first_item(&mut pst);
    let m = pst.message(&MessageRef::item(item)).unwrap();
    assert!(matches!(m.properties.get(0x6001), Some(PropValue::Damaged { ty: BINARY, reason: Error::NotFound(_) })));
    assert_eq!(m.properties.get(0x6002), Some(&PropValue::Binary(b"fine".to_vec())));
    assert_eq!(m.left_out.len(), 1);
    assert_eq!(m.left_out[0].part, Part::Property { id: 0x6001 });
}

#[test]
fn a_variable_cell_too_short_for_its_reference_loses_that_property_only() {
    let spec = sample(Layout::new(Format::Unicode, Encryption::None), None);
    let mut all = nodes(&spec);
    let (tag, number) = (0x6100_0102u32, 0x6101_0003u32);
    let rows = vec![(((0x20000) << 5) | 4, vec![(tag, binary(b"ok")), (number, int32(7))])];
    let (mut blocks, subs) = tc(&spec.layout, &[tag, number], &rows);
    // The binary column's description (tag, offset, width, bit): its width cut to 2 bytes.
    let heap = &mut blocks[0];
    let at = heap.windows(4).position(|w| w == tag.to_le_bytes()).unwrap();
    assert_eq!(heap[at + 6], 4);
    heap[at + 6] = 2;
    let table = all.iter_mut().find(|n| n.nid == (0x403 << 5) | 0x0E).unwrap();
    table.data = Data::Blocks(blocks);
    table.subnodes = subs;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes);
    let folder = inbox(&mut pst);
    let list = pst.items(folder).unwrap();
    assert_eq!(list.items.len(), 1);
    let props = &list.items[0].properties;
    assert_eq!(props.get(0x6101), Some(&PropValue::Int32(7)), "the rest of the row is read");
    assert!(matches!(props.get(0x6100), Some(PropValue::Damaged { .. })));
    assert_eq!(list.left_out.len(), 1);
    assert_eq!((list.left_out[0].property, &list.left_out[0].reason), (Some(0x6100), &Error::Corrupt("a structure runs past its end")));
}

#[test]
fn a_damaged_cell_of_an_item_list_is_named_in_left_out() {
    let spec = sample(Layout::new(Format::Unicode, Encryption::None), None);
    let mut all = nodes(&spec);
    let tag = 0x6100_0102u32;
    let rows = vec![(((0x20000) << 5) | 4, vec![(tag, hnid_ref(BINARY, NOWHERE))]), (((0x20001) << 5) | 4, vec![(tag, binary(b"ok"))])];
    let (blocks, subs) = tc(&spec.layout, &[tag], &rows);
    let table = all.iter_mut().find(|n| n.nid == (0x403 << 5) | 0x0E).unwrap();
    table.data = Data::Blocks(blocks);
    table.subnodes = subs;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes);
    let folder = inbox(&mut pst);
    let list = pst.items(folder).unwrap();
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.left_out.len(), 1);
    let l = &list.left_out[0];
    assert_eq!((l.id, l.parent, l.property), (Nid(((0x20000) << 5) | 4), Some(folder), Some(0x6100)));
    assert!(matches!(l.reason, Error::NotFound(_)));
}

#[test]
fn a_row_that_cannot_be_read_is_listed_and_named() {
    // The row index names a row id that its row in the matrix doesn't have.
    let spec = sample(Layout::new(Format::Unicode, Encryption::None), None);
    let mut all = nodes(&spec);
    let rows = vec![(((0x20000) << 5) | 4, vec![(0x0E07_0003, int32(1))])];
    let (mut blocks, subs) = tc(&spec.layout, &[0x0E07_0003], &rows);
    // The matrix is the heap item holding the row id: change the id in it, not in the index.
    let id = (((0x20000u32) << 5) | 4).to_le_bytes();
    let block = &mut blocks[0];
    let at = block.windows(4).rposition(|w| w == id).unwrap();
    block[at] ^= 0x80;
    let table = all.iter_mut().find(|n| n.nid == (0x403 << 5) | 0x0E).unwrap();
    table.data = Data::Blocks(blocks);
    table.subnodes = subs;
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes);
    let folder = inbox(&mut pst);
    let list = pst.items(folder).unwrap();
    assert_eq!(list.items.len(), 1);
    assert!(list.items[0].properties.is_empty());
    assert_eq!(list.left_out.len(), 1);
    assert_eq!(list.left_out[0].property, None);
    assert!(matches!(list.left_out[0].reason, Error::Corrupt(_) | Error::NotFound(_)), "{:?}", list.left_out[0].reason);
}

#[test]
fn passwords_are_also_tried_in_windows_1252() {
    for (password, ansi) in [("Müller", true), ("Müller", false), ("plain", true)] {
        let mut spec = sample(Layout::new(Format::Ansi, Encryption::Permute), Some(password));
        spec.password_ansi = ansi;
        let bytes = build(&spec);
        let mut pst = open(&bytes);
        assert!(pst.password_required());
        assert_eq!(pst.unlock("Muller"), Err(Error::WrongPassword));
        pst.unlock(password).unwrap();
    }
    // A character of the 0x80 to 0x9F range of the code page (the euro sign, 0x80).
    let mut spec = sample(Layout::new(Format::Unicode, Encryption::None), Some("x"));
    let mut all = nodes(&spec);
    spec.password = None;
    let mut spill = writer::Spill::new();
    let store = writer::pc(&spec.layout, &[(0x3001, writer::unicode("Personal Folders")), (0x67FF, int32(crc32(&[0x80, b'5']) as i32))], &mut spill);
    let n = all.iter_mut().find(|n| n.nid == 0x21).unwrap();
    n.data = Data::Blocks(store);
    let bytes = raw(&spec.layout, &all);
    let mut pst = open(&bytes);
    assert_eq!(pst.unlock("e5"), Err(Error::WrongPassword));
    pst.unlock("€5").unwrap();
}

fn with_version(version: u16) -> Vec<u8> {
    let mut b = build(&sample(Layout::new(Format::Unicode, Encryption::None), None));
    b[10..12].copy_from_slice(&version.to_le_bytes());
    let full = crc32(&b[8..524]);
    b[524..528].copy_from_slice(&full.to_le_bytes());
    let partial = crc32(&b[8..479]);
    b[4..8].copy_from_slice(&partial.to_le_bytes());
    b
}

#[test]
fn offline_files_with_4k_pages_and_protected_files_are_refused_in_words() {
    for (version, words) in [(36u16, "4 KB pages"), (37, "Windows Information Protection")] {
        let b = with_version(version);
        let e = Pst::open(Cursor::new(&b[..])).err().unwrap();
        assert_eq!(e, Error::UnsupportedVersion(version));
        assert!(e.to_string().contains(words), "{e}");
    }
    assert!(Pst::open(Cursor::new(&with_version(24)[..])).is_ok(), "other Unicode versions are read");
}

#[test]
fn the_reserved_bit_a_of_block_ids_is_ignored() {
    for (refs, trailers) in [(true, false), (false, true), (true, true)] {
        for format in [Format::Unicode, Format::Ansi] {
            let layout = Layout { bit_a_in_references: refs, bit_a_in_trailers: trailers, ..Layout::tiny(format, Encryption::Cyclic) };
            let bytes = build(&sample(layout, None));
            let mut pst = open(&bytes);
            let what = format!("{format:?} refs {refs} trailers {trailers}");
            let tree = pst.walk_folders().unwrap();
            assert!(tree.left_out.is_empty(), "{what}: {:?}", tree.left_out);
            assert_eq!(tree.folders.len(), 7, "{what}");
            let item = pst.items(tree.folders[2].folder.id).unwrap().items[0].id;
            let m = pst.message(&MessageRef::item(item)).unwrap();
            assert!(m.left_out.is_empty(), "{what}: {:?}", m.left_out);
            assert_eq!(pst.attachment_bytes(&m.attachments[0].id).unwrap(), pattern(20_000, 7), "{what}");
        }
    }
}

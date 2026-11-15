//! Damaged files: checksums, offsets, trees that lead back to themselves. Each ends in an
//! error value, never a panic.

use std::io::Cursor;

use super::writer::{self, HeapBuilder, Layout, build, nodes, raw};
use super::{exercise, sample};
use crate::crc::crc32;
use crate::header::{HEADER_READ, Header};
use crate::heap::Heap;
use crate::ndb::{Ndb, signature};
use crate::source::Source;
use crate::{Encryption, Error, Format, Limits, Pst};

fn base(format: Format) -> Vec<u8> {
    build(&sample(Layout::tiny(format, Encryption::Cyclic), None))
}

fn open(bytes: &[u8]) -> crate::Result<Pst<Cursor<&[u8]>>> {
    Pst::open(Cursor::new(bytes))
}

/// Puts a page's CRC right after it has been changed.
fn fix_page(bytes: &mut [u8], format: Format, ib: usize) {
    let t = 512 - format.trailer_size();
    let crc = crc32(&bytes[ib..ib + t]);
    let at = ib + t + if format == Format::Unicode { 4 } else { 8 };
    bytes[at..at + 4].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn not_a_pst_file() {
    assert_eq!(open(b"").err(), Some(Error::NotPst));
    assert_eq!(open(b"!BDN").err(), Some(Error::NotPst));
    assert_eq!(open(&[0u8; 600]).err(), Some(Error::NotPst));
    let mut b = base(Format::Unicode);
    b[10] = 20; // wVer
    assert_eq!(open(&b).err(), Some(Error::UnsupportedVersion(20)));
}

#[test]
fn a_damaged_header_is_refused() {
    for format in [Format::Unicode, Format::Ansi] {
        let mut b = base(format);
        b[40] ^= 1;
        assert_eq!(open(&b).err(), Some(Error::BadChecksum("the file's header")));
    }
}

#[test]
fn data_encrypted_another_way_is_refused() {
    let mut b = base(Format::Unicode);
    b[513] = 0x10;
    let full = crc32(&b[8..524]);
    b[524..528].copy_from_slice(&full.to_le_bytes());
    let partial = crc32(&b[8..479]);
    b[4..8].copy_from_slice(&partial.to_le_bytes());
    assert_eq!(open(&b).err(), Some(Error::UnsupportedEncryption(0x10)));
}

#[test]
fn a_damaged_block_or_page_is_found_by_its_checksum() {
    for format in [Format::Unicode, Format::Ansi] {
        let clean = base(format);
        let h = Header::parse(&clean).unwrap();
        // A byte of the first data block (the store's first heap block, written first).
        let mut b = clean.clone();
        b[0x4400 + 3] ^= 0xFF;
        assert!(matches!(open(&b).err(), Some(Error::BadChecksum(_))), "{format:?}");
        // A byte of the node tree's root page.
        let mut b = clean.clone();
        b[h.nbt.ib as usize + 5] ^= 0xFF;
        assert_eq!(open(&b).err(), Some(Error::BadChecksum("a B-tree page")));
        // The page's id.
        let mut b = clean.clone();
        let t = h.nbt.ib as usize + 512 - format.trailer_size();
        b[t + 2] ^= 1; // its signature
        assert_eq!(open(&b).err(), Some(Error::BadChecksum("a B-tree page's id")));
    }
}

#[test]
fn offsets_past_the_end_are_refused() {
    let clean = base(Format::Unicode);
    let h = Header::parse(&clean).unwrap();
    // Cut the file before its node tree.
    let cut = &clean[..h.nbt.ib as usize + 100];
    assert_eq!(open(cut).err(), Some(Error::OutOfBounds));
    // The header pointing far past the end.
    let mut b = clean.clone();
    b[224..232].copy_from_slice(&(1u64 << 40).to_le_bytes());
    let full = crc32(&b[8..524]);
    b[524..528].copy_from_slice(&full.to_le_bytes());
    let partial = crc32(&b[8..479]);
    b[4..8].copy_from_slice(&partial.to_le_bytes());
    assert_eq!(open(&b).err(), Some(Error::OutOfBounds));
    // Every cut of the file ends in an error or a partial read, never a panic.
    for len in (0..clean.len()).step_by(997) {
        exercise(&clean[..len], Limits::default());
    }
}

#[test]
fn a_node_tree_that_points_at_itself_is_refused() {
    for format in [Format::Unicode, Format::Ansi] {
        let mut b = base(format);
        let h = Header::parse(&b).unwrap();
        let root = h.nbt.ib as usize;
        let meta = if format == Format::Unicode { 488 } else { 496 };
        assert!(b[root + meta + 3] >= 1, "the tiny layout gives the node tree several levels");
        // Its first entry's child is the root page itself.
        let n = format.id_size();
        let me: Vec<u8> = match format {
            Format::Unicode => [h.nbt.bid.to_le_bytes(), h.nbt.ib.to_le_bytes()].concat(),
            Format::Ansi => [(h.nbt.bid as u32).to_le_bytes(), (h.nbt.ib as u32).to_le_bytes()].concat(),
        };
        b[root + n..root + 3 * n].copy_from_slice(&me);
        fix_page(&mut b, format, root);
        assert!(matches!(open(&b).err(), Some(Error::Corrupt(_) | Error::NotFound(_))), "{format:?}");
        exercise(&b, Limits::default());
    }
}

#[test]
fn a_page_deeper_than_the_format_allows_is_refused() {
    let mut b = base(Format::Unicode);
    let h = Header::parse(&b).unwrap();
    let root = h.nbt.ib as usize;
    b[root + 488 + 3] = 9;
    fix_page(&mut b, Format::Unicode, root);
    assert_eq!(open(&b).err(), Some(Error::Corrupt("a B-tree page's level")));
}

#[test]
fn a_folder_that_contains_its_parent_is_a_cycle() {
    let spec = sample(Layout::new(Format::Unicode, Encryption::None), None);
    let mut all = nodes(&spec);
    // Inbox's hierarchy table lists the top of the folders, its own parent, as a subfolder.
    let inbox = all.iter().find(|n| n.parent == writer::IPM_SUBTREE && n.nid & 0x1F == 2).unwrap().nid;
    let l = &spec.layout;
    let (blocks, subs) = writer::tc(l, &[0x3001_001F], &[(writer::IPM_SUBTREE, vec![(0x3001_001F, writer::unicode("Again"))])]);
    let table = all.iter_mut().find(|n| n.nid == (inbox & !0x1F) | 0x0D).unwrap();
    table.data = writer::Data::Blocks(blocks);
    table.subnodes = subs;
    let bytes = raw(l, &all);
    let mut pst = open(&bytes).unwrap();
    let tree = pst.walk_folders().unwrap();
    assert!(tree.left_out.iter().any(|x| x.id.0 == writer::IPM_SUBTREE && matches!(x.reason, Error::Cycle(_))));
    assert_eq!(tree.folders.iter().filter(|f| f.folder.id.0 == writer::IPM_SUBTREE).count(), 1);
}

#[test]
fn a_heap_tree_that_points_at_itself_is_refused() {
    let mut heap = HeapBuilder::new(0xBC, 3000);
    // The first item (HID 0x20) is an index record pointing at itself.
    let mut item = 0u16.to_le_bytes().to_vec();
    item.extend_from_slice(&0x20u32.to_le_bytes());
    assert_eq!(heap.alloc(item), 0x20);
    let mut header = vec![0xB5, 2, 6, 3];
    header.extend_from_slice(&0x20u32.to_le_bytes());
    heap.user_root = heap.alloc(header);
    let heap = Heap::new(heap.finish()).unwrap();
    assert!(matches!(crate::bth::read(&heap, heap.user_root, 1000, Error::Corrupt("")).err(), Some(Error::Cycle(_))));
}

#[test]
fn heap_ids_past_the_heap_are_refused() {
    let mut heap = HeapBuilder::new(0xBC, 3000);
    heap.user_root = heap.alloc(vec![1, 2, 3]);
    let heap = Heap::new(heap.finish()).unwrap();
    assert_eq!(heap.item(0x20).unwrap(), &[1, 2, 3]);
    assert!(heap.item(0x40).is_err());
    assert!(heap.item(0x1_0020).is_err());
    assert!(heap.item(0x21).is_err());
    assert!(Heap::new(vec![]).is_err());
    assert!(Heap::new(vec![vec![0, 0, 0xEB, 0xBC]]).is_err());
}

#[test]
fn block_signatures_are_checked() {
    let clean = base(Format::Ansi);
    // The message store's block, found through the file's own trees.
    let limits = Limits::default();
    let mut src = Source::new(Cursor::new(&clean[..]), &limits).unwrap();
    let header = Header::parse(&src.read_at(0, HEADER_READ).unwrap()).unwrap();
    let mut ndb = Ndb::new(src, header, limits);
    let store = ndb.node(0x21).unwrap();
    let entry = ndb.block_entry(store.data).unwrap();
    // Its trailer (ANSI: cb, wSig, bid, CRC): only the signature changed. The CRC covers the
    // data alone, so it still holds.
    let ts = Format::Ansi.trailer_size();
    let trailer = entry.ib as usize + (usize::from(entry.cb) + ts).div_ceil(64) * 64 - ts;
    let mut b = clean.clone();
    b[trailer + 2] ^= 0xFF;
    assert_eq!(open(&b).err(), Some(Error::BadChecksum("a block's signature")));
    assert_eq!(signature(0x4400, 4), 0x4404);
}

//! Tests of the HWP 5.0 reader on files written here record by record, from the record layouts
//! in Hancom's format document.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use crate::read::{self, BIN_DATA, CTRL_HEADER, LIST_HEADER, MAX_RECORDS, OLE_SIG, PARA_HEADER, PARA_TEXT, Records, SHAPE_COMPONENT, TABLE};
use crate::{HwpError, sniff};

/// One HWP 5.0 record: header (tag, level, size) and data. A size of 4095 bytes or more is
/// written as 4095 in the header and the size itself in the 4 bytes after it.
pub(crate) fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
    let size = data.len() as u32;
    let mut out = (tag | (level << 10) | (size.min(0xFFF) << 20)).to_le_bytes().to_vec();
    if size >= 0xFFF {
        out.extend_from_slice(&size.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

/// A compound file of these streams.
pub(crate) fn compound(streams: &[(&str, &[u8])]) -> Vec<u8> {
    craft_cfb::write_streams(craft_cfb::Version::V3, streams).unwrap_or_default()
}

/// A FileHeader stream with these flags (0x01 compressed, 0x02 password, 0x04 distribution-only).
fn file_header(flags: u32) -> Vec<u8> {
    let mut head = b"HWP Document File".to_vec();
    head.resize(256, 0);
    head[36..40].copy_from_slice(&flags.to_le_bytes());
    head
}

/// A minimal uncompressed HWP 5.0 file with one paragraph of `text`.
pub(crate) fn hwp(text: &str, flags: u32) -> Vec<u8> {
    let mut chars: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    chars.extend_from_slice(&13u16.to_le_bytes());
    let mut section = record(0x42, 0, &[0u8; 22]); // PARA_HEADER
    section.extend(record(0x43, 1, &chars)); // PARA_TEXT
    compound(&[("FileHeader", &file_header(flags)), ("DocInfo", &[]), ("BodyText/Section0", &section)])
}

/// An uncompressed HWP 5.0 file whose one section holds `section` (records).
pub(crate) fn hwp_with(section: &[u8]) -> Vec<u8> {
    compound(&[("FileHeader", &file_header(0)), ("DocInfo", &[]), ("BodyText/Section0", section)])
}

/// The bytes of the picture item `id` of an opened file.
fn binary<'a>(rebuilt: &'a read::Rebuilt, id: &str) -> Option<&'a [u8]> {
    rebuilt.binaries.iter().find(|b| b.id == id).map(|b| b.data.as_slice())
}

/// Records of 4095 bytes or more carry their size after the header, so the helper makes such
/// records as the reader takes them.
#[test]
fn the_record_helper_writes_extended_sizes() {
    for n in [0usize, 10, 0xFFE, 0xFFF, 5000] {
        let data = record(0x42, 0, &vec![7u8; n]);
        let r = Records::parse(&data);
        assert_eq!((r.recs.len(), r.data(0).len()), (1, n), "{n} bytes");
    }
}

/// A group's members are placed in final units (their rendering matrices multiplied in), so the
/// outermost group's drawing space is its current size, as a nested group's is. Its stored
/// original size (before the group was resized) squeezed the members: a group of lines 159 pt
/// tall was drawn in 220 pt of space, 0.72 times too short.
#[test]
fn hwp5_outer_group_draws_its_members_at_their_size() {
    let put = |d: &mut Vec<u8>, at: usize, v: &[u8]| d[at..at + v.len()].copy_from_slice(v);
    // the component: id(s), then offset, group level, original and current size, matrices
    let component = |ids: &[&[u8; 4]], org: (u32, u32), cur: (u32, u32), at: (f64, f64)| {
        let o = 4 * ids.len();
        let mut d = vec![0u8; o + 44 + 48 * 3];
        for (k, id) in ids.iter().enumerate() {
            put(&mut d, 4 * k, &id[..]);
        }
        put(&mut d, o + 12, &org.0.to_le_bytes());
        put(&mut d, o + 16, &org.1.to_le_bytes());
        put(&mut d, o + 20, &cur.0.to_le_bytes());
        put(&mut d, o + 24, &cur.1.to_le_bytes());
        put(&mut d, o + 42, &1u16.to_le_bytes()); // one scale and rotation pair
        // translation, scale, rotation: each a b c / d e f
        for (m, v) in [[1.0, 0.0, at.0, 0.0, 1.0, at.1], [1.0, 0.0, 0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]].iter().enumerate() {
            for (k, x) in v.iter().enumerate() {
                put(&mut d, o + 44 + 48 * m + 8 * k, &x.to_le_bytes());
            }
        }
        d
    };
    let mut gso = b" osg".to_vec();
    gso.resize(46, 0);
    put(&mut gso, 16, &15000u32.to_le_bytes());
    put(&mut gso, 20, &10000u32.to_le_bytes());
    let text: Vec<u8> = [11u16, 0, 0, 0, 0, 0, 0, 11, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut sec = record(PARA_HEADER, 0, &[0; 24]);
    sec.extend(record(PARA_TEXT, 1, &text));
    sec.extend(record(CTRL_HEADER, 1, &gso));
    // a group resized to half its original size; its member already placed at the final size
    sec.extend(record(SHAPE_COMPONENT, 2, &component(&[b"noc$", b"noc$"], (30000, 20000), (15000, 10000), (0.0, 0.0))));
    sec.extend(record(SHAPE_COMPONENT, 3, &component(&[b"cer$"], (15000, 10000), (15000, 10000), (0.0, 0.0))));
    let root = read::section(&sec);
    let group = root.iter("hp:container").next().unwrap_or_else(|| panic!("no group: {root:?}"));
    let size = |tag: &str| group.find(tag).map(|e| (e.get("width").unwrap_or("").to_string(), e.get("height").unwrap_or("").to_string()));
    assert_eq!(size("hp:curSz"), Some(("15000".into(), "10000".into())));
    assert_eq!(size("hp:orgSz"), size("hp:curSz"), "{group:?}");
    assert_eq!(group.children.iter().filter(|c| c.tag == "hp:rect").count(), 1, "{group:?}");
}

#[test]
fn hwp5_paragraphs_convert() {
    let bytes = hwp("안녕 HWP", 0);
    assert!(sniff(&bytes));
    let doc = crate::read(&bytes).unwrap();
    assert!(doc.body.iter().filter_map(|b| b.as_para()).any(|p| p.text == "안녕 HWP"), "{:?}", doc.body);
}

#[test]
fn hwp5_protected_and_damaged_files_are_errors() {
    assert_eq!(crate::read(&hwp("x", 0x02)).unwrap_err(), HwpError::Protected);
    assert_eq!(crate::read(&hwp("x", 0x04)).unwrap_err(), HwpError::Protected);
    let mut cut = hwp("x", 0);
    cut.truncate(700);
    assert!(crate::read(&cut).is_err());
    assert!(crate::read(&OLE_SIG.repeat(64)).is_err());
    // records that claim more than they hold, or nest absurdly deep, still read
    let mut deep = Vec::new();
    for level in 0..1000 {
        deep.extend(record(0x42, level, &[0u8; 22]));
    }
    deep.extend(record(0x43, 0, &[0xFF; 3]));
    deep.extend((0x42u32 | (0xFFF << 20)).to_le_bytes());
    deep.extend(u32::MAX.to_le_bytes());
    let _ = read::section(&deep);
}

/// Cell margins are signed (HWPUNIT16): a cell with its own margins and some sides -1 reads them
/// as 0. Before, -1 read as 65,535, and the cell's text width collapsed to its minimum.
#[test]
fn hwp5_cell_margins_are_signed() {
    let text: Vec<u8> = [11u16, 0, 0, 0, 0, 0, 0, 11, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut tbl = b" lbt".to_vec();
    tbl.resize(46, 0);
    let mut td = vec![0u8; 24];
    td[4..6].copy_from_slice(&1u16.to_le_bytes());
    td[6..8].copy_from_slice(&1u16.to_le_bytes());
    let margins = |sides: [u16; 4]| {
        let mut cell = vec![0u8; 47];
        cell[0..2].copy_from_slice(&1u16.to_le_bytes());
        cell[16..20].copy_from_slice(&5000u32.to_le_bytes());
        for (k, v) in sides.iter().enumerate() {
            cell[24 + 2 * k..26 + 2 * k].copy_from_slice(&v.to_le_bytes());
        }
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &text));
        sec.extend(record(CTRL_HEADER, 1, &tbl));
        sec.extend(record(TABLE, 2, &td));
        sec.extend(record(LIST_HEADER, 2, &cell));
        sec.extend(record(PARA_HEADER, 2, &[0; 24]));
        let root = read::section(&sec);
        let m = root.iter("hp:cellMargin").next().map(|m| ["left", "right", "top", "bottom"].map(|s| m.get(s).unwrap_or("").to_string()));
        m.unwrap_or_else(|| panic!("no cell margin: {root:?}"))
    };
    assert_eq!(margins([0xFFFF, 141, 0xFFFF, 141]), ["0", "141", "0", "141"]);
    // all four -1: the table's margins, kept as they are for a save
    assert_eq!(margins([0xFFFF; 4]), ["65535", "65535", "65535", "65535"]);
}

/// The DocInfo of `n` embedded PNG pictures (BIN_DATA records), numbered from 1.
fn bin_data_records(n: u16) -> Vec<u8> {
    let mut doc_info = Vec::new();
    for k in 1..=n {
        let mut d = Vec::new();
        d.extend(1u16.to_le_bytes()); // embedded
        d.extend(k.to_le_bytes());
        d.extend(3u16.to_le_bytes());
        d.extend("png".encode_utf16().flat_map(u16::to_le_bytes));
        doc_info.extend(record(BIN_DATA, 0, &d));
    }
    doc_info
}

/// An HWP 5.0 DocInfo of 60,000 BIN_DATA records (120,000 picture items, by stream name and by
/// number) opens, and its pictures are found by either id. Before, each item went through every
/// item before it.
#[test]
fn hwp5_many_picture_records_open() {
    let bytes = compound(&[
        ("FileHeader", &file_header(0)),
        ("DocInfo", &bin_data_records(60_000)),
        ("BodyText/Section0", &record(PARA_HEADER, 0, &[0; 24])),
        ("BinData/BIN0001.png", b"picture"),
    ]);
    let rebuilt = read::open(&bytes).unwrap();
    assert_eq!(binary(&rebuilt, "BIN0001"), Some(&b"picture"[..]));
    assert_eq!(binary(&rebuilt, "1"), Some(&b"picture"[..]));
    assert!(binary(&rebuilt, "2").is_none(), "listed, but no stream holds it");
}

/// A picture item marked always compressed (BIN_DATA 0x10) is inflated in a document that isn't
/// compressed. Before, only the document's flag counted, and its deflated bytes were taken as the
/// picture.
#[test]
fn a_picture_compressed_in_an_uncompressed_document_is_inflated() {
    let mut d = Vec::new();
    d.extend((1u16 | 0x10).to_le_bytes()); // embedded, always compressed
    d.extend(1u16.to_le_bytes());
    d.extend(3u16.to_le_bytes());
    d.extend("png".encode_utf16().flat_map(u16::to_le_bytes));
    let deflated = miniz_oxide::deflate::compress_to_vec(b"picture", 6);
    let bytes = compound(&[
        ("FileHeader", &file_header(0)),
        ("DocInfo", &record(BIN_DATA, 0, &d)),
        ("BodyText/Section0", &record(PARA_HEADER, 0, &[0; 24])),
        ("BinData/BIN0001.png", &deflated),
    ]);
    let rebuilt = read::open(&bytes).unwrap();
    assert_eq!(binary(&rebuilt, "BIN0001"), Some(&b"picture"[..]));
}

/// An HWP 5.0 file of 20,000 picture streams and their BIN_DATA records opens, each picture
/// read with its own bytes. Before, every stream read went through the whole stream list.
#[test]
fn hwp5_many_picture_streams_open() {
    let n: u16 = 20_000;
    let doc_info = bin_data_records(n);
    let head = file_header(0);
    let section = record(PARA_HEADER, 0, &[0; 24]);
    let names: Vec<String> = (1..=n).map(|k| format!("BinData/BIN{k:04X}.png")).collect();
    let data: Vec<[u8; 2]> = (1..=n).map(u16::to_le_bytes).collect();
    let mut streams: Vec<(&str, &[u8])> = vec![("FileHeader", &head), ("DocInfo", &doc_info), ("BodyText/Section0", &section)];
    streams.extend(names.iter().map(String::as_str).zip(data.iter().map(|d| &d[..])));
    let rebuilt = read::open(&compound(&streams)).unwrap();
    assert_eq!(binary(&rebuilt, "BIN4E20"), Some(&n.to_le_bytes()[..]));
    assert_eq!(binary(&rebuilt, "1"), Some(&1u16.to_le_bytes()[..]));
}

/// A stream of nothing but empty records (4 bytes each, about 50 in memory) is refused past
/// [`MAX_RECORDS`] instead of taking gigabytes. Before, 256 MB of zeros made 67 million records.
#[test]
fn hwp5_too_many_records_is_an_error() {
    let zeros = vec![0u8; 4 * (MAX_RECORDS + 1)];
    assert_eq!(crate::read(&hwp_with(&zeros)).unwrap_err(), HwpError::TooLarge("records"));
    assert!(crate::read(&hwp_with(&zeros[..4 * MAX_RECORDS])).is_ok());
}

/// A record header claiming more than the stream holds (the largest extended size) ends the
/// records there: no step past the end, which on a 32-bit target overflowed the offset.
#[test]
fn hwp5_record_sizes_past_the_end_stop_the_records() {
    let mut data = vec![0xFF; 11];
    let r = Records::parse(&data);
    assert_eq!(r.recs.len(), 1);
    data = record(0x42, 0, &[0; 22]);
    data.extend((0x43u32 | (0xFFF << 20)).to_le_bytes());
    data.extend(u32::MAX.to_le_bytes());
    data.extend(record(0x44, 0, &[0; 8])); // inside the oversized record: not a record of its own
    let r = Records::parse(&data);
    assert_eq!(r.recs.iter().map(|x| x.tag).collect::<Vec<_>>(), [0x42, 0x43]);
    assert_eq!(r.data(1).len(), 12, "the oversized record keeps what the stream holds");
}

/// A TABLE record gives its cell zone count (up to 65,535) and then 10 bytes a zone; only the
/// zones it holds are read. Before, a count with no zone bytes made 65,535 empty zones a table.
#[test]
fn hwp5_table_zones_are_only_those_the_record_holds() {
    let text: Vec<u8> = [11u16, 0, 0, 0, 0, 0, 0, 11, 13].iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut tbl = b" lbt".to_vec();
    tbl.resize(46, 0);
    // attributes, 1 row, 1 column, spacing, margins, one row size, border fill, 65,535 zones
    let mut td = vec![0u8; 24];
    td[4..6].copy_from_slice(&1u16.to_le_bytes());
    td[6..8].copy_from_slice(&1u16.to_le_bytes());
    td[22..24].copy_from_slice(&u16::MAX.to_le_bytes());
    let zones = |td: &[u8]| {
        let mut sec = record(PARA_HEADER, 0, &[0; 24]);
        sec.extend(record(PARA_TEXT, 1, &text));
        sec.extend(record(CTRL_HEADER, 1, &tbl));
        sec.extend(record(TABLE, 2, td));
        read::section(&sec).iter("hp:cellzone").count()
    };
    assert_eq!(zones(&td), 0);
    let mut one = td.clone();
    one.extend([0u8, 0, 0, 0, 1, 0, 1, 0, 2, 0]); // one zone, and a few bytes short of a second
    one.extend([0u8; 9]);
    assert_eq!(zones(&one), 1);
}

/// An HWP table cell whose four margins are -1 (0xFFFF; HWPUNIT16 is signed) has no margins of
/// its own: it takes the table's, as HWPX's hasMargin="0" does, instead of 655.35 pt each.
#[test]
fn hwp_cell_margins_of_minus_one_take_the_tables() {
    let u16s = |v: &[u16]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut text = u16s(&[0x0B, 0x6C20, 0x7462, 0, 0, 0, 0, 0x0B, 13]);
    let mut head = [0u8; 24];
    head[0..4].copy_from_slice(&(0x8000_0009u32).to_le_bytes());
    head[12..14].copy_from_slice(&1u16.to_le_bytes());
    let mut s = record(0x42, 0, &head);
    s.extend(record(0x43, 1, &text));
    s.extend(record(0x44, 1, &[0u8; 8]));
    let mut ctrl = 0x7462_6C20u32.to_le_bytes().to_vec();
    ctrl.extend([0u8; 12]);
    ctrl.extend(20000u32.to_le_bytes()); // width
    ctrl.extend(2000u32.to_le_bytes()); // height
    ctrl.extend([0u8; 22]);
    s.extend(record(0x47, 1, &ctrl)); // CTRL_HEADER
    let mut tbl = 0u32.to_le_bytes().to_vec();
    tbl.extend(u16s(&[1, 1, 0, 141, 141, 141, 141, 1, 1, 0]));
    s.extend(record(0x4D, 2, &tbl)); // TABLE
    let mut cell = u16s(&[1, 0]);
    cell.extend(0u32.to_le_bytes());
    cell.extend(u16s(&[0, 0, 1, 1]));
    cell.extend(20000u32.to_le_bytes());
    cell.extend(2000u32.to_le_bytes());
    cell.extend(u16s(&[0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 1]));
    cell.extend([0u8; 8]);
    s.extend(record(0x48, 2, &cell)); // LIST_HEADER
    text = u16s(&[0xCE78, 13]); // 칸
    let mut head = [0u8; 24];
    head[0..4].copy_from_slice(&(0x8000_0002u32).to_le_bytes());
    head[12..14].copy_from_slice(&1u16.to_le_bytes());
    s.extend(record(0x42, 2, &head));
    s.extend(record(0x43, 3, &text));
    s.extend(record(0x44, 3, &[0u8; 8]));
    let doc = crate::read(&hwp_with(&s)).unwrap();
    let t = doc.body.iter().find_map(|b| b.as_table()).expect("no table");
    let cell =
        t.rows.iter().flat_map(|r| &r.cells).find(|c| c.blocks.iter().filter_map(|b| b.as_para()).any(|p| p.text.contains('칸'))).expect("the cell");
    // the table's 141 HWPUNIT (about 1.4 pt) each, not 655.35 pt
    let margins: Vec<f32> =
        cell.props.margins.iter().flatten().copied().chain(cell.props.partial_margins.iter().flatten().flatten().copied()).collect();
    assert!(margins.iter().all(|m| *m < 10.0), "{margins:?}");
}

/// An object's placement among the text (common properties, bits 21-23) is read in the order
/// Hancom's files use: 0 square, 1 top and bottom, 2 behind text, 3 in front of text. The format
/// document's table lists HWPX's six values instead (1 tight ...), which real files don't follow
/// (2026-11-25: a picture of value 1 has its paragraph's stored lines below it, across the whole
/// width); read that way, the ordinary tables of a document float with text beside them.
#[test]
fn object_placement_follows_hancoms_files() {
    let wrap = |v: u32| {
        let mut d = vec![0u8; 64];
        d[4..8].copy_from_slice(&(v << 21).to_le_bytes());
        let mut obj = craft_hwpx::owpml::Element::new("hp:pic");
        read::common(&mut obj, &d);
        obj.get("textWrap").unwrap_or("").to_string()
    };
    let got: Vec<String> = (0..4).map(wrap).collect();
    assert_eq!(got, ["SQUARE", "TOP_AND_BOTTOM", "BEHIND_TEXT", "IN_FRONT_OF_TEXT"]);
}

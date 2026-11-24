//! craft-hwp integration tests: damaged files, and the documents of a local sample folder.

use craft_testkit::hostile::{HammerConfig, hammer};

/// One HWP 5.0 record: header (tag, level, size) and data, the size after the header from 4095
/// bytes on.
fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
    let size = data.len() as u32;
    let mut out = (tag | (level << 10) | (size.min(0xFFF) << 20)).to_le_bytes().to_vec();
    if size >= 0xFFF {
        out.extend_from_slice(&size.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A paragraph header: character count (the section's last paragraph marked) and one character
/// shape.
fn para_header(chars: u32, last: bool, level: u32) -> Vec<u8> {
    let mut head = [0u8; 24];
    head[0..4].copy_from_slice(&(chars | if last { 0x8000_0000 } else { 0 }).to_le_bytes());
    head[12..14].copy_from_slice(&1u16.to_le_bytes());
    record(0x42, level, &head)
}

/// DocInfo: two pictures (BIN_DATA, embedded PNG and JPEG).
fn doc_info() -> Vec<u8> {
    let mut out = Vec::new();
    for (k, ext) in [(1u16, "png"), (2, "jpg")] {
        let mut d = u16s(&[1, k, 3]);
        d.extend(u16s(&ext.encode_utf16().collect::<Vec<_>>()));
        out.extend(record(0x12, 0, &d));
    }
    out
}

/// A section of a paragraph of text, a paragraph holding a table of one cell with text, and a
/// paragraph holding a group of a rectangle.
fn section() -> Vec<u8> {
    let mut s = para_header(6, false, 0);
    s.extend(record(0x43, 1, &u16s(&[0xD55C, 0xAE00, 0x20, 0xBB38, 0xC11C, 13]))); // 한글 문서
    s.extend(record(0x44, 1, &[0u8; 8]));
    // the table: control (8 characters), then the cell
    s.extend(para_header(9, false, 0));
    s.extend(record(0x43, 1, &u16s(&[0x0B, 0x6C20, 0x7462, 0, 0, 0, 0, 0x0B, 13])));
    s.extend(record(0x44, 1, &[0u8; 8]));
    let mut ctrl = 0x7462_6C20u32.to_le_bytes().to_vec();
    ctrl.extend([0u8; 12]);
    ctrl.extend(20000u32.to_le_bytes());
    ctrl.extend(2000u32.to_le_bytes());
    ctrl.extend([0u8; 22]);
    s.extend(record(0x47, 1, &ctrl));
    let mut tbl = 0u32.to_le_bytes().to_vec();
    tbl.extend(u16s(&[1, 1, 0, 141, 141, 141, 141, 1, 1, 0]));
    s.extend(record(0x4D, 2, &tbl));
    let mut cell = u16s(&[1, 0]);
    cell.extend(0u32.to_le_bytes());
    cell.extend(u16s(&[0, 0, 1, 1]));
    cell.extend(20000u32.to_le_bytes());
    cell.extend(2000u32.to_le_bytes());
    cell.extend(u16s(&[141, 141, 141, 141, 1]));
    cell.extend([0u8; 8]);
    s.extend(record(0x48, 2, &cell));
    s.extend(para_header(2, true, 2));
    s.extend(record(0x43, 3, &u16s(&[0xCE78, 13]))); // 칸
    s.extend(record(0x44, 3, &[0u8; 8]));
    // the group: control, the group's component, one rectangle in it
    s.extend(para_header(9, true, 0));
    s.extend(record(0x43, 1, &u16s(&[11, 0, 0, 0, 0, 0, 0, 11, 13])));
    s.extend(record(0x44, 1, &[0u8; 8]));
    let mut gso = b" osg".to_vec();
    gso.resize(46, 0);
    gso[16..20].copy_from_slice(&15000u32.to_le_bytes());
    gso[20..24].copy_from_slice(&10000u32.to_le_bytes());
    s.extend(record(0x47, 1, &gso));
    let component = |ids: &[&[u8; 4]], size: (u32, u32)| {
        let o = 4 * ids.len();
        let mut d = vec![0u8; o + 44 + 48 * 3];
        for (k, id) in ids.iter().enumerate() {
            d[4 * k..4 * k + 4].copy_from_slice(&id[..]);
        }
        for at in [o + 12, o + 20] {
            d[at..at + 4].copy_from_slice(&size.0.to_le_bytes());
            d[at + 4..at + 8].copy_from_slice(&size.1.to_le_bytes());
        }
        d[o + 42..o + 44].copy_from_slice(&1u16.to_le_bytes());
        for m in 0..3 {
            for (k, x) in [1.0f64, 0.0, 0.0, 0.0, 1.0, 0.0].iter().enumerate() {
                d[o + 44 + 48 * m + 8 * k..o + 52 + 48 * m + 8 * k].copy_from_slice(&x.to_le_bytes());
            }
        }
        d
    };
    s.extend(record(0x4C, 2, &component(&[b"noc$", b"noc$"], (15000, 10000))));
    s.extend(record(0x4C, 3, &component(&[b"cer$"], (15000, 10000))));
    s
}

/// An HWP 5.0 file of these DocInfo and section streams; `compressed` deflates them (raw
/// DEFLATE, as Hancom stores them).
fn hwp(doc_info: &[u8], section: &[u8], compressed: bool) -> Vec<u8> {
    let mut head = b"HWP Document File".to_vec();
    head.resize(256, 0);
    head[36..40].copy_from_slice(&u32::from(compressed).to_le_bytes());
    let pack = |d: &[u8]| if compressed { miniz_oxide::deflate::compress_to_vec(d, 6) } else { d.to_vec() };
    let (info, sec) = (pack(doc_info), pack(section));
    craft_cfb::write_streams(
        craft_cfb::Version::V3,
        &[
            ("FileHeader", &head),
            ("DocInfo", &info),
            ("BodyText/Section0", &sec),
            ("BinData/BIN0001.png", b"\x89PNG"),
            ("BinData/BIN0002.jpg", b"\xFF\xD8\xFF"),
        ],
    )
    .unwrap_or_default()
}

/// The sample file reads, compressed and not, with something of each kind in it, so the sweep
/// below damages a file that exercises the reader rather than one it rejects at once.
#[test]
fn the_sweep_seed_reads_whole() {
    for compressed in [false, true] {
        let doc = craft_hwp::read(&hwp(&doc_info(), &section(), compressed)).unwrap();
        let texts: Vec<&str> = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect();
        assert!(texts.contains(&"한글 문서"), "{texts:?}");
        assert!(doc.body.iter().any(|b| b.as_table().is_some()), "no table");
        let objects: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
        assert!(objects.iter().any(|o| matches!(o, craft_wordmodel::InlineObject::Group { .. })), "no group");
        assert!(doc.passthrough.contains_key(craft_hwp::FILE_KEY), "the file is kept");
    }
}

/// Damaged files (every truncation and many mutations, of the compound file and of its record
/// streams, compressed and not) give a document or an error, never a panic or a hang. The rounds
/// go through the shared budget (`CRAFT_HOSTILE_DIVISOR`).
#[test]
fn damaged_files_never_panic() {
    let (info, sec) = (doc_info(), section());
    hammer(&HammerConfig::new("hwp file", 300), &hwp(&info, &sec, true), |b| {
        let _ = craft_hwp::read(b);
    });
    let i = info.clone();
    hammer(&HammerConfig::new("hwp section records", 400), &sec, move |b| {
        let _ = craft_hwp::read(&hwp(&i, b, false));
    });
    let s = sec.clone();
    hammer(&HammerConfig::new("hwp docinfo records", 200), &info, move |b| {
        let _ = craft_hwp::read(&hwp(b, &s, false));
    });
}

/// Every `.hwp` in `CRAFT_HWP_SAMPLES` reads, and its body has content. Without the variable the
/// test says so and passes, unless `CRAFT_HWP_SAMPLES_REQUIRED=1`. Failures name a sample by its
/// place in the sorted folder, not its name (samples may be private).
#[test]
fn local_samples_read() {
    let required = match std::env::var("CRAFT_HWP_SAMPLES_REQUIRED").unwrap_or_default().as_str() {
        "" => false,
        "1" => true,
        other => panic!("CRAFT_HWP_SAMPLES_REQUIRED is {other:?}: leave it unset or empty, or set it to 1"),
    };
    let Some(dir) = std::env::var_os("CRAFT_HWP_SAMPLES").filter(|d| !d.is_empty()) else {
        assert!(!required, "CRAFT_HWP_SAMPLES_REQUIRED=1 but CRAFT_HWP_SAMPLES is not set");
        println!("SKIPPED: CRAFT_HWP_SAMPLES is not set (a folder of .hwp files to read)");
        return;
    };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hwp"))).collect())
        .unwrap_or_default();
    files.sort();
    if required {
        assert!(!files.is_empty(), "no .hwp file in CRAFT_HWP_SAMPLES");
    }
    for (k, f) in files.iter().enumerate() {
        let bytes = std::fs::read(f).unwrap_or_default();
        assert!(craft_hwp::sniff(&bytes), "sample #{} is not recognised", k + 1);
        let doc = craft_hwp::read(&bytes).unwrap_or_else(|e| panic!("sample #{} did not read: {e}", k + 1));
        assert!(!doc.body.is_empty(), "sample #{} has an empty body", k + 1);
    }
    println!("{} samples read", files.len());
}

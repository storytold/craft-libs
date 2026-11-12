//! The package readers and writer against the cases the DOCX, XLSX and PPTX readers they came
//! from were tested with.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::sync::Arc;

use craft_opc::{
    Compression, Error, LazyPackage, Limits, Oversized, Package, PackageOut, RT_BASE, RelsOut, ZipOut, compression_for, declared_oversized,
    zip_entries,
};

const LIMITS: Limits = Limits::new(20_000, 256 << 20, 512 << 20);

fn limits(max_part: u64, max_total: u64) -> Limits {
    Limits::new(20_000, max_part, max_total)
}

fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let e: Vec<(String, Vec<u8>)> = entries.iter().map(|(n, b)| (n.to_string(), b.clone())).collect();
    zip_entries(&e).unwrap()
}

/// Flips the stored checksum of every entry, in its central and local header.
fn break_checksums(z: &mut [u8]) -> usize {
    let mut patched = 0;
    for sig in [[0x50u8, 0x4B, 0x01, 0x02], [0x50, 0x4B, 0x03, 0x04]] {
        let crc_at = if sig[2] == 1 { 16 } else { 14 };
        let found: Vec<usize> = z.windows(4).enumerate().filter(|(_, w)| *w == sig).map(|(i, _)| i).collect();
        for pos in found {
            z[pos + crc_at] ^= 0xFF;
            patched += 1;
        }
    }
    patched
}

#[test]
fn case_colliding_entries_are_one_part() {
    let z = zip_of(&[("word/media/Image.png", b"upper".to_vec()), ("word/media/image.png", b"lower".to_vec())]);
    let p = Package::open(&z, &LIMITS).unwrap();
    for spelling in ["word/media/Image.png", "word/media/image.png", "/WORD/MEDIA/IMAGE.PNG", "word\\media\\image.png"] {
        assert_eq!(p.canonical(spelling), Some("word/media/Image.png"), "{spelling}");
        assert_eq!(p.get(spelling), Some(&b"upper"[..]), "{spelling}");
    }
    assert_eq!(p.names().count(), 1);
}

#[test]
fn the_spelling_a_relationship_names_is_kept() {
    // The first spelling in byte order ("Image.png") was kept even when the document's
    // relationship named the other one, so the picture shown was not the one it pointed at.
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="image" Target="media/image.png"/><Relationship Id="rId2" Type="image" Target="media/SPACED%20name.png"/></Relationships>"#;
    let z = zip_of(&[
        ("word/_rels/document.xml.rels", rels.as_bytes().to_vec()),
        ("word/media/Image.png", b"upper".to_vec()),
        ("word/media/image.png", b"lower".to_vec()),
        ("word/media/Other.png", b"first".to_vec()),
        ("word/media/other.png", b"second".to_vec()),
        ("word/media/Spaced name.png", b"one".to_vec()),
        ("word/media/SPACED name.png", b"two".to_vec()),
    ]);
    let p = Package::open(&z, &LIMITS).unwrap();
    assert_eq!(p.canonical("word/media/IMAGE.png"), Some("word/media/image.png"));
    assert_eq!(p.get("word/media/Image.png"), Some(&b"lower"[..]));
    assert_eq!(p.get("word/media/OTHER.PNG"), Some(&b"first"[..]), "no relationship names it: first in byte order");
    assert_eq!(p.get("word/media/spaced name.png"), Some(&b"two"[..]), "named with an escape");
    assert_eq!(p.names().count(), 4);
    let r = p.rels("word/document.xml");
    assert_eq!(r.target("rId1"), Some("word/media/image.png"));
    assert_eq!(r.target("rId2"), Some("word/media/SPACED name.png"));
}

#[test]
fn the_entry_count_zip_finds_is_checked() {
    let z = zip_of(&[("a", vec![]), ("b", vec![]), ("c", vec![])]);
    let two = Limits::new(2, 100, 100);
    assert_eq!(Package::open(&z, &two).err(), Some(Error::TooManyEntries { declared: 3, limit: 2 }));
    assert_eq!(LazyPackage::open(&z, &two).err(), Some(Error::TooManyEntries { declared: 3, limit: 2 }));
    assert!(Package::open(&z, &two).is_err_and(|e| e.is_limit()));
    assert!(Package::open(&z, &Limits::new(3, 100, 100)).is_ok());
    assert!(matches!(Package::open(b"not a zip", &LIMITS), Err(Error::Zip(e)) if !Error::Zip(e.clone()).is_limit()));
}

#[test]
fn relationships_are_looked_up_by_id_the_first_one_winning() {
    let mut rels = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for i in 0..20_000 {
        rels.push_str(&format!(r#"<Relationship Id="rId{i}" Type="hyperlink" Target="https://example.com/{i}" TargetMode="External"/>"#));
    }
    rels.push_str(r#"<Relationship Id="rId7" Type="hyperlink" Target="https://example.com/dup" TargetMode="External"/></Relationships>"#);
    let z = zip_of(&[("word/_rels/document.xml.rels", rels.into_bytes())]);
    let r = Package::open(&z, &LIMITS).unwrap().rels("word/document.xml");
    assert_eq!(r.by_id("rId7").map(|r| r.target.as_str()), Some("https://example.com/7"), "the first rId7 wins");
    assert_eq!(r.by_id("rId19999").map(|r| r.target.as_str()), Some("https://example.com/19999"));
    assert!(r.by_id("rId20000").is_none());
    assert_eq!((0..20_000).filter(|i| r.by_id(&format!("rId{i}")).is_some()).count(), 20_000);
}

#[test]
fn decompression_bomb_is_rejected() {
    let z = zip_of(&[("word/document.xml", vec![b' '; 2_000_000])]);
    assert!(z.len() < 100_000);
    assert_eq!(
        Package::open(&z, &limits(1_000_000, 10_000_000)).err(),
        Some(Error::PartTooLarge { part: "word/document.xml".into(), limit: 1_000_000 })
    );
    assert_eq!(Package::open(&z, &limits(10_000_000, 1_000_000)).err(), Some(Error::PackageTooLarge { limit: 1_000_000 }));
    assert_eq!(Package::open(&z, &limits(10_000_000, 10_000_000)).map(|p| p.size()), Ok(2_000_000));
}

#[test]
fn entries_that_fail_their_check_still_count_against_the_total() {
    // Each entry inflates in full before its CRC is checked; a failed one dropped without being
    // charged would let any number of them inflate.
    let names: Vec<String> = (0..20).map(|i| format!("p{i}.bin")).collect();
    let entries: Vec<(&str, Vec<u8>)> = names.iter().map(|n| (n.as_str(), vec![0u8; 1_000_000])).collect();
    let mut z = zip_of(&entries);
    assert_eq!(break_checksums(&mut z), 40);
    assert_eq!(Package::open(&z, &limits(2_000_000, 4_000_000)).err(), Some(Error::PackageTooLarge { limit: 4_000_000 }));
    // With room for all of them the damaged entries are left out, not fatal.
    let p = Package::open(&z, &limits(2_000_000, 40_000_000)).unwrap();
    assert_eq!(p.names().count(), 0);
}

#[test]
fn lying_declared_size_is_rejected_or_bounded() {
    let mut z = zip_of(&[("a.xml", b"<a/>".to_vec())]);
    // Patch the uncompressed size in the central directory and local header to ~4 GB.
    let mut patched = 0;
    for sig in [[0x50u8, 0x4B, 0x01, 0x02], [0x50, 0x4B, 0x03, 0x04]] {
        if let Some(pos) = z.windows(4).position(|w| w == sig) {
            let off = if sig[2] == 1 { pos + 24 } else { pos + 22 };
            z[off..off + 4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
            patched += 1;
        }
    }
    assert_eq!(patched, 2);
    assert_eq!(declared_oversized(&z, &LIMITS), Some(("a.xml".to_string(), 0xFFFF_FFF0)));
    match Package::open(&z, &LIMITS) {
        Err(_) => {}
        Ok(p) => assert!(p.get("a.xml").is_none_or(|b| b.len() < 1024)),
    }
    let mut lazy = LazyPackage::open(&z, &LIMITS).unwrap();
    assert!(lazy.read("a.xml").is_err());
}

#[test]
fn parts_are_shared_not_copied() {
    let z = zip_of(&[("word/media/pic.png", b"\x89PNG picture bytes".to_vec())]);
    let pkg = Package::open(&z, &LIMITS).unwrap();
    let held = Arc::clone(pkg.get_shared("/WORD/media/PIC.png").unwrap());
    assert!(Arc::ptr_eq(&held, pkg.get_shared("word/media/pic.png").unwrap()));
    assert_eq!(pkg.get("word/media/pic.png"), Some(&b"\x89PNG picture bytes"[..]));
}

#[test]
fn skipped_parts_count_against_the_inflate_budget() {
    // Kept bytes cap at 100: b, c and d declare more than is left and are left out before
    // anything is inflated, so they cost no work and e, which fits, is still read.
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, len) in [("a", 80), ("b", 80), ("c", 80), ("d", 80), ("e", 5), ("big", 2000)] {
        zw.start_file(name, o).unwrap();
        zw.write_all(&vec![b'x'; len]).unwrap();
    }
    let z = zw.finish().unwrap().into_inner();
    let skip = |max_work| Limits { max_entries: 100, max_part: 1000, max_total: 100, max_work, oversized: Oversized::Skip };
    let p = Package::open(&z, &skip(130)).unwrap();
    assert!(p.get("a").is_some());
    assert!(p.get("b").is_none() && p.get("d").is_none());
    assert!(p.get("e").is_some(), "parts refused by their declared size cost nothing");
    assert_eq!(p.size(), 85);
    // With room to spare the same small part is kept, and a part declaring more than the part
    // limit is left out without being inflated.
    let p = Package::open(&z, &skip(10_000)).unwrap();
    assert!(p.get("e").is_some());
    assert!(p.get("big").is_none());
    assert_eq!(p.size(), 85);
    // The same package under `Refuse` is an error at the first part that does not fit.
    let refuse = Limits { oversized: Oversized::Refuse, ..skip(10_000) };
    assert_eq!(Package::open(&z, &refuse).err(), Some(Error::PackageTooLarge { limit: 100 }));
}

#[test]
fn backslash_zip_entry_names_are_found() {
    let z = zip_of(&[("PPT\\Slides\\Slide1.xml", b"<a/>".to_vec()), ("/docProps/core.xml", b"<b/>".to_vec())]);
    let p = Package::open(&z, &LIMITS).unwrap();
    assert!(p.get("ppt/slides/slide1.xml").is_some());
    assert_eq!(p.canonical("ppt/slides/slide1.xml"), Some("PPT/Slides/Slide1.xml"));
    assert_eq!(p.canonical("docprops/core.xml"), Some("docProps/core.xml"));
    let mut lazy = LazyPackage::open(&z, &LIMITS).unwrap();
    assert_eq!(lazy.names(), ["docprops/core.xml", "ppt/slides/slide1.xml"]);
    assert_eq!(lazy.canonical("ppt/slides/slide1.xml").as_deref(), Some("PPT/Slides/Slide1.xml"));
    assert_eq!(lazy.canonical("/DOCPROPS/core.xml").as_deref(), Some("docProps/core.xml"));
    assert_eq!(lazy.canonical("nothing"), None);
    assert_eq!(lazy.read("PPT/slides/slide1.xml").unwrap().as_deref(), Some(&b"<a/>"[..]));
}

#[test]
fn xml_parts_are_read_with_the_callers_options() {
    let z = zip_of(&[("a.xml", b"<w:a><w:b/></w:a>".to_vec()), ("bad.xml", b"<a><b></a>".to_vec())]);
    let p = Package::open(&z, &LIMITS).unwrap();
    let local = craft_xml::Options::local();
    assert_eq!(p.xml("a.xml", &local).unwrap().map(|e| e.name), Some("a".to_string()));
    assert_eq!(p.xml("a.xml", &craft_xml::Options::default()).unwrap().map(|e| e.name), Some("w:a".to_string()));
    assert_eq!(p.xml("missing.xml", &local), Ok(None));
    assert!(p.xml("bad.xml", &local).unwrap().is_some(), "lenient");
    assert!(matches!(p.xml("bad.xml", &local.strict()), Err(Error::Xml { part, .. }) if part == "bad.xml"));
    let tiny = local.with_limits(craft_xml::Limits { max_nodes: 1, ..craft_xml::Limits::DEFAULT });
    assert!(p.xml("a.xml", &tiny).is_err_and(|e| e.is_limit()));
    let mut lazy = LazyPackage::open(&z, &LIMITS).unwrap();
    assert_eq!(lazy.read_xml("a.xml", &local).unwrap().map(|e| e.name), Some("a".to_string()));
    assert!(lazy.read_xml("bad.xml", &local.strict()).is_err());
}

#[test]
fn from_parts_behaves_like_an_opened_package() {
    let mut parts = BTreeMap::new();
    parts.insert("/word/document.xml".to_string(), b"<d/>".to_vec());
    parts.insert(
        "word/_rels/document.xml.rels".to_string(),
        br#"<Relationships><Relationship Id="s" Type="x/styles" Target="Styles.xml"/></Relationships>"#.to_vec(),
    );
    parts.insert("word/styles.xml".to_string(), b"<s/>".to_vec());
    let p = Package::from_parts(parts);
    assert_eq!(p.get("word/document.xml"), Some(&b"<d/>"[..]));
    assert_eq!(p.rels("word/document.xml").first("styles").map(|r| r.target.as_str()), Some("word/styles.xml"), "the stored spelling");
    assert!(p.rels("word/styles.xml").list.is_empty());
    assert_eq!(p.content_types().of("word/document.xml"), None);
}

/// An over-budget read must spend the package's allowance once and for all: loops over many
/// broken parts otherwise re-decompress the remaining allowance again and again.
#[test]
fn a_failed_lazy_read_ends_the_package_budget() {
    let z = zip_of(&[("a.xml", b"0123456789".to_vec()), ("b.xml", b"hello".to_vec())]);
    let mut pkg = LazyPackage::open(&z, &limits(100, 12)).unwrap();
    assert!(pkg.has("A.xml") && !pkg.has("c.xml"));
    assert_eq!(pkg.read("c.xml"), Ok(None));
    assert!(pkg.read("a.xml").unwrap().is_some());
    assert_eq!(pkg.spent(), 10);
    assert_eq!(pkg.read("b.xml"), Err(Error::PackageTooLarge { limit: 12 }));
    assert_eq!(pkg.spent(), 12, "the failed read is charged");
    assert_eq!(pkg.read("b.xml"), Err(Error::PackageTooLarge { limit: 12 }));
    // The per-read maximum is its own error and costs nothing: the declared size is enough.
    let mut pkg = LazyPackage::open(&z, &limits(100, 100)).unwrap();
    assert_eq!(pkg.read_limited("a.xml", 4), Err(Error::PartTooLarge { part: "a.xml".into(), limit: 4 }));
    assert_eq!(pkg.spent(), 0);
    assert_eq!(pkg.read_limited("b.xml", 5).unwrap().as_deref(), Some(&b"hello"[..]));
}

/// A part the reader can do without, damaged inside an intact archive, costs its own
/// allowance: the parts after it are still read.
#[test]
fn a_damaged_optional_part_costs_only_its_allowance() {
    let mut w = ZipOut::new();
    w.add("a.xml", b"<a>damaged</a>", Compression::Store).unwrap();
    w.add("b.xml", b"hello", Compression::Deflate).unwrap();
    w.add(
        "xl/_rels/workbook.xml.rels",
        b"<Relationships><Relationship Id=\"r\" Type=\"t/worksheet\" Target=\"/B.XML\"/></Relationships>",
        Compression::Deflate,
    )
    .unwrap();
    let mut z = w.finish().unwrap();
    let at = z.windows(7).position(|x| x == b"damaged").unwrap();
    z[at] ^= 0x20;
    let max_total = 1 << 20;
    let mut pkg = LazyPackage::open(&z, &limits(1 << 16, max_total)).unwrap();
    assert!(matches!(pkg.read_optional_limited("a.xml", 1024), Err(Error::Zip(_))));
    assert_eq!(pkg.spent(), 1025, "the failed read is charged its allowance");
    assert_eq!(pkg.read("b.xml").unwrap().as_deref(), Some(&b"hello"[..]));
    let rels = pkg.rels("xl/workbook.xml").unwrap();
    assert_eq!(rels.first("worksheet").map(|r| (r.target.as_str(), r.kind())), Some(("B.XML", "worksheet")));
    assert!(pkg.rels("xl/styles.xml").unwrap().list.is_empty());
    // An ordinary read of it still ends the package's budget.
    assert!(pkg.read("a.xml").is_err());
    assert_eq!(pkg.spent(), max_total);
}

#[test]
fn relationships_being_written_reuse_identical_targets() {
    let n = 5_000;
    let mut out = RelsOut::default();
    let ids: Vec<String> = (0..n).map(|i| out.add("hyperlink", &format!("https://example.com/{i}"), true)).collect();
    assert_eq!(out.add("hyperlink", "https://example.com/7", true), ids[7], "an identical target reuses its id");
    assert_eq!(out.add("hyperlink", "https://example.com/7", false), format!("rId{}", n + 1), "internal is distinct");
    let fresh = out.add_new("image", "../media/a.png", false);
    assert_ne!(out.add_new("image", "../media/a.png", false), fresh);
    assert_eq!(out.add("image", "../media/a.png", false), fresh, "add reuses the first add_new made");
    assert_eq!(out.add("http://schemas.microsoft.com/office/2007/relationships/media", "a&b.mp4", false), format!("rId{}", n + 4));
    let back = craft_opc::Rels::parse(&out.to_xml(), "ppt/slides/slide1.xml");
    assert_eq!(back.list.len(), n + 4);
    assert_eq!(back.by_id(&fresh).map(|r| (r.ty.clone(), r.target.as_str())), Some((format!("{RT_BASE}image"), "ppt/media/a.png")));
    assert_eq!(back.first("media").map(|r| r.target_raw.as_str()), Some("a&b.mp4"), "escaped on the way out");
    assert!(back.by_id("rId8").is_some_and(|r| r.external));
}

#[test]
fn a_written_package_reads_back() {
    let mut out = PackageOut::default();
    out.xml_part("ppt/presentation.xml", "application/presentation+xml", b"<p/>".to_vec());
    out.binary("ppt/media/image1.png", "image/png", b"png".to_vec());
    out.binary("ppt/media/image2.png", "image/png", b"png2".to_vec());
    out.binary("ppt/media/odd.png", "image/x-odd", b"odd".to_vec());
    out.raw("docProps/thumbnail.xml", b"<t/>".to_vec());
    out.xml_part("PPT/Presentation.xml", "second", b"<second/>".to_vec());
    let mut rels = RelsOut::default();
    let id = rels.add("image", "media/image1.png", false);
    out.rels("ppt/presentation.xml", &RelsOut::default());
    out.rels("ppt/presentation.xml", &rels);
    rels.add("image", "media/image2.png", false);
    out.rels("ppt/presentation.xml", &rels);
    let mut root = RelsOut::default();
    root.add("officeDocument", "ppt/presentation.xml", false);
    out.rels("", &root);
    let bytes = out.finish().unwrap();

    let p = Package::open(&bytes, &LIMITS).unwrap();
    assert_eq!(p.names().next(), Some("[Content_Types].xml"), "names sort it first; it is also the first entry");
    assert!(bytes.windows(19).position(|w| w == b"[Content_Types].xml").is_some_and(|at| at < 64));
    assert_eq!(p.get("ppt/presentation.xml"), Some(&b"<p/>"[..]), "the first part under a name is the one written");
    let ct = p.content_types();
    assert_eq!(ct.of("ppt/presentation.xml"), Some("application/presentation+xml"));
    assert_eq!(ct.of("ppt/media/image2.png"), Some("image/png"));
    assert_eq!(ct.of("ppt/media/odd.png"), Some("image/x-odd"), "an Override where the extension is taken");
    assert_eq!(ct.of("ppt/_rels/presentation.xml.rels"), Some(craft_opc::CT_RELATIONSHIPS));
    assert_eq!(ct.of("docProps/thumbnail.xml"), Some("application/xml"));
    let main = p.rels("").first("officeDocument").map(|r| r.target.clone()).unwrap();
    let r = p.rels(&main);
    assert_eq!(r.target(&id), Some("ppt/media/image1.png"));
    assert_eq!(r.list.len(), 2, "the relationships part was replaced, not written twice");
    // Writing the same package again gives the same bytes.
    assert_eq!(zip_entries(&[("a.xml".into(), b"<a/>".to_vec())]), zip_entries(&[("a.xml".into(), b"<a/>".to_vec())]));
}

#[test]
fn packed_media_are_stored() {
    assert_eq!(compression_for("word/media/image1.PNG"), Compression::Store);
    assert_eq!(compression_for("ppt/media/movie.mp4"), Compression::Store);
    assert_eq!(compression_for("word/document.xml"), Compression::Deflate);
    assert_eq!(compression_for("word/media/image1.emf"), Compression::Deflate);
    assert_eq!(compression_for("noextension"), Compression::Deflate);
    let data = vec![b'a'; 100_000];
    let stored = zip_entries(&[("a.png".into(), data.clone())]).unwrap();
    let deflated = zip_entries(&[("a.xml".into(), data.clone())]).unwrap();
    assert!(stored.len() > 100_000 && deflated.len() < 10_000);
    for z in [stored, deflated] {
        let p = Package::open(&z, &LIMITS).unwrap();
        assert_eq!(p.size(), 100_000);
    }
}

/// Two entries of exactly one name: `zip` keeps one. This crate is lenient about part names
/// (spellings that differ only in case are one part), so the package still opens.
#[test]
fn an_entry_name_repeated_exactly_still_opens() {
    let mut z = zip_of(&[("word/a.xml", b"<a/>".to_vec()), ("word/b.xml", b"<b/>".to_vec())]);
    let mut i = 0;
    while let Some(at) = z[i..].windows(10).position(|w| w == b"word/b.xml") {
        z[i + at + 5] = b'a';
        i += at + 10;
    }
    assert_eq!(Package::open(&z, &LIMITS).unwrap().names().collect::<Vec<_>>(), ["word/a.xml"]);
    assert!(LazyPackage::open(&z, &LIMITS).unwrap().read("word/a.xml").unwrap().is_some());
}

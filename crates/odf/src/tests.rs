//! Packages, manifests, limits, password protection and flat documents.

use std::io::Write;

use crate::crypt::{CryptError, decrypt, encrypt, is_encrypted};
use crate::{Error, Limits, Manifest, Package, PackageWriter, flat, mime, zip_guard};

/// A ZIP of `files`, stored.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in files {
        z.start_file(*name, stored).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn secret() -> Vec<u8> {
    let mut w = PackageWriter::new(mime::TEXT);
    w.file("content.xml", "text/xml", b"<office:document-content>Top secret words.</office:document-content>");
    w.file("styles.xml", "text/xml", b"<office:document-styles/>");
    w.stored("Pictures/a.png", "image/png", b"\x89PNG not really");
    w.finish().unwrap()
}

#[test]
fn a_written_package_reads_back() {
    let bytes = secret();
    // The media type first and stored, at the offset `file` looks for it (section 3.3).
    assert_eq!(bytes.get(30..38), Some(b"mimetype".as_slice()));
    assert_eq!(bytes.get(38..38 + mime::TEXT.len()), Some(mime::TEXT.as_bytes()));
    let mut p = Package::open(&bytes, &Limits::DEFAULT).unwrap();
    assert_eq!(p.mimetype().as_deref(), Some(mime::TEXT));
    assert_eq!(p.names().collect::<Vec<_>>(), ["mimetype", "META-INF/manifest.xml", "content.xml", "styles.xml", "Pictures/a.png"]);
    assert!(p.contains("styles.xml") && !p.contains("nothing.xml"));
    assert_eq!(p.read("nothing.xml"), Ok(None));
    assert_eq!(p.read("Pictures/a.png").unwrap().as_deref(), Some(b"\x89PNG not really".as_slice()));
    let m = p.manifest();
    assert_eq!(m.version.as_deref(), Some("1.3"));
    assert_eq!(m.media_type("/"), Some(mime::TEXT));
    assert_eq!(m.media_type("Pictures/a.png"), Some("image/png"));
    assert!(!m.is_encrypted() && !p.is_encrypted());
    assert_eq!(Manifest::parse(m.to_xml().as_bytes()), m);
}

#[test]
fn a_manifest_is_read_by_namespace_and_survives_damage() {
    // Another prefix, a default namespace with plain attributes, and a prefix nothing declares.
    for xml in [
        r#"<m:manifest xmlns:m="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><m:file-entry m:full-path="/" m:media-type="a/b"/><m:file-entry m:full-path="x &amp; y.xml" m:media-type="text/xml" m:size="12"/></m:manifest>"#,
        r#"<manifest xmlns="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><file-entry full-path="/" media-type="a/b"/><file-entry full-path="x &amp; y.xml" media-type="text/xml" size="12"/></manifest>"#,
        r#"<manifest:manifest><manifest:file-entry manifest:full-path="/" manifest:media-type="a/b"/><manifest:file-entry manifest:full-path="x &amp; y.xml" manifest:media-type="text/xml" manifest:size="12"/></manifest:manifest>"#,
    ] {
        let m = Manifest::parse(xml.as_bytes());
        assert_eq!(m.media_type("/"), Some("a/b"), "{xml}");
        assert_eq!(m.entry("x & y.xml").and_then(|e| e.size), Some(12), "{xml}");
    }
    // Cut short: the entries before the cut.
    let cut = r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="a/b"/><manifest:file-entry manifest:full-pa"#;
    assert_eq!(Manifest::parse(cut.as_bytes()).entries.len(), 1);
    // Not a manifest, not XML, a document type: nothing, no panic.
    for junk in [&b"<html><file-entry full-path='/'/></html>"[..], b"\xff\xfe\x00", b"", b"<!DOCTYPE x [<!ENTITY a 'b'>]><manifest/>"] {
        assert!(Manifest::parse(junk).entries.is_empty());
    }
    // More entries than a package may hold are dropped.
    let many = format!("<manifest>{}</manifest>", r#"<file-entry full-path="a"/>"#.repeat(crate::manifest::MAX_ENTRIES + 5));
    assert_eq!(Manifest::parse(many.as_bytes()).entries.len(), crate::manifest::MAX_ENTRIES);
}

#[test]
fn a_strict_manifest_refuses_what_a_lenient_one_repairs() {
    const NS: &str = r#"xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0""#;
    let good = format!(
        r#"<?xml version="1.0"?><manifest:manifest {NS} manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="a/b"/></manifest:manifest>"#
    );
    let m = Manifest::parse_strict(good.as_bytes()).unwrap();
    assert_eq!((m.version.as_deref(), m.media_type("/")), (Some("1.3"), Some("a/b")));
    // Another prefix for the same namespace is the same manifest.
    let other = r#"<m:manifest xmlns:m="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" m:version="1.3"><m:file-entry m:full-path="/" m:media-type="a/b"/></m:manifest>"#;
    assert_eq!(Manifest::parse_strict(other.as_bytes()), Ok(m.clone()));
    for bad in [
        format!(r#"<manifest:manifest {NS}><manifest:file-entry manifest:full-path="/">"#),
        format!(r#"<!DOCTYPE manifest:manifest><manifest:manifest {NS}/>"#),
        format!(r#"<!DOCTYPE m [<!ENTITY a "b">]><manifest:manifest {NS}/>"#),
        format!(r#"<manifest:manifest {NS}/><manifest:manifest {NS}/>"#),
        format!(r#"<manifest:manifest {NS}/>trailing"#),
        format!(r#"<manifest:manifest {NS}><manifest:file-entry manifest:full-path="&bogus;"/></manifest:manifest>"#),
        format!(r#"<manifest:manifest {NS}><manifest:file-entry manifest:media-type="a/b"/></manifest:manifest>"#),
        format!(r#"<manifest:manifest {NS}></manifest:file-entry></manifest:manifest>"#),
        r#"<manifest:manifest><manifest:file-entry manifest:full-path="/"/></manifest:manifest>"#.to_string(),
        r#"<html/>"#.to_string(),
        String::new(),
    ] {
        assert!(matches!(Manifest::parse_strict(bad.as_bytes()), Err(Error::Manifest(_))), "{bad}");
        let _ = Manifest::parse(bad.as_bytes());
    }
    // The lenient read takes what it can of the first.
    assert_eq!(Manifest::parse(format!(r#"<manifest:manifest {NS}><manifest:file-entry manifest:full-path="/">"#).as_bytes()).entries.len(), 1);
    // Entries: up to the cap, not past it.
    let many = |n: usize| format!(r#"<manifest:manifest {NS}>{}</manifest:manifest>"#, r#"<manifest:file-entry manifest:full-path="a"/>"#.repeat(n));
    assert_eq!(Manifest::parse_strict(many(crate::manifest::MAX_ENTRIES).as_bytes()).map(|m| m.entries.len()), Ok(crate::manifest::MAX_ENTRIES));
    assert!(Manifest::parse_strict(many(crate::manifest::MAX_ENTRIES + 1).as_bytes()).is_err());
    // Through a package: missing and present.
    let mut p_bytes = zip_of(&[("mimetype", mime::SPREADSHEET.as_bytes())]);
    assert!(matches!(Package::open(&p_bytes, &Limits::DEFAULT).unwrap().manifest_strict(), Err(Error::Manifest(_))));
    p_bytes = zip_of(&[("mimetype", mime::SPREADSHEET.as_bytes()), ("META-INF/manifest.xml", good.as_bytes())]);
    assert_eq!(Package::open(&p_bytes, &Limits::DEFAULT).unwrap().manifest_strict(), Ok(m));
    // A written package passes.
    assert!(Package::open(&secret(), &Limits::DEFAULT).unwrap().manifest_strict().is_ok_and(|m| m.entries.len() == 4));
}

#[test]
fn a_package_is_held_to_its_limits() {
    let bytes = zip_of(&[("mimetype", mime::SPREADSHEET.as_bytes()), ("a.xml", &[b'a'; 100]), ("b.xml", &[b'b'; 100])]);
    // Entries.
    assert!(matches!(Package::open(&bytes, &Limits { max_entries: 2, ..Limits::DEFAULT }), Err(Error::TooManyEntries(_))));
    let bomb = zip_guard::zip64_entry_count_sample(Limits::DEFAULT.max_entries + 1);
    assert!(matches!(Package::open(&bomb, &Limits::DEFAULT), Err(e) if e.is_limit()));
    // One file.
    let mut p = Package::open(&bytes, &Limits { max_entry: 99, ..Limits::DEFAULT }).unwrap();
    assert_eq!(p.read("a.xml"), Err(Error::EntryTooLarge("a.xml".into(), 99)));
    let mut p = Package::open(&bytes, &Limits { max_entry: 100, ..Limits::DEFAULT }).unwrap();
    assert_eq!(p.read("a.xml").map(|d| d.map(|d| d.len())), Ok(Some(100)));
    // All files together; a file read twice counts twice.
    let mut p = Package::open(&bytes, &Limits { max_total: 250, ..Limits::DEFAULT }).unwrap();
    assert!(p.read("a.xml").is_ok() && p.read("b.xml").is_ok());
    assert_eq!(p.bytes_read(), 200);
    assert_eq!(p.read("a.xml"), Err(Error::TotalTooLarge(250)));
    // Not a ZIP.
    assert!(matches!(Package::open(b"not a zip", &Limits::DEFAULT), Err(Error::Zip(_))));
}

/// A deflated entry whose headers declare `declared` bytes but which inflates to `real.len()`.
fn lying_zip(real: &[u8], declared: u32) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("a.xml", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
    z.write_all(real).unwrap();
    let mut bytes = z.finish().unwrap().into_inner();
    // The uncompressed size, in the central directory header (offset 24) and the local one (22).
    let want = (real.len() as u32).to_le_bytes();
    let central = bytes.windows(4).rposition(|w| w == b"PK\x01\x02").unwrap();
    assert_eq!(&bytes[central + 24..central + 28], &want);
    bytes[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
    assert_eq!(&bytes[22..26], &want);
    bytes[22..26].copy_from_slice(&declared.to_le_bytes());
    bytes
}

#[test]
fn a_file_that_inflates_past_what_it_declares_is_not_read_past_the_limit() {
    let bytes = lying_zip(&vec![b'a'; 100_000], 10);
    let mut p = Package::open(&bytes, &Limits { max_entry: 1000, ..Limits::DEFAULT }).unwrap();
    assert_eq!(p.declared_size("a.xml"), Some(10));
    // Refused or unreadable, and never more than the limit (and the byte that shows it) inflated.
    assert!(!matches!(p.read("a.xml"), Ok(Some(_))));
    assert!(p.bytes_read() <= 1001, "{}", p.bytes_read());
}

#[test]
fn a_mimetype_is_short_or_nothing() {
    let long = "x".repeat(300);
    let mut p_bytes = zip_of(&[("mimetype", long.as_bytes())]);
    assert_eq!(Package::open(&p_bytes, &Limits::DEFAULT).unwrap().mimetype(), None);
    p_bytes = zip_of(&[("mimetype", b" application/vnd.oasis.opendocument.spreadsheet\n")]);
    assert_eq!(Package::open(&p_bytes, &Limits::DEFAULT).unwrap().mimetype().as_deref(), Some(mime::SPREADSHEET));
    p_bytes = zip_of(&[("content.xml", b"<a/>")]);
    assert_eq!(Package::open(&p_bytes, &Limits::DEFAULT).unwrap().mimetype(), None);
}

#[test]
fn an_encrypted_package_opens_with_its_password_only() {
    let plain = secret();
    assert!(!is_encrypted(&plain));
    let enc = encrypt(&plain, "pässword").unwrap();
    assert!(is_encrypted(&enc));
    // Nothing readable is left: the content is encrypted, the manifest says how.
    let mut p = Package::open(&enc, &Limits::DEFAULT).unwrap();
    assert_eq!(p.names().next(), Some("mimetype"));
    assert!(!enc.windows(10).any(|w| w == b"Top secret"));
    let manifest = String::from_utf8(p.read("META-INF/manifest.xml").unwrap().unwrap()).unwrap();
    for want in [
        r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml" manifest:size=""#,
        "http://www.w3.org/2001/04/xmlenc#aes256-cbc",
        r#"manifest:start-key-generation-name="http://www.w3.org/2000/09/xmldsig#sha256" manifest:key-size="32""#,
        r#"manifest:key-derivation-name="PBKDF2" manifest:iteration-count="100000""#,
        "#sha256-1k",
    ] {
        assert!(manifest.contains(want), "{want} in {manifest}");
    }
    let m = p.manifest();
    let c = m.entry("content.xml").and_then(|e| e.encryption.as_ref()).unwrap();
    assert_eq!((c.iterations, c.key_size, c.iv.len(), c.salt.len(), c.checksum.len()), (100_000, Some(32), 16, 16, 32));
    assert!(m.entry("/").is_some_and(|e| e.encryption.is_none()));
    assert_eq!(decrypt(&enc, "wrong"), Err(CryptError::WrongPassword));
    let opened = decrypt(&enc, "pässword").unwrap();
    assert!(!is_encrypted(&opened));
    let (mut a, mut b) = (Package::open(&plain, &Limits::DEFAULT).unwrap(), Package::open(&opened, &Limits::DEFAULT).unwrap());
    for name in ["mimetype", "content.xml", "styles.xml", "Pictures/a.png"] {
        assert_eq!(a.read(name), b.read(name), "{name}");
    }
    // The manifest is the plain one again, each file keeping the size it was given.
    let mut reopened = b.manifest();
    assert_eq!(reopened.entry("content.xml").and_then(|e| e.size), Some(68));
    reopened.entries.iter_mut().for_each(|e| e.size = None);
    assert_eq!(a.manifest(), reopened);
}

#[test]
fn unsupported_and_hostile_encryption_is_refused() {
    let manifest = |alg: &str, extra: &str| {
        format!(
            r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml" manifest:size="99999999999"><manifest:encryption-data manifest:checksum-type="SHA1/1K" manifest:checksum="AAAA"><manifest:algorithm manifest:algorithm-name="{alg}" manifest:initialisation-vector="AAAAAAAAAAA="/><manifest:key-derivation manifest:key-derivation-name="PBKDF2" {extra} manifest:salt="AAAA"/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"#
        )
    };
    let pkg = |m: String, content: &[u8]| {
        zip_of(&[("mimetype", b"application/vnd.oasis.opendocument.text"), ("META-INF/manifest.xml", m.as_bytes()), ("content.xml", content)])
    };
    // Blowfish: said, not guessed at.
    let blowfish = pkg(manifest("Blowfish CFB", r#"manifest:iteration-count="1024""#), &[0u8; 64]);
    assert!(is_encrypted(&blowfish));
    assert!(matches!(decrypt(&blowfish, "x"), Err(CryptError::Unsupported(w)) if w.contains("Blowfish")));
    // An iteration count that would hang, a vector of the wrong size, ragged data: refused.
    for (extra, data) in [
        (r#"manifest:iteration-count="4000000000""#, vec![0u8; 32]),
        (r#"manifest:iteration-count="1""#, vec![0u8; 33]),
        (r#"manifest:iteration-count="1""#, vec![]),
    ] {
        assert!(decrypt(&pkg(manifest("http://www.w3.org/2001/04/xmlenc#aes256-cbc", extra), &data), "x").is_err(), "{extra}");
    }
    // Junk that isn't a package.
    assert!(!is_encrypted(b"not a zip"));
    assert!(decrypt(b"not a zip", "x").is_err());
    // A plain package has nothing to decrypt.
    assert!(matches!(decrypt(&secret(), "x"), Err(CryptError::Unsupported(_))));
}

#[test]
fn a_package_declaring_too_many_entries_is_refused_before_it_is_decrypted() {
    let bomb = zip_guard::zip64_entry_count_sample(Limits::DEFAULT.max_entries + 1);
    assert!(!is_encrypted(&bomb));
    assert!(matches!(decrypt(&bomb, "x"), Err(CryptError::Unsupported(e)) if e.contains("declares")));
    assert!(matches!(encrypt(&bomb, "x"), Err(CryptError::Unsupported(e)) if e.contains("declares")));
}

#[test]
fn files_past_the_most_decrypted_are_not_reached() {
    // The one encrypted file sits after 10,000 others: it is never derived a key for.
    let manifest = r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="z.xml" manifest:media-type="text/xml"><manifest:encryption-data><manifest:algorithm manifest:algorithm-name="http://www.w3.org/2001/04/xmlenc#aes256-cbc" manifest:initialisation-vector="AAAAAAAAAAAAAAAAAAAAAA=="/><manifest:start-key-generation manifest:start-key-generation-name="SHA1"/><manifest:key-derivation manifest:key-derivation-name="PBKDF2" manifest:iteration-count="1" manifest:salt="AAAA"/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"#;
    let names: Vec<String> = (0..10_000).map(|i| format!("f{i}")).collect();
    let mut files: Vec<(&str, &[u8])> = vec![("META-INF/manifest.xml", manifest.as_bytes())];
    files.extend(names.iter().map(|n| (n.as_str(), &b""[..])));
    files.push(("z.xml", &[0u8; 32]));
    let last = zip_of(&files);
    assert_eq!(decrypt(&last, "x"), Err(CryptError::Unsupported("no encrypted file could be read".into())));
    // Within reach, the same file is tried, and refused for what it is.
    let first = zip_of(&[("META-INF/manifest.xml", manifest.as_bytes()), ("z.xml", &[0u8; 32])]);
    assert_eq!(decrypt(&first, "x"), Err(CryptError::Unsupported("the key isn't 256 bits".into())));
}

/// A 1×1 PNG, base64.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn flat_doc(mimetype: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:math="http://www.w3.org/1998/Math/MathML" office:version="1.3" office:mimetype="{mimetype}">
 <office:meta><dc:title>Flat title</dc:title></office:meta>
 <office:styles><style:style style:name="Quiet" style:family="paragraph"/></office:styles>
 <office:automatic-styles><style:style style:name="T1" style:family="text"/></office:automatic-styles>
 <office:master-styles><style:master-page style:name="Standard"/></office:master-styles>
 <office:body><office:text>
  <text:p text:style-name="Quiet">Some words.</text:p>
  <text:p><draw:frame><draw:image><office:binary-data>
{PNG_B64}
  </office:binary-data></draw:image></draw:frame></text:p>
  <text:p><draw:frame><draw:object><math:math><math:mi>x</math:mi></math:math></draw:object></draw:frame></text:p>
  <text:p><draw:frame><draw:object><office:document office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.chart"><office:body><office:chart/></office:body></office:document></draw:object></draw:frame></text:p>
 </office:text></office:body>
</office:document>"#
    )
}

#[test]
fn a_flat_document_becomes_a_package_and_back() {
    let png = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(PNG_B64).unwrap()
    };
    let doc = flat::split(flat_doc(mime::TEXT).as_bytes(), &Limits::DEFAULT).unwrap();
    assert_eq!(doc.mimetype.as_deref(), Some(mime::TEXT));
    // Each part has what a package's file has; the root's media type stays with the package.
    assert!(doc.content.contains("<office:document-content") && doc.content.contains("T1") && !doc.content.contains("Quiet\" style:family"));
    assert!(!doc.content.contains("office:mimetype=\"application/vnd.oasis.opendocument.text\""));
    assert!(doc.styles.contains("<office:document-styles") && doc.styles.contains("Quiet") && doc.styles.contains("Standard"));
    assert!(doc.meta.contains("Flat title") && doc.settings.contains("<office:document-settings"));
    // The picture, the formula and the chart are files the content points at.
    assert!(doc.content.contains(r#"<draw:image xlink:href="Pictures/flat1.png">"#), "{}", doc.content);
    assert!(doc.content.contains(r#"<draw:object xlink:href="./Object flat2"/>"#));
    let names: Vec<&str> = doc.files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "Pictures/flat1.png",
            "Object flat2/content.xml",
            "Object flat3/content.xml",
            "Object flat3/styles.xml",
            "Object flat3/meta.xml",
            "Object flat3/settings.xml"
        ]
    );
    assert_eq!(doc.files.first().map(|(_, d)| d.as_slice()), Some(png.as_slice()));
    assert_eq!(doc.folders, [("Object flat3/".to_string(), mime::CHART.to_string())]);

    let package = flat::to_package(flat_doc(mime::TEXT).as_bytes(), mime::GRAPHICS, &Limits::DEFAULT).unwrap();
    let mut p = Package::open(&package, &Limits::DEFAULT).unwrap();
    assert_eq!(p.mimetype().as_deref(), Some(mime::TEXT));
    assert_eq!(p.read("Pictures/flat1.png").unwrap().as_deref(), Some(png.as_slice()));
    assert_eq!(p.manifest().media_type("Object flat3/"), Some(mime::CHART));

    // And back: one document of the same media type, the picture, formula and chart inline.
    let again = String::from_utf8(flat::from_package(&package, &Limits::DEFAULT).unwrap()).unwrap();
    assert!(again.contains(&format!("office:mimetype=\"{}\">", mime::TEXT)), "{again}");
    assert!(again.contains(&format!("<draw:image><office:binary-data>{PNG_B64}</office:binary-data></draw:image>")), "{again}");
    assert!(again.contains("<draw:object><math:math><math:mi>x</math:mi></math:math></draw:object>"), "{again}");
    assert!(
        again.contains(
            r#"office:mimetype="application/vnd.oasis.opendocument.chart"><office:body><office:chart/></office:body></office:document></draw:object>"#
        ),
        "{again}"
    );
    assert!(!again.contains("xlink:href"), "{again}");
    // Taking that apart gives the same files.
    let twice = flat::split(again.as_bytes(), &Limits::DEFAULT).unwrap();
    assert_eq!((twice.files.len(), &twice.folders), (doc.files.len(), &doc.folders));
}

#[test]
fn a_flat_document_of_another_kind_keeps_its_media_type() {
    let package = flat::to_package(flat_doc(mime::SPREADSHEET).as_bytes(), mime::TEXT, &Limits::DEFAULT).unwrap();
    assert_eq!(Package::open(&package, &Limits::DEFAULT).unwrap().mimetype().as_deref(), Some(mime::SPREADSHEET));
    // One that declares none takes the caller's.
    let bare = "<office:document><office:body/></office:document>";
    let package = flat::to_package(bare.as_bytes(), mime::PRESENTATION, &Limits::DEFAULT).unwrap();
    assert_eq!(Package::open(&package, &Limits::DEFAULT).unwrap().mimetype().as_deref(), Some(mime::PRESENTATION));
}

#[test]
fn hostile_flat_documents_do_not_panic() {
    assert_eq!(flat::split(b"\xff\xfe", &Limits::DEFAULT), Err(Error::Flat("not UTF-8 text")));
    assert_eq!(flat::split(b"<html/>", &Limits::DEFAULT), Err(Error::Flat("no office:document")));
    assert!(flat::split(b"<office:document-content/>", &Limits::DEFAULT).is_err());
    for xml in [
        "<office:document",
        "<office:document><office:body><draw:image><office:binary-data>!!!not base64",
        "<office:document><office:body><draw:image><office:binary-data>!!!</office:binary-data></draw:image><draw:object><math:math>",
        "<office:document><office:body><office:binary-data>AAAA</office:binary-data></office:body></office:document>",
    ] {
        let _ = flat::split(xml.as_bytes(), &Limits::DEFAULT);
        let _ = flat::to_package(xml.as_bytes(), mime::TEXT, &Limits::DEFAULT);
    }
    // A picture over the per-file limit is left out; the document still opens.
    let doc = flat::split(flat_doc(mime::TEXT).as_bytes(), &Limits { max_entry: 10, ..Limits::DEFAULT }).unwrap();
    assert!(!doc.files.iter().any(|(n, _)| n.starts_with("Pictures/")));
    // More inline objects than are taken out: the first 10,000 become files, the rest go.
    let many = format!("<office:document><office:body>{}</office:body></office:document>", "<draw:object><math:math/></draw:object>".repeat(10_050));
    assert_eq!(flat::split(many.as_bytes(), &Limits::DEFAULT).unwrap().files.len(), 10_000);
    // A package with no media type has no flat form.
    assert_eq!(flat::from_package(&zip_of(&[("content.xml", b"<a/>")]), &Limits::DEFAULT), Err(Error::Flat("the package has no media type")));
}

/// Two files of one name: `zip` keeps one, and which one the writer meant is not known.
#[test]
fn two_files_of_one_name_are_refused() {
    let bytes = zip_of(&[("mimetype", mime::SPREADSHEET.as_bytes()), ("dupA.xml", b"<a/>"), ("dupB.xml", b"<b/>")]);
    assert!(Package::open(&bytes, &Limits::DEFAULT).is_ok());
    let mut renamed = bytes.clone();
    let mut i = 0;
    while let Some(at) = renamed[i..].windows(4).position(|w| w == b"dupB") {
        renamed[i + at + 3] = b'A';
        i += at + 4;
    }
    assert_eq!(Package::open(&renamed, &Limits::DEFAULT).err(), Some(Error::Zip("two files of the package share a name".into())));
}

/// Encrypting and decrypting read each file within the per-file limit: a file declaring more is
/// refused before it is inflated (encrypting read every file whole; decrypting cut it short).
#[test]
fn encrypting_and_decrypting_refuse_a_file_past_the_limit() {
    let over = u32::try_from(Limits::DEFAULT.max_entry + 1).unwrap();
    let bytes = lying_zip(b"<a/>", over);
    assert!(crate::crypt::encrypt(&bytes, "pw").is_err());
    assert!(crate::crypt::decrypt(&bytes, "pw").is_err());
    // The same file declaring its real size encrypts.
    assert!(crate::crypt::encrypt(&lying_zip(b"<a/>", 4), "pw").is_ok());
}

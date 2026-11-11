//! Integration tests of craft-vectorcraft-format: envelope.

use std::io::Write;

use craft_vectorcraft_format::{
    BLOB_MAGIC, Contents, EXTENSION, Entries, Error, File, Head, INLINE_MAX, Image, LEGACY_EXTENSION, MAX_ENTRY, ReadError, Tail, VERSION,
    is_compressed, json_part, sniff, split_tail,
};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Layer {
    name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Doc {
    layers: Vec<Layer>,
}

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

fn doc() -> Doc {
    Doc { layers: vec![Layer { name: "Layer 1".into() }] }
}

fn pdf() -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    pdf.resize(INLINE_MAX, b' ');
    pdf
}

/// A PNG entry at `[offset, length]` in the data after the JSON.
fn blob(at: [u64; 2]) -> Image {
    let mut image = Image::default();
    image.mime = "image/png".into();
    image.blob = Some(at);
    image
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn written(file: &File<Doc>, tail: &Tail<'_>, pretty: bool) -> Vec<u8> {
    let mut out = if pretty { serde_json::to_vec_pretty(file) } else { serde_json::to_vec(file) }.unwrap();
    tail.write_to(&mut out).unwrap();
    out
}

fn full_file(pdf: &[u8]) -> (File<Doc>, Tail<'_>) {
    let mut tail = Tail::new(VERSION);
    let mut file = File::new(doc());
    file.generator = Some("VectorCraft 0.17.0".into());
    file.preview = Some(Image::inline("image/png", PNG));
    file.pdf = Some(tail.image("application/pdf", pdf, false));
    file.images.insert("k".into(), tail.image("image/png", &[1, 2, 3, 250], false));
    (file, tail)
}

#[test]
fn a_version_2_file_keeps_large_entries_inline_under_the_former_name() {
    let pdf = pdf();
    let mut tail = Tail::new(2);
    let mut file = File::new(doc());
    file.format = LEGACY_EXTENSION.into();
    file.version = 2;
    file.profiles.insert("Gray".into(), tail.image(craft_vectorcraft_format::ICC_MIME, &pdf, false));
    let bytes = written(&file, &tail, false);
    assert!(tail.is_empty());
    assert!(!bytes.contains(&0));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with(r#"{"format":"drawcraft","version":2,"profiles":{"Gray":{"mime":"application/vnd.iccprofile","data":"JVBERi0xLjcK"#));
    let back = File::<Doc>::read(text.as_bytes()).unwrap();
    assert!(back.is_legacy());
    assert_eq!(back.profiles["Gray"].bytes(&[]).unwrap(), pdf);
}

#[test]
fn a_written_file_reads_back_with_its_entries() {
    let pdf = pdf();
    let (file, tail) = full_file(&pdf);
    for bytes in [written(&file, &tail, false), written(&file, &tail, true)] {
        let (json, after) = split_tail(&bytes);
        assert_eq!(json_part(&bytes), json);
        assert_eq!(after, pdf.as_slice());
        let back = File::<Doc>::read(json).unwrap();
        assert_eq!(back, file);
        back.check().unwrap();
        assert!(!back.is_legacy());
        assert_eq!(back.pdf.as_ref().unwrap().bytes(after).unwrap(), pdf);
        assert_eq!(back.pdf.as_ref().unwrap().len(), pdf.len() as u64);
        assert_eq!(back.preview.as_ref().unwrap().bytes(after).unwrap(), PNG);
        assert_eq!(back.images["k"].bytes(after).unwrap(), [1, 2, 3, 250]);
        assert_eq!(back.images["k"].len(), 4);
        let head: Head = serde_json::from_slice(json).unwrap();
        assert_eq!(head.preview, file.preview);
        assert_eq!(head.pdf, file.pdf);
        let skipped = File::<IgnoredAny>::read(json).unwrap();
        assert_eq!(skipped.generator.as_deref(), Some("VectorCraft 0.17.0"));
    }
}

#[test]
fn image_bytes_refuses_an_entry_past_the_limit_before_copying_it() {
    assert_eq!(MAX_ENTRY, 1 << 30);
    let tail = b"abcd";
    let inline = Image::inline("image/png", tail);
    for image in [blob([0, 4]), inline.clone()] {
        assert_eq!(image.bytes_at_most(tail, 4).as_deref(), Some(&tail[..]));
        assert_eq!(image.bytes_at_most(tail, 3), None);
        assert_eq!(image.bytes(tail).as_deref(), Some(&tail[..]));
    }
    // The stated length is checked, so a range past the limit is refused even where the data
    // holds it.
    assert_eq!(blob([0, MAX_ENTRY + 1]).bytes_at_most(&[0; 64], MAX_ENTRY), None);
    assert_eq!(blob([0, 2]).bytes_at_most(tail, 2).as_deref(), Some(&b"ab"[..]));
}

#[test]
fn entries_copy_at_most_the_data_after_the_json_in_all() {
    let tail = vec![7u8; 1 << 20];
    let len = tail.len() as u64;
    let whole = blob([0, len]);
    // Many entries naming the same range share one copy, which counts once.
    let mut entries = Entries::new(&tail);
    let first = entries.bytes(&whole).ok().flatten();
    assert_eq!(first.as_deref().map(Vec::as_slice), Some(&tail[..]));
    assert!(entries.blob([0, len]).ok().flatten().zip(first.as_ref()).is_some_and(|(a, b)| std::sync::Arc::ptr_eq(&a, b)));
    for _ in 0..3 {
        let again = entries.bytes(&whole).ok().flatten();
        assert!(again.as_ref().zip(first.as_ref()).is_some_and(|(a, b)| std::sync::Arc::ptr_eq(a, b)));
    }
    assert_eq!(entries.copied(), len);
    // Overlapping ranges that differ copy the shared bytes once each, and pass the data's size.
    let from = |at: u64| blob([at, len - at]);
    assert!(matches!(entries.bytes(&from(1)), Err(Error::TooBig)));
    // A refused entry is not counted, so the entries after it still read.
    assert_eq!(entries.copied(), len);
    let mut entries = Entries::new(&tail);
    let part = |at: u64, n: u64| blob([at, n]);
    assert!(entries.bytes(&part(0, len / 2)).is_ok_and(|b| b.is_some()));
    assert!(matches!(entries.bytes(&part(1, len - 1)), Err(Error::TooBig)));
    assert_eq!(entries.copied(), len / 2);
    assert!(entries.bytes(&part(len / 2, len / 2)).is_ok_and(|b| b.is_some()));
    assert_eq!(entries.copied(), len);
    // Entries that name each byte once read whole, with inline and missing entries beside them.
    let mut entries = Entries::new(&tail);
    let half = |at: u64| blob([at, len / 2]);
    assert_eq!(entries.bytes(&half(0)).unwrap().map(|b| b.len()), Some(tail.len() / 2));
    assert_eq!(entries.bytes(&Image::inline("image/png", PNG)).unwrap().as_deref().map(Vec::as_slice), Some(PNG));
    assert_eq!(entries.bytes(&half(len)), Ok(None));
    assert_eq!(entries.bytes(&half(len / 2)).unwrap().map(|b| b.len()), Some(tail.len() / 2));
    assert_eq!(entries.copied(), len);
    assert_eq!(Entries::new(&[]).bytes(&whole), Ok(None));
}

#[test]
fn contents_read_pdf_preference_while_skipping_the_payload() {
    for (members, expected) in [
        ("", false),
        (r#", "pdf":null"#, false),
        (r#", "pdf":{"data":"not base64"}"#, true),
        (r#", "pdf":[1,2,3]"#, true),
        (r#", "pdfCompatible":true"#, true),
        (r#", "pdf":{}, "pdfCompatible":false"#, false),
        (r#", "pdfCompatible":false, "pdf":{}"#, false),
        (r#", "pdfCompatible":true, "pdfCompatible":null, "pdf":{}"#, true),
        (r#", "pdf":{}, "pdf":null"#, false),
        (r#", "pdfCompatible":false, "pdfCompatible":true"#, true),
        (r#", "unknown":{"pdf":{}}"#, false),
    ] {
        let json = format!(r#"{{"format":"vectorcraft","version":4,"document":{{"layers":[]}}{members}}}"#);
        let (contents, compatible) = Contents::<Doc>::read_with_pdf_preference(json.as_bytes()).unwrap();
        assert_eq!(compatible, expected, "{members}");
        assert_eq!(contents, Contents::<Doc>::read(json.as_bytes()).unwrap());
    }
    let array = br#"["drawcraft",1,{"layers":[]}]"#;
    assert!(!Contents::<Doc>::read_with_pdf_preference(array).unwrap().1);
    let mut file = File::new(doc());
    file.pdf = Some(Image::inline("application/pdf", &vec![7; 128 << 10]));
    assert!(Contents::<Doc>::read_with_pdf_preference(&serde_json::to_vec(&file).unwrap()).unwrap().1);
}

#[test]
fn reading_refuses_another_format_and_a_newer_version() {
    let newer = br#"{"format":"vectorcraft","version":5,"document":0}"#;
    let other = br#"{"format":"other","version":1,"document":0}"#;
    for (json, error) in [(&newer[..], Error::TooNew(5)), (other, Error::Format("other".into()))] {
        let file = File::<IgnoredAny>::read(json);
        assert!(matches!(&file, Err(ReadError::Envelope(e)) if *e == error), "{file:?}");
        let contents = Contents::<IgnoredAny>::read(json);
        assert!(matches!(&contents, Err(ReadError::Envelope(e)) if *e == error), "{contents:?}");
        assert_eq!(contents.err().map(|e| e.to_string()), Some(error.to_string()));
    }
    for version in [0, 1, VERSION] {
        let json = format!(r#"{{"format":"drawcraft","version":{version},"document":0}}"#);
        assert!(File::<IgnoredAny>::read(json.as_bytes()).is_ok() && Contents::<IgnoredAny>::read(json.as_bytes()).is_ok());
    }
    // A document that does not read is a JSON error, as before the version is looked at.
    let bad = br#"{"format":"vectorcraft","version":5,"document":7}"#;
    assert!(matches!(Contents::<Doc>::read(bad), Err(ReadError::Json(_))));
    assert!(Contents::<Doc>::read(b"\"x\"").is_err_and(|e| e.to_string().contains("expected struct File")));
}

#[test]
fn an_array_holds_format_version_document_images_and_profiles_in_that_order() {
    // VectorCraft reads a top-level array by position, in this order.
    let png = r#"{"mime":"image/png","data":"iVBORw0KGgo="}"#;
    let images = format!(r#"{{"k":{png}}}"#);
    let profiles = format!(r#"{{"Gray":{png}}}"#);
    let full = format!(r#"["vectorcraft",3,{{"layers":[]}},{images},{profiles}]"#);
    for json in [full.clone(), format!(r#"["vectorcraft",3,{{"layers":[]}},{images}]"#), r#"["vectorcraft",3,{"layers":[]}]"#.into()] {
        let contents = Contents::<Doc>::read(json.as_bytes()).unwrap_or_else(|e| panic!("{json}: {e}"));
        let file = File::<Doc>::read(json.as_bytes()).unwrap_or_else(|e| panic!("{json}: {e}"));
        assert_eq!(
            (&file.format, file.version, &file.document, &file.images, &file.profiles),
            (&contents.format, 3, &contents.document, &contents.images, &contents.profiles)
        );
        assert!(file.generator.is_none() && file.preview.is_none() && file.pdf.is_none());
    }
    let contents = Contents::<Doc>::read(full.as_bytes()).unwrap();
    assert_eq!((contents.images.len(), contents.profiles.len()), (1, 1));
    assert!(contents.images.contains_key("k") && contents.profiles.contains_key("Gray"));
    for json in [r#"["vectorcraft",3,{},{"layers":[]},{}]"#, r#"["vectorcraft",3]"#, r#"["vectorcraft",5,{"layers":[]}]"#] {
        assert!(Contents::<Doc>::read(json.as_bytes()).is_err(), "{json}");
        assert!(File::<Doc>::read(json.as_bytes()).is_err(), "{json}");
    }
}

#[test]
fn the_json_is_split_only_at_a_marker_right_after_it() {
    let with = |json: &[u8], rest: &[u8]| {
        let mut f = json.to_vec();
        f.extend(BLOB_MAGIC);
        f.extend(rest);
        f
    };
    for json in [&b"{}"[..], b"[1]", b"\"s\"", b"1", b"{\"a\": [true, null]}"] {
        let f = with(json, b"xyz");
        assert_eq!(split_tail(&f), (json, &b"xyz"[..]), "{json:?}");
    }
    let empty: &[u8] = &[];
    let far = with(b"{} x", b"xyz");
    assert_eq!(split_tail(&far), (far.as_slice(), empty));
    let no_marker = b"{}\0rest".to_vec();
    assert_eq!(split_tail(&no_marker), (no_marker.as_slice(), empty));
    assert_eq!(split_tail(b"{}"), (&b"{}"[..], empty));
    let bad = with(b"{", b"x");
    assert_eq!(split_tail(&bad), (bad.as_slice(), empty));
}

#[test]
fn sniff_needs_a_vectorcraft_format_member() {
    let pdf = pdf();
    let (file, tail) = full_file(&pdf);
    let plain = written(&file, &tail, false);
    let pretty = written(&file, &tail, true);
    assert!(sniff(&plain) && sniff(&pretty));
    assert!(sniff(b" \n{\"format\" : \"drawcraft\"}"));
    // Keys and values are compared as JSON reads them, escapes decoded.
    let esc = |s: &str| s.replace('|', "\\");
    let escaped = esc(r#"{"|u0066ormat":"vector|u0063raft","version":4,"document":{"layers":[]}}"#);
    assert!(sniff(escaped.as_bytes()));
    assert_eq!(File::<Doc>::read(escaped.as_bytes()).unwrap().format, EXTENSION);
    assert!(sniff(esc(r#"{"format":"draw|u0063raft"}"#).as_bytes()));
    for no in [r#"{"form|u0061t":"other","format":"vectorcraft"}"#, r#"{"format":"vectorcraft|u0021"}"#, r#"{"format":"vectorcraft|"}"#] {
        assert!(!sniff(esc(no).as_bytes()), "{no}");
    }
    assert!(sniff(br#"{"generator":"format","x":{"format":"other","a":[1,"]"]},"n":-1.5e3,"t":true,"s":"\"format\":","format":"vectorcraft"}"#));
    assert!(is_compressed(&gzip(&plain)) && !is_compressed(&plain));
    assert!(sniff(&gzip(&plain)));
    let gz = gzip(&plain);
    assert!(sniff(&gz[..gz.len() / 2]));
    for no in [
        &br#"{"name":"vectorcraft"}"#[..],
        br#"{"format":"other","vectorcraft":1}"#,
        br#"["format","vectorcraft"]"#,
        br#"{"metadata":{"format":"vectorcraft"},"format":"other"}"#,
        br#"{"name":"\"format\":\"vectorcraft\""}"#,
        br#"{"format":["vectorcraft"]}"#,
        br#"{"format":"vectorcraft"#,
        b"%PDF-1.7",
        b"",
        b"\x1f\x8b",
        PNG,
    ] {
        assert!(!sniff(no), "{:?}", String::from_utf8_lossy(no));
    }
}

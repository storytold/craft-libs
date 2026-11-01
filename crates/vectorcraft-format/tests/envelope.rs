use std::io::Write;

use craft_vectorcraft_format::{
    BLOB_MAGIC, Contents, EXTENSION, Entries, Error, File, Head, INLINE_MAX, Image, LEGACY_EXTENSION, Tail, VERSION, base64, check, entry,
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
fn a_version_4_file_with_members_after_the_json_is_written_as_the_app_writes_it() {
    let pdf = pdf();
    let (file, tail) = full_file(&pdf);
    let mut expected = br#"{"format":"vectorcraft","version":4,"generator":"VectorCraft 0.17.0","preview":{"mime":"image/png","data":"iVBORw0KGgo="},"pdf":{"mime":"application/pdf","blob":[0,8192]},"document":{"layers":[{"name":"Layer 1"}]},"images":{"k":{"mime":"image/png","data":"AQID+g=="}}}"#.to_vec();
    expected.extend(b"\n\0VCBLOBS\0");
    expected.extend(&pdf);
    assert_eq!(written(&file, &tail, false), expected);
    assert_eq!(tail.len(), INLINE_MAX as u64);
    assert_eq!(tail.parts().len(), 1);
}

#[test]
fn a_pretty_file_without_optional_members_is_written_as_the_app_writes_it() {
    let mut file = File::new(doc());
    file.generator = Some("VectorCraft 0.17.0".into());
    let expected = r#"{
  "format": "vectorcraft",
  "version": 4,
  "generator": "VectorCraft 0.17.0",
  "document": {
    "layers": [
      {
        "name": "Layer 1"
      }
    ]
  },
  "images": {}
}"#;
    assert_eq!(String::from_utf8(written(&file, &Tail::new(VERSION), true)).unwrap(), expected);
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
    let back: File<Doc> = serde_json::from_str(&text).unwrap();
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
        let back: File<Doc> = serde_json::from_slice(json).unwrap();
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
        let skipped: File<IgnoredAny> = serde_json::from_slice(json).unwrap();
        assert_eq!(skipped.generator.as_deref(), Some("VectorCraft 0.17.0"));
    }
}

#[test]
fn a_linked_image_preview_is_marked_proxy() {
    let mut tail = Tail::new(VERSION);
    let image = tail.image("image/png", PNG, true);
    assert_eq!(serde_json::to_string(&image).unwrap(), r#"{"mime":"image/png","data":"iVBORw0KGgo=","proxy":true}"#);
    let back: Image = serde_json::from_str(r#"{"mime":"image/png","data":"iVBORw0KGgo=","proxy":true}"#).unwrap();
    assert_eq!(back, image);
}

#[test]
fn entries_outside_the_data_or_with_bad_base64_give_none() {
    let tail = b"abcd";
    assert_eq!(entry(tail, [1, 3]), Some(&b"bcd"[..]));
    assert_eq!(entry(tail, [1, 4]), None);
    assert_eq!(entry(tail, [u64::MAX, 1]), None);
    assert_eq!(entry(tail, [1, u64::MAX]), None);
    let blob = Image { mime: "image/png".into(), blob: Some([2, 8]), ..Image::default() };
    assert_eq!(blob.bytes(tail), None);
    assert_eq!(Image { data: "QU*D".into(), ..Image::default() }.bytes(tail), None);
    assert!(Image::default().is_empty());
}

#[test]
fn entries_copy_at_most_the_data_after_the_json_in_all() {
    let tail = vec![7u8; 1 << 20];
    let len = tail.len() as u64;
    let whole = Image { mime: "image/png".into(), blob: Some([0, len]), ..Image::default() };
    // Many entries naming the same range share one copy, which counts once.
    let mut entries = Entries::new(&tail);
    let first = entries.bytes(&whole).ok().flatten();
    assert_eq!(first.as_deref(), Some(&tail[..]));
    for _ in 0..3 {
        let again = entries.bytes(&whole).ok().flatten();
        assert!(again.as_ref().zip(first.as_ref()).is_some_and(|(a, b)| std::sync::Arc::ptr_eq(a, b)));
    }
    assert_eq!(entries.copied(), len);
    // Overlapping ranges that differ copy the shared bytes once each, and pass the data's size.
    let from = |at: u64| Image { mime: "image/png".into(), blob: Some([at, len - at]), ..Image::default() };
    assert!(matches!(entries.bytes(&from(1)), Err(Error::TooBig)));
    // A refused entry is not counted, so the entries after it still read.
    assert_eq!(entries.copied(), len);
    let mut entries = Entries::new(&tail);
    let part = |at: u64, n: u64| Image { mime: "image/png".into(), blob: Some([at, n]), ..Image::default() };
    assert!(entries.bytes(&part(0, len / 2)).is_ok_and(|b| b.is_some()));
    assert!(matches!(entries.bytes(&part(1, len - 1)), Err(Error::TooBig)));
    assert_eq!(entries.copied(), len / 2);
    assert!(entries.bytes(&part(len / 2, len / 2)).is_ok_and(|b| b.is_some()));
    assert_eq!(entries.copied(), len);
    // Entries that name each byte once read whole, with inline and missing entries beside them.
    let mut entries = Entries::new(&tail);
    let half = |at: u64| Image { mime: "image/png".into(), blob: Some([at, len / 2]), ..Image::default() };
    assert_eq!(entries.bytes(&half(0)).unwrap().map(|b| b.len()), Some(tail.len() / 2));
    assert_eq!(entries.bytes(&Image::inline("image/png", PNG)).unwrap().as_deref(), Some(PNG));
    assert_eq!(entries.bytes(&half(len)), Ok(None));
    assert_eq!(entries.bytes(&half(len / 2)).unwrap().map(|b| b.len()), Some(tail.len() / 2));
    assert_eq!(entries.copied(), len);
    assert_eq!(Entries::new(&[]).bytes(&whole), Ok(None));
}

#[test]
fn optional_members_of_the_wrong_shape_read_as_none() {
    let json = br#"{"format":"vectorcraft","version":4,"generator":7,"preview":"x","pdf":{"blob":"y"},"document":{"layers":[]}}"#;
    let file: File<Doc> = serde_json::from_slice(json).unwrap();
    assert_eq!((file.generator, file.preview, file.pdf), (None, None, None));
    let skipped: File<IgnoredAny> = serde_json::from_slice(json).unwrap();
    assert!(skipped.preview.is_none());
    let head: Head = serde_json::from_slice(json).unwrap();
    assert_eq!(head, Head::default());
    let good = br#"{"preview":{"mime":"image/png","data":"iVBORw0KGgo="},"pdf":null}"#;
    let head: Head = serde_json::from_slice(good).unwrap();
    assert_eq!(head.preview, Some(Image::inline("image/png", PNG)));
    assert!(head.pdf.is_none());
}

#[test]
fn contents_read_the_document_images_and_profiles_without_the_preview_and_pdf() {
    let big = vec![b'%'; 64 << 10];
    let mut file = File::new(doc());
    file.version = 2;
    file.generator = Some("VectorCraft 0.17.0".into());
    file.preview = Some(Image::inline("image/png", &big));
    file.pdf = Some(Image::inline("application/pdf", &big));
    file.profiles.insert("Gray".into(), Image::inline(craft_vectorcraft_format::ICC_MIME, &[4, 5]));
    file.images.insert("k".into(), Image::inline("image/png", PNG));
    for json in [serde_json::to_vec(&file).unwrap(), serde_json::to_vec_pretty(&file).unwrap()] {
        let contents: Contents<Doc> = serde_json::from_slice(&json).unwrap();
        let Contents { format, version, profiles, document, images, .. } = contents.clone();
        assert_eq!((format, version, profiles, document, images), (file.format.clone(), 2, file.profiles.clone(), doc(), file.images.clone()));
        contents.check().unwrap();
        assert!(contents.is_legacy() == file.is_legacy());
    }
    // Members of the wrong shape are skipped as `File` reads them as `None`; a repeated member,
    // a missing document and a newer version are refused as in `File`.
    let odd = br#"{"format":"vectorcraft","version":4,"generator":7,"preview":"x","pdf":{"blob":"y"},"document":{"layers":[]},"z":[1]}"#;
    let contents: Contents<Doc> = serde_json::from_slice(odd).unwrap();
    assert!(contents.document.layers.is_empty() && contents.images.is_empty());
    for bad in
        [&br#"{"format":"vectorcraft","version":4,"pdf":null,"pdf":null,"document":{"layers":[]}}"#[..], br#"{"format":"vectorcraft","version":4}"#]
    {
        assert!(serde_json::from_slice::<File<Doc>>(bad).is_err());
        assert!(serde_json::from_slice::<Contents<Doc>>(bad).is_err());
    }
    let newer: Contents<IgnoredAny> = serde_json::from_str(r#"{"format":"vectorcraft","version":5,"document":0}"#).unwrap();
    assert_eq!(newer.check(), Err(Error::TooNew(5)));
}

#[test]
fn a_files_tail_follows_its_version() {
    let mut file = File::new(doc());
    let pdf = pdf();
    assert_eq!(file.tail().image("application/pdf", &pdf, false).blob, Some([0, pdf.len() as u64]));
    file.version = 2;
    assert!(file.tail().image("application/pdf", &pdf, false).blob.is_none());
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
fn check_refuses_other_formats_and_newer_versions() {
    assert_eq!(check(EXTENSION, VERSION), Ok(()));
    assert_eq!(check(LEGACY_EXTENSION, 1), Ok(()));
    assert_eq!(check("other", 1), Err(Error::Format("other".into())));
    assert_eq!(check(EXTENSION, VERSION + 1), Err(Error::TooNew(VERSION + 1)));
    assert!(serde_json::from_str::<File<Doc>>(r#"{"format":"vectorcraft","version":4}"#).is_err());
    let minimal: File<Doc> = serde_json::from_str(r#"{"format":"vectorcraft","version":4,"document":{"layers":[]}}"#).unwrap();
    assert!(minimal.images.is_empty() && minimal.generator.is_none());
}

#[test]
fn sniff_needs_a_vectorcraft_format_member() {
    let pdf = pdf();
    let (file, tail) = full_file(&pdf);
    let plain = written(&file, &tail, false);
    let pretty = written(&file, &tail, true);
    assert!(sniff(&plain) && sniff(&pretty));
    assert!(sniff(b" \n{\"format\" : \"drawcraft\"}"));
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

#[test]
fn base64_is_the_encoding_of_data_members() {
    assert_eq!(base64::encode(PNG), "iVBORw0KGgo=");
    assert_eq!(base64::decode("AQID+g==").unwrap(), [1, 2, 3, 250]);
}

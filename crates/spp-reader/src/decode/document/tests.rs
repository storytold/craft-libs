use super::test_writer::{CompactWriter, DocWriter, compact_sample, sample};
use super::*;
use crate::decode::DecodeError;

#[test]
fn counts_classes_of_all_nesting_levels() {
    let census = ClassCensus::scan(&sample()).unwrap();
    assert_eq!(census.count("DataDocument"), 1);
    assert_eq!(census.count("DataLayerColor"), 2);
    assert_eq!(census.count("Data2DTransformation"), 2);
    assert_eq!(census.total(), 5);
    assert!(census.is_complete());
    assert_eq!(census.by_frequency()[2], ("DataDocument", 1));
}

#[test]
fn field_names_are_not_classes() {
    let census = ClassCensus::scan(&sample()).unwrap();
    assert_eq!(census.count("colorManagementACE"), 0);
    assert_eq!(census.count("uvTransformation"), 0);
}

#[test]
fn reads_the_format_version() {
    assert_eq!(DocumentHeader::parse(&sample()).unwrap().format_version, 78);
}

#[test]
fn rejects_other_signatures() {
    let mut data = sample();
    data[0] = 0;
    assert_eq!(ClassCensus::scan(&data), Err(DecodeError::BadMagic { record: RECORD }));
}

#[test]
fn unknown_tags_skip_only_the_enclosing_object() {
    let mut w = DocWriter::new(78);
    w.object(0x12, "DataDocument", 2);
    w.field("first").object(0x14, "DataFuture", 2);
    w.field("mystery").raw(&[0x7f, 1, 2, 3]);
    w.field("hidden").object(0x14, "DataHidden", 0).close();
    w.close();
    w.field("after").object(0x14, "DataAfter", 0).close();
    w.close();
    let census = ClassCensus::scan(&w.bytes).unwrap();
    assert_eq!(census.count("DataFuture"), 1);
    assert_eq!(census.count("DataHidden"), 0);
    assert_eq!(census.count("DataAfter"), 1);
    assert_eq!(census.unknown_tags().get(&0x7f), Some(&1));
    assert!(!census.is_complete());
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let data = sample();
    for len in 0..data.len() {
        assert!(ClassCensus::scan(&data[..len]).is_err(), "length {len}");
    }
}

#[test]
fn corrupt_counts_and_offsets_are_rejected() {
    let data = sample();
    // Field count of the root object: after header (12), tag, flag, end (4), class (4 + 12).
    let mut many_fields = data.clone();
    many_fields[34..36].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(matches!(ClassCensus::scan(&many_fields), Err(DecodeError::Invalid { .. })));
    let mut bad_end = data;
    bad_end[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(ClassCensus::scan(&bad_end), Err(DecodeError::Invalid { .. })));
}

#[test]
fn deep_nesting_is_bounded() {
    let mut w = DocWriter::new(78);
    for _ in 0..300 {
        w.object(0x14, "DataNest", 1).field("child");
    }
    w.raw(&[0x0a, 0]);
    for _ in 0..300 {
        w.close();
    }
    assert!(matches!(ClassCensus::scan(&w.bytes), Err(DecodeError::Invalid { .. })));
}

/// Real files are user content, so they stay outside the repository.
/// Run with `TC_IMPORT_CORPUS=<folders or project files> cargo test -p craft-spp-reader -- --ignored`: scans each project's document.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore]
fn scans_real_samples() {
    let Some(files) = crate::test_corpus::files() else {
        return;
    };
    for path in &files {
        let project = crate::project::ProjectFile::open(path).unwrap();
        let data = project.read(crate::project::paths::DOCUMENT).unwrap();
        let version = DocumentHeader::parse(&data).map(|h| h.format_version);
        match ClassCensus::scan(&data) {
            Ok(c) => println!("{} {version:?} total {} unknown {:?} {:?}", path.display(), c.total(), c.unknown_tags(), c.by_frequency()),
            Err(e) => println!("{} {version:?} error {e}", path.display()),
        }
    }
}

#[test]
fn compact_layout_is_selected_by_the_header() {
    let header = DocumentHeader::parse(&compact_sample()).unwrap();
    assert_eq!((header.layout, header.format_version), (Layout::Compact, 92));
    assert_eq!(DocumentHeader::parse(&sample()).unwrap().layout, Layout::Tagged);
    let mut data = sample();
    data[4] = 2;
    assert!(DocumentHeader::parse(&data).is_err());
}

#[test]
fn compact_layout_walks_like_the_tagged_one() {
    let census = ClassCensus::scan(&compact_sample()).unwrap();
    assert_eq!(census.count("DataDocument"), 1);
    assert_eq!(census.count("DataLayerColor"), 2);
    assert_eq!(census.count("Data2DTransformation"), 2);
    assert_eq!(census.count("DataBlending"), 2);
    assert_eq!(census.total(), 7);
    assert!(census.is_complete());
}

/// Records what a walk reports, to compare the order of events.
#[derive(Default)]
struct Events(Vec<String>);

impl Visitor for Events {
    fn object(&mut self, class: &str) {
        self.0.push(format!("object {class}"));
    }
    fn end_object(&mut self) {
        self.0.push("end".into());
    }
    fn field(&mut self, name: &[u8]) {
        self.0.push(String::from_utf8_lossy(name).into_owned());
    }
    fn scalar(&mut self, tag: ValueTag, payload: &[u8]) {
        self.0.push(format!("{:?}", Scalar::decode(tag, payload)));
    }
    fn list(&mut self, count: usize) {
        self.0.push(format!("list {count}"));
    }
    fn end_list(&mut self) {
        self.0.push("end list".into());
    }
    fn unknown_tag(&mut self, tag: u32, _offset: usize) {
        self.0.push(format!("unknown {tag:#x}"));
    }
}

#[test]
fn compact_values_arrive_with_their_field_names() {
    let mut events = Events::default();
    walk(&compact_sample(), &mut events).unwrap();
    let text = events.0.join(" | ");
    assert!(
        text.starts_with(
            "object DataDocument | colorManagementACE | enabled | Some(Bool(true)) | label | Some(String(\"Ink\")) | layers | list 2 | object DataLayerColor"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "blending | list 2 | object DataBlending | blendingMode | Some(Enum(11)) | channelTypes | Some(WideMask(128)) | opacity | Some(Float(1.0)) | end | object DataBlending | blendingMode | Some(Enum(29))"
        ),
        "{text}"
    );
    assert!(text.ends_with("end | end list | end"), "{text}");
}

#[test]
fn compact_reference_to_an_earlier_table_is_followed() {
    let fields: &[(&str, u32)] = &[("uid", 0x0c)];
    let mut w = CompactWriter::new(92, 2);
    w.object("DataDocument", &[("items", 0x13)]).u32(2);
    w.object("DataItem", fields).raw(&1u64.to_le_bytes()).close();
    w.object("DataItem", fields).raw(&2u64.to_le_bytes()).close();
    w.close();
    assert_eq!(ClassCensus::scan(&w.bytes).unwrap().count("DataItem"), 2);
    let second = w.bytes.len() - 16;
    assert_eq!(w.bytes[second + 4], 1, "table index of the second item");
    let mut bad = w.bytes.clone();
    bad[second + 4] = 7;
    assert!(ClassCensus::scan(&bad).is_err());
}

#[test]
fn compact_unknown_tag_skips_to_the_enclosing_object_end() {
    let mut w = CompactWriter::new(92, 3);
    w.object("DataDocument", &[("first", 0x14), ("after", 0x14)]);
    w.object("DataFuture", &[("list", 0x11), ("hidden", 0x14)]);
    w.value_list(2, "DataElement", &[("known", 0x05), ("mystery", 0x7f), ("rest", 0x05)]);
    w.raw(&[1, 0, 0, 0, 9, 9, 9]);
    w.close();
    w.object("DataAfter", &[]).close();
    w.close();
    let census = ClassCensus::scan(&w.bytes).unwrap();
    assert_eq!(census.count("DataFuture"), 1);
    assert_eq!(census.count("DataElement"), 1);
    assert_eq!(census.count("DataAfter"), 1);
    assert_eq!(census.unknown_tags().get(&0x7f), Some(&1));
    assert!(!census.is_complete());
}

#[test]
fn compact_every_truncation_is_an_error_not_a_panic() {
    let data = compact_sample();
    for len in 0..data.len() {
        assert!(ClassCensus::scan(&data[..len]).is_err(), "length {len}");
    }
}

#[test]
fn compact_corruption_never_panics() {
    let data = compact_sample();
    for at in 12..data.len() {
        for byte in [0x00, 0x7f, 0xff] {
            let mut bad = data.clone();
            bad[at] = byte;
            let mut events = Events::default();
            let _ = walk(&bad, &mut events);
        }
    }
}

#[test]
fn compact_huge_counts_are_refused() {
    let huge = u32::MAX - 1;
    for tag in [0x11, 0x13] {
        let mut w = CompactWriter::new(92, 1);
        w.object("DataDocument", &[("items", tag)]).u32(huge);
        w.close();
        assert!(ClassCensus::scan(&w.bytes).is_err());
    }
    let mut w = CompactWriter::new(92, 2);
    w.object("DataDocument", &[("items", 0x11)]).value_list(5000, "DataEmpty", &[]).raw(&[0; 6000]);
    w.close();
    assert!(ClassCensus::scan(&w.bytes).is_err());
}

#[test]
fn compact_nesting_is_limited() {
    let fields: &[(&str, u32)] = &[("next", 0x14)];
    let mut w = CompactWriter::new(92, 1);
    for _ in 0..1000 {
        w.object("DataNested", fields);
    }
    w.absent();
    for _ in 0..1000 {
        w.close();
    }
    assert!(ClassCensus::scan(&w.bytes).is_err());
}

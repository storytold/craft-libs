use super::*;

fn note() -> Text<'static> {
    Text {
        open: 1,
        flags: 0x1c,
        optional_blocks: 1,
        icon: [10, 20, 30, 36],
        popup: [30, 40, 190, 280],
        color_space: 0,
        color: [65535, 1000, 3000, 0],
        author: b"Writer",
        name: b"",
        modified: b"2026",
        data_length: 0,
        data_key: *b"txtC",
        data: &[0xfe, 0xff, 0, 65, 0, 13, 0, 66],
    }
}

#[test]
fn text_and_unknown_sound_round_trip_without_copying_payloads() {
    let limits = Limits::default();
    let text = write_text(&note(), limits).unwrap();
    let sound = [0, 0, 0, 11, b's', b'n', b'd', b'A', 42, 3, 9];
    let block = write(&[&text, &sound], limits).unwrap();
    let parsed = read(&block, limits).unwrap();
    assert_eq!(parsed.len(), 2);
    let view = parsed[0].text().unwrap().unwrap();
    assert_eq!(view.author, note().author);
    assert_eq!(view.data, note().data);
    assert_eq!(view.icon, note().icon);
    assert_eq!(view.color, note().color);
    assert_eq!(view.data_length, 20);
    assert!(parsed[1].text().unwrap().is_none());
    assert_eq!(parsed[1].raw(), sound);
    let slices: Vec<_> = parsed.iter().map(Record::raw).collect();
    assert_eq!(write(&slices, limits).unwrap(), block);
    assert!(view.data.as_ptr() >= block.as_ptr());
    assert!(view.data.as_ptr() < block.as_ptr().wrapping_add(block.len()));
}

#[test]
fn text_cannot_read_past_the_declared_entry_into_its_neighbour() {
    // Regression from PhotoCraft #11138, without its app Note/Color policy.
    let mut record = write_text(&note(), Limits::default()).unwrap();
    let shortened = record.len() - note().data.len();
    record[..4].copy_from_slice(&(shortened as u32).to_be_bytes());
    let mut block = vec![0, 2, 0, 1, 0, 0, 0, 1];
    block.extend_from_slice(&record);
    assert!(read(&block, Limits::default()).is_err());
    assert!(write(&[&record], Limits::default()).is_err());
}

#[test]
fn truncation_and_hostile_counts_lengths_fail_closed() {
    let record = write_text(&note(), Limits::default()).unwrap();
    let block = write(&[&record], Limits::default()).unwrap();
    for end in 0..block.len() {
        assert!(read(&block[..end], Limits::default()).is_err(), "{end}");
    }
    for len in [0u32, 1, 7, u32::MAX] {
        let mut bad = block.clone();
        bad[8..12].copy_from_slice(&len.to_be_bytes());
        assert!(read(&bad, Limits::default()).is_err());
    }
    let mut bad = block;
    bad[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(read(&bad, Limits::default()).is_err());
}

#[test]
fn explicit_limits_and_pascal_length_are_not_silently_truncated() {
    let record = write_text(&note(), Limits::default()).unwrap();
    let block = write(&[&record], Limits::default()).unwrap();
    assert!(read(&block, Limits { records: 0, ..Limits::default() }).is_err());
    assert!(read(&block, Limits { bytes: block.len() - 1, ..Limits::default() }).is_err());
    assert!(write(&[&record], Limits { records: 0, ..Limits::default() }).is_err());
    assert!(write_text(&note(), Limits { bytes: record.len() - 1, ..Limits::default() }).is_err());
    let long = [0; 256];
    let mut text = note();
    text.author = &long;
    assert!(write_text(&text, Limits::default()).is_err());
}

#[test]
fn preserves_legacy_version_and_unused_data_header_acceptance() {
    let record = write_text(&note(), Limits::default()).unwrap();
    let mut block = write(&[&record], Limits::default()).unwrap();
    block[..4].copy_from_slice(&[0, 9, 0, 8]);
    block.extend_from_slice(b"trailing");
    assert_eq!(read(&block, Limits::default()).unwrap()[0].raw(), record);
}

#[test]
fn canonical_layout_matches_the_existing_photocraft_writer() {
    // PhotoCraft annotations_map::write_note layout (Brandon Thomas, aa87012a).
    // App RGB/position/text conversion happens before this shared wire writer.
    let n = note();
    let mut expected = vec![0, 0, 0, 0];
    expected.extend_from_slice(b"txtA");
    expected.extend_from_slice(&[1, 0x1c, 0, 1]);
    for value in [10i32, 20, 30, 36, 30, 40, 190, 280] {
        expected.extend_from_slice(&value.to_be_bytes());
    }
    for value in [0u16, 65535, 1000, 3000, 0] {
        expected.extend_from_slice(&value.to_be_bytes());
    }
    expected.extend_from_slice(b"\x06Writer\x00\x00\x00\x042026\x00");
    expected.extend_from_slice(&20u32.to_be_bytes());
    expected.extend_from_slice(b"txtC");
    expected.extend_from_slice(&8u32.to_be_bytes());
    expected.extend_from_slice(n.data);
    let len = expected.len() as u32;
    expected[..4].copy_from_slice(&len.to_be_bytes());
    assert_eq!(write_text(&n, Limits::default()).unwrap(), expected);
}

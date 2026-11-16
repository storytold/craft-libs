//! Independently specified version16 wire streams: one header, consecutive set bodies.
use craft_psd::{atn, descriptor::Value, error::PsdError};
use proptest::prelude::*;

fn u32be(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn unicode(out: &mut Vec<u8>, units: &[u16]) {
    u32be(out, units.len() as u32);
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
}
fn text(out: &mut Vec<u8>, value: &str) {
    unicode(out, &value.encode_utf16().collect::<Vec<_>>());
}
fn id(out: &mut Vec<u8>, key: &str) {
    if key.len() == 4 {
        u32be(out, 0);
    } else {
        u32be(out, key.len() as u32);
    }
    out.extend_from_slice(key.as_bytes());
}
fn descriptor(items: &[(&str, &[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    unicode(&mut out, &[]);
    id(&mut out, "null");
    u32be(&mut out, items.len() as u32);
    for (key, kind, value) in items {
        id(&mut out, key);
        out.extend_from_slice(*kind);
        out.extend(value);
    }
    out
}
fn list(descriptors: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    u32be(&mut out, descriptors.len() as u32);
    for descriptor in descriptors {
        out.extend_from_slice(b"Objc");
        out.extend(descriptor);
    }
    out
}
fn string(value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    text(&mut out, value);
    out
}
fn set_header(out: &mut Vec<u8>, name: &[u16], actions: u32) {
    unicode(out, name);
    out.push(0);
    u32be(out, actions);
}
fn action_header(out: &mut Vec<u8>, name: &str, items: u32) {
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // Fn, Shift, Control and colour.
    text(out, name);
    out.push(0);
    u32be(out, items);
}
fn bare_item(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0, 1, 0, 0]);
    out.extend_from_slice(b"longInvr");
    u32be(out, 0);
    u32be(out, 0);
}

fn fixture() -> (Vec<u8>, usize, Vec<u8>, Vec<u8>) {
    let metadata = descriptor(&[
        ("actionDescriptionKey", b"TEXT", string("Description\0")),
        ("actionThumbnailTypeKey", b"long", 3i32.to_be_bytes().to_vec()),
        ("actionIconKey", b"TEXT", string("future-icon\0")),
        ("actionPreviewKey", b"bool", vec![0]),
        ("actionGenerativeKey", b"bool", vec![0]),
        ("actionPreconditionVal", b"long", (-1i32).to_be_bytes().to_vec()),
        ("futureDouble", b"doub", 0x7ff8_0000_0000_0042u64.to_be_bytes().to_vec()),
    ]);
    let first = descriptor(&[("setColorIndex", b"long", 7i32.to_be_bytes().to_vec()), ("actions", b"VlLs", list(&[metadata]))]);
    let empty = descriptor(&[("setColorIndex", b"long", 19i32.to_be_bytes().to_vec()), ("actions", b"VlLs", list(&[]))]);
    let root = descriptor(&[("sets", b"VlLs", list(&[first, empty])), ("futureRaw", b"tdta", vec![0, 0, 0, 3, 0, 255, 7])]);
    let mut payload = 16u32.to_be_bytes().to_vec();
    payload.extend(root);
    let mut raw = Vec::new();
    u32be(&mut raw, payload.len() as u32);
    raw.extend(&payload);
    let outer = descriptor(&[("Msge", b"TEXT", string("Original note\0")), ("Cntn", b"bool", vec![1]), ("modernizedActionData", b"tdta", raw)]);
    let mut bytes = 16u32.to_be_bytes().to_vec();
    set_header(&mut bytes, &"Real\0".encode_utf16().collect::<Vec<_>>(), 1);
    action_header(&mut bytes, "Invert\0", 1);
    bare_item(&mut bytes);
    let first_end = bytes.len();
    set_header(&mut bytes, &[0xd800, 0], 0); // Empty real set and unpaired UTF16 remain exact.
    set_header(&mut bytes, &"Action sidecar\0".encode_utf16().collect::<Vec<_>>(), 1);
    action_header(&mut bytes, "Play for details\0", 1);
    bytes.extend_from_slice(&[0, 1, 1, 2]);
    bytes.extend_from_slice(b"TEXT");
    u32be(&mut bytes, 4);
    bytes.extend_from_slice(b"stop");
    u32be(&mut bytes, 4);
    bytes.extend_from_slice(b"Stop");
    bytes.extend_from_slice(&(-1i32).to_be_bytes());
    bytes.extend(&outer);
    (bytes, first_end, outer, payload)
}

#[test]
fn one_version_and_three_set_bodies_preserve_raw_sidecar_descriptor_separately() {
    let (bytes, first_end, outer, payload) = fixture();
    assert_eq!(bytes.get(first_end..first_end + 4).unwrap(), &2u32.to_be_bytes());
    let sets = atn::parse_sets(&bytes).unwrap();
    assert_eq!(sets.len(), 3);
    assert_eq!(sets[0].name.to_string_lossy(), "Real");
    assert_eq!(sets[1].name.0, vec![0xd800, 0]);
    assert!(sets[1].actions.is_empty());
    let sidecar = sets[2].actions[0].items[0].descriptor.as_ref().unwrap();
    assert_eq!(sidecar.to_bytes(), outer, "descriptor bytes have their own independent oracle");
    assert_eq!(sidecar.get("modernizedActionData"), Some(&Value::RawData(payload)));
    assert_eq!(atn::write_sets(&sets).unwrap(), bytes);
    let single = atn::parse(&bytes).unwrap();
    assert_eq!(single.name, sets[0].name);
    assert_eq!(single.warnings, [format!("{} trailing bytes ignored", bytes.len() - first_end)]);
}

#[test]
fn every_truncated_later_set_fails_without_returning_partial_sets() {
    let (bytes, first_end, _, _) = fixture();
    // The first complete set alone is valid. The second complete empty set is another valid prefix.
    let second_end = first_end + 13;
    for cut in first_end + 1..bytes.len() {
        if cut != second_end {
            assert!(atn::parse_sets(&bytes[..cut]).is_err(), "cut {cut}");
        }
    }
    assert_eq!(atn::parse_sets(&bytes[..first_end]).unwrap().len(), 1);
    assert_eq!(atn::parse_sets(&bytes[..second_end]).unwrap().len(), 2);
    let mut repeated = bytes[..first_end].to_vec();
    repeated.extend_from_slice(&16u32.to_be_bytes());
    set_header(&mut repeated, &[], 0);
    assert!(atn::parse_sets(&repeated).is_err(), "a repeated version is not the measured set boundary");
    assert!(atn::parse_sets(&16u32.to_be_bytes()).is_err());
    assert!(atn::write_sets(&[]).is_err());
}

#[test]
fn forced_encoding_applies_to_all_sets_and_legacy_tails_remain_compatible() {
    let (bytes, _, _, _) = fixture();
    let sets = atn::parse_sets_with_encoding(&bytes, Some("shift_jis")).unwrap();
    assert!(sets.iter().all(|set| set.encoding.as_deref() == Some("Shift_JIS")));
    assert_eq!(atn::write_sets(&sets).unwrap(), bytes);
    assert!(atn::parse_sets_with_encoding(&bytes, Some("utf-16")).is_err());
    let mut legacy = 12u32.to_be_bytes().to_vec();
    set_header(&mut legacy, &[], 0);
    legacy.extend_from_slice(&[99, 1, 4]);
    let old = atn::parse(&legacy).unwrap();
    assert_eq!(atn::parse_sets(&legacy).unwrap(), vec![old.clone()]);
    assert_eq!(old.warnings, ["3 trailing bytes ignored"]);
}

#[test]
fn whole_archive_set_and_action_budgets_are_checked_across_boundaries() {
    let mut bytes = 16u32.to_be_bytes().to_vec();
    for _ in 0..atn::MAX_SETS {
        set_header(&mut bytes, &[], 0);
    }
    let sets = atn::parse_sets(&bytes).unwrap();
    assert_eq!(sets.len(), atn::MAX_SETS);
    assert_eq!(atn::write_sets(&sets).unwrap(), bytes);
    set_header(&mut bytes, &[], 0);
    assert!(matches!(atn::parse_sets(&bytes), Err(PsdError::LimitExceeded(_))));
    let mut excess = sets;
    excess.push(Default::default());
    assert!(matches!(atn::write_sets(&excess), Err(PsdError::LimitExceeded(_))));

    let mut bytes = 16u32.to_be_bytes().to_vec();
    for _ in 0..2 {
        set_header(&mut bytes, &[], (atn::MAX_TOTAL_ACTIONS / 2) as u32);
        for _ in 0..atn::MAX_TOTAL_ACTIONS / 2 {
            action_header(&mut bytes, "", 0);
        }
    }
    let sets = atn::parse_sets(&bytes).unwrap();
    assert_eq!(sets.iter().map(|set| set.actions.len()).sum::<usize>(), atn::MAX_TOTAL_ACTIONS);
    assert_eq!(atn::write_sets(&sets).unwrap(), bytes);
    set_header(&mut bytes, &[], 1);
    action_header(&mut bytes, "", 0);
    assert!(matches!(atn::parse_sets(&bytes), Err(PsdError::LimitExceeded(_))));
}

#[test]
fn aggregate_steps_are_bounded_before_a_later_action_allocates() {
    let mut bytes = 16u32.to_be_bytes().to_vec();
    set_header(&mut bytes, &[], 2);
    for _ in 0..2 {
        action_header(&mut bytes, "", (atn::MAX_TOTAL_ITEMS / 2) as u32);
        for _ in 0..atn::MAX_TOTAL_ITEMS / 2 {
            bare_item(&mut bytes);
        }
    }
    let sets = atn::parse_sets(&bytes).unwrap();
    assert_eq!(atn::write_sets(&sets).unwrap(), bytes);
    set_header(&mut bytes, &[], 1);
    action_header(&mut bytes, "", 1);
    bare_item(&mut bytes);
    assert!(matches!(atn::parse_sets(&bytes), Err(PsdError::LimitExceeded(_))));
    let mut excess = sets;
    excess[0].actions[0].items.push(Default::default());
    assert!(matches!(atn::write_sets(&excess), Err(PsdError::LimitExceeded(_))));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn arbitrary_streams_reject_or_reach_a_stable_writer(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        if let Ok(files) = atn::parse_sets(&data) {
            let encoded = atn::write_sets(&files).unwrap();
            let again = atn::parse_sets(&encoded).unwrap();
            prop_assert_eq!(atn::write_sets(&again).unwrap(), encoded);
        }
    }

    #[test]
    fn mutated_independent_multiset_stream_rejects_or_reaches_a_stable_writer(index in any::<usize>(), value in any::<u8>()) {
        let (mut bytes, _, _, _) = fixture();
        let at = index % bytes.len(); bytes[at] = value;
        if let Ok(files) = atn::parse_sets(&bytes) {
            let encoded = atn::write_sets(&files).unwrap();
            let again = atn::parse_sets(&encoded).unwrap();
            prop_assert_eq!(atn::write_sets(&again).unwrap(), encoded);
        }
    }
}

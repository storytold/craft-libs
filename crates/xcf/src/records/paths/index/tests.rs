use super::*;
use crate::Xcf;
use crate::tests::W;

// Original synthetic wire records from the public XCF layouts, not producer assets.
fn cubic(w: &mut W) {
    for value in [1, 0, 2, 3] {
        w.u32(value);
    }
    for (kind, x, y) in [(1, -2.0f32, 3.0f32), (0, 4.0, 5.0), (1, 6.0, 7.0)] {
        w.u32(kind);
        w.u32(x.to_bits());
        w.u32(y.to_bits());
    }
}

fn historical(kind: u32, label: &str) -> Vec<u8> {
    let mut w = W { out: Vec::new(), wide: false };
    if kind == 25 {
        w.u32(1);
    }
    w.u32(0); // Active source index.
    w.u32(1);
    w.string(label);
    if kind == 23 {
        w.u32(0); // Linked flag.
        w.out.push(0); // Legacy state.
        for value in [0, 0, 1] {
            // Open, no points, legacy version 1.
            w.u32(value);
        }
    } else {
        for value in [7, 1, 0, 0, 1] {
            // Tattoo, visible, linked, parasites, strokes.
            w.u32(value);
        }
        cubic(&mut w);
    }
    w.out
}

fn standalone(label: &str) -> Vec<u8> {
    let mut payload = W { out: Vec::new(), wide: false };
    payload.prop_u32(20, 11);
    payload.prop(43, &[]);
    payload.end();
    payload.u32(1);
    payload.u32(1);
    cubic(&mut payload);
    let mut w = W { out: Vec::new(), wide: false };
    w.string(label);
    w.u32(payload.out.len() as u32);
    w.out.extend(payload.out);
    w.out
}

fn file(version: u32, properties: &[(u32, Vec<u8>)], aliases: usize) -> (Vec<u8>, usize, usize) {
    let mut w = W::new(version);
    for value in [4, 4, 0] {
        w.u32(value);
    }
    if version >= 4 {
        w.u32(150);
    }
    for (kind, bytes) in properties {
        w.prop(*kind, bytes);
    }
    w.end();
    w.ptr(0); // Layer table.
    w.ptr(0); // Channel table.
    let table = w.out.len();
    let slots: Vec<_> = if version >= 18 { (0..aliases).map(|_| w.slot()).collect() } else { Vec::new() };
    if version >= 18 {
        w.ptr(0);
    }
    let source = w.out.len();
    if aliases != 0 {
        w.out.extend(standalone("Standalone"));
        for slot in slots {
            w.patch(slot, source);
        }
    }
    (w.out, table, source)
}

#[test]
fn indexed_paths_keep_coexisting_encodings_geometry_and_original_identity() {
    for version in [11, 18, 26] {
        let props = [(23, historical(23, "Legacy")), (25, historical(25, "Vectors"))];
        let (bytes, _, source) = file(version, &props, usize::from(version >= 18));
        let xcf = Xcf::parse(&bytes).unwrap();
        let declared = xcf.image().path_count;
        let paths = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(paths.stopped, None);
        let names: Vec<_> = paths.records.iter().map(|path| path.name).collect();
        assert_eq!(names, if version >= 18 { vec!["Legacy", "Vectors", "Standalone"] } else { vec!["Legacy", "Vectors"] });
        assert_eq!(declared, paths.records.len());
        assert_eq!(xcf.image().path_count, declared, "optional access does not mutate the legacy index");
        assert!(paths.records.iter().all(|path| path.selected && path.source_index == 0));
        for path in &paths.records {
            assert_eq!(bytes.get(path.offset..path.offset + path.raw.len()), Some(path.raw), "records borrow exact original source spans");
        }
        let vector = &paths.records[1];
        assert_eq!(vector.tattoo, Some(7));
        assert_eq!(vector.strokes[0].knots[0].anchor.position, [4., 5.]);
        assert_eq!(vector.strokes[0].knots[0].incoming.position, [-2., 3.]);
        if version >= 18 {
            assert_eq!(paths.records[2].offset, source);
            assert_eq!(paths.records[2].tattoo, Some(11));
            assert!(paths.records[2].geometry_complete());
            let strokes = &paths.records[2].strokes;
            assert_eq!(strokes.len(), 1);
            assert_eq!(strokes[0].knots.len(), 1);
            let knot = strokes[0].knots[0];
            assert_eq!(knot.incoming.position, [-2., 3.]);
            assert_eq!(knot.anchor.position, [4., 5.]);
            assert_eq!(knot.outgoing.position, [6., 7.]);
        }
    }
}

#[test]
fn optional_table_and_path_truncation_are_typed_without_rejecting_the_image() {
    let props = [(25, historical(25, "Vectors"))];
    let (bytes, table, _) = file(26, &props, 1);
    for truncated in [&bytes[..table + 4], &bytes[..bytes.len() - 1]] {
        let xcf = Xcf::parse(truncated).unwrap();
        assert_eq!((xcf.image().width, xcf.image().height), (4, 4));
        let paths = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(paths.records.len(), 1);
        assert_eq!(paths.records[0].name, "Vectors");
        assert_eq!(paths.stopped.unwrap().reason, StopReason::Malformed);
    }
    let xcf = Xcf::parse(&bytes[..table + 4]).unwrap();
    let pointers = xcf.path_offsets(super::super::super::Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(pointers.records.is_empty());
    assert_eq!(pointers.stopped.unwrap().offset, table);
}

#[test]
fn path_limits_cover_properties_aliases_and_shared_nonrefundable_work() {
    let (bytes, _, source) = file(26, &[(25, historical(25, "Vectors"))], 2);
    let xcf = Xcf::parse(&bytes).unwrap();
    let paths = xcf.paths(Limits { paths: 2, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(paths.records.len(), 2);
    assert_eq!(paths.records[1].offset, source);
    assert_eq!(paths.stopped.unwrap().reason, StopReason::RecordLimit);
    let full = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(full.records.len(), 3);
    assert_eq!(full.records[1].raw, full.records[2].raw);
    assert_eq!((full.records[1].source_index, full.records[2].source_index), (0, 1));
    let mut work = Budget::new(8, 1024);
    let first = xcf.path_offsets(super::super::super::Limits::default(), &mut work, &|| true).unwrap();
    assert_eq!(first.records, [source]);
    assert_eq!(first.stopped.unwrap().reason, StopReason::WorkLimit);
    let again = xcf.path_offsets(super::super::super::Limits::default(), &mut work, &|| true).unwrap();
    assert!(again.records.is_empty());
    assert_eq!(again.stopped.unwrap().reason, StopReason::WorkLimit);
    let no_storage = xcf.paths(Limits::default(), &mut Budget::new(1 << 20, 0), &|| true).unwrap();
    assert!(no_storage.records.is_empty());
    assert_eq!(no_storage.stopped.unwrap().reason, StopReason::RetainedLimit);
}

#[test]
fn a_malformed_historical_list_preserves_only_the_disclosed_prefix() {
    let props = [(23, historical(23, "Legacy")), (25, vec![0; 3])];
    let (bytes, _, _) = file(26, &props, 1);
    let xcf = Xcf::parse(&bytes).unwrap();
    let scan = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records.len(), 1);
    assert_eq!(scan.records[0].name, "Legacy");
    assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
}

#[test]
fn optional_path_cancellation_wins_at_entry_and_during_a_source_list() {
    let (bytes, _, _) = file(26, &[(25, historical(25, "Vectors"))], 2);
    let xcf = Xcf::parse(&bytes).unwrap();
    assert!(matches!(xcf.paths(Limits::default(), &mut Budget::new(0, 0), &|| false), Err(Error::Cancelled)));
    assert!(matches!(xcf.path_offsets(super::super::super::Limits::default(), &mut Budget::new(0, 0), &|| false), Err(Error::Cancelled)));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 20
    };
    assert!(matches!(xcf.paths(Limits::default(), &mut Budget::default(), &keep), Err(Error::Cancelled)));
    assert_eq!(calls.get(), 20);
    let completed = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(completed.stopped.is_none());
    assert_eq!(completed.records.len(), 3);
}

#[test]
fn standalone_pointer_targets_must_be_inside_the_source() {
    for version in [18, 26] {
        let (mut bytes, table, source) = file(version, &[], 2);
        let invalid = bytes.len();
        bytes[table + 8..table + 16].copy_from_slice(&(invalid as u64).to_be_bytes());
        let xcf = Xcf::parse(&bytes).unwrap();
        let scan = xcf.path_offsets(super::super::super::Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records, [source]);
        assert_eq!(scan.stopped.unwrap(), Diagnostic { offset: table + 8, ordinal: 1, reason: StopReason::Malformed });
    }
}

#[test]
fn standalone_table_shares_declared_point_limits_including_unknown_strokes() {
    for unknown in [false, true] {
        let (mut bytes, _, source) = file(26, &[], 2);
        if unknown {
            // Last part of the frame is stroke header + three 12-byte points.
            let kind = bytes.len() - 36 - 16;
            bytes[kind..kind + 4].copy_from_slice(&99u32.to_be_bytes());
        }
        let xcf = Xcf::parse(&bytes).unwrap();
        let scan = xcf.paths(Limits { points: 3, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.records[0].offset, source);
        assert_eq!(scan.stopped.unwrap().reason, StopReason::RecordLimit);
        let exact = xcf.paths(Limits { points: 6, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
        assert!(exact.stopped.is_none());
        assert_eq!(exact.records.len(), 2);
        assert_eq!(exact.records[0].geometry_complete(), !unknown);
    }
}

#[test]
fn cancelled_path_scans_restore_dropped_prefix_and_staging_but_not_work() {
    let (bytes, _, _) = file(26, &[(25, historical(25, "Vectors"))], 2);
    let xcf = Xcf::parse(&bytes).unwrap();
    let mut cancelled = 0;
    for stop in 2..90 {
        let calls = std::cell::Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() < stop
        };
        let mut budget = Budget::new(1 << 20, 1 << 20);
        if matches!(xcf.paths(Limits::default(), &mut budget, &keep), Err(Error::Cancelled)) {
            cancelled += 1;
            assert_eq!(budget.retained_remaining(), 1 << 20, "cancellation at checkpoint {stop}");
            assert!(budget.work_remaining() < 1 << 20);
            let retry = xcf.paths(Limits::default(), &mut budget, &|| true).unwrap();
            assert!(retry.stopped.is_none());
            assert_eq!(retry.records.len(), 3);
        }
    }
    assert!(cancelled > 10);
}

#[test]
fn standalone_visibility_preserves_both_saved_states() {
    for visible in [0, 1] {
        let mut payload = W { out: Vec::new(), wide: false };
        payload.prop_u32(8, visible);
        payload.end();
        payload.u32(1);
        payload.u32(1);
        cubic(&mut payload);
        let mut source = W { out: Vec::new(), wide: false };
        source.string("path");
        source.u32(payload.out.len() as u32);
        source.out.extend(payload.out);
        let scan = super::super::standalone(&source.out, 0, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(scan.stopped.is_none());
        assert_eq!(scan.records[0].visible, visible);
        assert_eq!(scan.records[0].properties[0].payload, visible.to_be_bytes());
        assert_eq!(scan.records[0].strokes[0].axes, 2);
    }
}

#[test]
fn standalone_table_has_one_stroke_allowance_including_unknown_frames() {
    for unknown in [false, true] {
        let (mut bytes, _, _) = file(26, &[], 2);
        if unknown {
            let kind = bytes.len() - 36 - 16;
            bytes[kind..kind + 4].copy_from_slice(&99u32.to_be_bytes());
        }
        let xcf = Xcf::parse(&bytes).unwrap();
        for strokes in [0, 1, 2] {
            let scan = xcf.paths(Limits { strokes, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
            assert_eq!(scan.records.len(), strokes);
            assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), (strokes < 2).then_some(StopReason::RecordLimit));
            if strokes == 1 {
                assert_eq!(scan.records[0].strokes.len(), 1);
                assert_eq!(scan.stopped.unwrap().ordinal, 1);
            }
        }
    }
}

#[test]
fn standalone_parasite_accessor_keeps_duplicate_and_empty_property_payloads() {
    let mut parasite = W { out: Vec::new(), wide: false };
    parasite.string("same");
    parasite.u32(1);
    parasite.u32(5);
    parasite.out.extend(b"first");
    let first = parasite.out.clone();
    let last = parasite.out.len() - 5;
    parasite.out[last..].copy_from_slice(b"final");
    let mut payload = W { out: Vec::new(), wide: false };
    payload.prop(21, &first);
    payload.prop_u32(8, 1);
    payload.prop(21, &parasite.out);
    payload.prop(21, &[]);
    payload.end();
    payload.u32(1);
    payload.u32(0);
    let mut source = W { out: Vec::new(), wide: false };
    source.string("path");
    source.u32(payload.out.len() as u32);
    source.out.extend(payload.out);
    let scan = super::super::standalone(&source.out, 0, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(scan.stopped.is_none());
    let path = &scan.records[0];
    let payloads = path.parasite_payloads().collect::<Vec<_>>();
    assert_eq!(payloads, [first.as_slice(), parasite.out.as_slice(), &[]]);
    assert!(path.parasites.is_empty(), "the legacy public field remains compatible");
    let original = path.properties.iter().filter(|property| property.kind == 21).collect::<Vec<_>>();
    for (bytes, property) in payloads.iter().zip(original) {
        assert_eq!(bytes.as_ptr(), property.payload.as_ptr());
        let decoded = super::super::super::parasites(bytes, property.offset + 8, Default::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(decoded.stopped.is_none());
        if !bytes.is_empty() {
            assert_eq!(decoded.records[0].name, b"same");
        }
    }
}

#[test]
fn standalone_pointers_cannot_target_previous_data_or_their_own_slot() {
    for second in [false, true] {
        let (original, table, source) = file(26, &[], 2);
        let slot = table + usize::from(second) * 8;
        for target in [1, table, slot, slot + 7] {
            let mut bytes = original.clone();
            bytes[slot..slot + 8].copy_from_slice(&(target as u64).to_be_bytes());
            let xcf = Xcf::parse(&bytes).unwrap();
            let pointers = xcf.path_offsets(super::super::super::Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert_eq!(pointers.records, if second { vec![source] } else { vec![] });
            assert_eq!(pointers.stopped, Some(Diagnostic { offset: slot, ordinal: usize::from(second), reason: StopReason::Malformed }));
            let paths = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert_eq!(paths.records.len(), usize::from(second));
            assert_eq!(paths.stopped, Some(Diagnostic { offset: slot, ordinal: usize::from(second), reason: StopReason::Malformed }));
        }
    }
}

#[test]
fn tight_work_retains_the_first_path_before_charging_a_later_pointer() {
    let (bytes, table, source) = file(26, &[], 2);
    let xcf = Xcf::parse(&bytes).unwrap();
    let mut measure = Budget::default();
    let before = measure.work_remaining();
    let direct = super::super::standalone(&bytes, source, 0, Limits::default(), &mut measure, &|| true).unwrap();
    assert!(direct.stopped.is_none());
    let first_path_work = before - measure.work_remaining();
    let allowance = 8 + 8 + first_path_work; // Image PROP_END, first pointer, target.
    let mut budget = Budget::new(allowance, 1 << 20);
    let scan = xcf.paths(Limits::default(), &mut budget, &|| true).unwrap();
    assert_eq!(scan.records.len(), 1);
    assert_eq!(scan.records[0].name, "Standalone");
    assert_eq!(scan.stopped, Some(Diagnostic { offset: table + 8, ordinal: 1, reason: StopReason::WorkLimit }));
    assert_eq!(budget.work_remaining(), 0);
    let complete = xcf.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(complete.records.len(), 2, "forward aliases remain independently identified");
    assert!(complete.stopped.is_none());
    let mut broken = bytes.clone();
    broken[table + 16..table + 24].copy_from_slice(&((table + 16) as u64).to_be_bytes());
    let broken = Xcf::parse(&broken).unwrap();
    let scan = broken.paths(Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records.len(), 2);
    assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "the final terminator is still checked");
}

use super::*;

fn property(kind: u32, bytes: &[u8]) -> Vec<u8> {
    [kind.to_be_bytes().as_slice(), (bytes.len() as u32).to_be_bytes().as_slice(), bytes].concat()
}

fn properties(bytes: &[u8]) -> Scan<Property<'_>> {
    super::super::properties(bytes, 0, RecordLimits::default(), &mut Budget::default(), &|| true).unwrap()
}

fn pair(x: f32, y: f32) -> Vec<u8> {
    [x.to_be_bytes(), y.to_be_bytes()].concat()
}

fn parasite(name: &[u8], payload: &[u8]) -> Vec<u8> {
    [(name.len() as u32 + 1).to_be_bytes().as_slice(), name, &[0], &7u32.to_be_bytes(), (payload.len() as u32).to_be_bytes().as_slice(), payload]
        .concat()
}

#[test]
fn scalar_metadata_keeps_the_last_valid_duplicate_and_discloses_invalid_values() {
    let bytes = [
        property(19, &pair(72., 96.)),
        property(19, &pair(300., 150.)),
        property(19, &pair(f32::NAN, 1.)),
        property(22, &2u32.to_be_bytes()),
        property(22, &[1, 2, 3]),
        property(0, &[]),
    ]
    .concat();
    let raw = properties(&bytes);
    let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(result.complete());
    let resolution = result.resolution.unwrap();
    assert_eq!(resolution.value, Resolution { horizontal: 300., vertical: 150. });
    assert_eq!(resolution.offset, raw.records[1].offset);
    assert_eq!(result.unit.unwrap().value, 2);
    assert_eq!(result.issues.len(), 2);
    assert!(result.issues.iter().all(|issue| issue.kind == IssueKind::InvalidValue));
}

#[test]
fn source_guides_and_modern_and_legacy_sample_evidence_keep_order_and_raw_axes() {
    let guides = [(-2i32).to_be_bytes().as_slice(), &[1], &7i32.to_be_bytes(), &[2], &3i32.to_be_bytes(), &[9], &[0]].concat();
    let legacy = [18i32.to_be_bytes(), 19i32.to_be_bytes()].concat();
    let modern = [(-1i32).to_be_bytes(), 4i32.to_be_bytes(), 99u32.to_be_bytes(), 17u32.to_be_bytes(), 23u32.to_be_bytes()].concat();
    let bytes = [property(18, &guides), property(27, &legacy), property(39, &modern), property(39, &[1, 2, 3]), property(0, &[])].concat();
    let result = image(&properties(&bytes), Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(result.complete());
    assert_eq!(
        result.guides.iter().map(|record| record.value).collect::<Vec<_>>(),
        [Guide { position: -2, orientation: Orientation::Horizontal }, Guide { position: 7, orientation: Orientation::Vertical },]
    );
    assert!(result.has_modern_sample_points);
    assert_eq!(result.legacy_sample_points[0].value, SamplePoint { x: 18, y: 19, pick_mode: None, reserved: [0; 2] });
    assert_eq!(result.sample_points[0].value, SamplePoint { x: -1, y: 4, pick_mode: Some(99), reserved: [17, 23] });
    assert_eq!(result.issues.iter().filter(|issue| issue.kind == IssueKind::PartialRecord).count(), 2);
}

#[test]
fn malformed_modern_samples_do_not_claim_a_usable_modern_list_but_empty_ones_do() {
    for (modern, expected) in [(&[1, 2][..], false), (&[][..], true)] {
        let bytes = [property(27, &[0; 8]), property(39, modern), property(0, &[])].concat();
        let result = image(&properties(&bytes), Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(result.has_modern_sample_points, expected);
        assert_eq!(result.legacy_sample_points.len(), 1);
        assert!(result.sample_points.is_empty());
    }
}

#[test]
fn custom_units_borrow_both_documented_string_layouts_and_keep_prior_valid_values() {
    for count in [3, 5] {
        let mut unit = [2.54f32.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for value in [b"cm\0".as_slice(), b"cm\0", b"centimeter\0", b"singular\0", b"plural\0"].into_iter().take(count) {
            unit.extend((value.len() as u32).to_be_bytes());
            unit.extend(value);
        }
        let mut invalid = unit.clone();
        invalid[..4].copy_from_slice(&f32::INFINITY.to_be_bytes());
        let bytes = [property(24, &unit), property(24, &invalid), property(0, &[])].concat();
        let records = [(&unit, 0), (&invalid, unit.len() + 8)]
            .into_iter()
            .map(|(payload, offset)| Property { offset, kind: 24, declared_length: payload.len() as u32, payload, raw: payload })
            .collect();
        let raw = Scan { records, stopped: None, end_offset: bytes.len() };
        let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(result.complete());
        let value = result.user_unit.unwrap().value;
        assert_eq!(value.factor, 2.54);
        assert_eq!(value.strings.iter().flatten().count(), count);
        let first = value.strings[0].unwrap();
        assert!(unit.as_ptr_range().contains(&first.as_ptr()));
        assert_eq!(first, b"cm");
        assert_eq!(result.issues.len(), 1);
    }
}

#[test]
fn profile_and_provenance_packets_stay_borrowed_with_source_flags_and_duplicate_order() {
    let xml = b"<metadata><tag name='Xmp.dc.creator'>Original fixture</tag></metadata>";
    let parasites = [
        parasite(b"icc-profile", b"first source bytes"),
        parasite(b"gimp-image-metadata", xml),
        parasite(b"unknown-preserved-by-raw-record", b"opaque"),
        parasite(b"icc-profile", b"last source bytes"),
        parasite(b"icc-profile", b""),
        parasite(b"gimp-comment", b"comment\0"),
    ]
    .concat();
    let bytes = [property(21, &parasites), property(0, &[])].concat();
    let raw = properties(&bytes);
    let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(result.complete());
    let profile = result.profile.unwrap().value;
    assert_eq!(profile, b"last source bytes");
    assert!(bytes.as_ptr_range().contains(&profile.as_ptr()));
    assert_eq!(result.packets.iter().map(|packet| packet.value.kind).collect::<Vec<_>>(), [PacketKind::ImageMetadata, PacketKind::Comment]);
    assert_eq!(result.packets[0].value.bytes, xml);
    assert_eq!(result.packets[0].value.flags, 7);
    assert!(raw.records[0].payload.windows(b"opaque".len()).any(|window| window == b"opaque"));
    assert_eq!(result.issues.len(), 1);
}

#[test]
fn source_and_typed_limits_never_disguise_prefix_values_as_complete() {
    let bytes = [property(19, &pair(72., 96.)), property(18, &[0; 5]), property(0, &[])].concat();
    let raw = properties(&bytes);
    let limits = Limits { issues: 0, ..Limits::default() };
    let result = image(&raw, limits, &mut Budget::default(), &|| true).unwrap();
    assert!(!result.complete());
    assert_eq!(result.stopped.unwrap().reason, StopReason::RecordLimit);
    assert!(result.resolution.is_some());
    let raw = super::super::properties(&bytes, 0, RecordLimits { records: 1, ..RecordLimits::default() }, &mut Budget::default(), &|| true).unwrap();
    let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(!result.complete());
    assert!(result.source_stopped.is_some());
    let raw = properties(&bytes);
    let mut spent = Budget::new(0, 0);
    let result = image(&raw, Limits::default(), &mut spent, &|| true).unwrap();
    assert_eq!(result.stopped.unwrap().reason, StopReason::WorkLimit);
    assert!(result.resolution.is_none());
}

#[test]
fn repeated_typed_reads_share_work_and_retained_lists_stop_before_unbudgeted_growth() {
    let guides = [0, 0, 0, 1, 1, 0, 0, 0, 2, 2];
    let bytes = [property(18, &guides), property(0, &[])].concat();
    let raw = properties(&bytes);
    let mut work = Budget::new(8 + guides.len(), crate::MAX_METADATA_BYTES);
    let first = image(&raw, Limits::default(), &mut work, &|| true).unwrap();
    assert!(first.complete());
    assert_eq!(first.guides.len(), 2);
    assert_eq!(work.work_remaining(), 0);
    let again = image(&raw, Limits::default(), &mut work, &|| true).unwrap();
    assert!(!again.complete());
    assert!(again.guides.is_empty());
    assert_eq!(again.stopped.unwrap().reason, StopReason::WorkLimit);
    let one_slot = 2 * std::mem::size_of::<Evidence<Guide>>();
    let result = image(&raw, Limits::default(), &mut Budget::new(1024, one_slot), &|| true).unwrap();
    assert_eq!(result.guides.len(), 1);
    assert_eq!(result.stopped.unwrap().reason, StopReason::RetainedLimit);
    let result = image(&raw, Limits { guides: 1, ..Limits::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(result.guides.len(), 1);
    assert_eq!(result.stopped.unwrap().reason, StopReason::RecordLimit);
}

#[test]
fn malformed_parasite_suffix_keeps_borrowed_prefix_but_cannot_establish_final_profile() {
    let payload = [parasite(b"icc-profile", b"borrowed prefix"), vec![0, 0, 0]].concat();
    let bytes = [property(21, &payload), property(0, &[])].concat();
    let raw = properties(&bytes);
    let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(!result.complete());
    assert_eq!(result.profile.unwrap().value, b"borrowed prefix");
    assert_eq!(result.stopped.unwrap().reason, StopReason::Malformed);
}

#[test]
fn selected_layer_and_mask_evidence_never_infers_native_identity_or_drops_valid_markers() {
    let bytes = [
        property(2, &[]),
        property(2, &[0]),
        property(12, &1u32.to_be_bytes()),
        property(13, &9u32.to_be_bytes()),
        property(13, &[0]),
        property(0, &[]),
    ]
    .concat();
    let result = layer(&properties(&bytes), Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(result.complete());
    assert!(result.selected.is_some());
    assert_eq!(result.edit_mask.unwrap().value, 1);
    assert_eq!(result.show_mask.unwrap().value, 9);
    assert_eq!(result.issues.len(), 2);
}

#[test]
fn cancellation_precedes_empty_spent_and_mid_record_metadata_reads() {
    let empty = property(0, &[]);
    let raw = properties(&empty);
    assert!(matches!(image(&raw, Limits::default(), &mut Budget::new(0, 0), &|| false), Err(Error::Cancelled)));
    assert!(matches!(layer(&raw, Limits::default(), &mut Budget::new(0, 0), &|| false), Err(Error::Cancelled)));
    let bytes = [property(18, &[0, 0, 0, 1, 1].repeat(20)), property(0, &[])].concat();
    let raw = properties(&bytes);
    let calls = std::cell::Cell::new(0);
    let keep_going = || {
        calls.set(calls.get() + 1);
        calls.get() < 8
    };
    assert!(matches!(image(&raw, Limits::default(), &mut Budget::default(), &keep_going), Err(Error::Cancelled)));
}

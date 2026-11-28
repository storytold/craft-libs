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

#[test]
fn versioned_xcf_user_units_do_not_confuse_short_historical_names_with_property_headers() {
    // The public XCF format removes the two grammatical names in version21.
    // These are original files made by our fixture writer, including deliberately
    // wrong PROP_USER_UNIT length words (a documented writer compatibility case).
    use crate::tests::TestImage;
    for version in [0, 10, 11, 20, 21, 26] {
        let count = if version < 21 { 5 } else { 3 };
        let mut unit = [2.54f32.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for value in [b"cm\0".as_slice(), b"cm\0", b"cm\0", b"c\0", b"cs\0"].into_iter().take(count) {
            unit.extend((value.len() as u32).to_be_bytes());
            unit.extend(value);
        }
        let mut fixture = TestImage::rgb8(1, 1);
        fixture.version = version;
        fixture.image_props = vec![(24, unit.clone()), (19, pair(123., 234.))];
        let mut bytes = fixture.write();
        // Compression's one-byte property is the writer's first property.
        let unit_header = 9 + 5 + 12 + usize::from(version >= 4) * 4 + 9;
        bytes[unit_header + 4..unit_header + 8].copy_from_slice(&1u32.to_be_bytes());
        let parsed = crate::Xcf::parse(&bytes).unwrap_or_else(|error| panic!("version {version}: {error}"));
        assert_eq!(parsed.image().resolution, Some((123., 234.)), "version {version}");
        let raw = parsed.image_properties(RecordLimits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(raw.stopped.is_none(), "version {version}: {:?}", raw.stopped);
        let property = raw.records.iter().find(|property| property.kind == 24).unwrap();
        assert_eq!(property.payload, unit, "version {version}");
        assert_eq!(property.declared_length, 1);
        let typed = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(typed.complete());
        assert_eq!(typed.user_unit.unwrap().value.strings.iter().flatten().count(), count);
        assert_eq!(typed.resolution.unwrap().value, Resolution { horizontal: 123., vertical: 234. });
        let start = 9 + 5 + 12 + usize::from(version >= 4) * 4;
        let direct =
            super::super::properties_with_version(&bytes, start, version, RecordLimits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(direct.stopped.is_none());
        assert_eq!(direct.records.iter().find(|property| property.kind == 24).unwrap().payload, unit);
    }
}

#[test]
fn truncated_versioned_units_disclose_a_stopped_raw_scan_and_still_prioritize_cancellation() {
    let mut unit = [1f32.to_be_bytes(), 2u32.to_be_bytes()].concat();
    for _ in 0..3 {
        unit.extend(2u32.to_be_bytes());
        unit.extend(b"u\0");
    }
    unit.extend(64u32.to_be_bytes());
    unit.push(b'x');
    let bytes = property(24, &unit);
    let raw = super::super::properties_with_version(&bytes, 0, 20, RecordLimits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(raw.records.is_empty());
    assert_eq!(raw.stopped.unwrap().reason, StopReason::Malformed);
    assert!(matches!(
        super::super::properties_with_version(&bytes, 0, 20, RecordLimits::default(), &mut Budget::new(0, 0), &|| false),
        Err(Error::Cancelled)
    ));
}

#[test]
fn complete_sample_points_survive_a_tail_when_issues_cannot_be_retained() {
    for (kind, stride) in [(27, 8), (39, 20)] {
        let mut payload = vec![0; stride];
        payload.push(1);
        let bytes = [property(kind, &payload), property(0, &[])].concat();
        let output = image(&properties(&bytes), Limits { issues: 0, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
        let points = if kind == 39 { &output.sample_points } else { &output.legacy_sample_points };
        assert_eq!(points.len(), 1);
        assert_eq!(output.stopped.unwrap().reason, StopReason::RecordLimit);
        assert_eq!(output.has_modern_sample_points, kind == 39);
    }
}

#[test]
fn temporary_parasite_records_do_not_starve_later_retained_metadata() {
    let bytes = [property(21, &parasite(b"icc-profile", b"source profile")), property(18, &[0, 0, 0, 7, 1]), property(0, &[])].concat();
    let raw = properties(&bytes);
    let allowance = 2 * std::mem::size_of::<super::super::Parasite<'_>>();
    let mut budget = Budget::new(4096, allowance);
    let output = image(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(output.complete());
    assert_eq!(output.profile.unwrap().value, b"source profile");
    assert_eq!(output.guides.len(), 1);
    assert_eq!(budget.retained_remaining(), allowance - 2 * std::mem::size_of::<Evidence<Guide>>());
    assert!(budget.work_remaining() < 4096);
    // Returned packets stay charged even though the temporary vector is freed.
    let bytes = [property(21, &parasite(b"gimp-comment", b"comment")), property(0, &[])].concat();
    let mut budget = Budget::new(4096, 4096);
    let output = image(&properties(&bytes), Limits::default(), &mut budget, &|| true).unwrap();
    assert_eq!(output.packets.len(), 1);
    assert_eq!(budget.retained_remaining(), 4096 - 2 * std::mem::size_of::<Evidence<Packet<'_>>>());
}

#[test]
fn cancelled_image_metadata_refunds_dropped_guides_but_keeps_attempted_work() {
    let bytes = [property(18, &[0, 0, 0, 7, 1]), property(0, &[])].concat();
    let raw = properties(&bytes);
    let calls = std::cell::Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        true
    };
    let probe = image(&raw, Limits::default(), &mut Budget::default(), &count).unwrap();
    assert_eq!(probe.guides.len(), 1);
    let final_check = calls.get();
    let retained = 2 * std::mem::size_of::<Evidence<Guide>>();
    let mut budget = Budget::new(1024, retained);
    calls.set(0);
    let cancel_at_finish = || {
        calls.set(calls.get() + 1);
        calls.get() < final_check
    };
    assert!(matches!(image(&raw, Limits::default(), &mut budget, &cancel_at_finish), Err(Error::Cancelled)));
    assert_eq!(budget.retained_remaining(), retained);
    let work_after_cancel = budget.work_remaining();
    assert!(work_after_cancel < 1024);
    let retry = image(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(retry.complete());
    assert_eq!(retry.guides.len(), 1);
    assert_eq!(retry.guides[0].value.position, 7);
    assert_eq!(budget.retained_remaining(), 0);
    assert!(budget.work_remaining() < work_after_cancel);
    let another = image(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(another.guides.is_empty());
    assert_eq!(another.stopped.unwrap().reason, StopReason::RetainedLimit);
}

#[test]
fn cancelled_layer_metadata_refunds_dropped_issues_but_not_returned_prefixes() {
    let bytes = [property(2, &[1]), property(0, &[])].concat();
    let raw = properties(&bytes);
    let calls = std::cell::Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        true
    };
    assert_eq!(layer(&raw, Limits::default(), &mut Budget::default(), &count).unwrap().issues.len(), 1);
    let final_check = calls.get();
    let retained = 2 * std::mem::size_of::<Issue>();
    let mut budget = Budget::new(1024, retained);
    calls.set(0);
    let cancel_at_finish = || {
        calls.set(calls.get() + 1);
        calls.get() < final_check
    };
    assert!(matches!(layer(&raw, Limits::default(), &mut budget, &cancel_at_finish), Err(Error::Cancelled)));
    assert_eq!(budget.retained_remaining(), retained);
    let work_after_cancel = budget.work_remaining();
    assert!(work_after_cancel < 1024);
    let retry = layer(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(retry.complete());
    assert_eq!(retry.issues.len(), 1);
    assert_eq!(retry.issues[0].kind, IssueKind::InvalidValue);
    assert_eq!(budget.retained_remaining(), 0);
    assert!(budget.work_remaining() < work_after_cancel);
    let another = layer(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(another.issues.is_empty());
    assert_eq!(another.stopped.unwrap().reason, StopReason::RetainedLimit);
}

#[test]
fn a_later_valid_user_unit_beyond_the_typed_string_cap_is_incomplete_prefix_evidence() {
    let unit = |name: &[u8]| {
        let mut bytes = [1f32.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for value in [name, b"x\0", b"y\0"] {
            bytes.extend((value.len() as u32).to_be_bytes());
            bytes.extend(value);
        }
        bytes
    };
    let first = unit(b"a\0");
    let later = unit(b"lengthy\0");
    let bytes = [property(24, &first), property(24, &later), property(0, &[])].concat();
    let raw = properties(&bytes);
    assert!(raw.stopped.is_none());
    let limits = Limits { records: RecordLimits { string_bytes: 4, ..Default::default() }, ..Default::default() };
    let result = image(&raw, limits, &mut Budget::default(), &|| true).unwrap();
    assert!(!result.complete());
    assert_eq!(result.stopped, Some(Diagnostic { offset: raw.records[1].offset, ordinal: 1, reason: StopReason::StringLimit }));
    assert_eq!(result.user_unit.unwrap().value.strings[0], Some(b"a".as_slice()));
    assert!(result.issues.is_empty());
    let complete = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(complete.complete());
    assert_eq!(complete.user_unit.unwrap().value.strings[0], Some(b"lengthy".as_slice()));

    // A malformed value within the allowance remains an ordinary invalid-value issue.
    let mut invalid = first.clone();
    invalid[..4].copy_from_slice(&f32::NAN.to_be_bytes());
    let bytes = [property(24, &first), property(24, &invalid), property(0, &[])].concat();
    let result = image(&properties(&bytes), limits, &mut Budget::default(), &|| true).unwrap();
    assert!(result.complete());
    assert_eq!(result.issues.len(), 1);
    assert_eq!(result.issues[0].kind, IssueKind::InvalidValue);
    assert_eq!(result.user_unit.unwrap().value.strings[0], Some(b"a".as_slice()));
}

#[test]
fn a_nested_parasite_stop_keeps_both_property_and_parasite_source_locations() {
    let first = property(22, &1u32.to_be_bytes());
    let ignored = parasite(b"unknown", b"ignored");
    let profile = parasite(b"icc-profile", b"valid prefix");
    let payload = [ignored.clone(), profile.clone(), vec![0, 0, 0]].concat();
    let bytes = [first.clone(), property(21, &payload), property(0, &[])].concat();
    let raw = properties(&bytes);
    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    let work = budget.work_remaining();
    let result = image(&raw, Limits::default(), &mut budget, &|| true).unwrap();
    assert!(!result.complete());
    assert_eq!(result.source_stopped, None);
    assert_eq!(result.stopped, Some(Diagnostic { offset: first.len(), ordinal: 1, reason: StopReason::Malformed }));
    assert_eq!(
        result.parasite_stopped,
        Some(Diagnostic { offset: first.len() + 8 + ignored.len() + profile.len(), ordinal: 2, reason: StopReason::Malformed })
    );
    assert_eq!(result.profile.unwrap(), Evidence { offset: first.len() + 8 + ignored.len(), value: b"valid prefix".as_slice() });
    assert!(result.issues.is_empty());
    assert_eq!(budget.retained_remaining(), retained);
    assert!(budget.work_remaining() < work);
}

#[test]
fn unterminated_user_unit_fields_do_not_replace_prior_valid_evidence() {
    let unit = |fields: &[&[u8]], factor: f32| {
        let mut bytes = [factor.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for field in fields {
            bytes.extend((field.len() as u32).to_be_bytes());
            bytes.extend(*field);
        }
        bytes
    };
    for count in [3, 5] {
        let fields = vec![b"ok\0".as_slice(); count];
        let first = unit(&fields, 2.);
        for bad in 0..count {
            let mut later = fields.clone();
            later[bad] = b"mm";
            let bytes = [property(24, &first), property(24, &unit(&later, 3.)), property(0, &[])].concat();
            let raw = super::super::properties_with_version(
                &bytes,
                0,
                if count == 5 { 20 } else { 26 },
                RecordLimits::default(),
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            assert!(raw.stopped.is_none());
            let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert!(result.complete());
            assert_eq!(result.user_unit.unwrap().value.factor, 2.);
            assert_eq!(result.issues, [Issue { offset: raw.records[1].offset, property_kind: 24, kind: IssueKind::InvalidValue }]);
        }
        // Both the zero-length spelling and a sole NUL remain valid empty STRINGs.
        let empty = (0..count).map(|i| if i % 2 == 0 { b"".as_slice() } else { b"\0".as_slice() }).collect::<Vec<_>>();
        let bytes = [property(24, &unit(&empty, 4.)), property(0, &[])].concat();
        let raw = super::super::properties_with_version(
            &bytes,
            0,
            if count == 5 { 20 } else { 26 },
            RecordLimits::default(),
            &mut Budget::default(),
            &|| true,
        )
        .unwrap();
        let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(result.complete());
        assert!(result.issues.is_empty());
        let value = result.user_unit.unwrap().value;
        assert_eq!(value.factor, 4.);
        assert_eq!(value.strings.iter().flatten().count(), count);
        assert!(value.strings.iter().flatten().all(|value| value.is_empty()));
    }
}

#[test]
fn typed_parasite_limits_keep_the_earlier_location_and_later_raw_stop() {
    let first = property(22, &1u32.to_be_bytes());
    let profile = parasite(b"icc-profile", b"kept profile");
    let comment = parasite(b"gimp-comment", b"kept comment");
    let exif = parasite(b"exif-data", b"omitted exif");
    for malformed in [false, true] {
        let suffix = if malformed { vec![0, 0, 0] } else { Vec::new() };
        let payload = [profile.clone(), comment.clone(), exif.clone(), suffix].concat();
        let bytes = [first.clone(), property(21, &payload), property(0, &[])].concat();
        let raw = properties(&bytes);
        let limits = Limits { packets: 1, ..Default::default() };
        let result = image(&raw, limits, &mut Budget::default(), &|| true).unwrap();
        let base = first.len() + 8;
        assert!(!result.complete());
        assert_eq!(result.stopped, Some(Diagnostic { offset: first.len(), ordinal: 1, reason: StopReason::RecordLimit }));
        assert_eq!(
            result.parasite_typed_stopped,
            Some(Diagnostic { offset: base + profile.len() + comment.len(), ordinal: 2, reason: StopReason::RecordLimit })
        );
        let raw_stop =
            malformed.then_some(Diagnostic { offset: base + profile.len() + comment.len() + exif.len(), ordinal: 3, reason: StopReason::Malformed });
        assert_eq!(result.parasite_stopped, raw_stop);
        assert_eq!(result.profile.unwrap().value, b"kept profile");
        assert_eq!(result.packets.len(), 1);
        assert_eq!(result.packets[0].value.bytes, b"kept comment");
        let all = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(all.parasite_typed_stopped, None);
        assert_eq!(all.parasite_stopped, raw_stop);
        assert_eq!(all.packets.len(), 2);
        assert_eq!(all.complete(), !malformed);
    }
}

#[test]
fn an_issue_limit_names_the_parasite_that_typed_conversion_cannot_retain() {
    let first = parasite(b"icc-profile", b"kept");
    let invalid = parasite(b"icc-profile", b"");
    let bytes = [property(21, &[first.clone(), invalid].concat()), property(0, &[])].concat();
    let result = image(&properties(&bytes), Limits { issues: 0, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(result.profile.unwrap().value, b"kept");
    assert!(result.issues.is_empty());
    assert_eq!(result.parasite_stopped, None);
    assert_eq!(result.parasite_typed_stopped, Some(Diagnostic { offset: 8 + first.len(), ordinal: 1, reason: StopReason::RecordLimit }));
    assert_eq!(result.stopped.unwrap().offset, 0);
    assert!(!result.complete());
}

#[test]
fn invalid_utf8_user_unit_fields_do_not_replace_valid_borrowed_evidence() {
    let unit = |fields: &[&[u8]], factor: f32| {
        let mut bytes = [factor.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for field in fields {
            bytes.extend((field.len() as u32).to_be_bytes());
            bytes.extend(*field);
        }
        bytes
    };
    for count in [3, 5] {
        let fields = vec!["微米\0".as_bytes(); count];
        let first = unit(&fields, 2.);
        for bad in 0..count {
            let mut later = fields.clone();
            later[bad] = &[0xff, 0];
            let invalid = unit(&later, 3.);
            let bytes = [property(24, &first), property(24, &invalid), property(0, &[])].concat();
            let raw = super::super::properties_with_version(
                &bytes,
                0,
                if count == 5 { 20 } else { 26 },
                RecordLimits::default(),
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            assert!(raw.stopped.is_none());
            assert_eq!(raw.records[1].payload, invalid, "raw evidence remains opaque and unmodified");
            let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert!(result.complete());
            let accepted = result.user_unit.unwrap().value;
            assert_eq!(accepted.factor, 2.);
            assert_eq!(accepted.strings[0], Some("微米".as_bytes()));
            assert!(raw.records[0].payload.as_ptr_range().contains(&accepted.strings[0].unwrap().as_ptr()));
            assert_eq!(result.issues, [Issue { offset: raw.records[1].offset, property_kind: 24, kind: IssueKind::InvalidValue }]);
        }
    }
}

#[test]
fn interior_nuls_in_user_unit_fields_keep_prior_values_and_opaque_raw_evidence() {
    let unit = |fields: &[&[u8]], factor: f32| {
        let mut bytes = [factor.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for field in fields {
            bytes.extend((field.len() as u32).to_be_bytes());
            bytes.extend(*field);
        }
        bytes
    };
    for count in [3, 5] {
        let fields = vec![b"cm\0".as_slice(); count];
        let first = unit(&fields, 2.);
        for bad in 0..count {
            let mut later = fields.clone();
            later[bad] = b"mm\0extra\0";
            let invalid = unit(&later, 3.);
            let bytes = [property(24, &first), property(24, &invalid), property(0, &[])].concat();
            let raw = super::super::properties_with_version(
                &bytes,
                0,
                if count == 5 { 20 } else { 26 },
                RecordLimits::default(),
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            assert!(raw.stopped.is_none());
            assert_eq!(raw.records[1].payload, invalid);
            let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert!(result.complete());
            assert_eq!(result.user_unit.unwrap().value.factor, 2.);
            assert_eq!(result.issues, [Issue { offset: raw.records[1].offset, property_kind: 24, kind: IssueKind::InvalidValue }]);
        }
    }
}

#[test]
fn a_user_unit_field_limit_precedes_utf8_and_nul_validation() {
    let unit = |fields: &[&[u8]], factor: f32| {
        let mut bytes = [factor.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for field in fields {
            bytes.extend((field.len() as u32).to_be_bytes());
            bytes.extend(*field);
        }
        bytes
    };
    let mut utf8 = vec![b'a'; 8192];
    utf8[8190] = 0xff;
    utf8[8191] = 0;
    let unterminated = vec![b'a'; 8192];
    let mut interior = vec![b'a'; 8192];
    interior[1] = 0;
    interior[8191] = 0;
    for count in [3, 5] {
        let fields = vec![b"a\0".as_slice(); count];
        let first = unit(&fields, 2.);
        for malformed in [&utf8, &unterminated, &interior] {
            for index in 0..count {
                let mut later = fields.clone();
                later[index] = malformed;
                let invalid = unit(&later, 3.);
                let bytes = [property(24, &first), property(24, &invalid), property(0, &[])].concat();
                let raw = super::super::properties_with_version(
                    &bytes,
                    0,
                    if count == 5 { 20 } else { 26 },
                    RecordLimits::default(),
                    &mut Budget::default(),
                    &|| true,
                )
                .unwrap();
                assert!(raw.stopped.is_none());
                assert_eq!(raw.records[1].payload, invalid);
                let limits = Limits { records: RecordLimits { string_bytes: 4, ..Default::default() }, ..Default::default() };
                let result = image(&raw, limits, &mut Budget::default(), &|| true).unwrap();
                assert!(!result.complete());
                assert_eq!(result.stopped, Some(Diagnostic { offset: raw.records[1].offset, ordinal: 1, reason: StopReason::StringLimit }));
                assert_eq!(result.user_unit.unwrap().value.factor, 2.);
                assert!(result.issues.is_empty());
                // Once the same bytes fit, their malformed spelling is an InvalidValue,
                // not a quota stop, and still cannot replace prior valid evidence.
                let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
                assert!(result.complete());
                assert_eq!(result.user_unit.unwrap().value.factor, 2.);
                assert_eq!(result.issues[0].kind, IssueKind::InvalidValue);
            }
        }
    }
}

/// A STRING field exactly as stored, including any intentionally absent terminator.
fn parasite_with_wire_name(name: &[u8], payload: &[u8]) -> Vec<u8> {
    [(name.len() as u32).to_be_bytes().as_slice(), name, &7u32.to_be_bytes(), (payload.len() as u32).to_be_bytes().as_slice(), payload].concat()
}

#[test]
fn unterminated_parasite_names_cannot_replace_recognized_typed_evidence() {
    let profile = parasite(b"icc-profile", b"kept profile");
    let comment = parasite(b"gimp-comment", b"kept comment");
    for name in [b"icc-profile".as_slice(), b"gimp-comment", b"exif-data", b"gimp-image-metadata", b"gimp-metadata"] {
        let invalid = parasite_with_wire_name(name, b"must stay raw");
        let payload = [profile.clone(), comment.clone(), invalid.clone()].concat();
        let bytes = [property(21, &payload), property(0, &[])].concat();
        let raw = properties(&bytes);
        let nested = super::super::parasites(&payload, 8, RecordLimits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(nested.stopped.is_none());
        assert_eq!(nested.records[2].name, name);
        assert_eq!(nested.records[2].raw, invalid);
        let result = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(result.complete());
        assert_eq!(result.parasite_stopped, None);
        assert_eq!(result.parasite_typed_stopped, None);
        assert_eq!(result.profile.unwrap().value, b"kept profile");
        assert_eq!(result.packets.len(), 1);
        assert_eq!(result.packets[0].value.bytes, b"kept comment");
        assert_eq!(result.issues, [Issue { offset: 0, property_kind: 21, kind: IssueKind::InvalidValue }]);
    }
}

#[test]
fn parasite_name_framing_keeps_empty_and_opaque_names_without_utf8_conversion() {
    let payload = [
        parasite_with_wire_name(b"", b"zero-length name"),
        parasite_with_wire_name(b"\0", b"empty terminated name"),
        parasite_with_wire_name(&[0xff, 0], b"opaque name"),
        parasite(b"icc-profile", b"first profile"),
        parasite(b"icc-profile", b"last valid profile"),
    ]
    .concat();
    let bytes = [property(21, &payload), property(0, &[])].concat();
    let result = image(&properties(&bytes), Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(result.complete());
    assert!(result.issues.is_empty());
    assert_eq!(result.profile.unwrap().value, b"last valid profile");
    let raw = super::super::parasites(&payload, 8, RecordLimits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(raw.records[2].name, &[0xff]);
    assert_eq!(raw.records[2].raw, parasite_with_wire_name(&[0xff, 0], b"opaque name"));
}

#[test]
fn an_unretained_name_issue_preserves_separate_typed_and_raw_parasite_stops() {
    let valid = parasite(b"icc-profile", b"kept");
    let invalid = parasite_with_wire_name(b"icc-profile", b"not a replacement");
    let payload = [valid.clone(), invalid.clone(), vec![0, 0, 0]].concat();
    let bytes = [property(21, &payload), property(0, &[])].concat();
    let result = image(&properties(&bytes), Limits { issues: 0, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
    assert!(!result.complete());
    assert_eq!(result.profile.unwrap().value, b"kept");
    assert!(result.issues.is_empty());
    assert_eq!(result.stopped, Some(Diagnostic { offset: 0, ordinal: 0, reason: StopReason::RecordLimit }));
    assert_eq!(result.parasite_typed_stopped, Some(Diagnostic { offset: 8 + valid.len(), ordinal: 1, reason: StopReason::RecordLimit }));
    assert_eq!(result.parasite_stopped, Some(Diagnostic { offset: 8 + valid.len() + invalid.len(), ordinal: 2, reason: StopReason::Malformed }));
}

#[test]
fn finite_positive_resolutions_are_source_evidence_without_native_bounds() {
    for (x, y) in [(1_048_577., 72.), (0.0001, f32::MAX), (f32::from_bits(1), 72.)] {
        let bytes = [property(19, &pair(x, y)), property(0, &[])].concat();
        let output = image(&properties(&bytes), Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(output.complete());
        assert!(output.issues.is_empty());
        assert_eq!(output.resolution.unwrap().value, Resolution { horizontal: x, vertical: y });
    }
    for invalid in [0., -0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for (x, y) in [(invalid, 72.), (72., invalid)] {
            let bytes = [property(19, &pair(300., 150.)), property(19, &pair(x, y)), property(0, &[])].concat();
            let output = image(&properties(&bytes), Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert_eq!(output.resolution.unwrap().value, Resolution { horizontal: 300., vertical: 150. });
            assert_eq!(output.issues.len(), 1);
        }
    }
}

#[test]
fn all_reachable_unit_lengths_precede_early_semantic_rejection() {
    let unit = |factor: f32, fields: &[&[u8]]| {
        let mut bytes = [factor.to_be_bytes(), 2u32.to_be_bytes()].concat();
        for value in fields {
            bytes.extend((value.len() as u32).to_be_bytes());
            bytes.extend(*value);
        }
        bytes
    };
    for count in [3, 5] {
        let normal = vec![b"ok\0".as_slice(); count];
        for case in 0..5 {
            let mut later = normal.clone();
            let factor = if case < 2 { f32::NAN } else { 3. };
            if case == 2 {
                later[0] = &[0xff, 0];
            }
            if case == 3 {
                later[0] = b"bad";
            }
            if case == 4 {
                later[0] = b"x\0y\0";
            }
            let oversized = if case == 0 { 0 } else { count - 1 };
            later[oversized] = b"lengthy\0";
            let bytes = [property(24, &unit(2., &normal)), property(24, &unit(factor, &later)), property(0, &[])].concat();
            let raw = super::super::properties_with_version(
                &bytes,
                0,
                if count == 3 { 26 } else { 20 },
                RecordLimits::default(),
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            let output = image(
                &raw,
                Limits { records: RecordLimits { string_bytes: 4, ..Default::default() }, ..Default::default() },
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            assert_eq!(output.user_unit.unwrap().value.factor, 2.);
            assert!(output.issues.is_empty());
            assert_eq!(output.stopped, Some(Diagnostic { offset: raw.records[1].offset, ordinal: 1, reason: StopReason::StringLimit }));
            later[oversized] = normal[oversized];
            let bytes = [property(24, &unit(2., &normal)), property(24, &unit(factor, &later)), property(0, &[])].concat();
            let raw = super::super::properties_with_version(
                &bytes,
                0,
                if count == 3 { 26 } else { 20 },
                RecordLimits::default(),
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            let output = image(
                &raw,
                Limits { records: RecordLimits { string_bytes: 4, ..Default::default() }, ..Default::default() },
                &mut Budget::default(),
                &|| true,
            )
            .unwrap();
            assert!(output.complete());
            assert_eq!(output.issues.len(), 1);
            assert_eq!(output.issues[0].kind, IssueKind::InvalidValue);
        }
    }
}

#[test]
fn guide_and_sample_evidence_identifies_each_wire_record_not_its_property() {
    let guides = [1i32.to_be_bytes().as_slice(), &[99], &2i32.to_be_bytes(), &[1], &3i32.to_be_bytes(), &[2], &[0]].concat();
    let legacy = [4i32.to_be_bytes(), 5i32.to_be_bytes(), 6i32.to_be_bytes(), 7i32.to_be_bytes()].concat();
    let modern = [
        8i32.to_be_bytes(),
        9i32.to_be_bytes(),
        10u32.to_be_bytes(),
        11u32.to_be_bytes(),
        12u32.to_be_bytes(),
        13i32.to_be_bytes(),
        14i32.to_be_bytes(),
        15u32.to_be_bytes(),
        16u32.to_be_bytes(),
        17u32.to_be_bytes(),
    ]
    .concat();
    let bytes = [property(100, &[1, 2, 3]), property(18, &guides), property(27, &legacy), property(39, &modern), property(0, &[])].concat();
    let raw = properties(&bytes);
    let output = image(&raw, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(output.complete());
    assert_eq!(output.guides.len(), 2, "both valid guide records must reach the offset assertions");
    for (record, index) in output.guides.iter().zip([1, 2]) {
        assert_eq!(record.offset, raw.records[1].offset + 8 + index * 5);
        assert_eq!(&bytes[record.offset..record.offset + 4], &record.value.position.to_be_bytes());
    }
    for (records, property, stride) in [(&output.legacy_sample_points, &raw.records[2], 8), (&output.sample_points, &raw.records[3], 20)] {
        assert_eq!(records.len(), 2);
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record.offset, property.offset + 8 + index * stride);
            assert_eq!(&bytes[record.offset..record.offset + 4], &record.value.x.to_be_bytes());
            assert_eq!(&bytes[record.offset + 4..record.offset + 8], &record.value.y.to_be_bytes());
        }
    }
    assert_eq!(output.issues.len(), 2);
    assert!(output.issues.iter().all(|issue| issue.offset == raw.records[1].offset));
}

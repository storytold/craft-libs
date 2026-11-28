use super::*;
use crate::Compression;
use crate::effects::{Budget, Owner, OwnerKind, Reader};
use crate::tests::{W, hierarchy};

struct Fixture {
    version: u32,
    effects_slot: Option<usize>,
    bytes: Vec<u8>,
    effect: usize,
    mask: usize,
    hierarchy: usize,
    mask_slot: usize,
}
fn fixture(code: u32, data: &[u8], sample_bytes: usize, present: bool) -> Fixture {
    fixture_named(code, data, sample_bytes, present, b"Coverage\0", true)
}
fn fixture_named(code: u32, data: &[u8], sample_bytes: usize, present: bool, wire_name: &[u8], properties: bool) -> Fixture {
    fixture_framed(code, data, sample_bytes, present, wire_name, properties, (22, &[]))
}
fn fixture_framed(code: u32, data: &[u8], sample_bytes: usize, present: bool, wire_name: &[u8], properties: bool, framing: (u32, &[u64])) -> Fixture {
    let (version, effects) = framing;
    let mut w = W::new(version);
    for value in [2, 1, 1, code] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    for _ in 0..3 {
        w.ptr(0);
    }
    let effect = w.out.len();
    for text in ["Effect", "", "gegl:exposure", "1:0"] {
        w.string(text);
    }
    w.end();
    let mask_slot = w.slot();
    w.out.extend_from_slice(&[39, 87]); // A tile can end exactly at the channel header.
    let mask = w.out.len();
    if present {
        w.patch(mask_slot, mask);
    }
    for value in [2, 1] {
        w.u32(value);
    }
    w.u32(wire_name.len().try_into().unwrap());
    w.out.extend_from_slice(wire_name);
    if properties {
        w.prop(15, &[0, 0, 0, 3, 0xff, 0xff, 0xff, 0xfe]);
        w.prop_u32(35, 2);
        w.prop_u32(36, 1);
        w.prop_u32(35, 0); // duplicates remain ordered for the caller's policy
    }
    w.end();
    let pixels = w.slot();
    let effects_slot = if version >= 26 {
        let at = w.out.len();
        for pointer in effects {
            w.out.extend_from_slice(&pointer.to_be_bytes());
        }
        w.ptr(0);
        Some(at)
    } else {
        None
    };
    let hierarchy = hierarchy(&mut w, 2, 1, sample_bytes, data, Compression::None);
    w.patch(pixels, hierarchy);
    Fixture { bytes: w.out, effect, mask, hierarchy, mask_slot, version, effects_slot }
}
fn record(f: &Fixture) -> crate::effects::StoredEffect<'_> {
    Reader::new(&f.bytes, f.version)
        .read(Owner { kind: OwnerKind::Layer, offset: 0 }, f.effect as u64, &mut Budget::default(), &|| true)
        .unwrap()
        .record
}

#[test]
fn auxiliary_header_and_tiles_keep_exact_single_component_source_precision() {
    for (code, width, data, expected) in [
        (150, 1, vec![0, 255], vec![0., 1.]),
        (250, 2, vec![0x12, 0x34, 0xff, 0xff], vec![f64::from(0x1234u16) / 65535., 1.]),
        (
            350,
            4,
            [0x1234_5678u32, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect(),
            vec![f64::from(0x1234_5678u32) / f64::from(u32::MAX), 1.],
        ),
        (550, 2, [0x3800u16, 0x3c00].into_iter().flat_map(u16::to_be_bytes).collect(), vec![0.5, 1.]),
        (650, 4, [-0.25f32, 2.].into_iter().flat_map(f32::to_be_bytes).collect(), vec![-0.25, 2.]),
        (750, 8, [0.123456789012345f64, 2.].into_iter().flat_map(f64::to_be_bytes).collect(), vec![0.123456789012345, 2.]),
    ] {
        let f = fixture(code, &data, width, true);
        let xcf = Xcf::parse(&f.bytes).unwrap();
        let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
        assert_eq!((header.source_offset(), header.width(), header.height()), (f.mask as u64, 2, 1));
        assert_eq!(header.channel().name, "Coverage");
        let properties = header.properties();
        assert_eq!(properties.iter().map(|p| p.kind).collect::<Vec<_>>(), [15, 35, 36, 35]);
        assert_eq!(properties.iter().map(|p| p.declared_length).collect::<Vec<_>>(), [8, 4, 4, 4]);
        assert_eq!(properties[0].payload, &[0, 0, 0, 3, 0xff, 0xff, 0xff, 0xfe]);
        assert_eq!(properties[1].payload, &2u32.to_be_bytes());
        assert_eq!(properties[2].payload, &1u32.to_be_bytes());
        assert_eq!(properties[3].payload, &0u32.to_be_bytes());
        for property in properties {
            assert_eq!(property.raw, &f.bytes[property.offset..property.offset + 8 + property.payload.len()]);
        }
        assert!(properties.windows(2).all(|pair| pair[0].offset < pair[1].offset));
        let mut budget = data.len();
        let mut actual = Vec::new();
        let info = xcf
            .visit_effect_mask_tiles(&header, SampleOptions::default(), &mut budget, &|| true, |tile| {
                for index in 0..2 {
                    actual.push(tile.sample(index)?.normalized_f64());
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(budget, 0);
        assert_eq!(info.channels, 1);
        assert!(!info.indexed);
        assert_eq!(info.precision.storage.bytes, xcf.source_precision().storage.bytes);
        assert_eq!(info.precision.storage.float, xcf.source_precision().storage.float);
        assert!(info.precision.storage.linear);
        assert_eq!(info.precision.transfer, crate::samples::Transfer::Linear);
        assert!(!info.transfer_is_assumed);
    }
}

#[test]
fn auxiliary_headers_are_bound_to_original_source_and_unmodified_envelope_identity() {
    let f = fixture(150, &[1, 2], 1, true);
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let other_bytes = f.bytes.clone();
    let other = Xcf::parse(&other_bytes).unwrap();
    let mut effect = record(&f);
    assert!(other.effect_mask_header(&effect, &mut records::Budget::default(), &|| true).is_err());
    let header = xcf.effect_mask_header(&effect, &mut records::Budget::default(), &|| true).unwrap().unwrap();
    let mut budget = 2;
    assert!(other.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut budget, &|| true, |_| panic!("wrong source")).is_err());
    assert_eq!(budget, 2);
    effect.mask_offset = 0;
    assert!(xcf.effect_mask_header(&effect, &mut records::Budget::default(), &|| true).is_err());
    effect.mask_offset = f.mask as u64;
    effect.record_offset += 1;
    assert!(xcf.effect_mask_header(&effect, &mut records::Budget::default(), &|| true).is_err());
    effect.record_offset -= 1;
    effect.raw = &f.bytes[f.effect + 1..f.mask];
    assert!(xcf.effect_mask_header(&effect, &mut records::Budget::default(), &|| true).is_err());
    assert!(format!("{header:?}").len() < 180, "debug never prints the source bytes");
}

#[test]
fn auxiliary_absence_bad_headers_budget_and_cancellation_never_decode_unadmitted_pixels() {
    let missing = fixture(150, &[1, 2], 1, false);
    let xcf = Xcf::parse(&missing.bytes).unwrap();
    assert!(xcf.effect_mask_header(&record(&missing), &mut records::Budget::default(), &|| true).unwrap().is_none());
    assert!(matches!(
        xcf.effect_mask_header(&record(&missing), &mut records::Budget::default(), &|| false),
        Err(EffectMaskError::Source(Error::Cancelled))
    ));
    let f = fixture(150, &[1, 2], 1, true);
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
    let mut tight = 1;
    assert!(xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut tight, &|| true, |_| panic!("over budget")).is_err());
    assert_eq!(tight, 1);
    let mut budget = 2;
    assert!(matches!(
        xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut budget, &|| false, |_| panic!("cancelled")),
        Err(Error::Cancelled)
    ));
    assert_eq!(budget, 2);
    assert!(xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut budget, &|| true, |_| Err(invalid("consumer refused"))).is_err());
    assert_eq!(budget, 0, "late failures do not refund decoded work");
    for at in [f.mask, f.hierarchy] {
        let mut malformed = fixture(150, &[1, 2], 1, true);
        malformed.bytes[at..at + 4].copy_from_slice(&3u32.to_be_bytes());
        let reader = Xcf::parse(&malformed.bytes).unwrap();
        assert!(reader.effect_mask_header(&record(&malformed), &mut records::Budget::default(), &|| true).is_err());
    }
    let mut outside = fixture(150, &[1, 2], 1, true);
    outside.bytes[outside.mask_slot..outside.mask_slot + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    let reader = Xcf::parse(&outside.bytes).unwrap();
    assert!(reader.effect_mask_header(&record(&outside), &mut records::Budget::default(), &|| true).is_err());
}

#[test]
fn auxiliary_header_requires_a_complete_first_level_header() {
    for available in [0, 7, 8, 15, 16] {
        let mut fixture = fixture(150, &[1, 2], 1, true);
        let level = u64::from_be_bytes(fixture.bytes[fixture.hierarchy + 12..fixture.hierarchy + 20].try_into().unwrap());
        let level = usize::try_from(level).unwrap();
        fixture.bytes.truncate(level + available);
        let xcf = Xcf::parse(&fixture.bytes).unwrap();
        let mut metadata = records::Budget::new(4096, 4096);
        let result = xcf.effect_mask_header(&record(&fixture), &mut metadata, &|| true);
        if available == 16 {
            assert!(result.unwrap().is_some(), "the fixed header alone is sufficient");
            continue;
        }
        let error = result.unwrap_err();
        if available >= 8 {
            assert!(error.to_string().contains(&format!("wanted 8 bytes at {}", level + 8)), "{error}");
        }
        assert_eq!(metadata.retained_remaining(), 4096);
    }
}

#[test]
fn auxiliary_header_rejects_an_unterminated_name_without_retaining_metadata() {
    let mut fixture = fixture(150, &[1, 2], 1, true);
    let terminator = fixture.mask + 12 + "Coverage".len();
    assert_eq!(fixture.bytes[terminator], 0);
    fixture.bytes[terminator] = b'X';
    let xcf = Xcf::parse(&fixture.bytes).unwrap();
    let name_budget = xcf.meta_budget.get();
    let mut metadata = records::Budget::new(4096, 4096);
    let error = xcf.effect_mask_header(&record(&fixture), &mut metadata, &|| true).unwrap_err();
    assert!(error.to_string().contains("terminating NUL"), "{error}");
    assert_eq!(metadata.retained_remaining(), 4096);
    assert_eq!(xcf.meta_budget.get(), name_budget);
}

#[test]
fn auxiliary_header_rejects_an_interior_nul_and_accepts_both_empty_names() {
    let invalid = fixture_named(150, &[1, 2], 1, true, b"a\0b\0", true);
    let xcf = Xcf::parse(&invalid.bytes).unwrap();
    let mut metadata = records::Budget::new(4096, 4096);
    let original_name_budget = xcf.meta_budget.get();
    let error = xcf.effect_mask_header(&record(&invalid), &mut metadata, &|| true).unwrap_err();
    assert!(error.to_string().contains("interior NUL"), "{error}");
    assert_eq!(metadata.retained_remaining(), 4096);
    assert_eq!(xcf.meta_budget.get(), original_name_budget);

    for name in [b"".as_slice(), b"\0"] {
        let empty = fixture_named(150, &[1, 2], 1, true, name, false);
        let xcf = Xcf::parse(&empty.bytes).unwrap();
        let header = xcf.effect_mask_header(&record(&empty), &mut records::Budget::default(), &|| true).unwrap().unwrap();
        assert!(header.channel().name.is_empty());
    }
}

#[test]
fn auxiliary_tiles_cannot_use_their_channel_header_as_samples() {
    for displacement in [-2, -1, 0, 4] {
        let mut f = fixture(150, &[1, 2], 1, true);
        let level = u64::from_be_bytes(f.bytes[f.hierarchy + 12..f.hierarchy + 20].try_into().unwrap()) as usize;
        let tile = f.mask.checked_add_signed(displacement).unwrap() as u64;
        f.bytes[level + 8..level + 16].copy_from_slice(&tile.to_be_bytes());
        let xcf = Xcf::parse(&f.bytes).unwrap();
        let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
        let mut samples = Vec::new();
        let result = xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut 2, &|| true, |tile| {
            samples.extend_from_slice(tile.data);
            Ok(())
        });
        if displacement == -2 {
            assert!(result.is_ok());
            assert_eq!(samples, [39, 87]);
        } else {
            assert!(result.is_err(), "tile at {displacement} relative to the channel header");
            assert!(samples.is_empty());
        }
    }
}

#[test]
fn auxiliary_zero_level_is_delivered_as_zero_and_large_header_is_rejected_before_tiles() {
    let mut f = fixture(150, &[1, 2], 1, true);
    let level = u64::from_be_bytes(f.bytes[f.hierarchy + 12..f.hierarchy + 20].try_into().unwrap()) as usize;
    f.bytes[level + 8..level + 16].copy_from_slice(&0u64.to_be_bytes());
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
    let mut budget = 2;
    let mut delivered = 0;
    xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut budget, &|| true, |tile| {
        delivered += 1;
        assert_eq!(tile.data, &[0, 0]);
        Ok(())
    })
    .unwrap();
    assert_eq!(budget, 0);
    assert_eq!(delivered, 1);
    let mut huge = fixture(650, &[0; 8], 4, true);
    for at in [huge.mask, huge.hierarchy] {
        huge.bytes[at..at + 4].copy_from_slice(&16_385u32.to_be_bytes());
        huge.bytes[at + 4..at + 8].copy_from_slice(&16_384u32.to_be_bytes());
    }
    let reader = Xcf::parse(&huge.bytes).unwrap();
    assert!(
        reader.effect_mask_header(&record(&huge), &mut records::Budget::default(), &|| true).unwrap_err().to_string().contains("source pixel limit")
    );
}

#[test]
fn repeated_empty_name_headers_meter_the_hierarchy_and_dummy_probes_before_publication() {
    for dummy in [false, true] {
        let mut f = fixture_named(150, &[1, 2], 1, true, b"", false);
        if dummy {
            let mut file = W { out: f.bytes, wide: true };
            let channel_hierarchy_slot = f.mask + (8 + 4 + 8);
            let hierarchy = file.out.len();
            file.patch(channel_hierarchy_slot, hierarchy);
            for word in [2, 1, 1] {
                file.u32(word);
            }
            let level_slot = file.slot();
            let dummy_slot = file.slot();
            file.ptr(0);
            let level = file.out.len();
            file.patch(level_slot, level);
            file.u32(2);
            file.u32(1);
            file.ptr(0);
            let dummy = file.out.len();
            file.patch(dummy_slot, dummy);
            for word in [1, 0, 0] {
                file.u32(word);
            }
            f.bytes = file.out;
        }
        let xcf = Xcf::parse(&f.bytes).unwrap();
        let effect = record(&f);
        // Channel dimensions, name length, property terminator and hierarchy
        // pointer; then hierarchy dimensions, BPP, level list and fixed level.
        let channel_properties = 8 + 4 + 8;
        let cost = channel_properties + 8 + (12 + 8 + 8) + (8 + 8) + if dummy { 8 + 12 } else { 0 };
        for short in [channel_properties, cost - 1] {
            let mut metadata = records::Budget::new(short, 0);
            assert!(matches!(
                xcf.effect_mask_header(&effect, &mut metadata, &|| true),
                Err(EffectMaskError::Metadata(records::StopReason::WorkLimit))
            ));
            assert_eq!(metadata.retained_remaining(), 0);
        }
        let mut metadata = records::Budget::new(2 * cost, 0);
        for remaining in [cost, 0] {
            let header = xcf.effect_mask_header(&effect, &mut metadata, &|| true).unwrap().unwrap();
            assert!(header.channel().name.is_empty());
            assert_eq!(metadata.work_remaining(), remaining);
        }
        assert!(matches!(xcf.effect_mask_header(&effect, &mut metadata, &|| true), Err(EffectMaskError::Metadata(records::StopReason::WorkLimit))));
        assert!(matches!(xcf.effect_mask_header(&effect, &mut metadata, &|| false), Err(EffectMaskError::Source(Error::Cancelled))));
        assert_eq!(metadata.work_remaining(), 0);
    }
}

#[test]
fn auxiliary_channel_must_be_disjoint_from_the_effect_and_keeps_marker_as_source_evidence() {
    for version in [22, 26] {
        for nested in [false, true] {
            let mut file = W::new(version);
            for word in [2, 1, 1, 150] {
                file.u32(word);
            }
            file.prop(17, &[0]);
            file.end();
            for _ in 0..3 {
                file.ptr(0);
            }
            let effect_offset = file.out.len();
            for value in ["Effect", "", "gegl:exposure", "1:0"] {
                file.string(value);
            }
            let mut channel = W { out: Vec::new(), wide: true };
            channel.u32(2);
            channel.u32(1);
            channel.string("Aux");
            channel.prop(4, &[]);
            channel.end();
            let hierarchy_slot = channel.slot();
            if version >= 26 {
                channel.ptr(0);
            }
            let nested_offset = file.out.len() + 8;
            if nested {
                file.prop(9000, &channel.out);
            }
            file.end();
            let effect_mask_slot = file.slot();
            let mask = if nested {
                nested_offset
            } else {
                let start = file.out.len();
                file.out.extend(channel.out);
                start
            };
            file.patch(effect_mask_slot, mask);
            let pixels = hierarchy(&mut file, 2, 1, 1, &[55, 77], Compression::None);
            file.patch(mask + hierarchy_slot, pixels);
            let effect = Reader::new(&file.out, version)
                .read(Owner { kind: OwnerKind::Layer, offset: 0 }, effect_offset as u64, &mut Budget::default(), &|| true)
                .unwrap()
                .record;
            let reader = Xcf::parse(&file.out).unwrap();
            let mut metadata = records::Budget::new(4096, 4096);
            let result = reader.effect_mask_header(&effect, &mut metadata, &|| true);
            if nested {
                let error = result.unwrap_err();
                assert!(matches!(error, EffectMaskError::Source(Error::Invalid(ref message))
                    if message == "effect mask channel overlaps its effect envelope"));
                assert_eq!(metadata.retained_remaining(), 4096);
            } else {
                let header = result.unwrap().unwrap();
                assert!(!header.channel().selection);
                assert!(header.properties().iter().any(|property| property.kind == 4 && property.payload.is_empty()));
                reader
                    .visit_effect_mask_tiles(&header, SampleOptions::default(), &mut 2, &|| true, |tile| {
                        assert_eq!(tile.data, [55, 77]);
                        Ok(())
                    })
                    .unwrap();
            }
        }
    }
}

#[test]
fn effect_mask_header_only_charges_retained_properties_on_success() {
    let f = fixture(150, &[1, 2], 1, true);
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let effect = record(&f);
    let name_bytes = "Coverage".len();
    xcf.meta_budget.set(name_bytes);

    let one_property = 2 * std::mem::size_of::<records::Property<'_>>();
    let mut limited = records::Budget::new(4096, one_property);
    assert!(matches!(xcf.effect_mask_header(&effect, &mut limited, &|| true), Err(EffectMaskError::Metadata(records::StopReason::RetainedLimit))));
    assert_eq!(limited.retained_remaining(), one_property);
    assert!(limited.work_remaining() < 4096);
    assert_eq!(xcf.meta_budget.get(), name_bytes);

    let mut available = records::Budget::new(4096, 8 * one_property);
    let polls = std::cell::Cell::new(0);
    let header = xcf
        .effect_mask_header(&effect, &mut available, &|| {
            polls.set(polls.get() + 1);
            true
        })
        .unwrap()
        .unwrap();
    assert_eq!(header.properties().len(), 4);
    assert!(available.retained_remaining() < 8 * one_property);
    assert_eq!(xcf.meta_budget.get(), 0);

    xcf.meta_budget.set(name_bytes);
    let last_poll = polls.replace(0);
    let mut cancelled = records::Budget::new(4096, 8 * one_property);
    let result = xcf.effect_mask_header(&effect, &mut cancelled, &|| {
        polls.set(polls.get() + 1);
        polls.get() < last_poll
    });
    assert!(matches!(result, Err(EffectMaskError::Source(Error::Cancelled))));
    assert_eq!(cancelled.retained_remaining(), 8 * one_property);
    assert!(cancelled.work_remaining() < 4096);
    assert_eq!(xcf.meta_budget.get(), name_bytes);
}

#[test]
fn a_rejected_mask_hierarchy_does_not_keep_property_or_name_charges() {
    let mut f = fixture(150, &[1, 2], 1, true);
    f.bytes[f.hierarchy..f.hierarchy + 4].copy_from_slice(&3u32.to_be_bytes());
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let mut metadata = records::Budget::new(4096, 4096);
    let name_bytes = "Coverage".len();
    xcf.meta_budget.set(name_bytes);
    let error = xcf.effect_mask_header(&record(&f), &mut metadata, &|| true).unwrap_err();
    assert!(error.to_string().contains("dimensions do not match"));
    assert_eq!(metadata.retained_remaining(), 4096);
    assert!(metadata.work_remaining() < 4096);
    assert_eq!(xcf.meta_budget.get(), name_bytes);
}

#[test]
fn auxiliary_header_rejects_a_fixed_level_in_its_channel_before_metadata_publication() {
    for version in [22, 26] {
        let mut f = fixture_framed(150, &[1, 2], 1, true, b"", true, (version, &[]));
        f.bytes[f.hierarchy + 12..f.hierarchy + 20].copy_from_slice(&(f.mask as u64).to_be_bytes());
        let xcf = Xcf::parse(&f.bytes).unwrap();
        let mut metadata = records::Budget::new(4096, 4096);
        let before = xcf.meta_budget.get();
        let error = xcf.effect_mask_header(&record(&f), &mut metadata, &|| true).unwrap_err();
        assert!(error.to_string().contains("level header overlaps its source record"), "{error}");
        assert_eq!(metadata.retained_remaining(), 4096);
        assert!(metadata.work_remaining() < 4096);
        assert_eq!(xcf.meta_budget.get(), before);
    }
}

#[test]
fn auxiliary_header_rejects_a_hierarchy_in_its_channel_before_metadata_publication() {
    let mut w = W::new(26);
    for value in [2, 1, 1, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    for _ in 0..3 {
        w.ptr(0);
    }
    let effect = w.out.len();
    for value in ["Effect", "", "gegl:exposure", "1:0"] {
        w.string(value);
    }
    w.end();
    let mask_slot = w.slot();
    let mask = w.out.len();
    w.patch(mask_slot, mask);
    w.u32(2);
    w.u32(1);
    w.string("");
    w.u32(9000);
    w.u32(28);
    let hierarchy = w.out.len();
    w.u32(2);
    w.u32(1);
    w.u32(1);
    let level_slot = w.slot();
    w.ptr(0);
    w.end();
    w.ptr(hierarchy);
    w.ptr(0);
    let level = w.out.len();
    w.patch(level_slot, level);
    w.u32(2);
    w.u32(1);
    w.ptr(0); // A disjoint, complete, empty level.
    let xcf = Xcf::parse(&w.out).unwrap();
    let effect =
        Reader::new(&w.out, 26).read(Owner { kind: OwnerKind::Layer, offset: 0 }, effect as u64, &mut Budget::default(), &|| true).unwrap().record;
    let mut metadata = records::Budget::new(4096, 4096);
    let before = xcf.meta_budget.get();
    let error = xcf.effect_mask_header(&effect, &mut metadata, &|| true).unwrap_err();
    assert!(error.to_string().contains("hierarchy overlaps its source record"), "{error}");
    assert_eq!(metadata.retained_remaining(), 4096);
    assert_eq!(xcf.meta_budget.get(), before);
}

#[test]
fn auxiliary_v26_effect_pointer_tail_is_excluded_without_following_targets() {
    for pointers in [&[][..], &[u64::MAX, u64::MAX][..]] {
        let mut f = fixture_framed(150, &[1, 2], 1, true, b"Coverage\0", true, (26, pointers));
        let level = u64::from_be_bytes(f.bytes[f.hierarchy + 12..f.hierarchy + 20].try_into().unwrap()) as usize;
        let tail = f.effects_slot.unwrap();
        f.bytes[level + 8..level + 16].copy_from_slice(&(tail as u64).to_be_bytes());
        let xcf = Xcf::parse(&f.bytes).unwrap();
        let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
        let mut callbacks = 0;
        let result = xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut 2, &|| true, |_| {
            callbacks += 1;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(callbacks, 0);
    }
}

#[test]
fn auxiliary_v26_tail_has_cumulative_work_count_and_cancellation_limits() {
    let pointers = vec![u64::MAX; 1024];
    let f = fixture_framed(150, &[1, 2], 1, true, b"\0", false, (26, &pointers));
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let mut measured = records::Budget::new(1 << 20, 4096);
    let before = measured.work_remaining();
    assert!(xcf.effect_mask_header(&record(&f), &mut measured, &|| true).unwrap().is_some());
    let cost = before - measured.work_remaining();
    assert!(cost >= (1024 + 1) * 8);
    let mut exact = records::Budget::new(cost, 4096);
    assert!(xcf.effect_mask_header(&record(&f), &mut exact, &|| true).unwrap().is_some());
    assert_eq!(exact.work_remaining(), 0);
    assert!(xcf.effect_mask_header(&record(&f), &mut exact, &|| true).is_err());
    let mut short = records::Budget::new(cost - 1, 4096);
    assert!(matches!(xcf.effect_mask_header(&record(&f), &mut short, &|| true), Err(EffectMaskError::Metadata(records::StopReason::WorkLimit))));
    assert_eq!(short.retained_remaining(), 4096);
    let polls = std::cell::Cell::new(0);
    let mut cancelled = records::Budget::new(1 << 20, 4096);
    assert!(matches!(
        xcf.effect_mask_header(&record(&f), &mut cancelled, &|| {
            polls.set(polls.get() + 1);
            polls.get() < 20
        }),
        Err(EffectMaskError::Source(Error::Cancelled))
    ));
    assert_eq!(cancelled.retained_remaining(), 4096);
    let too_many = vec![u64::MAX; 1025];
    let f = fixture_framed(150, &[1, 2], 1, true, b"\0", false, (26, &too_many));
    let xcf = Xcf::parse(&f.bytes).unwrap();
    assert!(xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).is_err());
}

#[test]
fn auxiliary_tail_work_refusal_retries_without_losing_name_storage_or_mislabeling_truncation() {
    let f = fixture_framed(150, &[1, 2], 1, true, b"Coverage\0", true, (26, &[]));
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let mut measured = records::Budget::new(4096, 4096);
    xcf.effect_mask_header(&record(&f), &mut measured, &|| true).unwrap().unwrap();
    let work = 4096 - measured.work_remaining();
    xcf.meta_budget.set("Coverage".len());
    let mut prefix = records::Budget::new(4096, 4096);
    let properties = records::channel_properties(&f.bytes, f.mask, records::Limits::default(), &mut prefix, &|| true).unwrap();
    assert!(properties.stopped.is_none());
    let pointer_bytes = 8;
    let tail_bytes_remaining = pointer_bytes - 1;
    let short_allowance = 4096 - prefix.work_remaining() + pointer_bytes + tail_bytes_remaining;
    let mut short = records::Budget::new(short_allowance, 4096);
    assert!(matches!(xcf.effect_mask_header(&record(&f), &mut short, &|| true), Err(EffectMaskError::Metadata(records::StopReason::WorkLimit))));
    assert_eq!(short.retained_remaining(), 4096);
    assert_eq!(short.work_remaining(), tail_bytes_remaining);
    assert_eq!(xcf.meta_budget.get(), "Coverage".len());
    let mut enough = records::Budget::new(work, 4096);
    assert!(xcf.effect_mask_header(&record(&f), &mut enough, &|| true).unwrap().is_some());
    assert_eq!(enough.work_remaining(), 0);
    assert_eq!(xcf.meta_budget.get(), 0);
    let mut broken = fixture_framed(150, &[1, 2], 1, true, b"Coverage\0", true, (26, &[]));
    broken.bytes.truncate(broken.effects_slot.unwrap() + 7);
    let xcf = Xcf::parse(&broken.bytes).unwrap();
    let mut available = records::Budget::new(4096, 4096);
    assert!(matches!(xcf.effect_mask_header(&record(&broken), &mut available, &|| true), Err(EffectMaskError::Source(Error::Invalid(_)))));
    assert_eq!(available.retained_remaining(), 4096);
}

fn fixture_with_tiles_around_envelope(version: u32) -> Fixture {
    let mut w = W::new(version);
    for value in [2, 1, 1, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    for _ in 0..3 {
        w.ptr(0);
    }
    w.out.extend([17, 29]); // Complete tile immediately before the effect.
    let effect = w.out.len();
    for value in ["Effect", "", "gegl:exposure", "1:0"] {
        w.string(value);
    }
    w.end();
    let mask_slot = w.slot();
    w.out.extend([39, 87]); // Complete tile between effect and channel.
    let mask = w.out.len();
    w.patch(mask_slot, mask);
    w.u32(2);
    w.u32(1);
    w.string("Coverage");
    w.end();
    let pixels = w.slot();
    let effects_slot = if version >= 26 {
        let at = w.out.len();
        w.ptr(0);
        Some(at)
    } else {
        None
    };
    let hierarchy = hierarchy(&mut w, 2, 1, 1, &[11, 23], Compression::None);
    w.patch(pixels, hierarchy);
    Fixture { version, effects_slot, bytes: w.out, effect, mask, hierarchy, mask_slot }
}

#[test]
fn auxiliary_tiles_exclude_authenticated_effect_and_channel_spans_but_keep_adjacent_gaps() {
    for version in [22, 26] {
        let original = fixture_with_tiles_around_envelope(version);
        for (tile, expected) in [
            (original.effect - 2, Some([17, 29])),
            (original.effect - 1, None),
            (original.effect, None),
            (original.effect + 1, None),
            (original.mask_slot, None),
            (original.mask - 3, None),
            (original.mask - 2, Some([39, 87])),
            (original.mask - 1, None),
            (original.mask, None),
        ] {
            let mut f = fixture_with_tiles_around_envelope(version);
            let level = u64::from_be_bytes(f.bytes[f.hierarchy + 12..f.hierarchy + 20].try_into().unwrap()) as usize;
            f.bytes[level + 8..level + 16].copy_from_slice(&(tile as u64).to_be_bytes());
            let xcf = Xcf::parse(&f.bytes).unwrap();
            let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
            let mut actual = Vec::new();
            let result = xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut 2, &|| true, |tile| {
                actual.extend_from_slice(tile.data);
                Ok(())
            });
            if let Some(expected) = expected {
                assert!(result.is_ok(), "v{version} tile {tile}: {result:?}");
                assert_eq!(actual, expected);
            } else {
                assert!(result.is_err(), "v{version} tile {tile} must not decode record bytes");
                assert!(actual.is_empty());
            }
        }
    }
}

#[test]
fn auxiliary_validation_excludes_the_effect_envelope_from_fixed_level_headers() {
    // An unknown property belongs to the authenticated effect, not available
    // hierarchy storage. Embed a matching fixed level solely as inert data.
    let mut w = W::new(26);
    for value in [2, 1, 1, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    for _ in 0..3 {
        w.ptr(0);
    }
    let effect = w.out.len();
    for value in ["Effect", "", "gegl:exposure", "1:0"] {
        w.string(value);
    }
    w.u32(9999);
    w.u32(16);
    let fixed = w.out.len();
    w.u32(2);
    w.u32(1);
    w.ptr(0);
    w.end();
    let mask_slot = w.slot();
    let mask = w.out.len();
    w.patch(mask_slot, mask);
    w.u32(2);
    w.u32(1);
    w.string("");
    w.end();
    let hierarchy_slot = w.slot();
    w.ptr(0);
    let hierarchy = w.out.len();
    w.patch(hierarchy_slot, hierarchy);
    w.u32(2);
    w.u32(1);
    w.u32(1);
    w.ptr(fixed);
    w.ptr(0);
    let f = Fixture { bytes: w.out, version: 26, effect, mask, hierarchy, mask_slot, effects_slot: None };
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let mut metadata = records::Budget::new(4096, 4096);
    let error = xcf.effect_mask_header(&record(&f), &mut metadata, &|| true).unwrap_err();
    assert!(error.to_string().contains("level header overlaps its source record"));
    assert_eq!(metadata.retained_remaining(), 4096);
}

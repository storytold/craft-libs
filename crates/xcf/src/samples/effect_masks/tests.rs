use super::*;
use crate::Compression;
use crate::effects::{Budget, Owner, OwnerKind, Reader};
use crate::tests::{W, hierarchy};

struct Fixture {
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
    let mut w = W::new(22);
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
    let hierarchy = hierarchy(&mut w, 2, 1, sample_bytes, data, Compression::None);
    w.patch(pixels, hierarchy);
    Fixture { bytes: w.out, effect, mask, hierarchy, mask_slot }
}
fn record(f: &Fixture) -> crate::effects::StoredEffect<'_> {
    Reader::new(&f.bytes, 22).read(Owner { kind: OwnerKind::Layer, offset: 0 }, f.effect as u64, &mut Budget::default(), &|| true).unwrap().record
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
fn auxiliary_zero_level_is_delivered_as_zero_and_large_header_is_rejected_before_tiles() {
    let mut f = fixture(150, &[1, 2], 1, true);
    let level = u64::from_be_bytes(f.bytes[f.hierarchy + 12..f.hierarchy + 20].try_into().unwrap()) as usize;
    f.bytes[level + 8..level + 16].copy_from_slice(&0u64.to_be_bytes());
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let header = xcf.effect_mask_header(&record(&f), &mut records::Budget::default(), &|| true).unwrap().unwrap();
    let mut budget = 2;
    xcf.visit_effect_mask_tiles(&header, SampleOptions::default(), &mut budget, &|| true, |tile| {
        assert_eq!(tile.data, &[0, 0]);
        Ok(())
    })
    .unwrap();
    assert_eq!(budget, 0);
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
fn repeated_empty_name_headers_spend_shared_work_and_stop_before_publishing() {
    let f = fixture_named(150, &[1, 2], 1, true, b"", false);
    let xcf = Xcf::parse(&f.bytes).unwrap();
    let effect = record(&f);
    // Eight dimension bytes, four name-length bytes, and eight PROP_END bytes.
    let mut metadata = records::Budget::new(40, 0);
    for remaining in [20, 0] {
        let header = xcf.effect_mask_header(&effect, &mut metadata, &|| true).unwrap().unwrap();
        assert!(header.channel().name.is_empty());
        assert_eq!(metadata.work_remaining(), remaining);
    }
    assert!(matches!(xcf.effect_mask_header(&effect, &mut metadata, &|| true), Err(EffectMaskError::Metadata(records::StopReason::WorkLimit))));
    assert!(matches!(xcf.effect_mask_header(&effect, &mut metadata, &|| false), Err(EffectMaskError::Source(Error::Cancelled))));
    assert_eq!(metadata.work_remaining(), 0);
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

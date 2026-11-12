//! Synthetic only: no personal PSDs, paths, or copied descriptor payloads.

use craft_psd::animation::{AnimationRecord, AnimationResource, frame_info, frame_sets, layer_frame_states, parse_descriptor};
use craft_psd::descriptor::{Descriptor, Value, VersionedDescriptor};
use craft_psd::metadata::{MetadataItem, parse_shmd, write_shmd};
use craft_psd::{ImageResource, PsdBuilder, PsdFile, ResourceData, TaggedBlock, Version};
use proptest::prelude::*;

fn descriptor() -> VersionedDescriptor {
    VersionedDescriptor::new(
        Descriptor::default()
            .with("AFSt", Value::Integer(7))
            .with(
                "FrIn",
                Value::List(vec![
                    Value::Descriptor(
                        Descriptor::default().with("FrID", Value::Integer(-3)).with("FrDl", Value::Integer(3)).with("future", Value::Boolean(true)),
                    ),
                    Value::Descriptor(Descriptor::default().with("FrID", Value::Integer(21)).with("FrDl", Value::Integer(53))),
                ]),
            )
            .with(
                "FSts",
                Value::List(vec![Value::Descriptor(
                    Descriptor::default()
                        .with("FsID", Value::Integer(7))
                        .with("AFrm", Value::Integer(0))
                        .with("FsFr", Value::List(vec![Value::Integer(21), Value::Integer(-3)]))
                        .with("future", Value::RawData(vec![1, 2, 3])),
                )]),
            ),
    )
}

fn resource(data: &[u8]) -> AnimationResource<'_> {
    AnimationResource {
        records: vec![
            AnimationRecord { key: *b"AnDs", data },
            AnimationRecord { key: *b"Roll", data: &[0; 8] },
            AnimationRecord { key: *b"next", data: &[9, 8, 7] },
        ],
    }
}

#[test]
fn raw_roundtrip_and_typed_views_preserve_order_and_unknown_data() {
    let data = descriptor().to_bytes();
    let bytes = resource(&data).to_bytes().unwrap();
    let parsed = AnimationResource::from_bytes(&bytes).unwrap();
    assert_eq!(parsed, resource(&data));
    let d = parsed.records[0].descriptor().unwrap().unwrap();
    assert_eq!(d, descriptor());
    let frames = frame_info(&d.descriptor).unwrap();
    assert_eq!(frames.iter().map(|f| (f.id, f.delay_centiseconds)).collect::<Vec<_>>(), [(-3, Some(3)), (21, Some(53))]);
    assert_eq!(frames[0].descriptor.get("future"), Some(&Value::Boolean(true)));
    let sets = frame_sets(&d.descriptor).unwrap();
    assert_eq!(sets[0].frame_ids, [21, -3], "playback order is not FrIn order or sorted id order");
    assert_eq!(sets[0].id, 7);
    assert_eq!(sets[0].active_frame, Some(0));
    assert_eq!(sets[0].descriptor.get("future"), Some(&Value::RawData(vec![1, 2, 3])));
    assert!(parsed.records[1].descriptor().is_none());
    assert_eq!(parsed.to_bytes().unwrap(), bytes);
}

#[test]
fn plugin_signature_not_a_single_resource_id_selects_the_parser() {
    let data = descriptor().to_bytes();
    let bytes = resource(&data).to_bytes().unwrap();
    for id in [4000, 4003, 4999] {
        let r = ImageResource::new(id, bytes.clone());
        assert!(matches!(r.parsed(), Some(Ok(ResourceData::Animation(_)))));
    }
    for id in [3999, 5000] {
        assert!(ImageResource::new(id, bytes.clone()).parsed().is_none());
    }
    assert!(ImageResource::new(4003, b"different plugin".to_vec()).parsed().is_none());
    assert!(matches!(ImageResource::new(4003, b"mani".to_vec()).parsed(), Some(Err(_))));
    assert!(matches!(
        ImageResource::new(1075, VersionedDescriptor::new(Descriptor::default().with("Vrsn", Value::Integer(1))).to_bytes()).parsed(),
        Some(Ok(ResourceData::TimelineInfo(_)))
    ));
}

#[test]
fn metadata_views_do_not_touch_xmp_or_other_items() {
    let d = VersionedDescriptor::new(Descriptor::default().with("LaID", Value::Integer(42)).with(
        "LaSt",
        Value::List(vec![Value::Descriptor(
                Descriptor::default()
                    .with("FrLs", Value::List(vec![Value::Integer(-3)]))
                    .with("blendOptions", Value::Descriptor(Descriptor::default().with("Opct", Value::Double(12.5)))),
            )]),
    ));
    let items = vec![
        MetadataItem::new(*b"cust", vec![7, 9, 11]),
        MetadataItem::new(*b"mlst", d.to_bytes()),
        MetadataItem::new(*b"tmln", VersionedDescriptor::new(Descriptor::default().with("trackList", Value::List(vec![]))).to_bytes()),
        MetadataItem::new(*b"next", vec![255; 4]),
    ];
    let raw = write_shmd(&items);
    let parsed = parse_shmd(&raw).unwrap();
    assert!(parsed[0].animation_descriptor().is_none());
    let layer_descriptor = parsed[1].animation_descriptor().unwrap().unwrap();
    assert_eq!(layer_descriptor, d);
    let states = layer_frame_states(&layer_descriptor.descriptor).unwrap();
    assert_eq!(states.layer_id, 42);
    assert_eq!(states.states[0].frame_ids, [-3]);
    assert_eq!(states.states[0].descriptor.get("enab"), None, "missing visibility is not false");
    assert_eq!(states.states[0].descriptor.get("blendOptions"), Some(&Value::Descriptor(Descriptor::default().with("Opct", Value::Double(12.5)))));
    assert!(parsed[2].animation_descriptor().unwrap().is_ok());
    assert!(parsed[3].animation_descriptor().is_none());
    assert_eq!(write_shmd(&parsed), raw);
    let mut bad = parsed[1].clone();
    bad.signature = *b"nope";
    assert!(bad.animation_descriptor().unwrap().is_err());
}

#[test]
fn psd_and_psb_keep_animation_resources_and_metadata_byte_stable() {
    let data = descriptor().to_bytes();
    let raw = resource(&data).to_bytes().unwrap();
    for version in [Version::Psd, Version::Psb] {
        let mut file = PsdBuilder::new(1, 1).version(version).build().unwrap();
        file.resources.push(ImageResource::new(4003, raw.clone()));
        file.global_blocks.push(TaggedBlock::new(*b"shmd", write_shmd(&[MetadataItem::new(*b"mlst", data.clone())])));
        let bytes = file.to_bytes().unwrap();
        let back = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(back.to_bytes().unwrap(), bytes);
        let animation = back.resource(4003).unwrap();
        assert!(matches!(animation.parsed(), Some(Ok(ResourceData::Animation(_)))));
    }
}

#[test]
fn every_truncation_and_hostile_envelope_length_is_rejected() {
    let data = descriptor().to_bytes();
    let bytes = resource(&data).to_bytes().unwrap();
    for end in 0..bytes.len() {
        assert!(AnimationResource::from_bytes(&bytes[..end]).is_err(), "prefix {end}");
    }
    for at in [8, 20] {
        for value in [0, 1, u32::MAX] {
            let mut bad = bytes.clone();
            bad[at..at + 4].copy_from_slice(&value.to_be_bytes());
            assert!(AnimationResource::from_bytes(&bad).is_err(), "length {at} = {value}");
        }
    }
    for at in [0, 4, 12] {
        let mut bad = bytes.clone();
        bad[at] = b'?';
        assert!(AnimationResource::from_bytes(&bad).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(AnimationResource::from_bytes(&trailing).is_err());
}

#[test]
fn descriptor_padding_truncation_and_unknown_versions() {
    let raw = descriptor().to_bytes();
    for end in 0..raw.len() {
        assert!(parse_descriptor(&raw[..end]).is_err(), "descriptor prefix {end}");
    }
    for padding in 0..=3 {
        let mut padded = raw.clone();
        padded.extend(vec![0; padding]);
        assert_eq!(parse_descriptor(&padded).unwrap(), descriptor());
    }
    for tail in [vec![0; 4], vec![1], vec![0, 2]] {
        let mut bad = raw.clone();
        bad.extend(tail);
        assert!(parse_descriptor(&bad).is_err());
    }
    let mut future = raw.clone();
    future[..4].copy_from_slice(&17u32.to_be_bytes());
    let bytes = resource(&future).to_bytes().unwrap();
    let r = AnimationResource::from_bytes(&bytes).unwrap();
    assert!(r.records[0].descriptor().unwrap().is_err());
    assert_eq!(r.to_bytes().unwrap(), bytes, "unsupported typed version must survive raw");
}

#[test]
fn absent_delays_and_duplicate_ids_are_not_silently_repaired() {
    let d = Descriptor::default().with(
        "FrIn",
        Value::List(vec![
            Value::Descriptor(Descriptor::default().with("FrID", Value::Integer(4))),
            Value::Descriptor(Descriptor::default().with("FrID", Value::Integer(4)).with("FrDl", Value::Integer(0))),
        ]),
    );
    let frames = frame_info(&d).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].delay_centiseconds, None);
    assert_eq!(frames[1].delay_centiseconds, Some(0));
    let bytes = AnimationResource { records: vec![AnimationRecord { key: *b"next", data: &[] }, AnimationRecord { key: *b"next", data: &[1] }] }
        .to_bytes()
        .unwrap();
    assert_eq!(AnimationResource::from_bytes(&bytes).unwrap().to_bytes().unwrap(), bytes);
}

#[test]
fn wrong_frame_and_set_types_return_errors() {
    for value in [Value::Boolean(false), Value::List(vec![Value::Integer(1)]), Value::List(vec![Value::Descriptor(Descriptor::default())])] {
        assert!(frame_info(&Descriptor::default().with("FrIn", value.clone())).is_err());
        assert!(frame_sets(&Descriptor::default().with("FSts", value)).is_err());
    }
    for delay in [Value::Integer(-1), Value::Double(3.0)] {
        let frame = Descriptor::default().with("FrID", Value::Integer(1)).with("FrDl", delay);
        assert!(frame_info(&Descriptor::default().with("FrIn", Value::List(vec![Value::Descriptor(frame)]))).is_err());
    }
    for (key, value) in [("FsID", Value::Boolean(false)), ("AFrm", Value::Double(0.0)), ("FsFr", Value::List(vec![Value::Boolean(true)]))] {
        let mut set = Descriptor::default().with("FsID", Value::Integer(1)).with("FsFr", Value::List(vec![]));
        set.items.retain(|(id, _)| !id.is(key));
        set = set.with(key, value);
        assert!(frame_sets(&Descriptor::default().with("FSts", Value::List(vec![Value::Descriptor(set)]))).is_err());
    }
}

#[test]
fn sparse_layer_states_retain_missing_properties_and_multiple_references() {
    let first = Descriptor::default()
        .with("FrLs", Value::List(vec![Value::Integer(21), Value::Integer(-3)]))
        .with("enab", Value::Boolean(true))
        .with("Ofst", Value::Descriptor(Descriptor::default().with("Hrzn", Value::Integer(-7)).with("Vrtc", Value::Integer(12))));
    let second = Descriptor::default().with("FrLs", Value::List(vec![Value::Integer(99)]));
    let d = Descriptor::default()
        .with("LaID", Value::Integer(8))
        .with("LaSt", Value::List(vec![Value::Descriptor(first.clone()), Value::Descriptor(second)]));
    let view = layer_frame_states(&d).unwrap();
    assert_eq!(view.states[0].frame_ids, [21, -3]);
    assert_eq!(view.states[0].descriptor, &first);
    assert_eq!(view.states[1].descriptor.get("Ofst"), None);
    assert_eq!(view.states[1].descriptor.get("enab"), None);
    for d in [
        Descriptor::default(),
        Descriptor::default().with("LaID", Value::Boolean(true)),
        Descriptor::default().with("LaID", Value::Integer(8)).with("LaSt", Value::Boolean(true)),
        Descriptor::default().with("LaID", Value::Integer(8)).with("LaSt", Value::List(vec![Value::Integer(0)])),
    ] {
        assert!(layer_frame_states(&d).is_err());
    }
    for state in [
        Descriptor::default(),
        Descriptor::default().with("FrLs", Value::Integer(0)),
        Descriptor::default().with("FrLs", Value::List(vec![Value::Boolean(true)])),
    ] {
        let d = Descriptor::default().with("LaID", Value::Integer(8)).with("LaSt", Value::List(vec![Value::Descriptor(state)]));
        assert!(layer_frame_states(&d).is_err());
    }
}

proptest! {
    #[test]
    fn arbitrary_animation_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = AnimationResource::from_bytes(&data);
        let _ = parse_descriptor(&data);
    }

    #[test]
    fn opaque_record_roundtrip(data in proptest::collection::vec(any::<u8>(), 0..2048), key in any::<[u8; 4]>()) {
        let r = AnimationResource { records: vec![AnimationRecord { key, data: &data }] };
        let bytes = r.to_bytes().unwrap();
        prop_assert_eq!(AnimationResource::from_bytes(&bytes).unwrap(), r);
    }
}

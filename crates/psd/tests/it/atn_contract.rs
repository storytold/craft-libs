//! Public-API contract for the `.atn` parser, against independently encoded wire fixtures.
use craft_psd::PsdError;
use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Class, Descriptor, Id, MAX_DEPTH, ObjectArray, ReferenceItem, UnicodeString, Value};

#[path = "common/atn_roundtrip.rs"]
mod atn_roundtrip;

const FIXTURES: &[(&str, &[u8])] = &[
    ("empty", include_bytes!("../fixtures/atn/empty.atn")),
    ("metadata", include_bytes!("../fixtures/atn/metadata.atn")),
    ("descriptors", include_bytes!("../fixtures/atn/descriptors.atn")),
    ("float bits", include_bytes!("../fixtures/atn/float-bits.atn")),
];

fn file_with(desc: Descriptor) -> AtnFile {
    AtnFile {
        actions: vec![AtnAction {
            items: vec![AtnItem { event: AtnEvent::Code(*b"test"), descriptor: Some(desc), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn class(id: &str) -> Class {
    Class { name: UnicodeString::default(), class_id: Id::new(id) }
}

#[test]
fn independent_wire_fixtures_parse_and_write_byte_exactly() {
    for (name, bytes) in FIXTURES {
        let parsed = atn::parse(bytes).unwrap();
        assert!(parsed.warnings.is_empty(), "{name}");
        assert_eq!(atn::write(&parsed).unwrap(), *bytes, "{name}");
        atn_roundtrip::check(bytes);
    }
    assert_eq!(atn::write(&AtnFile::default()).unwrap(), FIXTURES[0].1);
    assert_eq!(&FIXTURES[0].1[..4], &[0, 0, 0, 16], "the wire version is big-endian on every host");
}

#[test]
fn unicode_metadata_unknown_events_and_dialog_flags_are_preserved() {
    let file = atn::parse(FIXTURES[1].1).unwrap();
    assert_eq!(file.name.0, vec![0x0053, 0x0065, 0x0074, 0x0020, 0xd83d, 0xde80, 0xd800, 0]);
    assert!(file.expanded);
    assert_eq!(file.actions.len(), 2);
    let action = &file.actions[0];
    assert_eq!(action.name, UnicodeString::new_nul("Action Ω"));
    assert_eq!((action.function_key, action.shift, action.command, action.color, action.expanded), (12, true, true, 7, true));
    let blur = &action.items[0];
    assert_eq!((blur.expanded, blur.enabled, blur.with_dialog, blur.dialog_options), (true, true, true, 2));
    assert_eq!(blur.event, AtnEvent::Code(*b"GsnB"));
    assert_eq!(blur.descriptor, Some(Descriptor::new("null").with("Rds ", Value::UnitFloat { unit: *b"#Pxl", value: 2.5 })));
    let future = &action.items[1];
    assert_eq!(future.event, AtnEvent::Name("syntheticFutureEvent".into()));
    assert_eq!((future.expanded, future.enabled, future.with_dialog, future.dialog_options), (false, false, true, 255));
    assert_eq!(future.dictionary, b"Future event");
    assert!(future.descriptor.is_none());
    assert_eq!(file.actions[1].name, UnicodeString::new_nul("Empty"));
    assert!(file.actions[1].items.is_empty());
}

#[test]
fn every_descriptor_variant_and_reference_form_is_retained() {
    let expected = Descriptor::new("syntheticRoot")
        .with(
            "reference",
            Value::Reference(vec![
                ReferenceItem::Property { class: class("Lyr "), key: Id::new("Nm  ") },
                ReferenceItem::Class(class("Dcmn")),
                ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
                ReferenceItem::Offset { class: class("Lyr "), offset: -3 },
                ReferenceItem::Identifier { class: class("Lyr "), id: 0x1234_5678 },
                ReferenceItem::Index { class: class("Lyr "), index: 4 },
                ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Layer Ω") },
            ]),
        )
        .with("nested", Value::Descriptor(Descriptor::new("child").with("flag", Value::Boolean(true))))
        .with("global", Value::GlobalObject(Descriptor::new("global")))
        .with("list", Value::List(vec![Value::Boolean(false), Value::Double(-3.5), Value::Text(UnicodeString::new("Ω"))]))
        .with("double", Value::Double(1.25))
        .with("unit", Value::UnitFloat { unit: *b"#Prc", value: 37.5 })
        .with("units", Value::UnitFloats { unit: *b"#Pxl", values: vec![1.25, -0.0, -2.5] })
        .with("text", Value::Text(UnicodeString(vec![0x0054, 0x0065, 0x0078, 0x0074, 0x0020, 0xd83d, 0xde80, 0xdfff, 0])))
        .with("enum", Value::Enumerated { type_id: Id::new("Ordn"), value: Id::new("syntheticValue") })
        .with("integer", Value::Integer(i32::MIN))
        .with("large", Value::LargeInteger(0x0123_4567_89ab_cdef))
        .with("boolean", Value::Boolean(true))
        .with("class", Value::Class(class("Lyr ")))
        .with("globalClass", Value::GlobalClass(class("Dcmn")))
        .with("alias", Value::Alias(vec![0, 255, 1, 128]))
        .with("path", Value::Path(b"C:\\synthetic\\image.psd\0".to_vec()))
        .with("raw", Value::RawData(vec![0, 127, 128, 255]))
        .with(
            "objectArray",
            Value::ObjectArray(ObjectArray {
                prefix: 16,
                body: Descriptor::new("rationalPoint").with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: vec![1.0, 2.0] }),
            }),
        );
    let file = atn::parse(FIXTURES[2].1).unwrap();
    assert_eq!(file.actions[0].items[0].descriptor, Some(expected));
}

#[test]
fn nan_payloads_signed_zero_and_infinities_survive_without_false_fuzz_failures() {
    let file = atn::parse(FIXTURES[3].1).unwrap();
    let descriptor = file.actions[0].items[0].descriptor.as_ref().unwrap();
    for (key, bits) in [
        ("nan", 0x7ff8_0000_0000_1234),
        ("negativeZero", 0x8000_0000_0000_0000),
        ("positiveInfinity", 0x7ff0_0000_0000_0000),
        ("negativeInfinity", 0xfff0_0000_0000_0000),
    ] {
        let Some(Value::Double(value)) = descriptor.get(key) else { panic!("missing {key}") };
        assert_eq!(value.to_bits(), bits, "{key}");
    }
    // Ordinary AST PartialEq fails for NaN, despite this valid byte-exact round trip.
    assert_ne!(file.actions.clone(), file.actions);
    atn_roundtrip::check(FIXTURES[3].1);
}

#[test]
fn every_truncated_fixture_is_refused() {
    for (name, bytes) in FIXTURES {
        for end in 0..bytes.len() {
            assert!(atn::parse(&bytes[..end]).is_err(), "{name}: cut at {end}/{}", bytes.len());
        }
    }
}

#[test]
fn trailing_bytes_only_add_a_diagnostic() {
    for (name, bytes) in FIXTURES {
        let mut input = bytes.to_vec();
        input.extend_from_slice(b"extra");
        let mut parsed = atn::parse(&input).unwrap();
        assert_eq!(parsed.warnings, ["5 trailing bytes ignored"], "{name}");
        parsed.warnings.clear();
        assert_eq!(atn::write(&parsed).unwrap(), *bytes, "{name}");
        atn_roundtrip::check(&input);
    }
}

#[test]
fn unsupported_versions_and_invalid_item_tags_are_refused() {
    for version in [0_u32, 7, 11, 15, 17, u32::MAX] {
        let mut input = FIXTURES[0].1.to_vec();
        input[..4].copy_from_slice(&version.to_be_bytes());
        assert!(matches!(atn::parse(&input), Err(PsdError::Unsupported(_))));
    }
    // Version 12 (Photoshop 5) is read too (photocraft#10743): the smallest set parses to an empty set.
    let mut v12 = FIXTURES[0].1.to_vec();
    v12[..4].copy_from_slice(&12_u32.to_be_bytes());
    assert!(atn::parse(&v12).unwrap().actions.is_empty());
    let file = file_with(Descriptor::new("null"));
    let valid = atn::write(&file).unwrap();
    // The empty set/action names put the first step at offset 28.
    for kind in [*b"bad!", *b"LONG", [0; 4]] {
        let mut input = valid.clone();
        input[32..36].copy_from_slice(&kind);
        assert!(matches!(atn::parse(&input), Err(PsdError::Invalid(_))));
    }
    for flag in [-2_i32, 1, i32::MAX] {
        let mut input = valid.clone();
        input[44..48].copy_from_slice(&flag.to_be_bytes());
        assert!(matches!(atn::parse(&input), Err(PsdError::Invalid(_))));
    }
}

#[test]
fn declared_action_and_step_limits_are_enforced() {
    let mut actions = FIXTURES[0].1.to_vec();
    let count = (atn::MAX_ACTIONS + 1) as u32;
    actions[9..13].copy_from_slice(&count.to_be_bytes());
    actions.resize(13 + count as usize * 15, 0);
    assert!(matches!(atn::parse(&actions), Err(PsdError::LimitExceeded("too many actions in one set"))));

    let mut steps = atn::write(&AtnFile { actions: vec![AtnAction::default()], ..Default::default() }).unwrap();
    let count = (atn::MAX_ITEMS + 1) as u32;
    steps[24..28].copy_from_slice(&count.to_be_bytes());
    steps.resize(28 + count as usize * 20, 0);
    assert!(matches!(atn::parse(&steps), Err(PsdError::LimitExceeded("too many steps in one action"))));

    let mut short = FIXTURES[0].1.to_vec();
    short[9..13].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(atn::parse(&short), Err(PsdError::UnexpectedEof { .. })));
}

#[test]
fn nested_descriptors_obey_the_parser_depth_limit() {
    let mut desc = Descriptor::new("leaf").with("flag", Value::Boolean(true));
    for _ in 0..2 {
        desc = Descriptor::new("node").with("next", Value::Descriptor(desc));
    }
    let shallow = atn::write(&file_with(desc.clone())).unwrap();
    atn_roundtrip::check(&shallow);
    assert!(atn::parse(&shallow).is_ok());
    for _ in 0..MAX_DEPTH + 2 {
        desc = Descriptor::new("node").with("next", Value::Descriptor(desc));
    }
    let deep = atn::write(&file_with(desc)).unwrap();
    assert!(matches!(atn::parse(&deep), Err(PsdError::LimitExceeded(_))));
}

#[test]
fn generated_metadata_and_float_patterns_round_trip() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for index in 0..256_u16 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let mut file = file_with(Descriptor::new("generated").with("value", Value::Double(f64::from_bits(seed))));
        file.name = UnicodeString(vec![index, 0xd800 | index, 0, 0xffff]);
        file.expanded = index & 1 != 0;
        let action = &mut file.actions[0];
        action.function_key = index;
        action.shift = index & 2 != 0;
        action.command = index & 4 != 0;
        action.color = index;
        action.expanded = index & 8 != 0;
        action.name = UnicodeString(vec![0xdc00 | index, index]);
        let item = &mut action.items[0];
        item.expanded = index & 16 != 0;
        item.enabled = index & 32 != 0;
        item.with_dialog = index & 64 != 0;
        item.dialog_options = index as u8;
        atn_roundtrip::check(&atn::write(&file).unwrap());
    }
}

#[test]
fn actual_fuzz_invariant_handles_deterministic_seed_mutations() {
    for (_, bytes) in FIXTURES {
        for index in 0..128 {
            let mut mutated = bytes.to_vec();
            let at = index * 47 % mutated.len();
            mutated[at] ^= (index as u8).wrapping_mul(13).wrapping_add(1);
            atn_roundtrip::check(&mutated);
        }
    }
}

use craft_psd::PsdError;
use craft_psd::atn;
use craft_psd::descriptor::{Class, Descriptor, Id, MAX_DEPTH, ObjectArray, ReferenceItem as R, UnicodeString, Value as V};

const STOP: &[u8] = include_bytes!("../fixtures/atn/v12-stop.atn");
const VALUES: &[u8] = include_bytes!("../fixtures/atn/v12-values.atn");
const COMMANDS: &[u8] = include_bytes!("../fixtures/atn/v12-commands.atn");

fn code(id: &[u8; 4]) -> Id {
    Id::Code(*id)
}

fn class(id: &[u8; 4]) -> Class {
    Class { name: UnicodeString::default(), class_id: code(id) }
}

fn descriptor(id: &[u8; 4], items: Vec<(Id, V)>) -> Descriptor {
    Descriptor { name: UnicodeString::default(), class_id: code(id), items }
}

/// The fixture bytes are independent of the version 16 writer and legacy reader.
#[test]
fn stop_and_named_event_preserve_headers_and_canonicalize_to_v16() {
    let parsed = atn::parse(STOP).unwrap();
    assert_eq!(parsed.name, UnicodeString::new_nul("Legacy"));
    let action = &parsed.actions[0];
    assert_eq!((action.function_key, action.shift, action.command, action.color), (3, true, false, 2));
    assert_eq!(action.name, UnicodeString::new_nul("Run"));
    let step = &action.items[0];
    assert_eq!((step.expanded, step.enabled, step.with_dialog, step.dialog_options), (true, true, false, 2));
    assert_eq!(step.event, atn::AtnEvent::Code(*b"Stop"));
    assert_eq!(
        step.descriptor,
        Some(descriptor(b"null", vec![(code(b"Msge"), V::Text(UnicodeString::new_nul("Review ✓"))), (code(b"Cntn"), V::Boolean(true)),]))
    );
    let plugin = atn::parse(VALUES).unwrap();
    assert_eq!(plugin.actions[0].items[0].event, atn::AtnEvent::name("LegacyPlugin"));
    assert!(plugin.actions[0].items[1].descriptor.is_none());
    for bytes in [STOP, VALUES, COMMANDS] {
        let original = atn::parse(bytes).unwrap();
        let modern = atn::write(&original).unwrap();
        assert_eq!(&modern[..4], &16_u32.to_be_bytes());
        assert_eq!(atn::parse(&modern).unwrap(), original);
    }
}

#[test]
fn all_legacy_values_and_reference_classes_are_retained() {
    let parsed = atn::parse(VALUES).unwrap();
    let actual = parsed.actions[0].items[0].descriptor.as_ref().unwrap();
    let floats = |values| V::UnitFloats { unit: *b"#Pxl", values };
    let expected = descriptor(
        b"null",
        vec![
            (code(b"warp"), V::Boolean(true)),
            (code(b"list"), V::GlobalClass(class(b"list"))),
            (code(b"time"), V::Enumerated { type_id: code(b"time"), value: code(b"warp") }),
            (code(b"long"), V::Integer(-17)),
            (code(b"comp"), V::LargeInteger(i64::MIN + 23)),
            (code(b"doub"), V::Double(-0.5)),
            (code(b"unit"), V::UnitFloat { unit: *b"#Pxl", value: 2.5 }),
            (code(b"TEXT"), V::Text(UnicodeString(vec![0xd800, 0]))),
            (code(b"Objc"), V::Descriptor(descriptor(b"warp", vec![(code(b"flag"), V::Boolean(false))]))),
            (code(b"GlbO"), V::GlobalObject(descriptor(b"list", vec![]))),
            (code(b"type"), V::Class(class(b"time"))),
            (code(b"VlLs"), V::List(vec![V::Boolean(false), V::Descriptor(descriptor(b"time", vec![]))])),
            (
                code(b"ObAr"),
                V::ObjectArray(ObjectArray {
                    prefix: 3,
                    body: descriptor(b"time", vec![(code(b"Hrzn"), floats(vec![1.0, 2.0, 3.0])), (code(b"Vrtc"), floats(vec![4.0, 5.0, 6.0]))]),
                }),
            ),
            (
                code(b"refs"),
                V::Reference(vec![
                    R::Property { class: class(b"Lyr "), key: code(b"Nm  ") },
                    R::Class(class(b"Dcmn")),
                    R::Enumerated { class: class(b"Lyr "), type_id: code(b"Ordn"), value: code(b"Trgt") },
                    R::Offset { class: class(b"Lyr "), offset: -2 },
                    R::Index { class: class(b"Lyr "), index: 3 },
                    R::Identifier { class: class(b"Lyr "), id: 9 },
                    R::Name { class: class(b"Lyr "), name: UnicodeString::new_nul("Layer ✓") },
                ]),
            ),
            (code(b"Pth "), V::Path(vec![0, 1, 255, 3])),
            (code(b"alis"), V::Alias(b"alias".to_vec())),
            (code(b"tdta"), V::RawData(vec![])),
        ],
    );
    assert_eq!(actual, &expected);
}

fn with_descriptor(body: &[u8]) -> Vec<u8> {
    let start = STOP.windows(4).position(|w| w == [255; 4]).unwrap() + 4;
    let mut bytes = STOP[..start].to_vec();
    bytes.extend_from_slice(body);
    bytes
}

/// Reject every truncated fixture while preserving trailing data as a warning.
#[test]
fn truncations_fail_and_trailing_bytes_only_warn() {
    for bytes in [STOP, VALUES, COMMANDS] {
        for cut in 0..bytes.len() {
            assert!(atn::parse(&bytes[..cut]).is_err(), "length {}, cut {cut}", bytes.len());
        }
        let mut padded = bytes.to_vec();
        padded.extend_from_slice(b"junk");
        let mut parsed = atn::parse(&padded).unwrap();
        assert_eq!(parsed.warnings, ["4 trailing bytes ignored"]);
        parsed.warnings.clear();
        assert_eq!(parsed, atn::parse(bytes).unwrap());
    }
}

/// Reject excessive counts/depth and unsupported forms without dialect fallback.
#[test]
fn legacy_counts_depth_and_unknown_forms_fail_without_fallback() {
    assert!(atn::parse(&with_descriptor(&u32::MAX.to_be_bytes())).is_err());
    for prefix in [b"VlLs".as_slice(), b"obj ", b"UnFl#Pxl", b"TEXT", b"Pth ", b"ObArtime\0\0\0\x03"] {
        let mut body = b"\0\0\0\x01test".to_vec();
        body.extend_from_slice(prefix);
        body.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(atn::parse(&with_descriptor(&body)).is_err());
    }
    // The smallest descriptor item is a bare key/type and one Boolean byte.
    assert!(atn::parse(&with_descriptor(b"\0\0\0\x01testbool\x01")).is_ok());
    for ty in [b"VlLs\0\0\0\x01".as_slice(), b"Objcnull\0\0\0\x01test"] {
        let mut body = b"\0\0\0\x01test".to_vec();
        for _ in 0..=MAX_DEPTH {
            body.extend_from_slice(ty);
        }
        body.extend_from_slice(b"bool\x01");
        assert!(matches!(atn::parse(&with_descriptor(&body)), Err(PsdError::LimitExceeded(_))));
    }
    for body in [b"\0\0\0\x01testwhat\0".as_slice(), b"\0\0\0\x01testobj \0\0\0\x01whatLyr "] {
        assert!(matches!(atn::parse(&with_descriptor(body)), Err(PsdError::Unsupported(_))));
    }
    for version in [0_u32, 7, 11, 13, 17, 0x0005_1600] {
        let mut bytes = STOP.to_vec();
        bytes[..4].copy_from_slice(&version.to_be_bytes());
        assert!(matches!(atn::parse(&bytes), Err(PsdError::Unsupported(message)) if message.contains(&version.to_string())));
    }
}

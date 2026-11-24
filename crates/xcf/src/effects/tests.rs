//! Independently framed records; expectations never come from this parser.
use super::*;
use std::cell::Cell;

const OWNER: Owner = Owner { kind: OwnerKind::Layer, offset: 4 };
fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn string(out: &mut Vec<u8>, value: &str) {
    word(out, value.len() as u32 + 1);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}
fn property(out: &mut Vec<u8>, kind: u32, bytes: &[u8]) {
    word(out, kind);
    word(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}
fn text_value(value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    string(&mut out, value);
    out
}
fn effect(version: u32, args: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0; 8];
    for value in ["Stored effect", "", "example:inert-operation"] {
        string(&mut out, value);
    }
    if version >= 22 {
        string(&mut out, "1:0");
    }
    for (name, kind, bytes) in args {
        let mut payload = text_value(name);
        word(&mut payload, *kind);
        payload.extend_from_slice(bytes);
        property(&mut out, 45, &payload);
    }
    property(&mut out, 8, &0u32.to_be_bytes());
    property(&mut out, 33, &0.375f32.to_be_bytes());
    property(&mut out, 7, &62u32.to_be_bytes());
    property(&mut out, 36, &(-1i32).to_be_bytes());
    property(&mut out, 9876, &[9, 8, 7]);
    word(&mut out, 0);
    word(&mut out, 0);
    out.extend_from_slice(&0u64.to_be_bytes());
    out
}
fn read(bytes: &[u8], version: u32) -> OwnedEffect<'_> {
    Reader::new(bytes, version).read(OWNER, 8, &mut Budget::default(), &|| true).unwrap()
}

#[test]
fn source_owner_versions_envelope_and_unknown_properties_remain_inert() {
    for version in [20, 21, 22, 26] {
        let bytes = effect(version, &[]);
        let parsed = read(&bytes, version);
        assert_eq!(parsed.owner, OWNER);
        let effect = parsed.record;
        assert_eq!(effect.operation, "example:inert-operation");
        assert_eq!(effect.version, if version >= 22 { Some("1:0") } else { None });
        assert!(!effect.visible);
        assert_eq!((effect.opacity, effect.mode, effect.composite_space), (0.375, 62, -1));
        assert_eq!(effect.unknown_properties[0].kind, 9876);
        assert_eq!(effect.unknown_properties[0].bytes, &[9, 8, 7]);
        assert_eq!(effect.raw, &bytes[8..]);
        assert!(effect.errors.is_empty());
    }
}

#[test]
fn arguments_preserve_order_duplicate_names_types_and_borrowed_unknown_bytes() {
    let bytes = effect(
        26,
        &[
            ("same", 1, (-5i32).to_be_bytes().to_vec()),
            ("same", 7, u32::MAX.to_be_bytes().to_vec()),
            ("flag", 2, 1u32.to_be_bytes().to_vec()),
            ("amount", 3, 1.5f32.to_be_bytes().to_vec()),
            ("label", 4, text_value("original")),
            ("enum", 5, (-3i32).to_be_bytes().to_vec()),
            ("raw", 999, vec![0xde, 0xad]),
            ("missing-color", 8, vec![0; 4]),
        ],
    );
    let parsed = read(&bytes, 26).record;
    assert_eq!(
        parsed.arguments.iter().map(|arg| arg.name).collect::<Vec<_>>(),
        ["same", "same", "flag", "amount", "label", "enum", "raw", "missing-color"]
    );
    assert!(matches!(parsed.arguments[0].value, ArgumentValue::Int(-5)));
    assert!(matches!(parsed.arguments[1].value, ArgumentValue::UInt(u32::MAX)));
    assert!(matches!(parsed.arguments[2].value, ArgumentValue::Bool(true)));
    assert!(matches!(parsed.arguments[3].value, ArgumentValue::Float(value) if value == 1.5));
    assert!(matches!(parsed.arguments[4].value, ArgumentValue::String("original")));
    assert!(matches!(parsed.arguments[5].value, ArgumentValue::Enum(-3)));
    let ArgumentValue::Unknown { kind, bytes: raw } = &parsed.arguments[6].value else { panic!("unknown source value") };
    assert_eq!((*kind, *raw), (999, &[0xde, 0xad][..]));
    assert!(
        (bytes.as_ptr() as usize..bytes.as_ptr() as usize + bytes.len()).contains(&(raw.as_ptr() as usize)),
        "unknown payload borrows backing input"
    );
    assert!(matches!(parsed.arguments[7].value, ArgumentValue::Color(None)));
}

#[test]
fn finite_scalar_color_and_config_failures_do_not_hide_later_arguments() {
    let mut trailing = 0.25f32.to_be_bytes().to_vec();
    trailing.push(0xff);
    let mut color = text_value("RGB float");
    word(&mut color, 12);
    color.extend([0; 12]);
    word(&mut color, 3);
    color.extend([1, 2, 3]);
    let bytes = effect(
        26,
        &[
            ("nan", 3, f32::NAN.to_be_bytes().to_vec()),
            ("boolean", 2, 2u32.to_be_bytes().to_vec()),
            ("trailing", 3, trailing),
            ("config", 6, text_value("(named 1) ()")),
            ("color", 8, color),
            ("later", 7, 17u32.to_be_bytes().to_vec()),
        ],
    );
    let parsed = read(&bytes, 26).record;
    for argument in &parsed.arguments[..4] {
        let ArgumentValue::Invalid { message, .. } = &argument.value else { panic!("invalid optional argument") };
        assert!(!message.is_empty() && message.len() < 256);
    }
    let ArgumentValue::Color(Some(color)) = &parsed.arguments[4].value else { panic!("source color metadata") };
    assert_eq!((color.encoding, color.pixels, color.profile), ("RGB float", &[0; 12][..], &[1, 2, 3][..]));
    assert!(matches!(parsed.arguments[5].value, ArgumentValue::UInt(17)));
}

#[test]
fn configurations_are_ordered_bounded_data_not_graph_evaluation() {
    let raw = r#"# comment
(channel 1) (channel 2) (color "\000\200\377") (nested (value 1.25))"#;
    let bytes = effect(26, &[("config", 6, text_value(raw))]);
    let parsed = read(&bytes, 26).record;
    let ArgumentValue::Config(config) = &parsed.arguments[0].value else { panic!("inert configuration") };
    assert_eq!(config.raw, raw);
    assert_eq!(config.fields("channel").map(|values| values[0].as_number().unwrap()).collect::<Vec<_>>(), [1., 2.]);
    assert!(matches!(&config.forms[2].as_list().unwrap()[1], ConfigExpr::Bytes(bytes) if bytes == &[0, 128, 255]));
    for raw in [format!("{}0{}", "(x ".repeat(34), ")".repeat(34)), format!("(x {})", "0 ".repeat(8192)), "(x NaN)".into()] {
        let bytes = effect(26, &[("config", 6, text_value(&raw))]);
        assert!(matches!(read(&bytes, 26).record.arguments[0].value, ArgumentValue::Invalid { .. }));
    }
}

#[test]
fn mask_references_keep_wire_identity_without_following_auxiliary_pixels() {
    let original = effect(26, &[]);
    for offset in [8, original.len() as u64, u64::MAX] {
        let mut bytes = original.clone();
        let at = bytes.len() - 8;
        bytes[at..].copy_from_slice(&offset.to_be_bytes());
        let parsed = read(&bytes, 26).record;
        assert_eq!(parsed.mask_offset, offset);
        assert_eq!(parsed.errors.is_empty(), offset == 8);
        assert_eq!(parsed.raw, &bytes[8..]);
    }
}

#[test]
fn pointer_lists_preserve_repeated_wire_entries_but_bound_attempts() {
    for version in [10, 26] {
        let pointer = |out: &mut Vec<u8>, value: u64| {
            if version < 11 {
                word(out, value as u32);
            } else {
                out.extend_from_slice(&value.to_be_bytes());
            }
        };
        let mut bytes = Vec::new();
        for value in [80, 32, 91, 0] {
            pointer(&mut bytes, value);
        }
        assert_eq!(Reader::new(&bytes, version).pointers(0, &mut Budget::default(), &|| true).unwrap(), [80, 32, 91]);
        let repeated_pointer = if version < 11 { 80 } else { 0x1_0000_0050 };
        let mut repeated = Vec::new();
        for value in [repeated_pointer, 32, repeated_pointer, 0] {
            pointer(&mut repeated, value);
        }
        let mut budget = Budget::default();
        let before = budget.remaining_storage();
        assert_eq!(Reader::new(&repeated, version).pointers(0, &mut budget, &|| true).unwrap(), [repeated_pointer, 32, repeated_pointer]);
        assert!(budget.remaining_storage() < before);
        assert!(budget.remaining_work() < Budget::default().remaining_work());
        let mut many = Vec::new();
        for value in 1..=1025 {
            pointer(&mut many, value);
        }
        pointer(&mut many, 0);
        assert!(Reader::new(&many, version).pointers(0, &mut Budget::default(), &|| true).is_err());
    }
}

#[test]
fn failed_records_refund_only_storage_and_exhaust_shared_work() {
    let mut bytes = effect(26, &[]);
    bytes.pop();
    let mut budget = Budget::new(1 << 20, 1024);
    let reader = Reader::new(&bytes, 26);
    let before = budget.remaining_storage();
    assert!(matches!(reader.read(OWNER, 8, &mut budget, &|| true), Err(Error::Source(crate::Error::Invalid(_)))));
    assert_eq!(budget.remaining_storage(), before);
    assert!(budget.remaining_work() < 1024);
    for _ in 0..1024 {
        if matches!(reader.read(OWNER, 8, &mut budget, &|| true), Err(Error::Limit(Limit::Work))) {
            break;
        }
    }
    assert_eq!(budget.remaining_work(), 0);
    assert_eq!(budget.remaining_storage(), before);
    let valid = effect(26, &[]);
    assert!(matches!(Reader::new(&valid, 26).read(OWNER, 8, &mut budget, &|| true), Err(Error::Limit(Limit::Work))));
    let mut budget = Budget::new(1, 1024);
    assert!(matches!(Reader::new(&valid, 26).read(OWNER, 8, &mut budget, &|| true), Err(Error::Limit(Limit::Storage))));
    assert_eq!(budget.remaining_storage(), 1);
}

#[test]
fn cancellation_stays_shared_and_is_not_refunded_as_optional_damage() {
    let raw = format!("(text \"{}\")", "a".repeat(32_000));
    let bytes = effect(26, &[("config", 6, text_value(&raw))]);
    let reader = Reader::new(&bytes, 26);
    let mut budget = Budget::default();
    assert!(matches!(reader.read(OWNER, 8, &mut budget, &|| false), Err(Error::Source(crate::Error::Cancelled))));
    assert_eq!(budget.remaining_storage(), Budget::default().remaining_storage());
    let checks = Cell::new(0);
    let keep_going = || {
        checks.set(checks.get() + 1);
        checks.get() < 6
    };
    assert!(matches!(reader.read(OWNER, 8, &mut budget, &keep_going), Err(Error::Source(crate::Error::Cancelled))));
    assert!(budget.remaining_storage() < Budget::default().remaining_storage());
    assert!(budget.remaining_work() < Budget::default().remaining_work());
}

#[test]
fn essential_truncation_and_oversized_properties_are_errors_without_partial_records() {
    let bytes = effect(26, &[("finite", 3, 0.5f32.to_be_bytes().to_vec())]);
    for end in 0..bytes.len() {
        let mut budget = Budget::default();
        assert!(Reader::new(&bytes[..end], 26).read(OWNER, 8, &mut budget, &|| true).is_err(), "truncated at {end}");
        assert_eq!(budget.remaining_storage(), Budget::default().remaining_storage());
    }
    let mut oversized = vec![0; 8];
    for value in ["Name", "", "example:unknown", "1"] {
        string(&mut oversized, value);
    }
    word(&mut oversized, 45);
    word(&mut oversized, (crate::MAX_PAYLOAD + 1) as u32);
    // Enough work to reach the absent payload if the declaration cap regresses.
    let mut budget = Budget::new(1 << 20, crate::MAX_PAYLOAD + 4096);
    assert!(matches!(Reader::new(&oversized, 26).read(OWNER, 8, &mut budget, &|| true),
        Err(Error::Source(crate::Error::Invalid(message))) if message.contains("stored effect property exceeds 64 MiB")));
    // Use the same otherwise valid envelope for both versions. A v26 envelope
    // would fail v19 framing even if the version guard were accidentally removed.
    let v20 = effect(20, &[("finite", 3, 0.5f32.to_be_bytes().to_vec())]);
    let accepted = Reader::new(&v20, 20).read(OWNER, 8, &mut Budget::default(), &|| true).unwrap();
    assert_eq!(accepted.record.version, None);
    assert!(matches!(accepted.record.arguments[0].value, ArgumentValue::Float(0.5)));
    let rejected = Reader::new(&v20, 19).read(OWNER, 8, &mut Budget::default(), &|| true).unwrap_err();
    assert_eq!(rejected, Error::Source(crate::Error::Invalid("stored effect envelopes are not defined before XCF 20".into())));
}

#[test]
fn property_counts_and_declared_argument_sizes_are_bounded_before_payload_reads() {
    let original = effect(26, &[]); // Five independently framed properties.
    for extra in [4091, 4092] {
        let mut bytes = original.clone();
        let mut properties = Vec::new();
        for _ in 0..extra {
            property(&mut properties, 9000, &[]);
        }
        let at = bytes.len() - 16; // END and the mask pointer stay last.
        drop(bytes.splice(at..at, properties));
        let result = Reader::new(&bytes, 26).read(OWNER, 8, &mut Budget::default(), &|| true);
        assert_eq!(result.is_ok(), extra == 4091);
    }
    let mut huge_pixel = text_value("RGB float");
    word(&mut huge_pixel, 257);
    let mut huge_profile = text_value("RGB float");
    word(&mut huge_profile, 0);
    word(&mut huge_profile, (crate::MAX_PAYLOAD + 1) as u32);
    let bytes = effect(
        26,
        &[
            ("huge-string", 4, ((crate::MAX_STRING + 1) as u32).to_be_bytes().to_vec()),
            ("huge-color", 8, huge_pixel),
            ("huge-profile", 8, huge_profile),
            ("later", 7, 17u32.to_be_bytes().to_vec()),
        ],
    );
    let parsed = read(&bytes, 26).record;
    for (argument, expected) in
        parsed.arguments[..3].iter().zip(["effect string exceeds 1 MiB", "effect color pixel exceeds 256 bytes", "effect color ICC exceeds 64 MiB"])
    {
        assert!(matches!(&argument.value, ArgumentValue::Invalid { message, .. } if message.contains(expected)), "{argument:?}");
    }
    assert!(matches!(parsed.arguments[3].value, ArgumentValue::UInt(17)));
}

#[test]
fn null_colors_accept_both_documented_wire_forms_but_not_partial_words() {
    for version in [20, 26] {
        let bytes = effect(
            version,
            &[
                ("old-null", 8, Vec::new()),
                ("new-null", 8, vec![0; 4]),
                ("partial-one", 8, vec![0]),
                ("partial-three", 8, vec![0; 3]),
                ("later", 1, 7u32.to_be_bytes().to_vec()),
            ],
        );
        let parsed = read(&bytes, version);
        assert!(matches!(parsed.record.arguments[0].value, ArgumentValue::Color(None)));
        assert!(matches!(parsed.record.arguments[1].value, ArgumentValue::Color(None)));
        for index in [2, 3] {
            assert!(matches!(parsed.record.arguments[index].value, ArgumentValue::Invalid { .. }));
        }
        assert!(matches!(parsed.record.arguments[4].value, ArgumentValue::Int(7)));
        assert_eq!(parsed.record.raw, &bytes[8..]);
    }
}

#[test]
fn trailing_configuration_is_invalid_before_retention_and_keeps_later_arguments() {
    let raw = "(x) ".repeat(100);
    let valid = effect(26, &[("config", 6, text_value(&raw))]);
    assert!(matches!(Reader::new(&valid, 26).read(OWNER, 8, &mut Budget::new(2048, 1 << 20), &|| true), Err(Error::Limit(Limit::Storage))));
    let mut malformed = text_value(&raw);
    malformed.push(9);
    let bytes = effect(26, &[("config", 6, malformed), ("later", 1, 7u32.to_be_bytes().to_vec())]);
    let parsed = Reader::new(&bytes, 26).read(OWNER, 8, &mut Budget::new(2048, 1 << 20), &|| true).unwrap();
    assert!(matches!(&parsed.record.arguments[0].value, ArgumentValue::Invalid { message, .. } if message.contains("trailing")));
    assert!(matches!(parsed.record.arguments[1].value, ArgumentValue::Int(7)));
    assert_eq!(parsed.record.raw, &bytes[8..]);
}

#[test]
fn repeated_decodable_pointers_keep_both_applications_and_the_later_record() {
    for version in [20, 26] {
        let first = effect(version, &[("sequence", 1, 1u32.to_be_bytes().to_vec())]);
        let later = effect(version, &[("sequence", 1, 2u32.to_be_bytes().to_vec())]);
        let first_offset = 32u64;
        let later_offset = first_offset + first.len() as u64 - 8;
        let mut bytes = Vec::new();
        for offset in [first_offset, first_offset, later_offset, 0] {
            bytes.extend_from_slice(&offset.to_be_bytes());
        }
        bytes.extend_from_slice(&first[8..]);
        bytes.extend_from_slice(&later[8..]);
        let reader = Reader::new(&bytes, version);
        let mut budget = Budget::default();
        let offsets = reader.pointers(0, &mut budget, &|| true).unwrap();
        assert_eq!(offsets, [first_offset, first_offset, later_offset]);
        let values = offsets
            .into_iter()
            .map(|offset| {
                let parsed = reader.read(OWNER, offset, &mut budget, &|| true).unwrap();
                match parsed.record.arguments[0].value {
                    ArgumentValue::Int(value) => value,
                    _ => panic!("integer"),
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(values, [1, 1, 2]);
    }
}

#[test]
fn string_null_and_empty_and_color_empty_encoding_remain_distinct() {
    for version in [20, 26] {
        let bytes = effect(
            version,
            &[
                ("null-text", 4, vec![0; 4]),
                ("empty-text", 4, text_value("")),
                ("empty-color-encoding", 8, text_value("")),
                ("null-color", 8, vec![0; 4]),
                ("later", 1, 7u32.to_be_bytes().to_vec()),
            ],
        );
        let parsed = read(&bytes, version).record;
        assert!(matches!(parsed.arguments[0].value, ArgumentValue::NullString));
        assert!(matches!(parsed.arguments[1].value, ArgumentValue::String("")));
        assert!(matches!(parsed.arguments[2].value, ArgumentValue::Invalid { .. }));
        assert!(matches!(parsed.arguments[3].value, ArgumentValue::Color(None)));
        assert!(matches!(parsed.arguments[4].value, ArgumentValue::Int(7)));
        assert_eq!(parsed.raw, &bytes[8..]);
    }
}

#[test]
fn malformed_first_config_form_does_not_spend_retention_on_the_suffix() {
    for first in ["()", "(1)", "not-a-list"] {
        let raw = format!("{first} {}", "(x) ".repeat(100));
        let bytes = effect(26, &[("config", 6, text_value(&raw)), ("later", 1, 7u32.to_be_bytes().to_vec())]);
        let parsed = Reader::new(&bytes, 26).read(OWNER, 8, &mut Budget::new(2048, 1 << 20), &|| true).unwrap().record;
        assert!(matches!(&parsed.arguments[0].value, ArgumentValue::Invalid { message, .. } if message.contains("named lists")));
        assert!(matches!(parsed.arguments[1].value, ArgumentValue::Int(7)));
    }
}

#[test]
fn operation_version_preserves_absence_empty_and_unknown_constraints() {
    for version in [22, 26] {
        for (wire, expected) in
            [(vec![0; 4], None), (text_value(""), Some("")), (text_value("1:0"), Some("1:0")), (text_value("future"), Some("future"))]
        {
            let mut bytes = effect(version, &[("later", 1, 7u32.to_be_bytes().to_vec())]);
            let start = 8 + text_value("Stored effect").len() + text_value("").len() + text_value("example:inert-operation").len();
            drop(bytes.splice(start..start + text_value("1:0").len(), wire));
            let parsed = read(&bytes, version).record;
            assert_eq!(parsed.version, expected);
            assert!(matches!(parsed.arguments[0].value, ArgumentValue::Int(7)));
            assert_eq!(parsed.operation, "example:inert-operation");
            assert_eq!(parsed.raw, &bytes[8..]);
        }
    }
}

#[test]
fn malformed_operation_version_never_becomes_an_absent_constraint() {
    let start = 8 + text_value("Stored effect").len() + text_value("").len() + text_value("example:inert-operation").len();
    for wire in [vec![0, 0], vec![0, 0, 0, 1, 1], vec![0, 0, 0, 2, 0, 0], vec![0, 0, 0, 2, 255, 0]] {
        let mut bytes = effect(26, &[]);
        if wire.len() < 4 {
            // Only the incomplete length-word case deliberately ends the file.
            bytes.truncate(start);
            bytes.extend(wire);
        } else {
            // Keep the valid properties and mask pointer: accepting malformed
            // version text must not fail later for an unrelated missing suffix.
            drop(bytes.splice(start..start + text_value("1:0").len(), wire));
        }
        let mut budget = Budget::default();
        let before = budget.remaining_storage();
        assert!(matches!(Reader::new(&bytes, 26).read(OWNER, 8, &mut budget, &|| true), Err(Error::Source(crate::Error::Invalid(_)))));
        assert_eq!(budget.remaining_storage(), before);
    }
}

#[test]
fn null_and_empty_configs_are_invalid_without_hiding_later_values_or_source_bytes() {
    for version in [20, 26] {
        let null = vec![0; 4];
        let empty = text_value("");
        let bytes = effect(
            version,
            &[
                ("null-config", 6, null.clone()),
                ("empty-config", 6, empty.clone()),
                ("valid-config", 6, text_value("(x 1)")),
                ("later", 1, 7u32.to_be_bytes().to_vec()),
            ],
        );
        let parsed = read(&bytes, version).record;
        for (argument, wire) in parsed.arguments[..2].iter().zip([null.as_slice(), empty.as_slice()]) {
            assert!(matches!(&argument.value, ArgumentValue::Invalid { kind: 6, bytes, message }
                if *bytes == wire && message.contains("null or empty")));
        }
        assert!(matches!(&parsed.arguments[2].value, ArgumentValue::Config(config) if config.raw == "(x 1)" && config.forms.len() == 1));
        assert!(matches!(parsed.arguments[3].value, ArgumentValue::Int(7)));
        assert_eq!(parsed.raw, &bytes[8..]);
    }
}

#[test]
fn missing_argument_type_retains_later_arguments_mask_and_exact_envelope() {
    for tail in [vec![], vec![0], vec![0, 0, 0]] {
        let mut bytes = effect(26, &[("later", 1, 7u32.to_be_bytes().to_vec())]);
        let at = 8 + text_value("Stored effect").len() + text_value("").len() + text_value("example:inert-operation").len() + text_value("1:0").len();
        let mut payload = text_value("missing-type");
        payload.extend(tail);
        let mut malformed = Vec::new();
        property(&mut malformed, 45, &payload);
        drop(bytes.splice(at..at, malformed));
        let mask = bytes.len() - 8;
        bytes[mask..].copy_from_slice(&8u64.to_be_bytes());
        let parsed = read(&bytes, 26).record;
        assert_eq!(parsed.arguments.len(), 1);
        assert!(matches!(parsed.arguments[0].value, ArgumentValue::Int(7)));
        assert_eq!(parsed.mask_offset, 8);
        assert_eq!(parsed.errors, ["effect argument is missing its type word"]);
        assert_eq!(parsed.raw, &bytes[8..]);
    }
}

#[test]
fn effect_icons_preserve_null_empty_and_named_wire_forms() {
    for version in [20, 26] {
        for (wire, expected) in [(vec![0; 4], None), (text_value(""), Some("")), (text_value("example-icon"), Some("example-icon"))] {
            let mut bytes = effect(version, &[("later", 1, 7u32.to_be_bytes().to_vec())]);
            let at = 8 + text_value("Stored effect").len();
            drop(bytes.splice(at..at + text_value("").len(), wire));
            let parsed = read(&bytes, version).record;
            assert_eq!(parsed.icon, expected);
            assert_eq!(parsed.version, if version >= 22 { Some("1:0") } else { None });
            assert!(matches!(parsed.arguments[0].value, ArgumentValue::Int(7)));
            assert_eq!(parsed.raw, &bytes[8..]);
        }
    }
}

fn with_leading_argument_payload(payload: &[u8]) -> Vec<u8> {
    let mut bytes = effect(26, &[("later", 1, 7u32.to_be_bytes().to_vec())]);
    let at = 8 + text_value("Stored effect").len() + text_value("").len() + text_value("example:inert-operation").len() + text_value("1:0").len();
    let mut malformed = Vec::new();
    property(&mut malformed, 45, payload);
    drop(bytes.splice(at..at, malformed));
    let mask = bytes.len() - 8;
    bytes[mask..].copy_from_slice(&8u64.to_be_bytes());
    bytes
}

#[test]
fn malformed_argument_names_skip_only_their_framed_property_and_keep_resource_stops() {
    for payload in [vec![0, 0, 0, 5], vec![0, 0, 0, 2, 255, 0], vec![0, 0, 0, 1, 65], vec![0, 0, 0, 3, 0, 1, 0]] {
        let bytes = with_leading_argument_payload(&payload);
        let parsed = read(&bytes, 26).record;
        assert_eq!(parsed.arguments.len(), 1);
        assert!(matches!(parsed.arguments[0].value, ArgumentValue::Int(7)));
        assert_eq!(parsed.mask_offset, 8);
        assert_eq!(parsed.errors, ["effect argument has a malformed name"]);
        assert_eq!(parsed.raw, &bytes[8..]);
        assert!(matches!(Reader::new(&bytes, 26).read(OWNER, 8, &mut Budget::new(4096, 1), &|| true), Err(Error::Limit(Limit::Work))));
        let calls = Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() < 4
        };
        let mut budget = Budget::default();
        let work = budget.remaining_work();
        // Reader admission and envelope checks pass, as does the first
        // property check. Cancel at the next property, after its malformed
        // predecessor has been read and skipped, before decoding "later".
        assert!(matches!(Reader::new(&bytes, 26).read(OWNER, 8, &mut budget, &keep), Err(Error::Source(crate::Error::Cancelled))));
        assert_eq!(calls.get(), 4);
        let headers = ["Stored effect", "", "example:inert-operation", "1:0"].iter().map(|name| text_value(name).len()).sum::<usize>();
        let declared_name = u32::from_be_bytes(payload[..4].try_into().unwrap()) as usize;
        assert_eq!(work - budget.remaining_work(), 64 + headers + 8 + payload.len() + 4 + declared_name);
    }
}

#[test]
fn argument_name_work_stop_wins_over_optional_diagnostic_retention() {
    let payload = 5u32.to_be_bytes();
    let bytes = with_leading_argument_payload(&payload);
    let header_bytes =
        text_value("Stored effect").len() + text_value("").len() + text_value("example:inert-operation").len() + text_value("1:0").len();
    // Admit the envelope, outer property and name length word, but not the
    // declared name data. No storage remains for a fabricated optional warning.
    let work = 64 + header_bytes + 8 + payload.len() + 4;
    let storage = 2 * std::mem::size_of::<StoredEffect<'_>>() + "Stored effect".len() + "example:inert-operation".len() + "1:0".len();
    let mut budget = Budget::new(storage, work);
    assert!(matches!(Reader::new(&bytes, 26).read(OWNER, 8, &mut budget, &|| true), Err(Error::Limit(Limit::Work))));
    assert_eq!(budget.remaining_storage(), storage);
    assert_eq!(budget.remaining_work(), 0);
}

#[test]
fn skipped_argument_names_spend_work_but_no_argument_storage() {
    let mut remaining = Vec::new();
    for name in ["short".into(), "n".repeat(4096)] {
        let bytes = with_leading_argument_payload(&text_value(&name));
        let mut budget = Budget::new(2048, 1 << 20);
        let parsed = Reader::new(&bytes, 26).read(OWNER, 8, &mut budget, &|| true).unwrap().record;
        assert_eq!(parsed.arguments.len(), 1);
        assert!(matches!(parsed.arguments[0].value, ArgumentValue::Int(7)));
        assert_eq!(parsed.errors, ["effect argument is missing its type word"]);
        assert_eq!(parsed.mask_offset, 8);
        remaining.push((budget.remaining_storage(), budget.remaining_work()));
    }
    assert_eq!(remaining[0].0, remaining[1].0);
    assert!(remaining[1].1 < remaining[0].1);
    let name = "n".repeat(4096);
    let valid = effect(26, &[(&name, 1, 7u32.to_be_bytes().to_vec())]);
    assert!(matches!(Reader::new(&valid, 26).read(OWNER, 8, &mut Budget::new(2048, 1 << 20), &|| true), Err(Error::Limit(Limit::Storage))));
}

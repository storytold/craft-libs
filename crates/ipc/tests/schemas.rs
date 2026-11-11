//! The JSON schemas in `schemas/` and the Rust types
//! must accept the same documents. Each table row is checked against both.
//! The fixtures are stored records. The first one is the FilmCraft to
//! EffectCraft shape written before the asset fields existed.

use std::path::PathBuf;

use jsonschema::Validator;
use serde_json::{Value, json};

use craft_ipc::InstanceRecord;
use craft_ipc::record::{AssetKind, LinkRecord};

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas")
}

fn validator(name: &str) -> Validator {
    let path = schema_dir().join(name);
    let schema: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    jsonschema::options().should_validate_formats(true).build(&schema).unwrap()
}

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Schema verdict and Rust verdict for one document. Both must equal `want`.
fn check_link(schema: &Validator, doc: &Value, want: bool, why: &str) {
    let by_schema = schema.is_valid(doc);
    let by_rust = serde_json::from_value::<LinkRecord>(doc.clone()).is_ok_and(|r| r.validate().is_ok());
    assert_eq!(by_schema, want, "schema: {why}: {doc}");
    assert_eq!(by_rust, want, "rust: {why}: {doc}");
}

#[test]
fn stored_filmcraft_effectcraft_record_stays_valid() {
    let schema = validator("link-record.schema.json");
    let doc = fixture("link-record-filmcraft-effectcraft.json");
    check_link(&schema, &doc, true, "legacy fixture");
    let record: LinkRecord = serde_json::from_value(doc.clone()).unwrap();
    assert_eq!(record.target_app, "effectcraft");
    assert_eq!(record.target_comp.as_deref(), Some("comp-1"));
    assert_eq!(record.nested_sequence.as_deref(), Some("item-9"));
    assert_eq!(record.format.rate_num, Some(24));
    assert_eq!(record.format.duration_ticks, Some(240));
    assert!(record.asset.is_none() && record.producer.is_none());
    // Unknown fields survive a load and save, byte-for-byte as JSON values.
    assert_eq!(serde_json::to_value(&record).unwrap(), doc);
}

#[test]
fn still_record_has_no_video_fields_and_round_trips() {
    let schema = validator("link-record.schema.json");
    let doc = fixture("link-record-still.json");
    check_link(&schema, &doc, true, "still fixture");
    let record: LinkRecord = serde_json::from_value(doc.clone()).unwrap();
    assert_eq!(record.target_app, "photocraft");
    assert!(record.nested_sequence.is_none() && record.target_comp.is_none());
    assert_eq!(record.format.rate_num, None);
    assert_eq!(record.format.rate_den, None);
    assert_eq!(record.format.duration_ticks, None);
    assert_eq!(record.asset.as_ref().map(|a| a.kind), Some(AssetKind::Still));
    let out = serde_json::to_value(&record).unwrap();
    assert_eq!(out, doc);
    let format = out["format"].as_object().unwrap();
    for key in ["rate_num", "rate_den", "duration_ticks"] {
        assert!(!format.contains_key(key), "{key} must not be written as null");
    }
}

#[test]
fn schema_and_rust_agree_on_edge_cases() {
    let schema = validator("link-record.schema.json");
    let legacy = fixture("link-record-filmcraft-effectcraft.json");
    let still = fixture("link-record-still.json");

    let edit = |base: &Value, f: &dyn Fn(&mut Value)| {
        let mut doc = base.clone();
        f(&mut doc);
        doc
    };
    let cases: Vec<(Value, bool, &str)> = vec![
        (edit(&legacy, &|d| d["target_app"] = json!("photocraft")), true, "any app id"),
        (edit(&legacy, &|d| d["target_app"] = json!("Film")), false, "uppercase app id"),
        (edit(&legacy, &|d| d["target_app"] = json!("")), false, "empty app id"),
        (edit(&legacy, &|d| d["v"] = json!(2)), false, "v 2"),
        (
            edit(&legacy, &|d| {
                d.as_object_mut().unwrap().remove("nested_sequence");
            }),
            true,
            "nested_sequence optional",
        ),
        (
            edit(&legacy, &|d| {
                d["format"].as_object_mut().unwrap().remove("duration_ticks");
            }),
            true,
            "duration optional",
        ),
        (
            edit(&legacy, &|d| {
                d["format"].as_object_mut().unwrap().remove("rate_num");
            }),
            false,
            "rate_den without rate_num",
        ),
        (
            edit(&legacy, &|d| {
                d["format"].as_object_mut().unwrap().remove("rate_den");
            }),
            false,
            "rate_num without rate_den",
        ),
        (edit(&legacy, &|d| d["format"]["rate_num"] = json!(0)), false, "rate_num 0"),
        (edit(&legacy, &|d| d["format"]["w"] = json!(0)), false, "w 0"),
        (
            edit(&legacy, &|d| {
                d["format"].as_object_mut().unwrap().remove("alpha");
            }),
            false,
            "alpha stays required",
        ),
        (edit(&legacy, &|d| d["result_epoch"] = json!(0)), true, "epoch 0"),
        (edit(&legacy, &|d| d["created"] = json!("yesterday")), false, "bad date"),
        (edit(&legacy, &|d| d["created"] = json!("2026-02-31T09:30:00Z")), false, "31 February"),
        (edit(&legacy, &|d| d["created"] = json!("2026-02-29T09:30:00Z")), false, "29 February in a common year"),
        (edit(&legacy, &|d| d["created"] = json!("2026-04-31T09:30:00+02:00")), false, "31 April"),
        (edit(&legacy, &|d| d["created"] = json!("1900-02-29T00:00:00Z")), false, "29 February in a century year"),
        (edit(&legacy, &|d| d["created"] = json!("2024-02-29T09:30:00Z")), true, "29 February in a leap year"),
        (edit(&legacy, &|d| d["created"] = json!("2000-02-29T09:30:00.250Z")), true, "29 February in a 400 year"),
        (edit(&legacy, &|d| d["created"] = json!("2026-12-31T23:59:59-05:30")), true, "last day of the year"),
        (edit(&legacy, &|d| d["link_id"] = json!("nope")), false, "bad link id"),
        (
            edit(&legacy, &|d| {
                d.as_object_mut().unwrap().remove("target_comp");
            }),
            false,
            "no comp and no asset",
        ),
        (
            edit(&legacy, &|d| {
                d.as_object_mut().unwrap().remove("target_project");
            }),
            false,
            "target_project required",
        ),
        (
            edit(&still, &|d| {
                d.as_object_mut().unwrap().remove("asset");
            }),
            false,
            "still without asset or comp",
        ),
        (edit(&still, &|d| d["asset"]["kind"] = json!("video")), false, "unknown asset kind"),
        (edit(&still, &|d| d["asset"]["kind"] = json!("vector")), true, "vector kind"),
        (edit(&still, &|d| d["asset"]["kind"] = json!("sequence")), true, "sequence kind"),
        (edit(&still, &|d| d["asset"]["kind"] = json!("comp")), true, "comp kind"),
        (edit(&still, &|d| d["asset"]["moniker"] = json!("craft://photocraft/not-a-uuid/image/x")), false, "bad moniker doc"),
        (edit(&still, &|d| d["asset"]["moniker"] = json!("photocraft/x")), false, "moniker without scheme"),
        (
            edit(&still, &|d| d["asset"]["moniker"] = json!("craft://photocraft/9f8e7d6c-5b4a-4c3d-8e2f-1a0b9c8d7e6f/image/a/b")),
            false,
            "moniker with extra segment",
        ),
        (edit(&still, &|d| d["producer"]["app"] = json!("Bad App")), false, "bad producer app"),
        (
            edit(&still, &|d| {
                d.as_object_mut().unwrap().remove("producer");
            }),
            true,
            "producer optional",
        ),
        (edit(&still, &|d| d["asset"]["extra"] = json!(1)), true, "unknown asset field kept"),
    ];
    for (doc, want, why) in &cases {
        check_link(&schema, doc, *want, why);
    }
}

const MONIKER_BASE: &str = "craft://photocraft/9f8e7d6c-5b4a-4c3d-8e2f-1a0b9c8d7e6f/image/";

fn with_moniker(still: &Value, id: &str) -> Value {
    let mut doc = still.clone();
    doc["asset"]["moniker"] = json!(format!("{MONIKER_BASE}{id}"));
    doc
}

/// The moniker id rules of `Moniker::parse` as the schema states them: unreserved characters and
/// two-digit escapes only, 1 to 512 decoded bytes, and decoded bytes that are valid UTF-8.
#[test]
fn schema_and_rust_agree_on_moniker_ids() {
    let schema = validator("link-record.schema.json");
    let still = fixture("link-record-still.json");
    let escaped = |count: usize| "%41".repeat(count);
    let cases: Vec<(String, bool, &str)> = vec![
        ("layer-3".into(), true, "plain"),
        ("~._-AZaz09".into(), true, "every unreserved class"),
        ("a%20b".into(), true, "escaped space"),
        ("a%2fb".into(), true, "lowercase escape"),
        ("%C3%A9".into(), true, "two byte character"),
        ("%c3%a9".into(), true, "two byte character in lowercase hex"),
        ("%E2%82%AC".into(), true, "three byte character"),
        ("%F0%9F%98%80".into(), true, "four byte character"),
        ("%F4%8F%BF%BF".into(), true, "highest scalar"),
        ("%ED%9F%BF".into(), true, "just below the surrogates"),
        ("%00".into(), true, "escaped NUL is a byte like any other"),
        ("%GG".into(), false, "bad escape"),
        ("%4".into(), false, "truncated escape"),
        ("%".into(), false, "lone percent"),
        ("a b".into(), false, "raw space"),
        ("caf\u{e9}".into(), false, "raw non-ASCII"),
        ("a:b".into(), false, "raw colon"),
        ("a%2".into(), false, "truncated escape after text"),
        ("".into(), false, "empty id"),
        ("%FF".into(), false, "invalid UTF-8 byte"),
        ("%C3".into(), false, "truncated two byte character"),
        ("%80".into(), false, "lone continuation byte"),
        ("%C0%80".into(), false, "overlong two byte form"),
        ("%E0%80%80".into(), false, "overlong three byte form"),
        ("%ED%A0%80".into(), false, "surrogate"),
        ("%F0%80%80%80".into(), false, "overlong four byte form"),
        ("%F4%90%80%80".into(), false, "above U+10FFFF"),
        ("%F5%80%80%80".into(), false, "lead byte F5"),
        ("a/b".into(), false, "raw slash"),
        ("a?b".into(), false, "raw query"),
        ("a#b".into(), false, "raw fragment"),
        ("a".repeat(512), true, "512 raw bytes"),
        ("a".repeat(513), false, "513 raw bytes"),
        (escaped(512), true, "512 escaped bytes"),
        (escaped(513), false, "513 escaped bytes"),
        (format!("{}%C3%A9", "a".repeat(510)), true, "512 decoded bytes ending in a two byte character"),
        (format!("{}%C3%A9", "a".repeat(511)), false, "513 decoded bytes ending in a two byte character"),
        ("%C3%A9".repeat(256), true, "256 two byte characters make 512 bytes"),
        ("%C3%A9".repeat(257), false, "257 two byte characters make 514 bytes"),
    ];
    for (id, want, why) in &cases {
        check_link(&schema, &with_moniker(&still, id), *want, why);
    }

    // Every lead byte against a spread of second and third bytes: the two verdicts must match.
    let probes = [0x00u8, 0x41, 0x7F, 0x80, 0x8F, 0x90, 0x9F, 0xA0, 0xBF, 0xC0];
    let mut seen_valid = 0;
    for lead in 0u8..=0xFF {
        for second in probes {
            for tail in [&[][..], &[0x80][..], &[0x80, 0x80][..]] {
                for hex in [|b: u8| format!("%{b:02X}"), |b: u8| format!("%{b:02x}")] {
                    let id: String = [lead, second].iter().chain(tail).map(|byte| hex(*byte)).collect();
                    let doc = with_moniker(&still, &id);
                    let by_schema = schema.is_valid(&doc);
                    let by_rust = serde_json::from_value::<LinkRecord>(doc.clone()).is_ok_and(|r| r.validate().is_ok());
                    assert_eq!(by_schema, by_rust, "bytes {lead:02X} {second:02X} {tail:02X?} as {id}");
                    seen_valid += usize::from(by_schema);
                }
            }
        }
    }
    assert!(seen_valid > 100, "the probe must reach valid sequences too, got {seen_valid}");
}

fn instance_doc() -> Value {
    json!({
        "v": 1,
        "app": "hub",
        "app_version": "0.1.0",
        "pid": 4242,
        "instance": "550e8400-e29b-41d4-a716-446655440000",
        "endpoint": {"kind": "unix", "path": "/run/user/1000/craft/hub-4242.sock"},
        "token": "ab".repeat(32),
        "started": 1_760_000_000u64,
        "protocol": {"link": 1},
        "docs": []
    })
}

fn check_instance(schema: &Validator, doc: &Value, want: bool, why: &str) {
    let by_schema = schema.is_valid(doc);
    let by_rust = serde_json::from_value::<InstanceRecord>(doc.clone()).is_ok_and(|r| r.validate().is_ok());
    assert_eq!(by_schema, want, "schema: {why}: {doc}");
    assert_eq!(by_rust, want, "rust: {why}: {doc}");
}

#[test]
fn instance_record_caps_agree_between_schema_and_rust() {
    let schema = validator("instance-record.schema.json");
    let base = instance_doc();
    check_instance(&schema, &base, true, "no caps (existing records)");
    for (caps, want, why) in [
        (json!(["hub"]), true, "hub"),
        (json!(["hub", "assets"]), true, "two caps"),
        (json!([]), true, "empty caps"),
        (json!(["Hub"]), false, "uppercase"),
        (json!([""]), false, "empty cap"),
        (json!(["hub", "hub"]), false, "duplicate"),
        (json!("hub"), false, "not an array"),
        (json!([1]), false, "not strings"),
    ] {
        let mut doc = base.clone();
        doc["caps"] = caps;
        check_instance(&schema, &doc, want, why);
    }
    let mut hub: InstanceRecord = serde_json::from_value(base).unwrap();
    assert!(hub.caps().is_empty());
    hub.add_cap("hub").unwrap();
    hub.add_cap("hub").unwrap();
    assert_eq!(hub.caps(), vec!["hub".to_string()]);
    assert!(hub.add_cap("Not Valid").is_err());
    let value = serde_json::to_value(&hub).unwrap();
    assert_eq!(value["caps"], json!(["hub"]));
    assert!(schema.is_valid(&value));
}

use super::*;
use crate::decode::document::Scalar;

#[test]
fn the_table_is_sorted_and_has_one_row_per_field() {
    let rows: Vec<(&str, &str)> = table().map(|e| (e.class, e.field)).collect();
    assert!(rows.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
    assert!(lookup("DataLayerColor", "label").is_some_and(|e| e.rule == Rule::Read));
    assert!(lookup("DataLayerColor", "noSuchField").is_none());
}

#[test]
fn every_rule_but_read_says_why() {
    for e in table() {
        if e.rule != Rule::Read {
            assert!(!e.rule.note().is_empty(), "{}.{}", e.class, e.field);
        }
    }
}

#[test]
fn unhandled_defaults_are_in_the_describe_notation() {
    // a default that `describe` can never produce would flag every object
    for e in table() {
        if let Rule::Unhandled { default, .. } = e.rule {
            let plausible = default == "absent"
                || default == "true"
                || default == "false"
                || default.starts_with(['e', 'u', 'i', '[', '"'])
                || default.starts_with("ref:")
                || default.starts_with("list[")
                || default.parse::<f32>().is_ok_and(|_| default.contains('.') && default.split('.').nth(1).is_some_and(|d| d.len() == 4));
            assert!(plausible, "{}.{} = {default}", e.class, e.field);
        }
    }
}

#[test]
fn check_reports_unhandled_derived_and_unknown_fields() {
    let mut o = RawObject::new("DataActionPaint");
    o.fields.push(("label".into(), RawValue::Scalar(Scalar::String("x".into()))));
    o.fields.push(("stencilPostBlending".into(), RawValue::Scalar(Scalar::Bool(false))));
    o.fields.push(("projDuplicates".into(), RawValue::Scalar(Scalar::Bool(true))));
    o.fields.push(("tags".into(), RawValue::Scalar(Scalar::UInt(8))));
    o.fields.push(("brandNew".into(), RawValue::Absent));
    let mut found = Vec::new();
    check(&o, |s| found.push(s));
    let keys: Vec<(String, bool)> = found.iter().map(|s| (s.key(), s.may_change_pixels())).collect();
    assert_eq!(
        keys,
        [
            ("DataActionPaint.stencilPostBlending".to_string(), true),
            ("DataActionPaint.projDuplicates".to_string(), false),
            ("DataActionPaint.brandNew".to_string(), true),
        ]
    );
    assert_eq!(found[2].kind, SettingKind::Unknown);
}

#[test]
fn reasons_read_back_to_their_keys() {
    let s = UnhandledSetting {
        class: "DataActionPaint".into(),
        field: "selection".into(),
        value: "ref:set".into(),
        kind: SettingKind::Unhandled { default: "ref:0", what: "" },
    };
    let text = format!("strokes replayed; {}; {}", s.reason(), "setting of the brush");
    assert_eq!(UnhandledSetting::keys_in(&text), ["DataActionPaint.selection"]);
    assert!(UnhandledSetting::keys_in("blend setting guessed").is_empty());
}

#[test]
fn only_read_fields_are_looked_into() {
    let mut o = RawObject::new("DataDocument");
    o.fields.push(("materials".into(), RawValue::List(vec![RawValue::Object(Box::new(RawObject::new("DataMaterial")))])));
    o.fields.push(("colorManagementOCIO".into(), RawValue::Object(Box::new(RawObject::new("DataColorManagementOCIO")))));
    let classes: Vec<&str> = children(&o).map(|c| c.class.as_str()).collect();
    assert_eq!(classes, ["DataMaterial"]);
}

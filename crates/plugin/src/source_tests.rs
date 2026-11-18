//! Where a plug-in came from: the JSON of `source.json`, old and new.

use serde_json::json;

use crate::source::{MAX_SOURCE_FIELD, Source, SourceRecord};

#[test]
fn records_written_before_other_existed_still_read() {
    let old = r#"{"id": "org.example.a", "version": "1.0.0", "sha256": "ab", "source": {"type": "github", "repo": "example/tools", "tag": "v1", "asset": "t.craftplugin"}}"#;
    let record: SourceRecord = serde_json::from_str(old).unwrap();
    assert_eq!(record.source, Source::Github { repo: "example/tools".into(), tag: "v1".into(), asset: "t.craftplugin".into() });
    for (source, value) in [
        (Source::File { path: "a.craftplugin".into() }, json!({"type": "file", "path": "a.craftplugin"})),
        (Source::Catalog { catalog: "https://x".into(), package: "p".into() }, json!({"type": "catalog", "catalog": "https://x", "package": "p"})),
    ] {
        assert_eq!(serde_json::to_value(&source).unwrap(), value);
        assert_eq!(serde_json::from_value::<Source>(value).unwrap(), source);
    }
}

#[test]
fn an_apps_own_source() {
    let source = Source::Other { kind: "figcraft-server".into(), location: "https://team.example/plugins/tools".into() };
    let value = json!({"type": "other", "kind": "figcraft-server", "location": "https://team.example/plugins/tools"});
    assert_eq!(serde_json::to_value(&source).unwrap(), value);
    assert_eq!(serde_json::from_value::<Source>(value).unwrap(), source);
    assert_eq!(source.check(), Ok(()));
    for kind in ["", "Figcraft", "a b", "../x", &"k".repeat(33)] {
        assert!(Source::Other { kind: kind.into(), location: "x".into() }.check().is_err(), "{kind:?}");
    }
    assert!(Source::Other { kind: "x".into(), location: "a\nb".into() }.check().is_err());
    assert!(Source::Other { kind: "x".into(), location: "a".repeat(MAX_SOURCE_FIELD + 1) }.check().is_err());
    assert!(serde_json::from_value::<Source>(json!({"type": "ftp", "location": "x"})).is_err(), "unknown types are refused");
}

#[test]
fn the_store_names_the_same_types() {
    // `store::Source` (native) is `source::Source` (every target).
    #[cfg(not(target_arch = "wasm32"))]
    {
        let s: crate::store::Source = crate::Source::File { path: String::new() };
        assert_eq!(s.check(), Ok(()));
    }
}

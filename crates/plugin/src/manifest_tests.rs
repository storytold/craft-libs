//! The shared manifest's fields (`kind`, `apps`, `license`, `repo`) and whether an app can run a
//! plug-in.

use serde_json::{Value, json};

use crate::{AppInfo, AppTarget, KIND_COMMAND, LEGACY_MANIFEST_FILE, MANIFEST_FILE, MANIFEST_FILES, Manifest, Target};

fn manifest(extra: Value) -> Result<Manifest, String> {
    let mut v = json!({"id": "io.github.example.tools", "name": "Tools", "version": "1.2.0", "main": "tools.wasm"});
    if let (Some(base), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
        base.extend(extra.clone());
    }
    Manifest::from_value(&v)
}

const KINDS: &[&str] = &["command"];

fn app(id: &'static str, version: &'static str) -> AppInfo<'static> {
    AppInfo { id, version, api: 1..=6, kinds: KINDS, accept_legacy: false, kind_api: &[] }
}

#[test]
fn file_names() {
    assert_eq!((MANIFEST_FILE, LEGACY_MANIFEST_FILE), ("craft-plugin.json", "figcraft-plugin.json"));
    assert_eq!(MANIFEST_FILES, [MANIFEST_FILE, LEGACY_MANIFEST_FILE]);
}

#[test]
fn the_rfc_example_reads() {
    let m = manifest(json!({
        "author": "Example",
        "description": "Makes swatches and palettes from a list of colours.",
        "license": "MIT OR Apache-2.0",
        "repo": "https://github.com/example/palette-tools",
        "kind": "command",
        "apps": {
            "figcraft": { "api": 1 },
            "designcraft": { "api": 1 },
            "vectorcraft": { "api": 1, "minApp": "0.30.0" }
        },
        "permissions": { "access": "write", "commands": ["select.*", "swatch.*"] },
        "commands": [{ "id": "grid", "label": "Swatch grid…", "params": [] }]
    }))
    .unwrap();
    assert_eq!(m.kind, KIND_COMMAND);
    assert_eq!(m.license, "MIT OR Apache-2.0");
    assert_eq!(m.apps.len(), 3);
    assert_eq!(m.apps["vectorcraft"], AppTarget { api: 1, min_app: Some("0.30.0".into()) });
}

#[test]
fn a_figcraft_manifest_reads_and_writes_as_before() {
    let m = manifest(json!({"api": 6, "permissions": {"access": "write"}})).unwrap();
    assert_eq!((m.kind.as_str(), m.apps.is_empty(), m.license.as_str(), m.repo.as_str()), (KIND_COMMAND, true, "", ""));
    let out = serde_json::to_value(&m).unwrap();
    for key in ["kind", "apps", "license", "repo"] {
        assert!(out.get(key).is_none(), "{key} written for a manifest without it: {out}");
    }
    assert_eq!(Manifest::from_value(&out).unwrap(), m, "round trip");
}

#[test]
fn new_fields_are_checked() {
    for (extra, why) in [
        (json!({"kind": ""}), "kind"),
        (json!({"kind": "Filter"}), "kind"),
        (json!({"kind": "a/b"}), "kind"),
        (json!({"apps": {"FigCraft": {"api": 1}}}), "app id"),
        (json!({"apps": {"figcraft": {"api": 0}}}), "api"),
        (json!({"apps": {"figcraft": {"api": 1, "minApp": "latest"}}}), "minApp"),
        (json!({"apps": {"figcraft": {}}}), "api"),
        (json!({"repo": "https://gitlab.com/a/b"}), "GitHub"),
        (json!({"repo": "https://github.com/a/../b"}), "GitHub"),
        (json!({"repo": "https://github.com/a/b?x=1"}), "GitHub"),
        (json!({"license": "x".repeat(129)}), "license"),
        (json!({"license": "MIT\u{7}"}), "license"),
    ] {
        let e = manifest(extra.clone()).unwrap_err();
        assert!(e.contains(why), "{extra}: {e}");
    }
    let many: serde_json::Map<String, Value> = (0..65).map(|i| (format!("app{i}"), json!({"api": 1}))).collect();
    assert!(manifest(json!({ "apps": many })).unwrap_err().contains("apps"));
    // The API of an app's own kind is that app's to check; the command ABI's is checked here.
    assert!(manifest(json!({"kind": "filter", "api": 40})).is_ok());
    assert!(manifest(json!({"api": 7})).unwrap_err().contains("API 7"));
    // `apps.*.api` may be a version string, as the top-level `api` may.
    assert_eq!(manifest(json!({"apps": {"figcraft": {"api": "6.0.0"}}})).unwrap().apps["figcraft"].api, 6);
}

#[test]
fn an_app_named_in_apps_runs_it() {
    let m = manifest(json!({"apps": {"figcraft": {"api": 6}, "designcraft": {"api": 2, "minApp": "0.30.0"}}})).unwrap();
    assert_eq!(m.target(&app("figcraft", "1.0.0")), Ok(Target::Named { api: 6 }));
    assert_eq!(m.target(&app("designcraft", "0.30.0")).map(|t| t.api()), Ok(2));
    assert_eq!(m.target(&app("designcraft", "0.31.0-dev+abc")).map(|t| t.api()), Ok(2));
    let e = m.target(&app("designcraft", "0.29.9")).unwrap_err();
    assert!(e.contains("0.30.0 or later"), "{e}");
    let e = m.target(&app("designcraft", "nightly")).unwrap_err();
    assert!(e.contains("can't be compared"), "{e}");
    let e = m.target(&app("photocraft", "9.0.0")).unwrap_err();
    assert!(e.contains("designcraft, figcraft") && e.contains("not photocraft"), "{e}");
    let old = AppInfo { api: 1..=5, ..app("figcraft", "1.0.0") };
    assert!(m.target(&old).unwrap_err().contains("API 6"));
}

#[test]
fn legacy_manifests_are_the_apps_decision() {
    let m = manifest(json!({"api": 3})).unwrap();
    let e = m.target(&app("designcraft", "1.0.0")).unwrap_err();
    assert!(e.contains("no `apps`"), "{e}");
    let figcraft = AppInfo { accept_legacy: true, ..app("figcraft", "1.0.0") };
    assert_eq!(m.target(&figcraft), Ok(Target::Legacy { api: 3 }));
    // A manifest that names other apps is no legacy one.
    let other = manifest(json!({"apps": {"designcraft": {"api": 1}}})).unwrap();
    assert!(other.target(&figcraft).is_err());
}

#[test]
fn kinds_the_app_doesnt_run_are_refused() {
    let m = manifest(json!({"kind": "filter", "apps": {"photocraft": {"api": 1}}})).unwrap();
    let e = m.target(&app("photocraft", "1.0.0")).unwrap_err();
    assert!(e.contains("`filter`"), "{e}");
    let photocraft = AppInfo { kinds: &["command", "filter"], ..app("photocraft", "1.0.0") };
    assert_eq!(m.target(&photocraft), Ok(Target::Named { api: 1 }));
}

const FILTER_API: &[crate::KindApi<'static>] = &[crate::KindApi { kind: "filter", api: 1..=1 }];
const EFFECT_API: &[crate::KindApi<'static>] = &[crate::KindApi { kind: "effect", api: 1..=2 }];

#[test]
fn each_kind_has_its_own_api_range() {
    // PhotoCraft: command plug-ins of ABI 1–6, filters of ABI 1 only.
    let photocraft = AppInfo { kinds: &["command", "filter"], kind_api: FILTER_API, ..app("photocraft", "1.0.0") };
    assert_eq!(photocraft.api_for("filter"), 1..=1);
    assert_eq!(photocraft.api_for(KIND_COMMAND), 1..=6, "command plug-ins keep `api`");
    let filter = |api: u32| manifest(json!({"kind": "filter", "apps": {"photocraft": {"api": api}}})).unwrap();
    assert_eq!(filter(1).target(&photocraft), Ok(Target::Named { api: 1 }));
    let e = filter(2).target(&photocraft).unwrap_err();
    assert!(e.contains("`filter` plug-in API 2") && e.contains("`filter` API 1–1"), "{e}");
    let command = |api: u32| manifest(json!({"apps": {"photocraft": {"api": api}}})).unwrap();
    assert_eq!(command(6).target(&photocraft), Ok(Target::Named { api: 6 }));
    let e = command(7).target(&photocraft).unwrap_err();
    assert_eq!(e, "it needs plug-in API 7, and this photocraft runs API 1–6");

    // EffectCraft: one AppInfo for both kinds, effects of ABI 1–2.
    let effectcraft = AppInfo { kinds: &["command", "effect"], kind_api: EFFECT_API, ..app("effectcraft", "1.0.0") };
    let effect = |api: u32| manifest(json!({"kind": "effect", "apps": {"effectcraft": {"api": api}}})).unwrap();
    assert_eq!(effect(2).target(&effectcraft), Ok(Target::Named { api: 2 }));
    assert!(effect(3).target(&effectcraft).is_err());
    assert!(manifest(json!({"apps": {"effectcraft": {"api": 6}}})).unwrap().target(&effectcraft).is_ok());
    // An entry for a kind the app doesn't run opens nothing.
    let narrow = AppInfo { kinds: &["command"], ..effectcraft };
    assert!(effect(1).target(&narrow).unwrap_err().contains("`effect`"));
}

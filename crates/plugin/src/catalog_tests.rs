//! Catalogs: EffectCraft's format with `kind` and `apps`, the URL rules, partial-bad catalogs and
//! picking the release an app can run.

use serde_json::{Value, json};

use crate::AppInfo;
use crate::catalog::{Location, check_redirect, check_url, file_url_path, parse_catalog};

const CATALOG: &str = "https://raw.githubusercontent.com/storytold/craft-plugins/main/index.json";

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

fn url(version: &str) -> String {
    format!("https://github.com/example/tools/releases/download/v{version}/tools.craftplugin")
}

fn release(version: &str, apps: Value) -> Value {
    json!({"version": version, "apps": apps, "url": url(version), "sha256": sha('a'), "size": 1000})
}

fn package(versions: Value) -> Value {
    json!({"id": "io.github.example.tools", "name": "Tools", "repo": "https://github.com/example/tools", "versions": versions})
}

fn read(packages: Value) -> (crate::catalog::Catalog, Vec<String>) {
    parse_catalog(json!({"schema": 1, "packages": packages}).to_string().as_bytes(), CATALOG).unwrap()
}

fn app(id: &'static str, version: &'static str) -> AppInfo<'static> {
    AppInfo { id, version, api: 1..=6, kinds: &["command"], accept_legacy: false }
}

#[test]
fn reads_the_generalized_format() {
    let (c, warnings) = read(json!([package(json!([release("1.0.0", json!({"figcraft": {"api": 6}}))]))]));
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = c.package("IO.GITHUB.EXAMPLE.TOOLS").unwrap();
    assert_eq!((p.kind.as_str(), p.versions.len()), ("command", 1));
    assert_eq!(p.repo_ref().unwrap().slug(), "example/tools");
    assert_eq!(p.release("1.0").unwrap().version, "1.0.0");
}

#[test]
fn reads_effectcrafts_format_and_keeps_unknown_fields() {
    let entry = json!({
        "id": "org.example.glow", "name": "Glow", "repo": "https://github.com/example/glow", "kind": "effect",
        "preview": {"video": "https://youtu.be/x"},
        "versions": [{"version": "1.0.0", "api": 2, "minApp": "0.18.0", "effects": ["glow"],
            "url": "https://github.com/example/glow/releases/download/v1.0.0/glow.wasm", "sha256": sha('B')}]
    });
    let (c, warnings) = read(json!([entry]));
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = &c.packages[0];
    assert_eq!(p.extra["preview"]["video"], "https://youtu.be/x");
    assert_eq!(p.versions[0].extra["effects"], json!(["glow"]));
    assert_eq!(p.versions[0].sha256, sha('b'), "lower-cased");
    let effectcraft = AppInfo { id: "effectcraft", version: "0.18.0", api: 1..=2, kinds: &["effect"], accept_legacy: true };
    assert_eq!(p.latest_for(&effectcraft).map(|r| r.version.as_str()), Some("1.0.0"));
    assert!(p.latest_for(&AppInfo { version: "0.17.9", ..effectcraft.clone() }).is_none());
    assert!(p.latest_for(&AppInfo { accept_legacy: false, ..effectcraft }).is_none());
}

#[test]
fn bad_entries_are_left_out_and_the_rest_kept() {
    let good = release("1.0.0", json!({"figcraft": {"api": 6}}));
    let mut wrong_type = good.clone();
    wrong_type["size"] = json!("big");
    let mut other_repo = good.clone();
    other_repo["url"] = json!("https://github.com/attacker/tools/releases/download/v1/tools.craftplugin");
    let mut off_github = good.clone();
    off_github["url"] = json!("https://example.com/tools.craftplugin");
    let mut plain_http = good.clone();
    plain_http["url"] = json!("http://github.com/example/tools/releases/download/v1/tools.craftplugin");
    let mut climbing = good.clone();
    climbing["url"] = json!("https://github.com/example/tools/releases/download/v1/%2e%2e/x.craftplugin");
    let mut local = good.clone();
    local["url"] = json!("file:///tmp/tools.craftplugin");
    let mut bad_sha = good.clone();
    bad_sha["sha256"] = json!("abc");
    let mut too_big = good.clone();
    too_big["size"] = json!(1u64 << 40);
    let mut bad_version = good.clone();
    bad_version["version"] = json!("1.0-beta");
    let no_target = json!({"version": "1.0.0", "url": url("1.0.0"), "sha256": sha('a')});
    let bad_apps = release("1.0.0", json!({"Fig Craft": {"api": 1}}));
    let packages = json!([
        package(json!([good, wrong_type, other_repo, off_github, plain_http, climbing, local, bad_sha, too_big, bad_version, no_target, bad_apps])),
        {"id": "../evil", "name": "Evil", "versions": []},
        {"id": "no.name", "name": "  ", "versions": []},
        {"id": "bad.repo", "name": "R", "repo": "https://gitlab.com/x/y", "versions": []},
        {"id": "bad.kind", "name": "K", "kind": "Not A Kind", "versions": []},
        {"id": "bad.versions", "name": "V", "versions": "1.0"},
        {"id": "ctl", "name": "A\u{1b}[31m", "versions": []},
        {"id": "IO.github.example.TOOLS", "name": "Copy", "versions": []},
        "not an object",
    ]);
    let (c, warnings) = read(packages);
    assert_eq!(c.packages.len(), 1, "{warnings:?}");
    assert_eq!(c.packages[0].versions.len(), 1, "{warnings:?}");
    assert_eq!(warnings.len(), 11 + 8, "{warnings:#?}");
}

#[test]
fn whole_catalog_errors() {
    assert!(parse_catalog(b"{", CATALOG).is_err());
    assert!(parse_catalog(br#"{"schema": 2, "packages": []}"#, CATALOG).unwrap_err().contains("format 2"));
    assert!(parse_catalog(&vec![b' '; (4 << 20) + 1], CATALOG).unwrap_err().contains("larger"));
    // Deep nesting is refused by the JSON reader, not a stack overflow.
    let deep = format!(r#"{{"schema": 1, "packages": [{{"id": "a", "name": "A", "x": {}{}}}]}}"#, "[".repeat(100_000), "]".repeat(100_000));
    assert!(parse_catalog(deep.as_bytes(), CATALOG).is_err());
}

#[test]
fn a_local_catalog_may_name_local_files() {
    let mut r = release("1.0.0", json!({"figcraft": {"api": 6}}));
    r["url"] = json!("file:///home/dev/tools.craftplugin");
    let text = json!({"schema": 1, "packages": [package(json!([r]))]}).to_string();
    let (c, warnings) = parse_catalog(text.as_bytes(), "file:///home/dev/index.json").unwrap();
    assert_eq!(c.packages[0].versions.len(), 1, "{warnings:?}");
}

#[test]
fn the_newest_release_the_app_can_run_is_picked() {
    let versions = json!([
        release("1.0.0", json!({"figcraft": {"api": 6}})),
        release("1.10.0", json!({"figcraft": {"api": 6, "minApp": "2.0.0"}})),
        release("1.9.0", json!({"figcraft": {"api": 6}, "designcraft": {"api": 6}})),
        release("1.2.0", json!({"figcraft": {"api": 7}})),
        release("2.0.0", json!({"designcraft": {"api": 6}})),
    ]);
    let (c, _) = read(json!([package(versions)]));
    let p = &c.packages[0];
    let pick = |a: AppInfo<'_>| p.latest_for(&a).map(|r| r.version.clone());
    assert_eq!(pick(app("figcraft", "1.5.0")).as_deref(), Some("1.9.0"));
    assert_eq!(pick(app("figcraft", "2.0.0")).as_deref(), Some("1.10.0"));
    assert_eq!(pick(app("designcraft", "0.1.0")).as_deref(), Some("2.0.0"));
    assert_eq!(pick(app("photocraft", "9.0.0")), None);
    assert_eq!(pick(AppInfo { kinds: &["filter"], ..app("figcraft", "2.0.0") }), None);
}

#[test]
fn url_rules() {
    assert_eq!(check_url(CATALOG, None), Ok(Location::Https(CATALOG.into())));
    for bad in [
        "http://raw.githubusercontent.com/x",
        "https://example.com/index.json",
        "https://github.com.evil.example/x",
        "https://user@github.com/x",
        "https://github.com:8443/x",
        "ftp://github.com/x",
        "https://github.com/a b",
    ] {
        assert!(check_url(bad, None).is_err(), "{bad}");
    }
    // `file://` only from a local catalog (or as the catalog itself).
    assert!(check_url("file:///x/index.json", None).is_ok());
    assert!(check_url("file:///x/t.craftplugin", Some(CATALOG)).is_err());
    assert!(check_redirect("https://objects.githubusercontent.com/x").is_ok());
    assert!(check_redirect("https://evil.example/x").is_err());
    assert!(check_redirect("file:///etc/passwd").is_err());
}

#[test]
fn file_urls_stay_local() {
    assert_eq!(file_url_path("file:///home/a%20b/x.json").unwrap(), std::path::PathBuf::from("/home/a b/x.json"));
    assert_eq!(file_url_path("file:///C:/x/y.json").unwrap(), std::path::PathBuf::from("C:/x/y.json"));
    for bad in [
        "file://server/share/x",
        "file:////server/share/x",
        "file:///a%2fb",
        "file:///a%5cb",
        "file:///a\\b",
        "file:///a%zz",
        "file:///a%",
        "file:///a%00b",
    ] {
        assert!(file_url_path(bad).is_err(), "{bad}");
    }
}

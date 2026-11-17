//! Repositories as people type them, the releases API's answer, `SHA256SUMS` and verification.

use serde_json::json;

use crate::github::{
    RepoRef, expected_download_url, is_asset_url, parse_release, parse_repo_spec, parse_repo_url, parse_sha256sums, plan_download, sum_for,
    valid_file_name, valid_tag, verify_sha256,
};
use crate::sha256_hex;

fn repo(spec: &str) -> RepoRef {
    parse_repo_spec(spec).unwrap()
}

#[test]
fn repository_specs() {
    assert_eq!(repo("example/palette-tools"), RepoRef { owner: "example".into(), name: "palette-tools".into(), tag: None });
    assert_eq!(repo(" example/palette-tools@v1.2.0 ").tag.as_deref(), Some("v1.2.0"));
    assert_eq!(repo("https://github.com/Example/palette-tools.git").slug(), "Example/palette-tools");
    assert_eq!(repo("HTTPS://github.com/example/tools/@1.0+build").tag.as_deref(), Some("1.0+build"));
    assert_eq!(repo("example/tools.git").name, "tools");
    for bad in [
        "",
        "example",
        "example/",
        "/tools",
        "a/b/c",
        "-a/b",
        "a b/c",
        "a/..",
        "a/.",
        "a/b@",
        "a/b@../x",
        "a/b@.hidden",
        "a/b@v1/2",
        "https://gitlab.com/a/b",
        "https://github.com/a/b/tree/main/x",
        "https://github.com/a/b?tab=1",
        "a/b\u{202e}",
        &"a/".repeat(300),
    ] {
        assert!(parse_repo_spec(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn repository_urls() {
    assert_eq!(parse_repo_url("https://github.com/a/b/"), Some(("a".into(), "b".into())));
    assert_eq!(parse_repo_url("https://github.com/a/b/tree/main/plugins/x"), Some(("a".into(), "b".into())));
    for bad in [
        "http://github.com/a/b",
        "https://github.com/a",
        "https://github.com/a/b/tree/main/../x",
        "https://github.com/a/b/tree/main/%2e%2e",
        "https://github.com/a/b#x",
    ] {
        assert_eq!(parse_repo_url(bad), None, "{bad}");
    }
}

#[test]
fn api_urls() {
    assert_eq!(repo("a/b").releases_api_url(), "https://api.github.com/repos/a/b/releases/latest");
    assert_eq!(repo("a/b@v1.0+2").releases_api_url(), "https://api.github.com/repos/a/b/releases/tags/v1.0%2B2");
}

#[test]
fn names_and_links() {
    assert!(valid_tag("v1.2.0") && valid_tag("1.0+build") && !valid_tag(".x") && !valid_tag("a/b") && !valid_tag(&"1".repeat(65)));
    assert!(
        valid_file_name("x.craftplugin")
            && !valid_file_name("con.txt")
            && !valid_file_name("COM1")
            && !valid_file_name("a/b")
            && !valid_file_name("x.")
    );
    assert_eq!(
        expected_download_url("a", "b", "v1+2", "x+y.craftplugin").as_deref(),
        Some("https://github.com/a/b/releases/download/v1%2B2/x%2By.craftplugin")
    );
    let r = repo("Example/Tools");
    assert!(is_asset_url("https://github.com/example/tools/releases/download/v1/x.craftplugin", &r, "v1", "x.craftplugin"));
    assert!(is_asset_url("https://github.com/Example/Tools/releases/download/v1%2B2/x.craftplugin", &r, "v1+2", "x.craftplugin"));
    for bad in [
        "https://github.com/other/tools/releases/download/v1/x.craftplugin",
        "https://github.com/example/tools/releases/download/v2/x.craftplugin",
        "https://github.com/example/tools/releases/download/v1/../../../evil/x.craftplugin",
        "https://github.com/example/tools/releases/download/v1/X.craftplugin",
        "http://github.com/example/tools/releases/download/v1/x.craftplugin",
        "https://evil.example/example/tools/releases/download/v1/x.craftplugin",
    ] {
        assert!(!is_asset_url(bad, &r, "v1", "x.craftplugin"), "{bad}");
    }
}

fn release_json(assets: serde_json::Value) -> String {
    json!({"tag_name": "v1.2.0", "name": "Palette tools 1.2", "draft": false, "prerelease": false, "assets": assets}).to_string()
}

fn asset(name: &str, digest: Option<&str>) -> serde_json::Value {
    let mut a =
        json!({"name": name, "size": 123, "browser_download_url": format!("https://github.com/example/tools/releases/download/v1.2.0/{name}")});
    if let Some(d) = digest {
        a["digest"] = json!(format!("sha256:{d}"));
    }
    a
}

#[test]
fn releases_keep_only_their_own_assets() {
    let mut forged = asset("evil.craftplugin", None);
    forged["browser_download_url"] = json!("https://github.com/attacker/x/releases/download/v1.2.0/evil.craftplugin");
    let json = release_json(json!([asset("tools.craftplugin", None), asset("SHA256SUMS", None), forged, asset("../x", None), {"name": 5}]));
    let r = parse_release(&json, &repo("example/tools")).unwrap();
    assert_eq!((r.tag.as_str(), r.name.as_str()), ("v1.2.0", "Palette tools 1.2"));
    let names: Vec<&str> = r.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["tools.craftplugin", "SHA256SUMS"]);
    assert_eq!(r.bundle_asset().unwrap().size, 123);
    assert_eq!(r.sums_asset().unwrap().name, "SHA256SUMS");
}

#[test]
fn bad_answers_are_errors() {
    let r = repo("example/tools");
    assert!(parse_release(r#"{"message": "Not Found"}"#, &r).unwrap_err().contains("no such release"));
    assert!(parse_release(r#"{"message": "API rate limit exceeded"}"#, &r).unwrap_err().contains("rate limit"));
    assert!(parse_release("[", &r).is_err());
    assert!(parse_release(r#"{"tag_name": "../x"}"#, &r).is_err());
    assert!(parse_release(r#"{"tag_name": "v1", "draft": true}"#, &r).unwrap_err().contains("draft"));
    assert!(parse_release(r#"{"tag_name": "v2"}"#, &repo("example/tools@v1")).unwrap_err().contains("not v1"));
    assert!(parse_release(&" ".repeat(5 << 20), &r).unwrap_err().contains("larger"));
    let none = parse_release(&release_json(json!([])), &r).unwrap();
    assert!(none.bundle_asset().unwrap_err().contains("no .craftplugin"));
    let two = parse_release(&release_json(json!([asset("a.craftplugin", None), asset("b.craftplugin", None)])), &r).unwrap();
    assert!(two.bundle_asset().unwrap_err().contains("several"));
}

#[test]
fn sha256sums_are_read_strictly() {
    let h1 = "a".repeat(64);
    let h2 = "B".repeat(64);
    let text = format!(
        "{h1}  tools.craftplugin\n{h2} *other.bin\nnot a line\n{h1}  ../escape\n{h1}  twice.bin\n{h2}  twice.bin\n{h1}  same.bin\n{h1}  same.bin\n"
    );
    let sums = parse_sha256sums(&text);
    assert_eq!(sum_for(&sums, "tools.craftplugin"), Some(h1.as_str()));
    assert_eq!(sum_for(&sums, "other.bin"), Some("b".repeat(64).as_str()));
    assert_eq!(sum_for(&sums, "twice.bin"), None, "a name with two hashes vouches for neither");
    assert_eq!(sum_for(&sums, "same.bin"), Some(h1.as_str()));
    assert_eq!(sums.len(), 3);
}

#[test]
fn downloads_are_verified() {
    let sha = sha256_hex(b"bundle");
    assert_eq!(verify_sha256(b"bundle", &sha.to_ascii_uppercase()), Ok(sha.clone()));
    assert!(verify_sha256(b"bundle!", &sha).is_err());
    assert!(verify_sha256(b"bundle", "").is_err());
}

#[test]
fn a_download_needs_a_published_checksum() {
    let r = repo("example/tools");
    let sha = "c".repeat(64);
    let sums = format!("{sha}  tools.craftplugin\n");
    let plain = parse_release(&release_json(json!([asset("tools.craftplugin", None), asset("SHA256SUMS", None)])), &r).unwrap();
    let plan = plan_download(&r, &plain, Some(&sums)).unwrap();
    assert_eq!((plan.sha256.as_str(), plan.asset.as_str(), plan.repo.tag.as_deref()), (sha.as_str(), "tools.craftplugin", Some("v1.2.0")));
    assert!(plan_download(&r, &plain, None).unwrap_err().contains("no SHA-256"));
    assert!(plan_download(&r, &plain, Some("nothing here")).is_err());
    // GitHub's own digest vouches too, and must agree with the list when both are there.
    let digested = parse_release(&release_json(json!([asset("tools.craftplugin", Some(&sha))])), &r).unwrap();
    assert_eq!(plan_download(&r, &digested, None).unwrap().sha256, sha);
    let other = format!("{}  tools.craftplugin\n", "d".repeat(64));
    assert!(plan_download(&r, &digested, Some(&other)).unwrap_err().contains("different"));
    let mut huge = asset("tools.craftplugin", Some(&sha));
    huge["size"] = json!(1u64 << 40);
    let huge = parse_release(&release_json(json!([huge])), &r).unwrap();
    assert!(plan_download(&r, &huge, None).unwrap_err().contains("larger"));
}

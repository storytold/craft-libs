//! Installing from a catalog and from a repository over a fake HTTP client.

use std::collections::HashMap;

use serde_json::json;

use crate::bundle;
use crate::fetch::{Fetch, download_release, download_repo, fetch_catalog, resolve_repo};
use crate::github::parse_repo_spec;
use crate::sha256_hex;
use crate::test_support::bundle_bytes;

/// Serves fixed bodies and records what was asked for.
#[derive(Default)]
struct Fake {
    files: HashMap<String, Vec<u8>>,
    asked: Vec<String>,
}

impl Fetch for Fake {
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        self.asked.push(url.to_string());
        match self.files.get(url) {
            Some(b) if b.len() > max => Err(format!("more than {max} bytes")),
            Some(b) => Ok(Some(b.clone())),
            None => Ok(None),
        }
    }
}

const CATALOG: &str = "https://raw.githubusercontent.com/storytold/craft-plugins/main/index.json";
const ASSET: &str = "https://github.com/example/tools/releases/download/v1.0.0/tools.craftplugin";
const SUMS: &str = "https://github.com/example/tools/releases/download/v1.0.0/SHA256SUMS";
const API: &str = "https://api.github.com/repos/example/tools/releases/latest";

fn catalog(sha: &str, size: usize) -> Vec<u8> {
    json!({"schema": 1, "packages": [{"id": "org.example.tools", "name": "Tools", "repo": "https://github.com/example/tools",
        "versions": [{"version": "1.0.0", "apps": {"figcraft": {"api": 6}}, "url": ASSET, "sha256": sha, "size": size}]}]})
    .to_string()
    .into_bytes()
}

#[test]
fn installs_from_a_catalog() {
    let bytes = bundle_bytes("org.example.tools", "1.0.0", "read");
    let mut fake = Fake::default();
    fake.files.insert(CATALOG.into(), catalog(&sha256_hex(&bytes), bytes.len()));
    fake.files.insert(ASSET.into(), bytes.clone());
    let (c, warnings) = fetch_catalog(&mut fake, CATALOG).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = &c.packages[0];
    let got = download_release(&mut fake, CATALOG, p, &p.versions[0]).unwrap();
    assert_eq!(got, bytes);
    let (_, manifest) = bundle::open(&got).unwrap();
    assert_eq!(p.check_manifest(&p.versions[0], &manifest), Ok(()));
    let (_, other) = bundle::open(&bundle_bytes("org.example.other", "1.0.0", "read")).unwrap();
    assert!(p.check_manifest(&p.versions[0], &other).unwrap_err().contains("org.example.other"));
    let (_, newer) = bundle::open(&bundle_bytes("org.example.tools", "1.0.1", "read")).unwrap();
    assert!(p.check_manifest(&p.versions[0], &newer).unwrap_err().contains("1.0.1"));
}

#[test]
fn a_catalog_download_must_match_its_pin() {
    let bytes = bundle_bytes("org.example.tools", "1.0.0", "read");
    let mut fake = Fake::default();
    fake.files.insert(ASSET.into(), bytes.clone());
    for (sha, size, why) in [("e".repeat(64), bytes.len(), "SHA-256"), (sha256_hex(&bytes), bytes.len() + 1, "bytes")] {
        fake.files.insert(CATALOG.into(), catalog(&sha, size));
        let (c, _) = fetch_catalog(&mut fake, CATALOG).unwrap();
        let e = download_release(&mut fake, CATALOG, &c.packages[0], &c.packages[0].versions[0]).unwrap_err();
        assert!(e.contains(why), "{e}");
    }
    // A smaller pinned size caps what is read.
    fake.files.insert(CATALOG.into(), catalog(&sha256_hex(&bytes), bytes.len() - 1));
    let (c, _) = fetch_catalog(&mut fake, CATALOG).unwrap();
    assert!(download_release(&mut fake, CATALOG, &c.packages[0], &c.packages[0].versions[0]).is_err());
    assert!(fetch_catalog(&mut fake, "https://example.com/index.json").is_err());
    assert!(fetch_catalog(&mut fake, "https://raw.githubusercontent.com/missing.json").unwrap_err().contains("wasn't found"));
}

fn release(with_sums: bool) -> Vec<u8> {
    let mut assets = vec![json!({"name": "tools.craftplugin", "size": 0, "browser_download_url": ASSET})];
    if with_sums {
        assets.push(json!({"name": "SHA256SUMS", "size": 100, "browser_download_url": SUMS}));
    }
    json!({"tag_name": "v1.0.0", "assets": assets}).to_string().into_bytes()
}

#[test]
fn installs_from_a_repository() {
    let bytes = bundle_bytes("org.example.tools", "1.0.0", "write");
    let mut fake = Fake::default();
    fake.files.insert(API.into(), release(true));
    fake.files.insert(SUMS.into(), format!("{}  tools.craftplugin\n", sha256_hex(&bytes)).into_bytes());
    fake.files.insert(ASSET.into(), bytes.clone());
    let repo = parse_repo_spec("example/tools").unwrap();
    let plan = resolve_repo(&mut fake, &repo).unwrap();
    assert_eq!(fake.asked, [API, SUMS], "nothing but the API answer and the checksums before consent");
    assert_eq!((plan.repo.tag.as_deref(), plan.sha256.clone()), (Some("v1.0.0"), sha256_hex(&bytes)));
    assert_eq!(download_repo(&mut fake, &plan).unwrap(), bytes);
    fake.files.insert(ASSET.into(), bundle_bytes("org.example.tools", "1.0.1", "write"));
    assert!(download_repo(&mut fake, &plan).unwrap_err().contains("SHA-256"));
}

#[test]
fn a_repository_without_checksums_or_release_is_refused() {
    let repo = parse_repo_spec("example/tools").unwrap();
    let mut fake = Fake::default();
    assert!(resolve_repo(&mut fake, &repo).unwrap_err().contains("no such release"));
    fake.files.insert(API.into(), release(false));
    assert!(resolve_repo(&mut fake, &repo).unwrap_err().contains("no SHA-256"));
    fake.files.insert(API.into(), release(true));
    assert!(resolve_repo(&mut fake, &repo).unwrap_err().contains("wasn't found"));
}

/// A client that may read local files only under `root`, and records what it was asked for.
struct Confined {
    root: std::path::PathBuf,
    read: Vec<std::path::PathBuf>,
}

impl Fetch for Confined {
    fn get(&mut self, url: &str, _max: usize) -> Result<Option<Vec<u8>>, String> {
        Err(format!("no network in this test ({url})"))
    }

    fn read_local(&mut self, path: &std::path::Path, max: usize) -> Result<Vec<u8>, String> {
        self.read.push(path.to_path_buf());
        if !path.starts_with(&self.root) {
            return Err(format!("{} is outside the automation roots", path.display()));
        }
        crate::fetch::read_local_file(path, max)
    }
}

/// A local catalog in `dir` that names `bundle` (a local file) at `bundle_path`.
fn local_catalog(dir: &std::path::Path, bundle: &[u8], bundle_path: &std::path::Path) -> String {
    let url = format!("file://{}", bundle_path.display());
    let catalog = json!({"schema": 1, "packages": [{"id": "org.example.tools", "name": "Tools",
        "versions": [{"version": "1.0.0", "apps": {"figcraft": {"api": 6}}, "url": url, "sha256": sha256_hex(bundle), "size": bundle.len()}]}]});
    std::fs::write(dir.join("index.json"), catalog.to_string()).unwrap();
    format!("file://{}", dir.join("index.json").display())
}

#[cfg(unix)]
#[test]
fn local_catalogs_are_read_through_the_apps_file_policy() {
    let inside = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let bytes = bundle_bytes("org.example.tools", "1.0.0", "read");
    std::fs::write(inside.path().join("tools.craftplugin"), &bytes).unwrap();
    std::fs::write(outside.path().join("tools.craftplugin"), &bytes).unwrap();

    // The default reads the disk.
    let url = local_catalog(inside.path(), &bytes, &inside.path().join("tools.craftplugin"));
    let (c, warnings) = fetch_catalog(&mut Fake::default(), &url).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(download_release(&mut Fake::default(), &url, &c.packages[0], &c.packages[0].versions[0]).unwrap(), bytes);

    // A confined client reads only under its root: the catalog and a release inside pass…
    let mut confined = Confined { root: inside.path().to_path_buf(), read: vec![] };
    let (c, _) = fetch_catalog(&mut confined, &url).unwrap();
    assert_eq!(download_release(&mut confined, &url, &c.packages[0], &c.packages[0].versions[0]).unwrap(), bytes);
    assert_eq!(confined.read, [inside.path().join("index.json"), inside.path().join("tools.craftplugin")]);
    // …a release a local catalog names outside it is refused…
    let url = local_catalog(inside.path(), &bytes, &outside.path().join("tools.craftplugin"));
    let (c, _) = fetch_catalog(&mut confined, &url).unwrap();
    let e = download_release(&mut confined, &url, &c.packages[0], &c.packages[0].versions[0]).unwrap_err();
    assert!(e.contains("outside the automation roots"), "{e}");
    // …and so is a catalog outside it.
    let url = local_catalog(outside.path(), &bytes, &outside.path().join("tools.craftplugin"));
    assert!(fetch_catalog(&mut confined, &url).unwrap_err().contains("outside the automation roots"));
}

#[test]
fn redirects_are_checked_hop_by_hop() {
    use crate::fetch::{MAX_REDIRECTS, follow_redirects};
    let hops: HashMap<&str, &str> = HashMap::from([
        (ASSET, "https://release-assets.githubusercontent.com/github-production-release-asset/1?sig=x"),
        ("https://github.com/a/b/releases/download/v1/rel", "/a/b/releases/download/v1/abs"),
        ("https://github.com/a/b/releases/download/v1/abs", "//objects.githubusercontent.com/x"),
        ("https://github.com/a/b/releases/download/v1/evil", "https://evil.example/x"),
        ("https://github.com/a/b/releases/download/v1/plain", "http://objects.githubusercontent.com/x"),
        ("https://github.com/a/b/releases/download/v1/odd", "x/y"),
        ("https://github.com/a/b/releases/download/v1/loop", "https://github.com/a/b/releases/download/v1/loop"),
    ]);
    let mut asked = vec![];
    let mut probe = |u: &str| {
        asked.push(u.to_string());
        hops.get(u).map(|s| s.to_string())
    };
    assert_eq!(follow_redirects(ASSET, &mut probe).unwrap(), "https://release-assets.githubusercontent.com/github-production-release-asset/1?sig=x");
    assert_eq!(follow_redirects("https://github.com/a/b/releases/download/v1/rel", &mut probe).unwrap(), "https://objects.githubusercontent.com/x");
    for bad in ["evil", "plain", "odd"] {
        let e = follow_redirects(&format!("https://github.com/a/b/releases/download/v1/{bad}"), &mut probe).unwrap_err();
        assert!(e.contains("redirected"), "{bad}: {e}");
    }
    let e = follow_redirects("https://github.com/a/b/releases/download/v1/loop", &mut probe).unwrap_err();
    assert!(e.contains(&format!("more than {MAX_REDIRECTS}")), "{e}");
    assert!(follow_redirects("https://example.com/x", &mut probe).is_err(), "the start is checked too");
    assert!(follow_redirects(CATALOG, &mut |_| None).is_ok());
    assert_eq!(asked.iter().filter(|u| u.ends_with("/loop")).count(), MAX_REDIRECTS);
}

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

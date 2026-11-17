//! The pinned public corpus: the folder `AFFINITY_CORPUS` names, filled by
//! `scripts/fetch-affinity-corpus.sh`. Every file `corpus.tsv` lists must be there unmodified.
//! Without the variable the corpus tests are skipped.
#![allow(dead_code)]

use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// `sha256  name  licence  url` rows.
pub const MANIFEST: &str = include_str!("corpus.tsv");

/// One manifest row.
pub struct Row<'a> {
    pub digest: &'a str,
    pub name: &'a str,
    pub license: &'a str,
    pub url: &'a str,
}

/// The rows of `manifest`. A row has exactly four tab-separated fields: a lowercase hexadecimal
/// SHA-256, a relative `/`-separated name without empty, `.` or `..` parts, a license and an
/// `https://` URL. Each name is listed once; several names may share a digest.
pub fn rows(manifest: &str) -> Result<Vec<Row<'_>>, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut rows = Vec::new();
    for line in manifest.lines().filter(|l| !l.trim().is_empty()) {
        let cols: Vec<&str> = line.split('\t').collect();
        let &[digest, name, license, url] = cols.as_slice() else {
            return Err(format!("corpus row needs four tab-separated fields: {line}"));
        };
        if digest.len() != 64 || !digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!("{name}: invalid SHA-256 {digest:?}"));
        }
        if name.starts_with('/')
            || name.contains(['\\', ':'])
            || name.chars().any(char::is_control)
            || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(format!("{name:?}: the name must be a relative path inside the corpus folder"));
        }
        if license.trim().is_empty() {
            return Err(format!("{name}: no license"));
        }
        if !url.starts_with("https://") || url.len() == "https://".len() {
            return Err(format!("{name}: the URL must use https: {url:?}"));
        }
        if !seen.insert(name) {
            return Err(format!("{name}: listed twice"));
        }
        rows.push(Row { digest, name, license, url });
    }
    Ok(rows)
}

fn manifest() -> Vec<Row<'static>> {
    rows(MANIFEST).unwrap_or_else(|e| panic!("malformed corpus.tsv: {e}"))
}

/// The names of the pinned files.
pub fn names() -> impl Iterator<Item = &'static str> {
    manifest().into_iter().map(|row| row.name)
}

pub fn pinned() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("AFFINITY_CORPUS").map(PathBuf::from) else {
        eprintln!("Affinity corpus not given: run scripts/fetch-affinity-corpus.sh and set AFFINITY_CORPUS to its folder");
        return None;
    };
    let mut count = 0;
    for Row { digest, name, .. } in manifest() {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("missing pinned Affinity file {name}: {e}"));
        let hex: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, digest, "{name}: pinned corpus modified; run scripts/fetch-affinity-corpus.sh");
        count += 1;
    }
    assert!(count >= 4, "the pinned manifest must include the four Affinity 3 .af fixtures");
    Some(dir)
}

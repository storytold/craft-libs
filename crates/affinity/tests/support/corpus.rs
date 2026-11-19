//! The pinned public corpus: the folder `AFFINITY_CORPUS` names, filled by
//! `scripts/fetch-affinity-corpus.sh`. Every file `corpus.tsv` lists must be there unmodified.
//! Without the variable the corpus tests are skipped.
#![allow(dead_code)]

use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// `sha256  name  licence  url` rows.
pub const MANIFEST: &str = include_str!("corpus.tsv");

/// The names of the pinned files.
pub fn names() -> impl Iterator<Item = &'static str> {
    MANIFEST.lines().filter_map(|l| l.split('\t').nth(1))
}

pub fn pinned() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("AFFINITY_CORPUS").map(PathBuf::from) else {
        eprintln!("Affinity corpus not given: run scripts/fetch-affinity-corpus.sh and set AFFINITY_CORPUS to its folder");
        return None;
    };
    let mut count = 0;
    for line in MANIFEST.lines().filter(|l| !l.trim().is_empty()) {
        let mut cols = line.split('\t');
        let (Some(digest), Some(name)) = (cols.next(), cols.next()) else { panic!("malformed corpus row: {line}") };
        assert_eq!(digest.len(), 64, "invalid SHA256 for {name}");
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("missing pinned Affinity file {name}: {e}"));
        let hex: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, digest, "{name}: pinned corpus modified; run scripts/fetch-affinity-corpus.sh");
        count += 1;
    }
    assert!(count >= 4, "the pinned manifest must include the four Affinity 3 .af fixtures");
    Some(dir)
}

//! The pinned RAF corpus list `docs/raf-corpus.sha256` holds `#` comment lines, then one
//! `sha256<TAB>path[<TAB>license[<TAB>url]]` row per file. The test reads the list only, not the corpus files.

use std::collections::HashSet;

const MANIFEST: &str = include_str!("../../docs/raf-corpus.sha256");

#[test]
fn raf_corpus_list_is_a_tab_separated_manifest() {
    let mut paths = HashSet::new();
    let mut digests = HashSet::new();
    for line in MANIFEST.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let cols: Vec<&str> = line.split('\t').collect();
        assert!((2..=4).contains(&cols.len()), "not a tab-separated manifest row: {line:?}");
        let (digest, path) = (cols[0], cols[1]);
        assert!(digest.len() == 64 && digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "not a lowercase SHA-256 digest: {line:?}");
        assert!(path.starts_with("corpus/raw/") && path.len() > "corpus/raw/.raf".len() && path.ends_with(".raf"), "not a corpus RAF path: {line:?}");
        assert!(!path.contains(char::is_whitespace), "whitespace in the path: {line:?}");
        assert!(cols[2..].iter().all(|c| !c.is_empty()), "empty optional column: {line:?}");
        assert!(paths.insert(path), "path listed twice: {path}");
        assert!(digests.insert(digest), "digest listed twice: {digest}");
    }
    assert_eq!(paths.len(), 17, "the pinned list holds the 17 CC0 RAF files");
}

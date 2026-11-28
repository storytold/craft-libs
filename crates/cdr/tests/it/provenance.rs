//! Every layout id the code cites (`R1` ...) has a row in `PROVENANCE.md`.

use std::collections::BTreeSet;

/// The ids `R<n>` in `text`.
fn ids(text: &str) -> BTreeSet<u32> {
    let b = text.as_bytes();
    let mut out = BTreeSet::new();
    for (i, c) in b.iter().enumerate() {
        let starts = *c == b'R' && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if !starts {
            continue;
        }
        let digits: String = b[i + 1..].iter().take_while(|d| d.is_ascii_digit()).map(|d| *d as char).collect();
        let ends = b.get(i + 1 + digits.len()).is_none_or(|d| !d.is_ascii_alphanumeric());
        if !digits.is_empty() && ends {
            out.insert(digits.parse().unwrap());
        }
    }
    out
}

#[test]
fn every_cited_layout_has_a_row() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let table = std::fs::read_to_string(dir.join("PROVENANCE.md")).unwrap();
    let rows: BTreeSet<u32> = table.lines().filter_map(|l| l.strip_prefix("| R")).filter_map(|l| l.split(' ').next()?.parse().ok()).collect();
    let mut cited = BTreeSet::new();
    for entry in std::fs::read_dir(dir.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            cited.extend(ids(&std::fs::read_to_string(&path).unwrap()));
        }
    }
    assert!(!cited.is_empty());
    let missing: Vec<_> = cited.difference(&rows).collect();
    assert!(missing.is_empty(), "cited without a PROVENANCE.md row: {missing:?}");
}

#[test]
fn ids_are_found_only_as_whole_words() {
    assert_eq!(ids("(R1, R12) RIFF R3x xR4 R"), BTreeSet::from([1, 12]));
}

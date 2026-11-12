//! Public Affinity documents: the pinned set `scripts/fetch-affinity-corpus.sh` fetches (its
//! folder named by `AFFINITY_CORPUS`, checked against `tests/support/corpus.tsv`), or any folder
//! of `.af`/`.afdesign`/`.afphoto`/`.afpub` files named by `AFFINITY_SAMPLES` (searched
//! recursively). Without either the tests are skipped.

use std::path::{Path, PathBuf};

use craft_affinity::{Archive, Limits, stream};

use crate::corpus;
use crate::features;

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["af", "afdesign", "afphoto", "afpub", "aftemplate"].contains(&e)) {
            out.push(p);
        }
    }
}

fn samples() -> Option<PathBuf> {
    let pinned = corpus::pinned();
    std::env::var_os("AFFINITY_SAMPLES").map(PathBuf::from).or(pinned)
}

#[test]
fn public_documents_parse_completely() {
    let Some(dir) = samples() else { return };
    let mut paths = Vec::new();
    files(&dir, &mut paths);
    assert!(!paths.is_empty(), "no Affinity documents under {dir:?}");
    let mut failures = Vec::new();
    for p in &paths {
        let bytes = std::fs::read(p).unwrap();
        let result = Archive::open(&bytes, Limits::default()).and_then(|mut a| {
            let doc = a.read("doc.dat")?;
            stream::parse(&doc).map(|s| s.objects.len())
        });
        match result {
            Ok(n) => assert!(n > 0),
            Err(e) => failures.push(format!("{}: {e}", p.display())),
        }
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), paths.len(), failures.join("\n"));
    eprintln!("{} Affinity documents parsed", paths.len());
}

#[test]
fn public_documents_map_to_the_model() {
    let Some(dir) = samples() else { return };
    let mut paths = Vec::new();
    files(&dir, &mut paths);
    assert!(!paths.is_empty(), "no Affinity documents under {dir:?}");
    let mut warnings = std::collections::BTreeMap::<String, usize>::new();
    let mut failures = Vec::new();
    let (mut nodes, mut docs) = (0usize, 0usize);
    fn count(n: &[craft_affinity::model::Node]) -> usize {
        n.iter().map(|n| 1 + count(&n.children)).sum()
    }
    for p in &paths {
        let bytes = std::fs::read(p).unwrap();
        match craft_affinity::read(&bytes, Limits::default()) {
            Ok(d) => {
                docs += 1;
                nodes += d.spreads.iter().map(|s| count(&s.nodes)).sum::<usize>();
                for w in d.warnings {
                    let key = w.split(" (").next().unwrap_or(&w).to_string();
                    *warnings.entry(key).or_default() += 1;
                }
            }
            Err(e) => failures.push(format!("{}: {e}", p.display())),
        }
    }
    eprintln!("{docs} documents, {nodes} nodes");
    for (w, n) in &warnings {
        eprintln!("{n:4} documents: {w}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Measured structural baselines for actual Affinity 3 files. Unlike a parse-success test,
/// this detects disappearing nodes, curves, runs, font names, artboards and loss warnings.
/// It does not claim that an unsupported feature is faithfully rendered.
#[test]
fn affinity3_feature_retention_matches_pinned_signatures() {
    let Some(dir) = corpus::pinned() else { return };
    let expected = include_str!("../support/affinity3-signatures.txt");
    let mut checked = std::collections::BTreeSet::new();
    for line in expected.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let (name, signature) = line.split_once('\t').expect("signature row must have a tab");
        assert!(checked.insert(name), "duplicate fixture {name}");
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let doc = craft_affinity::read(&bytes, Limits::default()).unwrap();
        assert_eq!(
            features::signature(&doc),
            signature,
            "{name}: imported feature retention changed; review the pinned input and warning changes before updating this baseline"
        );
        assert!(
            doc.saved_by.as_deref().is_some_and(|v| v.starts_with("Affinity 3.")),
            "{name}: expected an actual Affinity 3 document, got {:?}",
            doc.saved_by
        );
    }
    let af_names: std::collections::BTreeSet<_> = corpus::names().filter(|n| n.ends_with(".af")).collect();
    assert_eq!(checked, af_names, "every pinned .af file needs a reviewed structural baseline");
}

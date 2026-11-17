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
    folder(std::env::var_os("AFFINITY_SAMPLES"), corpus::pinned)
}

/// The `AFFINITY_SAMPLES` folder when one is given, without reading the pinned corpus; otherwise
/// the pinned corpus.
fn folder(samples: Option<std::ffi::OsString>, pinned: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    samples.map(PathBuf::from).or_else(pinned)
}

#[test]
fn a_samples_folder_is_used_without_the_pinned_corpus() {
    assert_eq!(folder(Some("samples".into()), || panic!("the pinned corpus was read")), Some(PathBuf::from("samples")));
    assert_eq!(folder(None, || Some(PathBuf::from("pinned"))), Some(PathBuf::from("pinned")));
    assert_eq!(folder(None, || None), None);
}

#[test]
fn the_pinned_manifest_is_well_formed() {
    let rows = corpus::rows(corpus::MANIFEST).unwrap();
    assert!(rows.iter().filter(|r| r.name.ends_with(".af")).count() >= 4);
}

#[test]
fn the_manifest_check_rejects_malformed_rows() {
    let digest = "0".repeat(64);
    let url = "https://example.com/a.af";
    let row = |name: &str| format!("{digest}\t{name}\tMIT\t{url}\n");
    // Two names may share a digest.
    assert!(corpus::rows(&(row("a.af") + &row("dir/b.af"))).is_ok());
    for (bad, why) in [
        (row("a.af") + &row("a.af"), "listed twice"),
        (format!("{digest}\ta.af\tMIT\n"), "four tab-separated fields"),
        (format!("{digest}\ta.af\tMIT\t{url}\textra\n"), "four tab-separated fields"),
        (format!("{}\ta.af\tMIT\t{url}\n", "0".repeat(63)), "invalid SHA-256"),
        (format!("{}\ta.af\tMIT\t{url}\n", "A".repeat(64)), "invalid SHA-256"),
        (row("../a.af"), "relative path"),
        (row("/a.af"), "relative path"),
        (row("dir//a.af"), "relative path"),
        (row("dir\\a.af"), "relative path"),
        (row("C:a.af"), "relative path"),
        (format!("{digest}\ta.af\t \t{url}\n"), "no license"),
        (format!("{digest}\ta.af\tMIT\thttp://example.com/a.af\n"), "https"),
        (format!("{digest}\ta.af\tMIT\thttps://\n"), "https"),
    ] {
        match corpus::rows(&bad) {
            Ok(_) => panic!("accepted {bad:?}"),
            Err(e) => assert!(e.contains(why), "{bad:?}: {e}"),
        }
    }
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

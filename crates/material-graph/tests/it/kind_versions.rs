//! The node kind versions (`src/versions.rs`, written by `build.rs`) over the evaluator's real source: that the
//! splitting works on the real code, not only on small examples. `TC_VERSIONS_DUMP=1` prints what each kind's version
//! covers. The build script's own modules are compiled here as well, so their unit tests run.

// The build script's `digest` module is declared at the test crate's root (`main.rs`), where `source` looks for it.
#[allow(dead_code)]
#[path = "../../build/source/mod.rs"]
mod source;

use source::items::Dispatcher;
use source::modules::reachable;
use source::{SourceFile, SourceTree};
use std::path::Path;

const DISPATCHERS: &[Dispatcher] = &[
    Dispatcher { file: "src/graph.rs", function: "compute" },
    Dispatcher { file: "src/graph.rs", function: "finish_node" },
    Dispatcher { file: "src/nodes2.rs", function: "compute" },
    Dispatcher { file: "src/atomic.rs", function: "compute" },
    Dispatcher { file: "src/compiled_ops.rs", function: "compute" },
];
const SHARED: &[&str] = &["eval_node", "evaluate_with_inputs", "parse"];

fn rust_files(dir: &Path, root: &Path, out: &mut Vec<SourceFile>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path.strip_prefix(root).unwrap().display().to_string().replace('\\', "/");
            let text = std::fs::read_to_string(&path).unwrap();
            let included = source::included_files(&text, path.parent().unwrap());
            out.push(SourceFile { name, text, included });
        }
    }
}

/// This crate's files by their names in it (`src/graph.rs`).
fn engine() -> Vec<SourceFile> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), root, &mut files);
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

#[test]
fn a_fix_in_one_operation_leaves_most_kinds_alone() {
    let all = engine();
    let files: Vec<SourceFile> = reachable(&all, "src", &["src/graph.rs"]).into_iter().cloned().collect();
    if std::env::var_os("TC_VERSIONS_DUMP").is_some() {
        println!("{} of {} files reachable", files.len(), all.len());
        files.iter().for_each(|f| println!("  {}", f.name));
    }
    assert!(files.len() > 20, "{} of {} files reachable", files.len(), all.len());
    let started = std::time::Instant::now();
    let tree = SourceTree::scan(&files, DISPATCHERS);
    eprintln!("DIAG kind_versions first scan {:?}", started.elapsed());
    let kinds: Vec<String> = tree.kinds().map(str::to_string).collect();
    assert!(kinds.len() > 100, "the kinds were found: {}", kinds.len());
    let versions = |t: &SourceTree| kinds.iter().map(|k| t.kind_version(k, SHARED).unwrap()).collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let before = versions(&tree);
    eprintln!("DIAG kind_versions first versions {:?}", started.elapsed());
    let distinct: std::collections::BTreeSet<_> = before.iter().collect();
    if std::env::var_os("TC_VERSIONS_DUMP").is_some() {
        for k in &kinds {
            let c = tree.kind_covers(k, SHARED).unwrap();
            println!("{k}\t{} functions\t{} files", c.functions, c.files.len());
        }
        println!("{} kinds, {} distinct versions", kinds.len(), distinct.len());
        for (file, names) in tree.functions_covered(None, SHARED) {
            let shown: Vec<&str> = names.iter().map(String::as_str).collect();
            println!("shared {file} {}: {}", names.len(), shown.join(" "));
        }
    }
    assert!(distinct.len() > kinds.len() / 2, "{} distinct versions of {} kinds", distinct.len(), kinds.len());

    let changed_by_all = |edits: &[(&str, &str, &str)]| {
        let edited: Vec<SourceFile> = files
            .iter()
            .map(|f| {
                let mut text = f.text.clone();
                for (file, from, to) in edits.iter().filter(|e| e.0 == f.name) {
                    let next = text.replacen(from, to, 1);
                    assert!(next != text, "{from} is in {file}");
                    text = next;
                }
                SourceFile { text, ..f.clone() }
            })
            .collect();
        let started = std::time::Instant::now();
        let scanned = SourceTree::scan(&edited, DISPATCHERS);
        let scan = started.elapsed();
        let after = versions(&scanned);
        eprintln!(
            "DIAG kind_versions edit of {:?}: scan {scan:?}, versions {:?}",
            edits.iter().map(|e| e.0).collect::<Vec<_>>(),
            started.elapsed() - scan
        );
        kinds.iter().zip(before.iter().zip(&after)).filter(|(_, (a, b))| a != b).map(|(k, _)| k.clone()).collect::<Vec<String>>()
    };
    let changed_by = |file: &str, from: &str, to: &str| changed_by_all(&[(file, from, to)]);

    // an added function nothing uses, and a comment: both change nothing (edits in two files, one pass)
    let unchanged = changed_by_all(&[
        ("src/compiled_ops.rs", "pub fn compute(kind: &str", "fn __added_helper() -> u8 { 1 }\npub fn compute(kind: &str"),
        ("src/atomic.rs", "\"levels_mid\" => {", "\"levels_mid\" => { // a comment\n"),
    ]);
    assert_eq!(unchanged, Vec::<String>::new(), "an unused added function and a comment change nothing");

    // a fix inside one node's arm
    let arm = changed_by("src/atomic.rs", "\"levels_mid\" => {", "\"levels_mid\" => { let _fixed = 1;");
    assert_eq!(arm, ["levels_mid"], "only that kind");

    // a fix in a helper only some kinds reach: those kinds and few others
    let helper = changed_by(
        "src/atomic.rs",
        "fn hsl_shift(p: [f32; 4], h: f32, s: f32, l: f32) -> [f32; 4] {",
        "fn hsl_shift(p: [f32; 4], h: f32, s: f32, l: f32) -> [f32; 4] { let _fixed = 1;",
    );
    assert!(helper.contains(&"hsl_centered".to_string()), "{helper:?}");
    assert!(helper.len() < 5, "{helper:?}");

    // what every kind goes through changes every kind
    // (the cancellation check every node evaluation makes, outside any kind's arm)
    let everyone = changed_by("src/graph.rs", "if ctx.ec.is_cancelled() {", "if ctx.ec.is_cancelled() { let _changed = 1;");
    assert_eq!(everyone.len(), kinds.len());
}

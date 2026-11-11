//! Corpus test (feature `corpus`): every `.atn` under
//! `corpus/atn/` (or `$CRAFT_PSD_CORPUS/atn/`; PhotoCraft's `cargo xtask corpus --atn` fetches it) parses, and writing it back
//! reproduces the bytes exactly. A missing corpus fails.
#![cfg(feature = "corpus")]

use std::path::{Path, PathBuf};

use craft_psd::atn;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    // An unreadable folder fails the test rather than shrinking the corpus.
    let rd = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for e in rd {
        let p = e.unwrap_or_else(|e| panic!("{}: {e}", dir.display())).path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("atn")) {
            out.push(p);
        }
    }
}

/// Sets in the corpus; raise, never lower.
const FLOOR: usize = 16;

#[test]
fn atn_corpus_parses_and_is_byte_stable() {
    let root = crate::corpus_dir("atn");
    assert!(root.is_dir(), "{} is missing: run `cargo xtask corpus --atn`", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut failures = Vec::new();
    let (mut actions, mut steps) = (0usize, 0usize);
    for p in &files {
        let name = p.strip_prefix(&root).unwrap_or(p).display().to_string();
        let bytes = std::fs::read(p).unwrap();
        let file = match atn::parse(&bytes) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        if !file.warnings.is_empty() {
            failures.push(format!("{name}: warnings {:?}", file.warnings));
        }
        actions += file.actions.len();
        steps += file.actions.iter().map(|a| a.items.len()).sum::<usize>();
        match atn::write(&file) {
            Ok(written) if written == bytes => {}
            Ok(written) => failures.push(format!("{name}: rewritten bytes differ ({} vs {} bytes)", written.len(), bytes.len())),
            Err(e) => failures.push(format!("{name}: write failed: {e}")),
        }
        println!("{name}: {} actions, {} steps", file.actions.len(), file.actions.iter().map(|a| a.items.len()).sum::<usize>());
    }
    println!("{} files, {actions} actions, {steps} steps", files.len());
    assert!(files.len() >= FLOOR, "only {} .atn files; the corpus floor is {FLOOR}", files.len());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

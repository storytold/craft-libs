//! Paths long enough that a per-level recursive descent would exhaust the stack. Parsing is
//! iterative — `resolve` and `classify` reach `parse_share` at a fixed depth — so these come
//! back; a refusal is fine, a crash is not.

use craft_docpath::{classify, resolve};

/// Half a megabyte of components under a UNC share: one component per level for a parser that
/// recursed on each separator.
#[test]
fn a_half_megabyte_unc_path_comes_back() {
    let mut path = String::from(r"\\?\UNC\host\share");
    for _ in 0..(1 << 18) {
        path.push_str(r"\d");
    }
    let _ = resolve(&path, None);
    let _ = classify(&path);
}

/// Sixteen thousand nested verbatim prefixes: each level was a candidate recursive call when the
/// UNC prefixes were first reviewed.
#[test]
fn nested_verbatim_prefixes_come_back() {
    let mut path = String::new();
    for _ in 0..(1 << 14) {
        path.push_str(r"\\?\UNC\");
    }
    let _ = resolve(&path, None);
    let _ = classify(&path);
}

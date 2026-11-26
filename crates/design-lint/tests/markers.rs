//! Integration tests of craft-design-lint: markers.

#![allow(clippy::unwrap_used, reason = "tests")]

use craft_design_lint::{Finding, analyze_with_markers};

fn run(src: &str) -> (Vec<Finding>, usize) {
    let marked = analyze_with_markers(src, false, false, &|_| false);
    (marked.findings, marked.markers)
}

#[test]
fn alone_marker_before_a_fn_covers_its_header_not_its_body() {
    let src = "// design: allow(canvas units)\n#[inline]\nfn f(ui: &mut Ui) {\n    ui.add_space(4.0);\n    let _ = Color32::WHITE;\n}\n";
    let (findings, _) = run(src);
    assert_eq!(
        findings.iter().map(|f| (f.rule, f.line)).collect::<Vec<_>>(),
        [("marker/unused", 1), ("literal-size/add-space", 4), ("raw-colour/const", 5)]
    );
}

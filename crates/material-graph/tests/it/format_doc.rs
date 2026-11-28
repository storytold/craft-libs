//! The format reference's example parses and renders and its tables list every node kind.

use craft_material_graph::graph::{self, Graph};

const RAW_DOC: &str = include_str!("../../docs/format.md");

/// The reference as LF text: a Windows checkout may convert its line endings.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The first ```text block of the reference.
fn example(raw: &str) -> String {
    let doc = lf(raw);
    let start = doc.find("```text\n").expect("an example") + "```text\n".len();
    let len = doc.get(start..).and_then(|r| r.find("```")).expect("the example ends");
    doc[start..start + len].to_string()
}

#[test]
fn the_example_renders() {
    let g = Graph::parse(&example(RAW_DOC)).expect("the example parses");
    assert_eq!(g.format, graph::FORMAT_VERSION);
    let out = g.evaluate(Some(16)).expect("the example renders");
    assert!(out.contains_key("height"));
}

/// A Windows checkout has CRLF line endings; the example must read the same there.
#[test]
fn the_example_reads_the_same_with_crlf_line_endings() {
    let crlf = lf(RAW_DOC).replace('\n', "\r\n");
    assert_eq!(example(&crlf), example(RAW_DOC));
}

#[test]
fn every_node_kind_is_listed() {
    let doc = lf(RAW_DOC);
    let all = graph::SPECS
        .iter()
        .chain(craft_material_graph::compiled_ops::SPECS)
        .chain(craft_material_graph::atomic::SPECS)
        .chain(craft_material_graph::func::FUNC_SPECS)
        .chain(craft_material_graph::fxmap::FXM_SPECS)
        .chain(craft_material_graph::patterns::SPECS);
    let missing: Vec<&str> = all.map(|s| s.kind).filter(|k| !doc.contains(&format!("| `{k}` |"))).collect();
    assert!(missing.is_empty(), "docs/format.md does not list {missing:?}");
}

//! The format reference's example parses and renders and its tables list every node kind.

use craft_material_graph::graph::{self, Graph};

const DOC: &str = include_str!("../../docs/format.md");

#[test]
fn the_example_renders() {
    let start = DOC.find("```text\n").expect("an example") + "```text\n".len();
    let len = DOC.get(start..).and_then(|r| r.find("```")).expect("the example ends");
    let g = Graph::parse(&DOC[start..start + len]).expect("the example parses");
    assert_eq!(g.format, graph::FORMAT_VERSION);
    let out = g.evaluate(Some(16)).expect("the example renders");
    assert!(out.contains_key("height"));
}

#[test]
fn every_node_kind_is_listed() {
    let all = graph::SPECS
        .iter()
        .chain(craft_material_graph::compiled_ops::SPECS)
        .chain(craft_material_graph::atomic::SPECS)
        .chain(craft_material_graph::func::FUNC_SPECS)
        .chain(craft_material_graph::fxmap::FXM_SPECS)
        .chain(craft_material_graph::patterns::SPECS);
    let missing: Vec<&str> = all.map(|s| s.kind).filter(|k| !DOC.contains(&format!("| `{k}` |"))).collect();
    assert!(missing.is_empty(), "docs/format.md does not list {missing:?}");
}

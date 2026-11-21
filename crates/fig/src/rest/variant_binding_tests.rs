//! A variant property bound to a variable: the file keeps it as a `RESOLVE_VARIANT` expression
//! over a map from the property to the variable (in an instance's variable or parameter
//! consumption map, `VARIANT_PROPERTIES`). It maps to REST `boundVariables.componentProperties`,
//! and every variant of the main's set comes across, since the variable can pick any of them.

use super::tests::{arr, g, node, o, s};
use super::{Kv, Options, to_rest};

fn e(n: &'static str) -> Kv<'static> {
    Kv::Enum(n)
}

/// `{Mode: variable 60}` as the file writes it.
fn resolve_variant() -> Kv<'static> {
    let alias = o(vec![("dataType", e("ALIAS")), ("resolvedDataType", e("STRING")), ("value", o(vec![("alias", o(vec![("guid", g(60))]))]))]);
    let entry = o(vec![("key", s("Mode")), ("value", alias)]);
    let arg =
        o(vec![("dataType", e("MAP")), ("resolvedDataType", e("MAP")), ("value", o(vec![("mapValue", o(vec![("values", arr(vec![entry]))]))]))]);
    let expr = o(vec![("expressionFunction", e("RESOLVE_VARIANT")), ("expressionArguments", arr(vec![arg]))]);
    let data = o(vec![("dataType", e("EXPRESSION")), ("resolvedDataType", e("SYMBOL_ID")), ("value", o(vec![("expressionValue", expr)]))]);
    o(vec![("entries", arr(vec![o(vec![("variableField", e("VARIANT_PROPERTIES")), ("variableData", data)])]))])
}

/// A page with a Chip instance of a library copy's Light variant, its Mode bound to a string
/// variable in `map`; the copy's set also holds a Dark variant no instance names.
fn file(map: &'static str) -> Kv<'static> {
    file_with(map, vec![], vec![])
}

/// [`file`], with `more` fields on the instance and `extra` layers.
fn file_with(map: &'static str, more: Vec<(&'static str, Kv<'static>)>, extra: Vec<Kv<'static>>) -> Kv<'static> {
    let modes = arr(vec![o(vec![("id", g(70)), ("name", s("Light"))]), o(vec![("id", g(71)), ("name", s("Dark"))])]);
    o(vec![(
        "nodeChanges",
        Kv::Array(
            vec![
                node(0, None, "DOCUMENT", "Document", vec![]),
                node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
                node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
                node(3, Some((2, "!")), "FRAME", "Chip", vec![("isStateGroup", Kv::Bool(true))]),
                node(4, Some((3, "!")), "SYMBOL", "Mode=Light", vec![]),
                node(5, Some((3, "\"")), "SYMBOL", "Mode=Dark", vec![]),
                node(6, Some((2, "\"")), "VARIABLE_SET", "Scheme", vec![("variableSetModes", modes)]),
                node(60, Some((2, "#")), "VARIABLE", "Mode", vec![("variableSetID", o(vec![("guid", g(6))])), ("variableResolvedType", e("STRING"))]),
                node(
                    10,
                    Some((1, "!")),
                    "INSTANCE",
                    "Chip instance",
                    [vec![("symbolData", o(vec![("symbolID", g(4))])), (map, resolve_variant())], more].concat(),
                ),
            ]
            .into_iter()
            .chain(extra)
            .collect(),
        ),
    )])
}

fn has_layer(v: &serde_json::Value, name: &str) -> bool {
    v["name"] == name || v["children"].as_array().into_iter().flatten().any(|c| has_layer(c, name))
}

#[test]
fn a_variant_bound_to_a_variable_maps_to_component_property_bindings_and_keeps_its_set() {
    for map in ["parameterConsumptionMap", "variableConsumptionMap"] {
        let out = to_rest(&file(map), "Fixture", &Options::default());
        let inst = super::tests::named(&out.json, "Chip instance");
        assert_eq!(inst["boundVariables"]["componentProperties"]["Mode"]["id"], "1:60", "{map}: {inst}");
        assert_eq!(inst["boundVariables"]["componentProperties"]["Mode"]["type"], "VARIABLE_ALIAS", "{map}");
        assert!(!out.warnings.iter().any(|w| w.contains("variant properties")), "{map}: {:?}", out.warnings);
        let libs = &out.json[super::FIG_LIBRARY_CONTEXTS_FIELD];
        let found = libs.as_array().into_iter().flatten().any(|c| has_layer(c, "Mode=Dark"));
        assert!(found, "{map}: the Dark variant comes across: {libs}");
    }
}

fn libs_have(out: &super::Mapped, name: &str) -> bool {
    out.json[super::FIG_LIBRARY_CONTEXTS_FIELD].as_array().into_iter().flatten().any(|c| has_layer(c, name))
}

/// The library copy holds a second set too, Badge (Light and Dark), and the Chip instance on the
/// page is swapped to Badge's Light, its Mode bound as before.
#[test]
fn a_swapped_instance_keeps_the_set_of_the_main_it_shows() {
    let badge = vec![
        node(20, Some((2, "$")), "FRAME", "Badge", vec![("isStateGroup", Kv::Bool(true))]),
        node(21, Some((20, "!")), "SYMBOL", "Badge=Light", vec![]),
        node(22, Some((20, "\"")), "SYMBOL", "Badge=Dark", vec![]),
    ];
    let out = to_rest(&file_with("variableConsumptionMap", vec![("overriddenSymbolID", g(21))], badge), "Fixture", &Options::default());
    assert!(libs_have(&out, "Badge=Dark"), "the swapped-in main's Dark variant comes across");
}

/// A set of 300 variants: the one the variable picks may be the last.
#[test]
fn every_variant_of_a_large_set_comes_across() {
    let extra = (0..300u64).map(|i| node(1000 + i, Some((3, &format!("#{i:04}"))), "SYMBOL", &format!("Mode=Extra {i}"), vec![])).collect();
    let out = to_rest(&file_with("variableConsumptionMap", vec![], extra), "Fixture", &Options::default());
    for name in ["Mode=Extra 0", "Mode=Extra 299"] {
        assert!(libs_have(&out, name), "{name} comes across");
    }
}

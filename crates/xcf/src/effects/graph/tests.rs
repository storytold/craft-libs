//! Source syntax and independent dependency expectations, never rendered snapshots.
use super::*;
use std::cell::Cell;

const FIELDS: &[Field<'_>] = &[
    Field { name: "n", kind: FieldType::Number },
    Field { name: "signed", kind: FieldType::Integer },
    Field { name: "seed", kind: FieldType::UnsignedInteger },
    Field { name: "enabled", kind: FieldType::Boolean },
    Field { name: "text", kind: FieldType::Text },
    Field { name: "color", kind: FieldType::Color },
    Field { name: "configuration", kind: FieldType::Configuration },
    Field { name: "mode", kind: FieldType::Choice(&[("first", -1), ("second", 7)]) },
];
const SCHEMA: &[Operation<'_>] = &[
    Operation { name: "gain", fields: FIELDS },
    Operation { name: "invert-linear", fields: &[] },
    Operation { name: "over", fields: &[Field { name: "aux", kind: FieldType::Input }] },
    Operation { name: "color", fields: &[Field { name: "value", kind: FieldType::Color }] },
];
fn read(raw: &str) -> Graph<'_> {
    parse(raw, SCHEMA, &mut Budget::default(), &|| true).unwrap()
}
fn check_edges(graph: &Graph<'_>) {
    for (index, node) in graph.nodes.iter().enumerate() {
        for input in node
            .input
            .iter()
            .chain(node.properties.iter().filter_map(|property| if let Value::Input(input) = &property.value { Some(input) } else { None }))
        {
            if let Input::Node(input) = input {
                assert!(*input < index, "every resolved dependency precedes its user");
            }
        }
    }
}

#[test]
fn application_authored_corpus_graph_forms_preserve_source_and_dependency_order() {
    // Original public GIMP3.2.6 inputs in photocraft-corpus/tools/gimp-xcf/
    // graph_filters.source-facts.json (no bootstrap or private binary fixtures).
    let linear = read("invert-linear");
    assert_eq!(linear.output, Input::Node(0));
    assert_eq!(linear.nodes[0].input, Some(Input::Source));
    assert!(linear.diagnostics.is_empty());
    let joined = read("id=original over aux=[ ref=original invert-linear ]");
    assert_eq!(joined.labels, [Label { name: "original".into(), input: Input::Source }]);
    assert_eq!(joined.nodes.iter().map(|node| node.operation.as_str()).collect::<Vec<_>>(), ["invert-linear", "over"]);
    assert_eq!(joined.nodes.iter().map(|node| node.source_ordinal).collect::<Vec<_>>(), [1, 0]);
    assert_eq!(joined.nodes[1].properties[0].value, Value::Input(Input::Node(0)));
    assert_eq!(joined.output, Input::Node(1));
    assert!(joined.diagnostics.is_empty());
    check_edges(&joined);
}

#[test]
fn declarations_preserve_signedness_exact_literals_and_property_order() {
    let graph = read("gain seed=0 signed=9007199254740993 n=0.12345678901234567 enabled=true mode=second text='hello world'");
    let properties = &graph.nodes[0].properties;
    assert_eq!(properties.iter().map(|property| property.name.as_str()).collect::<Vec<_>>(), ["seed", "signed", "n", "enabled", "mode", "text"]);
    assert_eq!(properties[0].value, Value::UnsignedInteger(0));
    assert_eq!(properties[1].value, Value::Integer(9_007_199_254_740_993));
    assert_eq!(properties[2].value, Value::Number(0.12345678901234567));
    assert_eq!(properties[2].literal, "0.12345678901234567");
    assert_eq!(properties[3].value, Value::Boolean(true));
    assert_eq!(properties[4].value, Value::Choice(7));
    assert_eq!((properties[5].literal.as_str(), &properties[5].value), ("hello world", &Value::Text));
    for invalid in
        ["gain n=NaN", "gain n=inf", "gain signed=1.5", "gain seed=-1", "gain seed=4294967296", "gain enabled=1", "gain mode=missing", "gain n=1 n=2"]
    {
        assert!(parse(invalid, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{invalid}");
    }
}

#[test]
fn colors_and_nested_configuration_remain_typed_inert_source_data() {
    let graph = read(r##"gain color='rgba(-2,1.25,0.5,0.25)' configuration='(x 1) (blob "\377")'"##);
    assert_eq!(graph.nodes[0].properties[0].value, Value::Color(Color { rgba: [-2., 1.25, 0.5, 0.25], space: ColorSpace::LinearRgb }));
    assert_eq!(graph.nodes[0].properties[1].literal, r#"(x 1) (blob "\377")"#);
    assert_eq!(graph.nodes[0].properties[1].value, Value::Configuration);
    for (literal, rgba) in [("#abc", [170., 187., 204., 255.]), ("#abcd", [170., 187., 204., 221.]), ("red", [255., 0., 0., 255.])] {
        let source = format!("color value={literal}");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Color(Color { rgba: rgba.map(|value| value / 255.), space: ColorSpace::Srgb }));
    }
    for invalid in ["color value='rgb(1,2)'", "color value='rgba(1,2,3,2)'", "color value='#xyz'", "color value='rgb(NaN,0,0)'"] {
        assert!(parse(invalid, SCHEMA, &mut Budget::default(), &|| true).is_err());
    }
}

#[test]
fn unsupported_entries_keep_literals_and_indexed_diagnostics_without_execution() {
    let raw = "example:unknown path='file:///never-opened' command='$(not-a-command) ${UNSET}'";
    let graph = read(raw);
    assert_eq!(graph.raw, raw);
    assert_eq!(graph.nodes[0].properties[0].literal, "file:///never-opened");
    assert_eq!(graph.nodes[0].properties[1].literal, "$(not-a-command) ${UNSET}");
    assert!(graph.nodes[0].properties.iter().all(|property| property.value == Value::Unknown));
    assert_eq!(
        graph.diagnostics,
        [
            Diagnostic::UnknownOperation { source_ordinal: 0 },
            Diagnostic::UnknownProperty { source_ordinal: 0, property: 0 },
            Diagnostic::UnknownProperty { source_ordinal: 0, property: 1 }
        ]
    );
}

#[test]
fn graph_wide_labels_bind_only_earlier_results_and_never_cross_parse_calls() {
    let graph = read("id=original over aux=[ref=original invert-linear id=inside] ref=inside");
    assert_eq!(graph.output, Input::Node(0));
    check_edges(&graph);
    for raw in [
        "ref=later invert-linear id=later",
        "id=x id=x",
        "over aux=[ref=parent] id=parent",
        "ref=inside",
        "over aux=[]",
        "invert-linear ]",
        "over aux=[invert-linear",
        "over aux=invert-linear",
    ] {
        assert!(parse(raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{raw}");
    }
    let graph = read("id='\u{03bb}' over aux=[ref='\u{03bb}' invert-linear]");
    assert_eq!(graph.labels[0].name, "\u{03bb}");
    check_edges(&graph);
}

#[test]
fn shell_quoting_comments_and_escapes_only_change_literal_bytes() {
    for (source, expected) in [
        (r#"gain text='one\n$HOME'"#, r"one\n$HOME"),
        (r#"gain text="one\"two\\three\n\$HOME\`tick""#, "one\"two\\three\\n$HOME`tick"),
        (r#"gain text='foo'\''bar'"#, "foo'bar"),
        ("gain text=foo\\ bar", "foo bar"),
        ("gain text=\"a\\\nb\"", "ab"),
    ] {
        assert_eq!(read(source).nodes[0].properties[0].literal, expected);
    }
    assert_eq!(read("# ignored\ngain color=#abc # trailing\n").nodes.len(), 1);
    for invalid in ["gain text='unterminated", "gain text=\"unterminated", "gain text=last\\", "gain text=\"last\\", "# comment\0"] {
        assert!(parse(invalid, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{invalid:?}");
    }
}

#[test]
fn syntax_caps_have_small_exact_boundary_controls() {
    assert_eq!(read(&"invert-linear ".repeat(64)).nodes.len(), 64);
    assert!(parse(&"invert-linear ".repeat(65), SCHEMA, &mut Budget::default(), &|| true).is_err());
    for count in [256, 257] {
        let raw = format!("gain {}", (0..count).map(|index| format!("p{index}=0 ")).collect::<String>());
        assert_eq!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_ok(), count == 256);
    }
    for depth in [8, 9] {
        let mut body = String::from("ref=source invert-linear");
        for _ in 0..depth {
            body = format!("over aux=[ref=source {body}]");
        }
        let raw = format!("id=source {body}");
        assert_eq!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_ok(), depth == 8);
    }
    for repeats in [681, 682] {
        let raw = format!("id=base {}", "ref=base ".repeat(repeats));
        assert_eq!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_ok(), repeats == 681);
    }
    let mut exact = String::from("invert-linear");
    exact.extend(std::iter::repeat_n(' ', MAX_BYTES - exact.len()));
    assert!(parse(&exact, SCHEMA, &mut Budget::default(), &|| true).is_ok());
    exact.push(' ');
    assert!(parse(&exact, SCHEMA, &mut Budget::default(), &|| true).is_err());
}

#[test]
fn failed_or_cancelled_graphs_keep_the_shared_budget_contract() {
    let mut budget = Budget::new(8, 1 << 20);
    assert!(matches!(parse("gain n=1", SCHEMA, &mut budget, &|| true), Err(Error::Limit(Limit::Storage))));
    assert_eq!(budget.remaining_storage(), 8);
    assert!(budget.remaining_work() < 1 << 20);
    let mut budget = Budget::new(1 << 20, 1);
    assert!(matches!(parse("invert-linear", SCHEMA, &mut budget, &|| true), Err(Error::Limit(Limit::Work))));
    assert_eq!(budget.remaining_work(), 0);
    assert!(matches!(parse("invert-linear", SCHEMA, &mut budget, &|| true), Err(Error::Limit(Limit::Work))));
    let raw = format!("gain text='{}'", "a".repeat(20_000));
    let checks = Cell::new(0);
    let keep = || {
        checks.set(checks.get() + 1);
        checks.get() < 4
    };
    let mut budget = Budget::default();
    assert!(matches!(parse(&raw, SCHEMA, &mut budget, &keep), Err(Error::Source(crate::Error::Cancelled))));
    assert!(budget.remaining_storage() < Budget::default().remaining_storage());
    assert!(budget.remaining_work() < Budget::default().remaining_work());
}

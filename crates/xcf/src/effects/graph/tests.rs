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
/// Three tokens for id=base and each ref=base, followed by two one-token operations:
/// 3 + 681 * 3 + 2 = 2048 logical lexical items.
fn exact_token_cap_graph() -> String {
    format!("id=base {}invert-linear invert-linear", "ref=base ".repeat(681))
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
    for invalid in ["gain n=NaN", "gain n=inf", "gain mode=missing", "gain n=1 n=2"] {
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
    for invalid in ["color value='rgb(1,2)'", "color value='rgba(1,2,3,NaN)'", "color value='#xyz'", "color value='rgb(NaN,0,0)'"] {
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
        ("gain text=\"a\\\nb\"", "a\nb"),
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
    let exact_tokens = exact_token_cap_graph();
    assert_eq!(tokenize(&exact_tokens, &mut Budget::default(), &|| true).unwrap().len(), 2048);
    assert!(parse(&exact_tokens, SCHEMA, &mut Budget::default(), &|| true).is_ok());
    let excessive = format!("{exact_tokens} invert-linear");
    assert!(parse(&excessive, SCHEMA, &mut Budget::default(), &|| true).unwrap_err().to_string().contains("2048 lexical items"));
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

#[test]
fn qualified_and_unqualified_gegl_declarations_share_types_without_rewriting_source() {
    for declaration in ["gegl:exposure", "exposure"] {
        let schema = [Operation { name: declaration, fields: &[Field { name: "exposure", kind: FieldType::Number }] }];
        for spelling in ["exposure", "gegl:exposure"] {
            let raw = format!("{spelling} exposure=1.25");
            let graph = parse(&raw, &schema, &mut Budget::default(), &|| true).unwrap();
            assert_eq!(graph.nodes[0].operation, spelling);
            assert_eq!(graph.nodes[0].properties[0].value, Value::Number(1.25));
            assert!(graph.diagnostics.is_empty());
        }
        for spelling in ["other:exposure", "gegl:other:exposure"] {
            let raw = format!("{spelling} exposure=1.25");
            let graph = parse(&raw, &schema, &mut Budget::default(), &|| true).unwrap();
            assert_eq!(graph.nodes[0].operation, spelling);
            assert_eq!(graph.nodes[0].properties[0].value, Value::Unknown);
            assert_eq!(graph.diagnostics.len(), 2);
        }
    }
    let schema = [Operation { name: "other:exposure", fields: &[] }];
    let graph = parse("other:exposure", &schema, &mut Budget::default(), &|| true).unwrap();
    assert!(graph.diagnostics.is_empty());
    assert_eq!(parse("exposure", &schema, &mut Budget::default(), &|| true).unwrap().diagnostics.len(), 1);
}

#[test]
fn exact_choice_names_take_priority_over_numeric_values_in_either_declaration_order() {
    for choices in [[("second", 7), ("7", -1)], [("7", -1), ("second", 7)]] {
        let fields = [Field { name: "mode", kind: FieldType::Choice(&choices) }];
        let schema = [Operation { name: "choice", fields: &fields }];
        for (literal, expected) in [("7", -1), ("second", 7), ("-1", -1)] {
            let raw = format!("choice mode={literal}");
            let graph = parse(&raw, &schema, &mut Budget::default(), &|| true).unwrap();
            assert_eq!(graph.nodes[0].properties[0].value, Value::Choice(expected));
        }
    }
    let graph = read("gain mode=7");
    assert_eq!(graph.nodes[0].properties[0].value, Value::Choice(7), "numeric fallback remains supported");
}

#[test]
fn hexadecimal_color_bytes_require_exactly_two_ascii_digits() {
    for literal in ["#+f0000", "#00+f00", "#000000+f", "#0g0000", "#１２３", "#f", "#fffff"] {
        let raw = format!("color value='{literal}'");
        assert!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{literal}");
    }
    for (literal, rgba) in [("#fF0000", [1., 0., 0., 1.]), ("#fF000080", [1., 0., 0., 128. / 255.])] {
        let raw = format!("color value='{literal}'");
        assert_eq!(read(&raw).nodes[0].properties[0].value, Value::Color(Color { rgba, space: ColorSpace::Srgb }));
    }
}

#[test]
fn transparent_color_names_keep_zero_alpha_and_opaque_names_keep_one() {
    for literal in ["transparent", "none", "TRANSPARENT", "None"] {
        let raw = format!("color value={literal}");
        assert_eq!(read(&raw).nodes[0].properties[0].value, Value::Color(Color { rgba: [0.; 4], space: ColorSpace::Srgb }));
    }
    assert_eq!(read("color value=aqua").nodes[0].properties[0].value, Value::Color(Color { rgba: [0., 1., 1., 1.], space: ColorSpace::Srgb }));
    assert!(parse("color value=not-a-color", SCHEMA, &mut Budget::default(), &|| true).is_err());
}

#[test]
fn unquoted_line_continuations_do_not_emit_words_but_explicit_empty_quotes_do() {
    for raw in ["invert-linear \\\n", "\\\n invert-linear", "invert-linear \\\n \\\n", "invert-\\\nlinear"] {
        let graph = read(raw);
        assert_eq!(graph.nodes.len(), 1, "{raw:?}");
        assert_eq!(graph.nodes[0].operation, "invert-linear");
        assert!(graph.diagnostics.is_empty());
    }
    assert_eq!(read("gain text='' \\\n").nodes[0].properties[0].literal, "");
    assert_eq!(read("gain text=\"a\\\nb\"").nodes[0].properties[0].literal, "a\nb");
    assert_eq!(read("gain text=a\\\nb").nodes[0].properties[0].literal, "ab");
    assert!(parse("invert-linear ''", SCHEMA, &mut Budget::default(), &|| true).is_err());
    let exact_tokens = format!("{} \\\n", exact_token_cap_graph());
    assert!(parse(&exact_tokens, SCHEMA, &mut Budget::default(), &|| true).is_ok(), "a continuation contributes no item at the lexical cap");
}

#[test]
fn relative_numeric_coefficients_remain_inert_until_a_dimension_is_known() {
    for (field, absolute) in [("n", Value::Number(0.5)), ("signed", Value::Integer(1)), ("seed", Value::UnsignedInteger(1))] {
        for literal in ["0.1rel", "0.5rel", "-0.25rel", "1e-3rel"] {
            let source = format!("gain {field}={literal}");
            let graph = read(&source);
            assert_eq!(graph.nodes[0].properties[0].literal, literal);
            let expected = literal.strip_suffix("rel").unwrap().parse::<f64>().unwrap();
            assert_eq!(graph.nodes[0].properties[0].value, Value::RelativeNumber(expected));
            assert!(graph.diagnostics.is_empty());
        }
        let literal = if field == "n" { "0.5" } else { "1" };
        let source = format!("gain {field}={literal}");
        assert_eq!(read(&source).nodes[0].properties[0].value, absolute);
        for literal in ["rel", "NaNrel", "infrel", "-infrel", "1e999rel", "0.5relrel", "junkrel"] {
            let source = format!("gain {field}={literal}");
            assert!(parse(&source, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{source}");
        }
    }
    let graph = read("gain unknown=0.5rel text=0.5rel");
    assert_eq!(graph.nodes[0].properties[0].value, Value::Unknown);
    assert_eq!(graph.nodes[0].properties[1].value, Value::Text);
    assert_eq!(graph.nodes[0].properties[0].literal, "0.5rel");
}

#[test]
fn scalar_assignments_preserve_internal_delimiters_and_empty_right_hand_sides() {
    let fields = [Field { name: "string", kind: FieldType::Text }];
    let schema = [Operation { name: "gegl:text", fields: &fields }];
    for (source, expected) in [
        ("gegl:text string=a=b", "a=b"),
        ("gegl:text string=a[b]c", "a[b]c"),
        ("gegl:text string=a]]b", "a]]b"),
        ("gegl:text string=", ""),
        ("gegl:text string==", "="),
        ("gegl:text string=a=b=c", "a=b=c"),
        ("gegl:text string='[a=b]'", "[a=b]"),
    ] {
        let graph = parse(source, &schema, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].properties[0].literal, expected, "{source}");
        assert_eq!(graph.nodes[0].properties[0].value, Value::Text);
        assert!(graph.diagnostics.is_empty());
    }
    let graph = read("gain text= n=1 configuration=(x[0]=1)");
    assert_eq!(graph.nodes[0].properties[0].literal, "");
    assert_eq!(graph.nodes[0].properties[1].value, Value::Number(1.));
    assert_eq!(graph.nodes[0].properties[2].literal, "(x[0]=1)");
    assert_eq!(graph.nodes[0].properties[2].value, Value::Configuration);
}

#[test]
fn scalar_delimiters_do_not_consume_auxiliary_block_boundaries() {
    for (source, expected) in [
        ("id=base over aux=[ref=base gain text=a[b]c n=1]", "a[b]c"),
        ("over aux=[gain text=a=b]", "a=b"),
        ("over aux=[gain text=]", ""),
        ("over aux=[over aux=[gain text=a[b]c]]", "a[b]c"),
    ] {
        let graph = read(source);
        assert_eq!(graph.nodes[0].operation, "gain");
        assert_eq!(graph.nodes[0].properties[0].literal, expected, "{source}");
        assert_eq!(graph.nodes.last().unwrap().operation, "over");
        assert!(graph.diagnostics.is_empty());
        check_edges(&graph);
    }
    for source in ["over aux=[]", "over aux=[gain text=a[b]c", "gain n=a=b"] {
        assert!(parse(source, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{source}");
    }
}

#[test]
fn cmyk_components_are_retained_without_rgb_conversion_or_unit_normalization() {
    for (literal, cmyk, alpha) in [
        ("cmyk(0.0,25.0,100.0,0.0)", [0., 25., 100., 0.], 1.),
        ("cmyka(0.0,25.0,100.0,0.0,0.5)", [0., 25., 100., 0.], 0.5),
        ("cmyka(0,25,100,0,50)", [0., 25., 100., 0.], 50.),
    ] {
        let source = format!("color value='{literal}'");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::CmykColor { cmyk, alpha });
        assert!(graph.diagnostics.is_empty());
    }
    assert_eq!(
        read("color value='rgb(0,0.25,1)'").nodes[0].properties[0].value,
        Value::Color(Color { rgba: [0., 0.25, 1., 1.], space: ColorSpace::LinearRgb })
    );
    assert_eq!(
        read("color value='#0040ff'").nodes[0].properties[0].value,
        Value::Color(Color { rgba: [0., 64. / 255., 1., 1.], space: ColorSpace::Srgb })
    );
    for literal in ["cmyk(1,2,3)", "cmyk(1,2,3,4,5)", "cmyka(1,2,3,4)", "cmyka(1,2,3,4,5,6)", "cmyk(NaN,2,3,4)", "cmyka(1,2,3,4,inf)"] {
        let source = format!("color value='{literal}'");
        assert!(parse(&source, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{source}");
    }
}

#[test]
fn boolean_spellings_match_the_public_gegl_0462_chain_api() {
    for literal in ["true", "True", "TRUE", "1", "yes", "Yes", "YES", "on", "y", "Y"] {
        let raw = format!("gain enabled='{literal}'");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Boolean(true), "{literal}");
        assert!(graph.diagnostics.is_empty());
    }
    for literal in
        ["false", "False", "FALSE", "0", "no", "off", "n", "N", "ON", "On", "t", "T", "2", "-1", "", "TrUe", "yEs", "true-ish", "true ", " true"]
    {
        let raw = format!("gain enabled='{literal}'");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Boolean(false), "{literal}");
        assert!(graph.diagnostics.is_empty());
    }
}

#[test]
fn finite_integer_spellings_truncate_without_losing_exact_integer_source_precision() {
    for (literal, expected) in [("1.5", 1), ("-1.5", -1), ("1e1", 10), ("3.9e0", 3), ("-.5", 0), (".5", 0), ("1.0", 1), ("-1.999", -1)] {
        let raw = format!("gain signed={literal}");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Integer(expected));
        assert!(graph.diagnostics.is_empty());
    }
    for value in [i64::MIN, i64::MAX, 9_007_199_254_740_993] {
        let raw = format!("gain signed={value}");
        assert_eq!(read(&raw).nodes[0].properties[0].value, Value::Integer(value));
    }
    for (literal, expected) in [("1.5", 1), ("1e1", 10), (".5", 0), ("-.5", 0), ("4294967295.9", u32::MAX)] {
        let raw = format!("gain seed={literal}");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::UnsignedInteger(expected));
    }
    for value in [0, u32::MAX] {
        let raw = format!("gain seed={value}");
        assert_eq!(read(&raw).nodes[0].properties[0].value, Value::UnsignedInteger(value));
    }
    assert_eq!(read("gain signed=1.5rel").nodes[0].properties[0].value, Value::RelativeNumber(1.5), "relative units remain unevaluated");
}

#[test]
fn unsupported_absolute_integer_literals_retain_data_with_indexed_diagnostics() {
    for field in ["signed", "seed"] {
        for literal in ["0x10", "1junk", "junk", "unrelated", "", "NaN", "inf", "-inf", "1e999", "9223372036854775808.0", "-1e20"] {
            let raw = format!("gain text=kept {field}='{literal}' n=1");
            let graph = read(&raw);
            assert_eq!(graph.nodes[0].properties[1].literal, literal);
            assert_eq!(graph.nodes[0].properties[1].value, Value::Unknown);
            assert_eq!(graph.nodes[0].properties[0].value, Value::Text);
            assert_eq!(graph.nodes[0].properties[2].value, Value::Number(1.));
            assert_eq!(graph.diagnostics, [Diagnostic::UnsupportedInteger { source_ordinal: 0, property: 1 }], "{raw}");
        }
    }
    for literal in ["-1", "-1.5", "4294967296", "4294967296.0"] {
        let raw = format!("gain seed={literal}");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Unknown);
        assert_eq!(graph.diagnostics, [Diagnostic::UnsupportedInteger { source_ordinal: 0, property: 0 }]);
    }
    let graph = read("gain signed=0x10 invert-linear gain seed=4294967296");
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(
        graph.diagnostics,
        [Diagnostic::UnsupportedInteger { source_ordinal: 0, property: 0 }, Diagnostic::UnsupportedInteger { source_ordinal: 2, property: 0 }]
    );
    check_edges(&graph);
    let graph = read("over aux=[gain signed=1junk]");
    assert_eq!(graph.nodes[0].source_ordinal, 1);
    assert_eq!(
        graph.diagnostics,
        [Diagnostic::UnsupportedInteger { source_ordinal: 1, property: 0 }],
        "source order differs from dependency-first node order"
    );
    check_edges(&graph);
    let graph = read("example:unknown signed=0x10");
    assert_eq!(
        graph.diagnostics,
        [Diagnostic::UnknownOperation { source_ordinal: 0 }, Diagnostic::UnknownProperty { source_ordinal: 0, property: 0 }]
    );
}

#[test]
fn integer_fallback_does_not_hide_relative_structural_or_strict_number_errors() {
    for field in ["n", "signed", "seed"] {
        for literal in ["rel", "NaNrel", "infrel", "1e999rel", "1relrel", "junkrel", "1reljunk"] {
            let raw = format!("gain {field}='{literal}'");
            assert!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{raw}");
        }
    }
    for raw in [
        "gain n=0x10",
        "gain n=1junk",
        "gain n=NaN",
        "gain n=inf",
        "over aux=[gain signed=0x10",
        "gain signed=[invert-linear]",
        "gain signed=0x10 signed=1",
    ] {
        assert!(parse(raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{raw}");
    }
    let mut budget = Budget::new(1, 1 << 20);
    assert!(matches!(parse("gain signed=0x10", SCHEMA, &mut budget, &|| true), Err(Error::Limit(Limit::Storage))));
    assert_eq!(budget.remaining_storage(), 1);
    assert!(budget.remaining_work() < 1 << 20);
    assert!(matches!(parse("gain signed=0x10", SCHEMA, &mut Budget::default(), &|| false), Err(Error::Source(crate::Error::Cancelled))));
}

#[test]
fn value_boundary_continuations_preserve_auxiliary_dispatch_and_literal_escaping() {
    for source in [
        "over aux=\\\n[ invert-linear ]",
        "over aux=\\\n[invert-linear]",
        "over aux=\\\n\\\n[invert-linear]",
        "over aux=[over aux=\\\n[invert-linear]]",
    ] {
        let graph = read(source);
        assert_eq!(graph.nodes[0].operation, "invert-linear");
        assert_eq!(graph.nodes.last().unwrap().operation, "over");
        assert!(graph.diagnostics.is_empty());
        check_edges(&graph);
    }
    for source in ["gain text=\\\n'[kept]'", "gain text=\\\n\\[kept\\]"] {
        let graph = read(source);
        assert_eq!(graph.nodes[0].properties[0].literal, "[kept]");
        assert_eq!(graph.nodes[0].properties[0].value, Value::Text);
    }
    assert_eq!(read("gain text=\\\n").nodes[0].properties[0].literal, "");
    assert_eq!(
        read("color value=\\\n#abc").nodes[0].properties[0].value,
        Value::Color(Color { rgba: [170. / 255., 187. / 255., 204. / 255., 1.], space: ColorSpace::Srgb })
    );
    for source in ["over aux=\\\n'[invert-linear]'", "over aux=\\\n\\[invert-linear\\]"] {
        assert!(parse(source, SCHEMA, &mut Budget::default(), &|| true).is_err(), "quoted/escaped brackets must not become a block: {source:?}");
    }
}

#[test]
fn overflowing_exact_integers_remain_unsupported_without_floating_point_rounding() {
    for literal in ["-9223372036854775809", "+9223372036854775808", "9223372036854775808", "-09223372036854775809"] {
        let source = format!("gain signed={literal}");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Unknown, "{literal}");
        assert_eq!(graph.diagnostics, [Diagnostic::UnsupportedInteger { source_ordinal: 0, property: 0 }]);
    }
    for literal in ["4294967296", "+4294967296", "18446744073709551616"] {
        let source = format!("gain seed={literal}");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Unknown);
        assert_eq!(graph.diagnostics, [Diagnostic::UnsupportedInteger { source_ordinal: 0, property: 0 }]);
    }
    for (literal, expected) in
        [("-9223372036854775808", i64::MIN), ("9223372036854775807", i64::MAX), ("-1.5", -1), ("18446744073709551615e-10", 1_844_674_407)]
    {
        let source = format!("gain signed={literal}");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Integer(expected));
        assert!(graph.diagnostics.is_empty());
    }
    assert_eq!(read("gain seed=-0").nodes[0].properties[0].value, Value::UnsignedInteger(0));
    assert_eq!(read("gain seed=4294967295.9").nodes[0].properties[0].value, Value::UnsignedInteger(u32::MAX));
}

#[test]
fn color_function_names_accept_ascii_case_and_separating_whitespace_without_conversion() {
    for literal in ["RGB (1,0,0)", "rGb(1,0,0)", "RGB\t(1,0,0)", "RGB\n(1,0,0)"] {
        let source = format!("color value='{literal}'");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Color(Color { rgba: [1., 0., 0., 1.], space: ColorSpace::LinearRgb }));
    }
    for (literal, cmyk, alpha) in [("CMYK (0,25,100,0)", [0., 25., 100., 0.], 1.), ("cMyKa\t(0,25,100,0,0.5)", [0., 25., 100., 0.], 0.5)] {
        let source = format!("color value='{literal}'");
        let graph = read(&source);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::CmykColor { cmyk, alpha });
    }
    assert_eq!(
        read("color value='RGBA (1,0,0,0.5)'").nodes[0].properties[0].value,
        Value::Color(Color { rgba: [1., 0., 0., 0.5], space: ColorSpace::LinearRgb })
    );
    assert_eq!(read("color value='#ff0000'").nodes[0].properties[0].value, Value::Color(Color { rgba: [1., 0., 0., 1.], space: ColorSpace::Srgb }));
    for literal in ["RGBX (1,0,0)", "RGB junk (1,0,0)", "RGB (1,0)", "RGBA (1,0,0,NaN)", "CMYK (0,25,100)", "CMYKA (0,25,100,0,NaN)"] {
        let source = format!("color value='{literal}'");
        assert!(parse(&source, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{literal}");
    }
}

#[test]
fn empty_and_comment_only_root_graphs_preserve_the_source_input() {
    for raw in ["", " \t\n", "# default pipeline\n# still a comment", "\\\n # comment"] {
        let mut budget = Budget::default();
        let work = budget.remaining_work();
        let storage = budget.remaining_storage();
        let graph = parse(raw, SCHEMA, &mut budget, &|| true).unwrap();
        assert_eq!(graph.raw, raw);
        assert_eq!(graph.output, Input::Source);
        assert!(graph.nodes.is_empty() && graph.labels.is_empty() && graph.diagnostics.is_empty());
        assert_eq!(budget.remaining_storage(), storage);
        assert!(budget.remaining_work() < work);
    }
    for raw in ["over aux=[]", "over aux=[ # no source\n ]", "[]", "# comment\0"] {
        assert!(parse(raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{raw:?}");
    }
    let checks = Cell::new(0);
    let keep_going = || {
        checks.set(checks.get() + 1);
        checks.get() < 2
    };
    assert!(matches!(parse("", SCHEMA, &mut Budget::default(), &keep_going), Err(Error::Source(crate::Error::Cancelled))));
}

#[test]
fn whole_shell_word_assignments_keep_the_first_equals_and_the_complete_value() {
    let fields = [Field { name: "string", kind: FieldType::Text }];
    let schema = [Operation { name: "gegl:text", fields: &fields }];
    for (source, expected) in [
        ("gegl:text 'string=hello world'", "hello world"),
        ("gegl:text \"string=hello world\"", "hello world"),
        ("gegl:text string='hello world'", "hello world"),
        ("gegl:text 'str'\"ing=a=b\"", "a=b"),
        ("gegl:text 'string='", ""),
        ("gegl:text 'string=[kept]'", "[kept]"),
        ("gegl:text string\\=a=b", "a=b"),
        ("gegl:text 'string=\u{03bb}=\u{03b2}'", "\u{03bb}=\u{03b2}"),
    ] {
        let graph = parse(source, &schema, &mut Budget::default(), &|| true).unwrap_or_else(|error| panic!("{source:?}: {error}"));
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].properties[0].name, "string");
        assert_eq!(graph.nodes[0].properties[0].literal, expected, "{source}");
        assert_eq!(graph.nodes[0].properties[0].value, Value::Text);
        assert!(graph.diagnostics.is_empty());
    }
    let graph = read("'id=base' over aux=[ 'ref=base' gain 'n=1.5' 'text=a[b]c' ]");
    assert_eq!(graph.nodes[0].properties[0].value, Value::Number(1.5));
    assert_eq!(graph.nodes[0].properties[1].literal, "a[b]c");
    check_edges(&graph);
    let quoted_tokens = format!("'id=base' {}invert-linear invert-linear", "'ref=base' ".repeat(681));
    assert_eq!(tokenize(&quoted_tokens, &mut Budget::default(), &|| true).unwrap().len(), 2048);
    assert!(parse(&quoted_tokens, SCHEMA, &mut Budget::default(), &|| true).is_ok());
    assert!(parse(&format!("{quoted_tokens} invert-linear"), SCHEMA, &mut Budget::default(), &|| true).is_err());
}

#[test]
fn color_function_outer_ascii_whitespace_does_not_change_the_retained_literal() {
    for literal in [" rgb(1,0,0) ", "\tRGB (1,0,0)\r\n", "\n rGb(1,0,0)\t"] {
        let raw = format!("color value='{literal}'");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::Color(Color { rgba: [1., 0., 0., 1.], space: ColorSpace::LinearRgb }));
    }
    let rgba = "\tRGBA (1,0,0,0.5)\n";
    assert_eq!(
        read(&format!("color value='{rgba}'")).nodes[0].properties[0].value,
        Value::Color(Color { rgba: [1., 0., 0., 0.5], space: ColorSpace::LinearRgb })
    );
    for literal in [" CMYK (0,25,100,0) ", "\ncMyK(0,25,100,0)\t"] {
        let raw = format!("color value='{literal}'");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, literal);
        assert_eq!(graph.nodes[0].properties[0].value, Value::CmykColor { cmyk: [0., 25., 100., 0.], alpha: 1. });
    }
    for literal in ["red ", " red", "#ff0000 ", " #ff0000", " RGB (1,2) ", " RGBA (1,0,0,NaN) ", " CMYK (0,25,100) ", "rgbx(1,0,0) "] {
        let raw = format!("color value='{literal}'");
        assert!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "do not broaden nonfunction or component rules: {literal:?}");
    }
}

#[test]
fn quoted_assignment_prefixes_keep_a_following_unquoted_auxiliary_opener_structural() {
    for prefix in ["'aux='", "\"aux=\"", "'au'\"x=\"", "aux\\=", "'aux='''", "'aux='\"\""] {
        let raw = format!("gegl:over {prefix}[invert-linear ]");
        let graph = read(&raw);
        assert_eq!(graph.nodes.len(), 2, "{raw}");
        assert_eq!(graph.nodes[0].operation, "invert-linear");
        assert_eq!(graph.nodes[1].operation, "gegl:over");
        assert_eq!(graph.nodes[1].properties[0].value, Value::Input(Input::Node(0)));
        assert!(graph.diagnostics.is_empty());
        check_edges(&graph);
    }
    for raw in ["gain 'text='\"[literal]\"", "gain 'text=[literal]'", "gain text='[literal]'", "gain 'text='\\[literal\\]"] {
        let graph = read(raw);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].properties[0].literal, "[literal]", "{raw}");
        assert_eq!(graph.nodes[0].properties[0].value, Value::Text);
    }
    let graph = read("gain 'text=a'[literal]b");
    assert_eq!(graph.nodes[0].properties[0].literal, "a[literal]b");
}

#[test]
fn empty_quote_fragments_preserve_pending_auxiliary_closers() {
    for raw in [
        "over aux=[gain text=a]''",
        "over aux=[gain text=a]\"\"",
        "over aux=[gain text=a]''\"\"",
        "over aux=[over aux=[gain text=a]]''",
        "over aux=[gain 'text=a']''",
    ] {
        let graph = read(raw);
        assert_eq!(graph.nodes[0].properties[0].literal, "a", "{raw}");
        assert!(graph.diagnostics.is_empty());
        assert!(matches!(graph.nodes.last().unwrap().properties[0].value, Value::Input(_)));
        check_edges(&graph);
    }
    // Nonempty quotes put the earlier bracket inside the decoded scalar.
    assert_eq!(read("over aux=[gain text=a]'tail' ]").nodes[0].properties[0].literal, "a]tail");
    assert_eq!(read("gain text='' ").nodes[0].properties[0].literal, "");
    assert!(parse("invert-linear ''", SCHEMA, &mut Budget::default(), &|| true).is_err());
}

#[test]
fn rgba_keeps_all_finite_written_alpha_values_without_hiding_later_nodes() {
    for (literal, alpha) in
        [("2", 2.), ("-0.5", -0.5), ("0", 0.), ("1", 1.), ("-0", -0.), ("1.7976931348623157e308", f64::MAX), ("-1.7976931348623157e308", -f64::MAX)]
    {
        let color = format!("rgba(-2,1.25,0.5,{literal})");
        let raw = format!("color value='{color}' gain n=7");
        let graph = read(&raw);
        assert_eq!(graph.nodes[0].properties[0].literal, color);
        let Value::Color(color) = &graph.nodes[0].properties[0].value else { panic!("source color") };
        assert_eq!(color.rgba, [-2., 1.25, 0.5, alpha]);
        assert_eq!(color.rgba[3].to_bits(), alpha.to_bits());
        assert_eq!(color.space, ColorSpace::LinearRgb);
        assert_eq!(graph.nodes[1].properties[0].value, Value::Number(7.));
        assert!(graph.diagnostics.is_empty());
        check_edges(&graph);
    }
    for literal in ["rgba(1,0,0,NaN)", "rgba(1,0,0,inf)", "rgba(1,0,0,-inf)", "rgba(1,0,0,1e999)", "rgba(1,0,0)", "rgba(1,0,0,2,3)", "rgb(1,0,0,2)"] {
        let raw = format!("color value='{literal}' gain n=7");
        assert!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{literal}");
    }
}

#[test]
fn empty_value_quotes_do_not_hide_a_following_unquoted_auxiliary_opener() {
    for quoted in ["''", "\"\"", "''\"\"", "\"\"''"] {
        let raw = format!("over aux={quoted}[invert-linear ]");
        let graph = read(&raw);
        assert_eq!(graph.nodes.len(), 2, "{raw}");
        assert_eq!(graph.nodes[0].operation, "invert-linear");
        assert_eq!(graph.nodes[1].properties[0].value, Value::Input(Input::Node(0)));
        assert!(graph.diagnostics.is_empty());
        check_edges(&graph);
        let empty = format!("gain text={quoted}");
        assert_eq!(read(&empty).nodes[0].properties[0].literal, "");
        for bracket in ["'[literal]'", "\"[literal]\"", "\\[literal\\]"] {
            let scalar = format!("gain text={quoted}{bracket}");
            assert_eq!(read(&scalar).nodes[0].properties[0].literal, "[literal]", "{scalar}");
        }
        let scalar = format!("gain text={quoted}a[literal]b");
        assert_eq!(read(&scalar).nodes[0].properties[0].literal, "a[literal]b");
    }
    let nested = read("over aux=''[over aux=\"\"[invert-linear]]");
    assert_eq!(nested.nodes.len(), 3);
    check_edges(&nested);
    // No synthetic empty-value token may consume the exact lexical allowance.
    let exact = format!("id=base {}over aux=''[invert-linear] invert-linear invert-linear", "ref=base ".repeat(679));
    assert_eq!(tokenize(&exact, &mut Budget::default(), &|| true).unwrap().len(), 2048);
    assert!(parse(&exact, SCHEMA, &mut Budget::default(), &|| true).is_ok());
    assert!(parse(&format!("{exact} invert-linear"), SCHEMA, &mut Budget::default(), &|| true).is_err());
}

#[test]
fn intervening_node_ids_keep_later_properties_and_resolved_labels_together() {
    let fields = [Field { name: "exposure", kind: FieldType::Number }];
    let schema = [Operation { name: "gegl:exposure", fields: &fields }];
    for raw in [
        "gegl:exposure id=base exposure=1.25 ref=base",
        "gegl:exposure exposure=1.25 id=base ref=base",
        "gegl:exposure id=base exposure=1.25 id=alias ref=base",
    ] {
        let graph = parse(raw, &schema, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(graph.nodes.len(), 1, "{raw}");
        assert_eq!(graph.nodes[0].properties.len(), 1);
        assert_eq!(graph.nodes[0].properties[0].name, "exposure");
        assert_eq!(graph.nodes[0].properties[0].value, Value::Number(1.25));
        assert_eq!(graph.labels[0], Label { name: "base".into(), input: Input::Node(0) });
        assert!(graph.labels.iter().all(|label| label.input == Input::Node(0)));
        assert_eq!(graph.output, Input::Node(0));
        assert!(graph.diagnostics.is_empty());
        check_edges(&graph);
    }
    for raw in ["gain n=1 id=base n=2", "gain id=base n=1 id=base"] {
        assert!(parse(raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "duplicate controls remain errors: {raw}");
    }
}

#[test]
fn pending_node_labels_resolve_after_auxiliary_dependencies_without_reordering_definitions() {
    let graph = read("gain id=base n=1 over id=merged aux=[ref=base gain id=child n=2 ref=child] ref=merged");
    assert_eq!(graph.nodes.iter().map(|node| node.operation.as_str()).collect::<Vec<_>>(), ["gain", "gain", "over"]);
    assert_eq!(graph.nodes.iter().map(|node| node.source_ordinal).collect::<Vec<_>>(), [0, 2, 1]);
    assert_eq!(graph.nodes[0].properties[0].value, Value::Number(1.));
    assert_eq!(graph.nodes[1].properties[0].value, Value::Number(2.));
    assert_eq!(graph.nodes[2].input, Some(Input::Node(0)));
    assert_eq!(graph.nodes[2].properties[0].value, Value::Input(Input::Node(1)));
    assert_eq!(
        graph.labels,
        [
            Label { name: "base".into(), input: Input::Node(0) },
            Label { name: "merged".into(), input: Input::Node(2) },
            Label { name: "child".into(), input: Input::Node(1) },
        ]
    );
    assert_eq!(graph.output, Input::Node(2));
    assert!(graph.diagnostics.is_empty());
    check_edges(&graph);
}

#[test]
fn auxiliary_references_to_pending_nodes_reject_cycles_without_retained_debt() {
    for raw in [
        "over id=loop aux=[ref=loop]",
        "over id=loop aux=[ref=loop invert-linear]",
        "over id=parent aux=[over id=child aux=[ref=parent]]",
        "over id=parent aux=[gain id=child n=1 ref=parent]",
        "gain id=first over id=second aux=[ref=second]",
    ] {
        let mut budget = Budget::default();
        let storage = budget.remaining_storage();
        let work = budget.remaining_work();
        let error = parse(raw, SCHEMA, &mut budget, &|| true).unwrap_err();
        assert!(error.to_string().contains("cyclic"), "{raw}: {error}");
        assert_eq!(budget.remaining_storage(), storage);
        assert!(budget.remaining_work() < work);
    }
    let graph = read("gain id=base n=1 ref=base invert-linear");
    assert_eq!(graph.nodes[1].input, Some(Input::Node(0)));
    check_edges(&graph);
}

#[test]
fn reference_scalar_edits_target_the_selected_node_and_keep_later_edges() {
    let raw = "gain n=0 signed=unrepresentable id=base invert-linear ref=base n=1.25 signed=7 invert-linear";
    let graph = read(raw);
    assert_eq!(graph.raw, raw);
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.labels[0].input, Input::Node(0));
    assert_eq!(graph.nodes[0].properties.len(), 2);
    assert_eq!(graph.nodes[0].properties[0].value, Value::Number(1.25));
    assert_eq!(graph.nodes[0].properties[1].value, Value::Integer(7));
    assert!(graph.diagnostics.is_empty());
    assert_eq!(graph.nodes[1].input, Some(Input::Node(0)));
    assert_eq!(graph.nodes[2].input, Some(Input::Node(0)));
    assert_eq!(graph.output, Input::Node(2));
    check_edges(&graph);

    let graph = read("gain id=base ref=base n=1.25");
    assert_eq!(graph.output, Input::Node(0));
    assert_eq!(graph.nodes[0].properties[0].value, Value::Number(1.25));
    for invalid in ["ref=missing n=1.25", "id=source ref=source n=1.25", "over id=base ref=base aux=[ ref=base ]", "gain n=0 n=1"] {
        assert!(parse(invalid, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{invalid}");
    }
}

#[test]
fn quoted_ascii_padding_changes_only_the_numeric_view() {
    let plain = read("gain n=1.25 signed=-7 seed=17");
    let raw = "gain n=' \t1.25 \t' signed=' -7 ' seed=' 17 '";
    let padded = read(raw);
    assert_eq!(padded.raw, raw);
    for (property, unpadded) in padded.nodes[0].properties.iter().zip(&plain.nodes[0].properties) {
        assert_eq!(property.value, unpadded.value);
        assert_eq!(property.literal.trim_ascii(), unpadded.literal);
        assert_ne!(property.literal, unpadded.literal);
    }
    for literal in [" NaN ", " inf ", " 1.25tail ", "\u{a0}1.25\u{a0}"] {
        let raw = format!("gain n='{literal}'");
        assert!(parse(&raw, SCHEMA, &mut Budget::default(), &|| true).is_err(), "{literal}");
    }
}

#[test]
fn unquoted_keyframe_syntax_cannot_add_properties_or_operation_nodes() {
    for raw in [
        "gegl:exposure exposure={ 0=0 1=1 } gegl:invert-linear",
        "gain n= { 0=0 1=1 } invert-linear",
        "gain n=''{ 0=0 1=1 } invert-linear",
        "gain 'n='{\n0=0 1=1} invert-linear",
    ] {
        for declarations in [&[][..], SCHEMA] {
            let error = parse(raw, declarations, &mut Budget::default(), &|| true).unwrap_err();
            assert!(error.to_string().contains("keyframe syntax is unsupported"), "{error}: {raw}");
        }
    }
    for raw in ["gain text='{ 0=0 1=1 }' invert-linear", r"gain text=\{0=0\} invert-linear"] {
        let graph = read(raw);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[1].input, Some(Input::Node(0)));
        let text = &graph.nodes[0].properties[0];
        assert_eq!(text.value, Value::Text);
        assert!(text.literal.starts_with('{') && text.literal.ends_with('}'));
    }
}

#[test]
fn cancellation_after_numeric_conversion_precedes_property_publication() {
    let literal = format!("0.{}1", "0".repeat(2000));
    let mut budget = Budget::default();
    let checks = Cell::new(0);
    let keep = || {
        checks.set(checks.get() + 1);
        // The assignment and declaration lookup may proceed. Cancel the
        // publication check after conversion of the last scalar.
        checks.get() < 3
    };
    let mut decoder = Decoder {
        tokens: vec![Token::Word("n".into()), Token::Equal, Token::Word(literal)],
        cursor: 0,
        declarations: SCHEMA,
        budget: &mut budget,
        keep_going: &keep,
        nodes: Vec::new(),
        labels: Vec::new(),
        label_sources: Vec::new(),
        diagnostics: Vec::new(),
        source_nodes: 1,
        assignments: 0,
    };
    let mut properties = Vec::new();
    let result = decoder.properties(&mut properties, SCHEMA.first().copied(), 0, Some(0));
    assert!(matches!(result, Err(Error::Source(crate::Error::Cancelled))));
    assert!(properties.is_empty());
}

#[test]
fn choice_lookup_polls_before_reaching_a_late_match() {
    let mut choices = [("other", 1); 130];
    choices[129] = ("target", 7);
    let kind = Some(FieldType::Choice(&choices));
    let checks = Cell::new(0);
    let keep = || {
        checks.set(checks.get() + 1);
        checks.get() < 2
    };
    let mut budget = Budget::default();
    let before = budget.remaining_work();
    assert!(matches!(typed_value("target", kind, &mut budget, &keep), Err(Error::Source(crate::Error::Cancelled))));
    let comparison = "target".len().min("other".len()) + 1;
    assert_eq!(before - budget.remaining_work(), 64 * comparison);
    assert_eq!(typed_value("target", kind, &mut budget, &|| true).unwrap(), Value::Choice(7));
}

//! Reading and writing file JSON written by hand from the REST API reference.
#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::{Value, json};

use super::ext::{VectorNetwork, Winding};
use super::*;

/// A file using each typed record, plus fields the model has no name for at every level.
fn file_json() -> Value {
    serde_json::from_str(FILE).unwrap()
}

const FILE: &str = r##"{
        "name": "Fixture",
        "lastModified": "2026-01-01T00:00:00Z",
        "schemaVersion": 0.0,
        "role": "owner",
        "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT", "children": [{
            "id": "0:1", "name": "Page 1", "type": "CANVAS", "backgroundColor": {"r": 0.9, "g": 0.9, "b": 0.9, "a": 1.0},
            "prototypeStartNodeID": "1:2",
            "flowStartingPoints": [{"nodeId": "1:2", "name": "Flow 1"}],
            "children": [{
                "id": "1:2", "name": "Card", "type": "FRAME", "blendMode": "PASS_THROUGH", "clipsContent": true,
                "absoluteBoundingBox": {"x": 10.5, "y": 20.0, "width": 300.0, "height": 200.0},
                "relativeTransform": [[1.0, 0.0, 10.5], [0.0, 1.0, 20.0]], "size": {"x": 300.0, "y": 200.0},
                "constraints": {"vertical": "TOP", "horizontal": "LEFT_RIGHT"},
                "fills": [
                    {"type": "SOLID", "blendMode": "NORMAL", "color": {"r": 1.0, "g": 0.5, "b": 0.0, "a": 1.0}, "boundVariables": {"color": {"type": "VARIABLE_ALIAS", "id": "VariableID:1:5"}}},
                    {"type": "GRADIENT_LINEAR", "opacity": 0.5, "gradientHandlePositions": [{"x": 0.0, "y": 0.5}, {"x": 1.0, "y": 0.5}, {"x": 0.0, "y": 1.0}],
                     "gradientStops": [{"position": 0.0, "color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}, {"position": 1.0, "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 0.0}}]},
                    {"type": "IMAGE", "scaleMode": "FILL", "imageRef": "0a1b2c", "filters": {"exposure": 0.25}},
                    {"type": "SHINY_NEW_PAINT", "sparkle": 3.5}
                ],
                "strokes": [{"type": "SOLID", "color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}],
                "strokeWeight": 2.0, "strokeAlign": "INSIDE", "strokeDashes": [4.0, 2.0],
                "individualStrokeWeights": {"top": 1.0, "right": 2.0, "bottom": 3.0, "left": 4.0},
                "cornerRadius": 8.0, "rectangleCornerRadii": [8.0, 8.0, 0.0, 0.0], "cornerSmoothing": 0.6,
                "effects": [{"type": "DROP_SHADOW", "visible": true, "radius": 4.0, "color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 0.25},
                             "blendMode": "NORMAL", "offset": {"x": 0.0, "y": 2.0}, "spread": 1.0, "showShadowBehindNode": false},
                            {"type": "LAYER_BLUR", "radius": 3.0, "blurType": "PROGRESSIVE"}],
                "layoutMode": "VERTICAL", "primaryAxisSizingMode": "AUTO", "counterAxisAlignItems": "CENTER", "itemSpacing": 12.0,
                "paddingLeft": 16.0, "paddingRight": 16.0, "paddingTop": 8.0, "paddingBottom": 8.0, "layoutWrap": "NO_WRAP",
                "layoutGrids": [{"pattern": "COLUMNS", "sectionSize": 60.0, "visible": true, "color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 0.1},
                                 "alignment": "STRETCH", "gutterSize": 20.0, "offset": 0.0, "count": 4.0}],
                "exportSettings": [{"suffix": "@2x", "format": "PNG", "constraint": {"type": "SCALE", "value": 2.0}}],
                "styles": {"fill": "5:1", "effect": "5:2"},
                "interactions": [{"trigger": {"type": "ON_CLICK"}, "actions": [{"type": "NODE", "destinationId": "1:9", "navigation": "NAVIGATE"}]}],
                "boundVariables": {"itemSpacing": {"type": "VARIABLE_ALIAS", "id": "VariableID:1:7"}},
                "children": [
                    {"id": "1:3", "name": "Title", "type": "TEXT", "characters": "Hello\nworld",
                     "style": {"fontFamily": "Inter", "fontPostScriptName": "Inter-Bold", "fontWeight": 700.0, "fontSize": 24.0, "italic": false,
                               "textAlignHorizontal": "LEFT", "textAlignVertical": "TOP", "letterSpacing": 0.5, "lineHeightPx": 28.0,
                               "lineHeightPercentFontSize": 116.5, "lineHeightUnit": "PIXELS", "opentypeFlags": {"LIGA": 0.0},
                               "hyperlink": {"type": "URL", "url": "https://example.com"}, "_letterSpacingPercent": 2.0},
                     "characterStyleOverrides": [0.0, 0.0, 1.0, 1.0],
                     "styleOverrideTable": {"1": {"fontWeight": 400.0, "fills": [{"type": "SOLID", "color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}}]}},
                     "lineTypes": ["NONE", "ORDERED"], "lineIndentations": [0.0, 1.0]},
                    {"id": "1:4", "name": "Shape", "type": "VECTOR",
                     "fillGeometry": [{"path": "M0 0 L10 0 L5 8 Z", "windingRule": "NONZERO"}], "strokeGeometry": [],
                     "vectorNetwork": {"vertices": [{"x": 0.0, "y": 0.0, "radius": 0.0}, {"x": 10.0, "y": 0.0, "radius": 2.0, "cap": "round"}, {"x": 5.0, "y": 8.0, "radius": 0.0}],
                                       "segments": [{"a": 0, "b": 1, "ta": [0.0, 0.0], "tb": [0.0, 0.0]}, {"a": 1, "b": 2, "ta": [1.0, 2.0], "tb": [-1.0, 0.5]}, {"a": 2, "b": 0, "ta": [0.0, 0.0], "tb": [0.0, 0.0]}],
                                       "regions": [{"loops": [[0, 1, 2]], "winding": "evenOdd", "_figRegionFills": [{"type": "SOLID"}]}]}},
                    {"id": "1:5", "name": "Ring", "type": "ELLIPSE", "arcData": {"startingAngle": 0.0, "endingAngle": 3.5, "innerRadius": 0.5}},
                    {"id": "1:6", "name": "Star", "type": "STAR", "pointCount": 5.0, "innerRadius": 0.4, "vectorNetworkRef": 3.0},
                    {"id": "1:7", "name": "Button", "type": "INSTANCE", "componentId": "2:1",
                     "componentProperties": {"Label#1:0": {"type": "TEXT", "value": "OK"}}, "overrides": [{"id": "I1:7;2:2", "overriddenFields": ["characters"]}]},
                    {"id": "1:8", "name": "Cut", "type": "BOOLEAN_OPERATION", "booleanOperation": "SUBTRACT", "isMask": true, "maskType": "ALPHA", "children": []},
                    {"id": "1:9", "name": "Later", "type": "HOLOGRAM", "depth": 3.0}
                ]
            }]
        }]},
        "components": {"2:1": {"key": "abc", "name": "Button", "description": "", "componentSetId": "2:0", "remote": false, "documentationLinks": []}},
        "componentSets": {"2:0": {"key": "set", "name": "Buttons", "description": ""}},
        "styles": {"5:1": {"key": "s1", "name": "Brand", "styleType": "FILL", "description": "", "remote": false}, "5:2": {"key": "s2", "name": "Glow", "styleType": "SPARKLE"}}
    }"##;

fn first_page(file: &File) -> &Node {
    &file.document.as_ref().unwrap().children.as_ref().unwrap()[0]
}

#[test]
fn a_file_reads_into_the_typed_records() {
    let file = File::from_value(file_json(), 100).unwrap();
    assert_eq!(file.name.as_deref(), Some("Fixture"));
    let page = first_page(&file);
    assert_eq!(page.kind, Some(NodeType::Canvas));
    assert_eq!(page.background_color, Some(Color { r: 0.9, g: 0.9, b: 0.9, a: 1.0 }));
    let card = &page.children.as_ref().unwrap()[0];
    assert_eq!((card.kind.clone(), card.blend_mode.clone()), (Some(NodeType::Frame), Some(BlendMode::PassThrough)));
    assert_eq!(card.absolute_bounding_box, Some(Rectangle { x: 10.5, y: 20.0, width: 300.0, height: 200.0 }));
    assert_eq!(card.relative_transform, Some([[1.0, 0.0, 10.5], [0.0, 1.0, 20.0]]));
    assert_eq!(card.constraints.as_ref().unwrap().horizontal.as_deref(), Some("LEFT_RIGHT"));
    let fills = card.fills.as_ref().unwrap();
    assert_eq!(
        fills.iter().map(|p| p.kind.clone().unwrap()).collect::<Vec<_>>(),
        [PaintType::Solid, PaintType::GradientLinear, PaintType::Image, PaintType::Other("SHINY_NEW_PAINT".into())]
    );
    assert_eq!(fills[1].gradient_stops.as_ref().unwrap()[1].color.a, 0.0);
    assert_eq!(fills[2].image_ref.as_deref(), Some("0a1b2c"));
    assert_eq!(card.rectangle_corner_radii, Some([8.0, 8.0, 0.0, 0.0]));
    let effects = card.effects.as_ref().unwrap();
    assert_eq!((effects[0].kind.clone(), effects[0].offset), (Some(EffectType::DropShadow), Some(Vector { x: 0.0, y: 2.0 })));
    assert_eq!(card.styles.as_ref().unwrap()["fill"], "5:1");
    assert_eq!(card.layout_grids.as_ref().unwrap()[0].count, Some(4.0));
    let kids = card.children.as_ref().unwrap();
    let text = &kids[0];
    assert_eq!(text.characters.as_deref(), Some("Hello\nworld"));
    assert_eq!(text.style.as_ref().unwrap().font_weight, Some(700.0));
    assert_eq!(text.style.as_ref().unwrap().opentype_flags.as_ref().unwrap()["LIGA"], 0.0);
    assert_eq!(text.style_override_table.as_ref().unwrap()["1"].fills.as_ref().unwrap()[0].kind, Some(PaintType::Solid));
    assert_eq!(kids[1].fill_geometry.as_ref().unwrap()[0].path.as_deref(), Some("M0 0 L10 0 L5 8 Z"));
    assert_eq!(kids[2].arc_data.as_ref().unwrap().inner_radius, Some(0.5));
    assert_eq!(kids[4].component_id.as_deref(), Some("2:1"));
    assert_eq!(kids[5].boolean_operation.as_deref(), Some("SUBTRACT"));
    assert_eq!(kids[6].kind, Some(NodeType::Other("HOLOGRAM".into())));
    assert_eq!(file.components.as_ref().unwrap()["2:1"].component_set_id.as_deref(), Some("2:0"));
    assert_eq!(file.styles.as_ref().unwrap()["5:1"].style_type, Some(StyleType::Fill));
    assert_eq!(file.styles.as_ref().unwrap()["5:2"].style_type, Some(StyleType::Other("SPARKLE".into())));
    // Every node, parents first.
    let ids: Vec<&str> = file.document.as_ref().unwrap().walk().filter_map(|n| n.id.as_deref()).collect();
    assert_eq!(ids, ["0:0", "0:1", "1:2", "1:3", "1:4", "1:5", "1:6", "1:7", "1:8", "1:9"]);
}

#[test]
fn nothing_is_lost_between_reading_and_writing() {
    let original = file_json();
    let file = File::from_value(original.clone(), 100).unwrap();
    assert_eq!(file.to_value().unwrap(), original);
    // Fields without a name here stay where they were, as they were.
    assert_eq!(file.extra["role"], "owner");
    let card = &first_page(&file).children.as_ref().unwrap()[0];
    assert!(card.extra.contains_key("interactions") && card.extra.contains_key("boundVariables"));
    assert_eq!(card.fills.as_ref().unwrap()[2].extra["filters"]["exposure"], 0.25);
    assert_eq!(card.fills.as_ref().unwrap()[3].extra["sparkle"], 3.5);
    assert_eq!(card.effects.as_ref().unwrap()[1].extra["blurType"], "PROGRESSIVE");
    assert_eq!(card.children.as_ref().unwrap()[6].extra["depth"], 3.0);
    // Through text and back, too.
    let text = serde_json::to_vec(&file).unwrap();
    assert_eq!(File::from_slice(&text).unwrap(), file);
}

#[test]
fn the_extensions_are_read_and_kept() {
    let file = File::from_value(file_json(), 100).unwrap();
    let kids = first_page(&file).children.as_ref().unwrap()[0].children.as_ref().unwrap();
    let net = kids[1].vector_network.as_ref().unwrap();
    assert_eq!((net.vertices.len(), net.segments.len(), net.regions.len()), (3, 3, 1));
    assert_eq!(net.vertices[1].radius, 2.0);
    assert_eq!(net.vertices[1].extra["cap"], "round");
    assert_eq!(net.segments[1].ta, (1.0, 2.0));
    assert_eq!(net.regions[0].winding, Winding::EvenOdd);
    assert!(net.regions[0].extra.contains_key("_figRegionFills"));
    assert!(net.is_consistent());
    // The others have no typed field: they are kept by name.
    assert_eq!(kids[3].extra[ext::VECTOR_NETWORK_REF], 3.0);
    assert_eq!(kids[3].extra["pointCount"], 5.0);
    assert_eq!(kids[0].style.as_ref().unwrap().extra["_letterSpacingPercent"], 2.0);
    // A network naming a vertex or a segment it lacks is told before its indices are used.
    let bad: VectorNetwork = serde_json::from_value(json!({"vertices": [{"x": 0, "y": 0}], "segments": [{"a": 0, "b": 1}], "regions": []})).unwrap();
    assert!(!bad.is_consistent());
    let bad: VectorNetwork = serde_json::from_value(json!({"vertices": [], "segments": [], "regions": [{"loops": [[0]]}]})).unwrap();
    assert!(!bad.is_consistent());
    assert_eq!(bad.regions[0].winding, Winding::NonZero);
}

#[test]
fn a_built_file_writes_only_what_it_holds() {
    let rect = Node::new(NodeProps {
        id: Some("1:1".into()),
        name: Some("R".into()),
        kind: Some(NodeType::Rectangle),
        size: Some(Vector { x: 10.0, y: 5.0 }),
        fills: Some(vec![Paint { kind: Some(PaintType::Solid), color: Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }), ..Paint::default() }]),
        ..NodeProps::default()
    });
    let mut page = Node::with_children(NodeProps { id: Some("0:1".into()), kind: Some(NodeType::Canvas), ..NodeProps::default() }, vec![rect]);
    page.extra.insert("pointCount".into(), json!(3));
    let document = Node::with_children(
        NodeProps { kind: Some(NodeType::Document), ..NodeProps::default() },
        vec![page, Node::with_children(NodeProps::default(), vec![])],
    );
    let file = File { document: Some(document), ..File::default() };
    assert_eq!(
        file.to_value().unwrap(),
        json!({"document": {"type": "DOCUMENT", "children": [{"id": "0:1", "type": "CANVAS", "pointCount": 3, "children": [
            {"id": "1:1", "name": "R", "type": "RECTANGLE", "size": {"x": 10.0, "y": 5.0},
             "fills": [{"type": "SOLID", "color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}}]}]}, {"children": []}]}})
    );
}

#[test]
fn nulls_are_left_out_and_whole_numbers_come_back_as_floats() {
    let file = File::from_value(json!({"name": null, "schemaVersion": 3, "document": {"id": "0:0", "opacity": 1, "locked": null}}), 10).unwrap();
    assert_eq!(file.to_value().unwrap(), json!({"schemaVersion": 3.0, "document": {"id": "0:0", "opacity": 1.0}}));
    // A `children` that is not a list is kept as it is; an empty list stays a list.
    let odd = json!({"document": {"id": "0:0", "children": {"id": "1"}}});
    assert_eq!(File::from_value(odd.clone(), 10).unwrap().to_value().unwrap(), odd);
    let empty = json!({"id": "1", "children": []});
    assert_eq!(Node::from_value(empty.clone(), 10).unwrap().children, Some(vec![]));
    assert_eq!(Node::from_value(empty.clone(), 10).unwrap().to_value().unwrap(), empty);
    // A colour without alpha is opaque.
    let paint: Paint = serde_json::from_value(json!({"color": {"r": 0, "g": 0, "b": 0}})).unwrap();
    assert_eq!(paint.color.unwrap().a, 1.0);
}

#[test]
fn a_field_of_the_wrong_type_fails_the_read_and_says_which() {
    for (bad, field) in [
        (json!({"document": {"visible": "yes"}}), "boolean"),
        (json!({"document": {"children": [{"opacity": []}]}}), "f64"),
        (json!({"document": {"fills": [{"color": "red"}]}}), "Color"),
        (json!({"document": {"relativeTransform": [[1, 0], [0, 1]]}}), "array of length 3"),
        (json!({"name": 5}), "string"),
    ] {
        let e = File::from_value(bad, 10).unwrap_err().to_string();
        assert!(e.starts_with("file JSON: ") && e.contains(field), "{e}");
    }
    assert!(File::from_slice(b"{not json").is_err());
    assert!(File::from_slice(b"[]").is_err());
    assert_eq!(File::from_slice(b"{}").unwrap(), File::default());
}

/// `depth` layers inside one another.
fn nested(depth: usize) -> Value {
    let mut node = json!({"id": "leaf", "type": "RECTANGLE"});
    for i in 0..depth {
        node = json!({"id": format!("g{i}"), "type": "GROUP", "children": [node]});
    }
    node
}

#[test]
fn the_depth_limit_holds() {
    // Each layer is two levels (its `children` array and itself); the file adds one. The tree is
    // read, written and dropped at the limit on this test's thread (2 MiB of stack).
    let deepest = (MAX_DEPTH - 1) / 2;
    let original = json!({"document": nested(deepest)});
    let file = File::from_value(original.clone(), 1000).unwrap();
    assert_eq!(file.document.as_ref().unwrap().walk().count(), deepest + 1);
    assert_eq!(file.to_value().unwrap(), original);
    assert_eq!(File::from_value(json!({"document": nested(deepest + 1)}), 1000).err(), Some(Error::TooDeep));
    // Depth inside an unnamed field counts as well.
    let mut deep = json!(1);
    for _ in 0..MAX_DEPTH + 1 {
        deep = json!([deep]);
    }
    assert_eq!(File::from_value(json!({"mystery": deep}), 10).err(), Some(Error::TooDeep));
}

#[test]
fn the_node_limit_holds() {
    let wide = |n: usize| json!({"document": {"id": "0:0", "type": "DOCUMENT", "children": (0..n).map(|i| json!({"id": format!("1:{i}"), "type": "RECTANGLE"})).collect::<Vec<_>>()}});
    assert!(File::from_value(wide(99), 100).is_ok());
    assert_eq!(File::from_value(wide(100), 100).err(), Some(Error::TooManyNodes(100)));
    let nodes = json!({"name": "N", "nodes": {"1:2": {"document": nested(3)}, "9:9": null}});
    let read = FileNodes::from_value(nodes.clone(), 4).unwrap();
    assert!(read.nodes.as_ref().unwrap()["9:9"].is_none());
    assert_eq!(serde_json::to_value(&read).unwrap(), nodes);
    assert_eq!(FileNodes::from_value(nodes, 3).err(), Some(Error::TooManyNodes(3)));
}

#[test]
fn junk_values_never_panic() {
    // Every value of the fixture in turn replaced by a value of another type: the read ends in a
    // file or an error.
    fn paths(v: &Value, at: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(at.clone());
        match v {
            Value::Object(m) => {
                for (k, c) in m {
                    at.push(k.clone());
                    paths(c, at, out);
                    at.pop();
                }
            }
            Value::Array(a) => {
                for (i, c) in a.iter().enumerate() {
                    at.push(i.to_string());
                    paths(c, at, out);
                    at.pop();
                }
            }
            _ => {}
        }
    }
    let original = file_json();
    let mut all = Vec::new();
    paths(&original, &mut Vec::new(), &mut all);
    let junk = [json!(null), json!(5), json!(-1e300), json!("x"), json!(true), json!([]), json!({}), json!([1, "a"])];
    let mut read = 0;
    for (n, at) in all.iter().enumerate().skip(1) {
        let mut changed = original.clone();
        let mut v = &mut changed;
        for k in at {
            v = match v {
                Value::Object(m) => m.get_mut(k).unwrap(),
                Value::Array(a) => &mut a[k.parse::<usize>().unwrap()],
                _ => panic!("a path only runs through objects and arrays"),
            };
        }
        *v = junk[n % junk.len()].clone();
        if let Ok(file) = File::from_value(changed, 1000) {
            read += 1;
            assert!(file.to_value().is_ok());
        }
    }
    assert!(read > 0 && read < all.len());
}

#[test]
fn warnings_are_counted_once_each_and_capped() {
    let mut w = Warnings::default();
    for _ in 0..3 {
        w.add("a".into());
    }
    w.add("b".into());
    assert_eq!(w.seen().collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(w.finish(), ["a (×3)", "b"]);
    let mut w = Warnings::default();
    for i in 0..MAX_WARNINGS + 5 {
        w.add(format!("{i}"));
    }
    w.add("0".into());
    let out = w.finish();
    assert_eq!(out.len(), MAX_WARNINGS);
    assert_eq!(out[0], "0 (×2)");
}

#[test]
fn style_names_are_spelled_as_words() {
    for (from, to) in [
        ("SemiBold", "Semi Bold"),
        ("ExtraBoldItalic", "Extra Bold Italic"),
        ("Regular Italic", "Italic"),
        ("italic", "Italic"),
        ("bold", "Bold"),
        ("Demi-Bold", "Demi Bold"),
        ("Condensed", "Condensed"),
        ("", ""),
    ] {
        assert_eq!(style_name(from), to, "{from}");
    }
}

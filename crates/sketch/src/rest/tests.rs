//! The mapping on files built in code.
#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::{Value, json};

use super::fixture::*;
use super::*;

fn map(z: &[u8]) -> Mapped {
    to_rest(&Archive::open(z).unwrap(), "f", Options::default())
}

/// The fixture file: an artboard holding a filled rectangle, a triangle path, a star, a text with
/// two runs, a stack of two, and an instance of a symbol with its text overridden.
fn fixture_file() -> Vec<u8> {
    let red = styled(rect("R", 10.0, 10.0, 40.0, 20.0), "fills", json!([solid_fill(1.0, 0.0, 0.0, 1.0)]));
    let tri = shape_path("T", 60.0, 10.0, 20.0, 10.0, &[(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)], true);
    let mut star = shape_path("S", 0.0, 50.0, 20.0, 20.0, &[], true);
    star["_class"] = json!("star");
    star["numberOfPoints"] = json!(5);
    star["radius"] = json!(0.5);
    let pts: Vec<(f64, f64)> = (0..10)
        .map(|i| {
            let t = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * f64::from(i) / 5.0;
            let f = if i % 2 == 0 { 1.0 } else { 0.5 };
            (0.5 + 0.5 * f * t.cos(), 0.5 + 0.5 * f * t.sin())
        })
        .collect();
    star["points"] = Value::Array(pts.iter().map(|(x, y)| point(*x, *y)).collect());
    let label = text(
        "L",
        0.0,
        80.0,
        100.0,
        20.0,
        "Hello there",
        &[(0, 6, "Helvetica", 14.0, [0.0, 0.0, 0.0, 1.0]), (6, 5, "Helvetica-Bold", 14.0, [1.0, 0.0, 0.0, 1.0])],
    );
    let row = stack(
        frame_group("K", 0.0, 120.0, 100.0, 30.0, vec![rect("K1", 0.0, 0.0, 10.0, 10.0), rect("K2", 20.0, 0.0, 10.0, 10.0)]),
        0,
        0,
        1,
        8.0,
        [4.0, 2.0, 4.0, 2.0],
    );
    let inst = instance("I", "SYM", 0.0, 160.0, 80.0, 20.0, &[("MT_stringValue", json!("Changed"))]);
    let board = artboard("A", "Board", 0.0, 0.0, 400.0, 300.0, vec![red, tri, star, label, row, inst]);
    let master_text = text("MT", 0.0, 0.0, 80.0, 20.0, "Button", &[(0, 6, "Helvetica", 12.0, [0.0, 0.0, 0.0, 1.0])]);
    let sym = master("M", "SYM", "Button", 80.0, 20.0, vec![master_text]);
    sketch_zip("2026.3.1", 196, document(&["P", "Q"]), &[page("P", "Page 1", vec![board]), page("Q", "Symbols", vec![sym])], &[])
}

/// A node of the mapped document by id.
fn by_id<'v>(json: &'v Value, id: &str) -> &'v Value {
    let mut pending = vec![&json["document"]];
    while let Some(n) = pending.pop() {
        if n["id"] == id {
            return n;
        }
        pending.extend(n["children"].as_array().into_iter().flatten());
    }
    panic!("no layer {id}")
}

#[test]
fn the_fixture_maps_onto_the_rest_shape() {
    let out = map(&fixture_file());
    let json = &out.json;
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let pages: Vec<&str> = json["document"]["children"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(pages, ["Page 1", "Symbols"]);
    let board = by_id(json, "A");
    assert_eq!((board["type"].as_str(), board["size"]["x"].as_f64()), (Some("FRAME"), Some(400.0)));
    // A rectangle with its fill, placed in the artboard.
    let red = by_id(json, "R");
    assert_eq!(red["type"], "RECTANGLE");
    assert_eq!(red["fills"][0]["color"], json!({"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0}));
    assert_eq!(red["relativeTransform"], json!([[1.0, 0.0, 10.0], [0.0, 1.0, 10.0]]));
    // A path: a vector whose network is handed over beside the JSON.
    let tri = by_id(json, "T");
    assert_eq!((tri["type"].as_str(), tri["vectorNetworkRef"].as_u64()), (Some("VECTOR"), Some(0)));
    let net = out.networks[0].as_ref().unwrap();
    assert_eq!(net.vertices.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>(), [(0.0, 0.0), (20.0, 0.0), (10.0, 10.0)]);
    assert_eq!(net.regions[0].loops, [vec![0, 1, 2]]);
    assert!(net.is_consistent());
    // A star whose points are still the generated ones: a star with the extensions.
    let star = by_id(json, "S");
    assert_eq!((star["type"].as_str(), star["pointCount"].as_u64(), star["innerRadius"].as_f64()), (Some("STAR"), Some(5), Some(0.5)));
    // Text: the second run bold and red, by UTF-16 offsets.
    let label = by_id(json, "L");
    assert_eq!(label["characters"], "Hello there");
    assert_eq!(label["characterStyleOverrides"], json!([0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1]));
    assert_eq!(label["style"]["fontFamily"], "Helvetica");
    assert_eq!(label["styleOverrideTable"]["1"]["fontStyle"], "Bold");
    // A stack: auto layout with its gap and padding; the first item is listed last.
    let row = by_id(json, "K");
    assert_eq!((row["layoutMode"].as_str(), row["itemSpacing"].as_f64(), row["paddingLeft"].as_f64()), (Some("HORIZONTAL"), Some(8.0), Some(4.0)));
    // A symbol master and an instance of it, its text overridden.
    assert_eq!(by_id(json, "M")["type"], "COMPONENT");
    let inst = by_id(json, "I");
    assert_eq!((inst["type"].as_str(), inst["componentId"].as_str()), (Some("INSTANCE"), Some("M")));
    assert_eq!(inst["children"][0]["characters"], "Changed");
}

#[test]
fn the_result_reads_as_the_typed_model() {
    let out = map(&fixture_file());
    let file = craft_figma_rest::File::from_value(out.json.clone(), 1000).unwrap();
    let doc = file.document.as_ref().unwrap();
    let label = doc.walk().find(|n| n.id.as_deref() == Some("L")).unwrap();
    assert_eq!(label.kind, Some(craft_figma_rest::NodeType::Text));
    assert_eq!(label.character_style_overrides.as_ref().map(Vec::len), Some(11));
    let star = doc.walk().find(|n| n.id.as_deref() == Some("S")).unwrap();
    assert_eq!(star.extra["pointCount"], 5);
    assert_eq!(craft_figma_rest::File::from_value(file.to_value().unwrap(), 1000).unwrap(), file);
}

#[test]
fn text_stored_the_old_way_marks_the_file_too_old() {
    let old = json!({"_class": "text", "do_objectID": "T", "name": "T", "frame": {"_class": "rect", "x": 0, "y": 0, "width": 10, "height": 10},
        "attributedString": {"_class": "MSAttributedString", "archivedAttributedString": {"_archive": "YnBsaXN0MDA="}}});
    let z = raw_zip(&document(&["P"]), &json!({"app": "com.bohemiancoding.sketch3"}), &[page("P", "P", vec![old])], &[]);
    assert!(map(&z).too_old);
    assert!(!map(&fixture_file()).too_old);
}

#[test]
fn deep_nesting_stops_with_a_warning_within_a_small_stack() {
    // 70 masters, each a group holding an instance of the next: expanded, the layers nest 140
    // deep, past the limit, while no page's JSON is deep. Nested instances take the most stack
    // per level; the mapping runs on a 1 MiB thread, the stack of a WebAssembly build.
    let run = || {
        let n = 70;
        let masters: Vec<Value> = (0..n)
            .map(|i| {
                let inner = if i < n - 1 {
                    instance(&format!("I{i}"), &format!("S{}", i + 1), 0.0, 0.0, 1.0, 1.0, &[])
                } else {
                    rect("R", 0.0, 0.0, 1.0, 1.0)
                };
                master(&format!("M{i}"), &format!("S{i}"), "Nest", 1.0, 1.0, vec![group(&format!("G{i}"), 0.0, 0.0, 1.0, 1.0, vec![inner])])
            })
            .collect();
        let top = instance("TOP", "S0", 0.0, 0.0, 1.0, 1.0, &[]);
        let z = sketch_zip("50.2", 103, document(&["P", "Q"]), &[page("P", "P", vec![top]), page("Q", "Symbols", masters)], &[]);
        let out = map(&z);
        assert!(
            out.warnings.iter().any(|w| *w == format!("layers nested deeper than {} levels were skipped", tree::MAX_DEPTH)),
            "{:?}",
            out.warnings
        );
        assert!(craft_figma_rest::check(&out.json, usize::MAX).is_ok());
        // Within the limit, nothing is skipped.
        let shallow: Vec<Value> = (0..20)
            .map(|i| {
                let inner =
                    if i < 19 { instance(&format!("I{i}"), &format!("S{}", i + 1), 0.0, 0.0, 1.0, 1.0, &[]) } else { rect("R", 0.0, 0.0, 1.0, 1.0) };
                master(&format!("M{i}"), &format!("S{i}"), "Nest", 1.0, 1.0, vec![group(&format!("G{i}"), 0.0, 0.0, 1.0, 1.0, vec![inner])])
            })
            .collect();
        let top = instance("TOP", "S0", 0.0, 0.0, 1.0, 1.0, &[]);
        let z = sketch_zip("50.2", 103, document(&["P", "Q"]), &[page("P", "P", vec![top]), page("Q", "Symbols", shallow)], &[]);
        assert!(!map(&z).warnings.iter().any(|w| w.contains("nested deeper")));
    };
    std::thread::Builder::new().stack_size(1 << 20).spawn(run).unwrap().join().unwrap();
}

#[test]
fn the_layer_cap_holds() {
    let rects: Vec<Value> = (0..10).map(|i| rect(&format!("R{i}"), 0.0, 0.0, 1.0, 1.0)).collect();
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", rects)], &[]);
    let out = to_rest(&Archive::open(&z).unwrap(), "f", Options { max_nodes: 4, ..Options::default() });
    assert_eq!(out.json["document"]["children"][0]["children"].as_array().map(Vec::len), Some(4));
    assert!(out.warnings.iter().any(|w| w.contains("more than 4 layers")), "{:?}", out.warnings);
}

#[test]
fn a_path_past_the_point_cap_is_cut() {
    let pts: Vec<(f64, f64)> = (0..100_005).map(|i| (f64::from(i % 2), f64::from(i) / 100_005.0)).collect();
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", vec![shape_path("L", 0.0, 0.0, 10.0, 10.0, &pts, false)])], &[]);
    let out = map(&z);
    assert!(out.warnings.iter().any(|w| w == "paths with more than 100000 points were cut"), "{:?}", out.warnings);
    assert_eq!(out.networks[0].as_ref().map(|n| n.vertices.len()), Some(100_000));
}

#[test]
fn notes_are_kept_once_each_and_capped() {
    // Layers of 250 unknown kinds: one note each, and past 200 one saying the rest were left out.
    let layers: Vec<Value> = (0..250)
        .map(|i| json!({"_class": format!("mystery{i}"), "do_objectID": format!("X{i}"), "frame": {"x": 0, "y": 0, "width": 1, "height": 1}}))
        .collect();
    let mut twice = layers.clone();
    twice.extend(layers);
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", twice)], &[]);
    let out = map(&z);
    assert!(out.warnings.len() <= 202, "{}", out.warnings.len());
    assert_eq!(out.warnings.iter().filter(|w| *w == "more notes were left out").count(), 1, "{:?}", &out.warnings[out.warnings.len() - 3..]);
    let distinct: std::collections::BTreeSet<&String> = out.warnings.iter().collect();
    assert_eq!(distinct.len(), out.warnings.len());
}

#[test]
fn symbol_expansion_stops_at_its_work_budget() {
    // A master of 100 layers and 50 instances of it: each expansion copies the master's values.
    let leaves: Vec<Value> = (0..100).map(|i| rect(&format!("L{i}"), 0.0, 0.0, 1.0, 1.0)).collect();
    let insts: Vec<Value> = (0..50).map(|i| instance(&format!("I{i}"), "S0", 0.0, 0.0, 10.0, 10.0, &[])).collect();
    let z = sketch_zip(
        "50.2",
        103,
        document(&["P", "Q"]),
        &[page("P", "P", insts), page("Q", "Symbols", vec![master("M0", "S0", "Leaf", 10.0, 10.0, leaves)])],
        &[],
    );
    let archive = Archive::open(&z).unwrap();
    let note = "the file's symbols expand to more than the import handles; the instances past that were imported as empty frames";
    let full = tree::to_rest_within(&archive, "f", usize::MAX, None, 60_000_000);
    assert!(!full.warnings.iter().any(|w| w == note), "{:?}", full.warnings);
    let capped = tree::to_rest_within(&archive, "f", usize::MAX, None, 20_000);
    assert!(capped.warnings.iter().any(|w| w == note), "{:?}", capped.warnings);
    let kept = |m: &tree::Map| {
        m.json["document"]["children"][0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["children"].as_array().is_some_and(|c| !c.is_empty()))
            .count()
    };
    assert_eq!(kept(&full), 50);
    assert!(kept(&capped) < 50);
}

/// A file using most of what the mapping reads, with each of its values in turn replaced by a
/// value of another type: every mapping ends without a panic.
#[test]
fn junk_values_never_panic() {
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
    let archive_bytes = fixture_file();
    let archive = Archive::open(&archive_bytes).unwrap();
    let pages: Vec<Value> = archive.pages.iter().map(|(_, p)| p.clone()).collect();
    let junk = [json!(null), json!(5), json!(-1e300), json!("x"), json!(true), json!([]), json!({}), json!([1, "a"])];
    for (pi, p) in pages.iter().enumerate() {
        let mut all = Vec::new();
        paths(p, &mut Vec::new(), &mut all);
        for (n, at) in all.iter().enumerate().skip(1).step_by(3) {
            let mut changed = pages.clone();
            let mut v = &mut changed[pi];
            for k in at {
                v = match v {
                    Value::Object(m) => m.get_mut(k).unwrap(),
                    Value::Array(a) => &mut a[k.parse::<usize>().unwrap()],
                    _ => panic!("a path only runs through objects and arrays"),
                };
            }
            *v = junk[n % junk.len()].clone();
            let z = sketch_zip("2026.3.1", 196, document(&["P", "Q"]), &changed, &[]);
            if let Ok(a) = Archive::open(&z) {
                let _ = to_rest(&a, "f", Options::default());
            }
        }
    }
}

#[test]
fn installed_fonts_are_asked_by_postscript_name() {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = std::rc::Rc::clone(&asked);
    let fonts: FontLookup = Box::new(move |ps: &str| {
        seen.borrow_mut().push(ps.to_string());
        (ps == "Helvetica-Bold").then(|| ("Helvetica Neue".to_string(), "Heavy".to_string()))
    });
    let out = to_rest(&Archive::open(&fixture_file()).unwrap(), "f", Options { fonts: Some(fonts), ..Options::default() });
    assert!(asked.borrow().iter().any(|p| p == "Helvetica-Bold"));
    let label = by_id(&out.json, "L");
    assert_eq!(label["styleOverrideTable"]["1"]["fontFamily"], "Helvetica Neue");
    assert_eq!(label["styleOverrideTable"]["1"]["fontStyle"], "Heavy");
}

#[test]
fn a_shape_group_combines_its_paths() {
    // Sketch 51 and older: a shape group in its own style; a second path subtracted from the first.
    let a = shape_path("A", 0.0, 0.0, 20.0, 20.0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], true);
    let mut b = shape_path("B", 5.0, 5.0, 10.0, 10.0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], true);
    b["booleanOperation"] = json!(1);
    let g = styled(shape_group("G", 0.0, 0.0, 20.0, 20.0, vec![a, b]), "fills", json!([solid_fill(0.0, 0.0, 1.0, 1.0)]));
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", vec![g])], &[]);
    let out = map(&z);
    let group = by_id(&out.json, "G");
    let ops: Vec<&str> = std::iter::successors(Some(group), |n| n["children"].as_array().and_then(|c| c.first()))
        .filter_map(|n| n["booleanOperation"].as_str())
        .collect();
    assert!(ops.contains(&"SUBTRACT"), "{group}");
    assert!(out.networks.iter().flatten().all(craft_figma_rest::ext::VectorNetwork::is_consistent));
}

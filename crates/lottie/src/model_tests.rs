use serde_json::json;

use crate::model::{Animated, Animation, Asset, Fill, Group, Layer, LayerContent, Limits, Marker, Rect, Shape, ShapeKind, SolidLayer, Transform};
use crate::{Json, KeyOut, LottieError, PropValue, Tangents};

fn read(doc: &Json) -> Animation {
    Animation::from_document(doc.clone(), &Limits::default()).unwrap()
}

/// A document with every layer type, every shape item, masks, effects, text, markers, fonts,
/// glyphs and both kinds of asset.
fn everything() -> Json {
    let p = |k: Json| json!({"a": 0, "k": k});
    let ks = json!({"a": p(json!([0, 0, 0])), "p": p(json!([50, 50, 0])), "s": p(json!([100, 100, 100])), "r": p(json!(0)), "o": p(json!(100))});
    let shapes = json!([
        {"ty": "gr", "nm": "Group", "np": 3, "it": [
            {"ty": "rc", "nm": "Rect", "d": 1, "p": p(json!([0, 0])), "s": p(json!([10, 20])), "r": p(json!(2))},
            {"ty": "el", "d": 3, "p": p(json!([0, 0])), "s": p(json!([5, 5]))},
            {"ty": "sr", "sy": 1, "d": 1, "p": p(json!([0, 0])), "pt": p(json!(5)), "r": p(json!(0)), "or": p(json!(10)), "os": p(json!(0)), "ir": p(json!(4)), "is": p(json!(0))},
            {"ty": "sh", "d": 1, "ks": p(json!({"c": true, "v": [[0, 0], [1, 0]], "i": [[0, 0], [0, 0]], "o": [[0, 0], [0, 0]]}))},
            {"ty": "fl", "c": p(json!([1, 0, 0, 1])), "o": p(json!(100)), "r": 1, "bm": 0},
            {"ty": "st", "c": p(json!([0, 0, 1, 1])), "o": p(json!(50)), "w": p(json!(2)), "lc": 2, "lj": 1, "ml": 4,
             "d": [{"n": "d", "nm": "dash", "v": p(json!(3))}, {"n": "g", "nm": "gap", "v": p(json!(1))}, {"n": "o", "nm": "offset", "v": p(json!(0))}]},
            {"ty": "gf", "g": {"p": 2, "k": p(json!([0, 1, 0, 0, 1, 0, 0, 1]))}, "s": p(json!([0, 0])), "e": p(json!([10, 0])), "t": 2, "h": p(json!(0)), "a": p(json!(0)), "o": p(json!(100)), "r": 2},
            {"ty": "gs", "g": {"p": 2, "k": p(json!([0, 1, 0, 0, 1, 0, 0, 1]))}, "s": p(json!([0, 0])), "e": p(json!([10, 0])), "t": 1, "o": p(json!(100)), "w": p(json!(1)), "lc": 1, "lj": 2, "ml2": p(json!(4))},
            {"ty": "tm", "s": p(json!(0)), "e": p(json!(50)), "o": p(json!(0)), "m": 1},
            {"ty": "rd", "r": p(json!(3))},
            {"ty": "rp", "c": p(json!(3)), "o": p(json!(0)), "m": 1, "tr": {"p": p(json!([10, 0])), "s": p(json!([100, 100])), "so": p(json!(100)), "eo": p(json!(0))}},
            {"ty": "mm", "mm": 2},
            {"ty": "pb", "a": p(json!(10))},
            {"ty": "tw", "a": p(json!(30)), "c": p(json!([0, 0]))},
            {"ty": "zz", "r": p(json!(5)), "s": p(json!(3)), "pt": p(json!(1))},
            {"ty": "op", "a": p(json!(2)), "lj": 1, "ml": p(json!(4))},
            {"ty": "tr", "nm": "Transform", "a": p(json!([0, 0])), "p": p(json!([0, 0])), "s": p(json!([100, 100])), "r": p(json!(0)), "o": p(json!(100)), "sk": p(json!(0)), "sa": p(json!(0))}
        ]},
        {"ty": "future", "nm": "Later", "q": [1, 2]}
    ]);
    let text = json!({
        "d": {"k": [{"t": 0, "s": {"f": "Sans-Bold", "s": 24, "fc": [1, 1, 1], "sc": [0, 0, 0], "sw": 1, "of": true, "lh": 28.8, "ls": 0, "tr": 20, "j": 2, "t": "Hi\rthere", "sz": [200, 80], "ps": [-100, -40], "ca": 0}}]},
        "a": [{"nm": "Fade", "s": {"t": 1, "s": p(json!(0)), "e": p(json!(100)), "o": p(json!(0)), "a": p(json!(100)), "b": 1, "r": 1, "sh": 1, "rn": 0}, "a": {"o": p(json!(0)), "p": p(json!([0, 10])), "t": p(json!(5))}}],
        "m": {"g": 1, "a": p(json!([0, 0]))},
        "p": {"m": 0, "f": p(json!(0)), "l": p(json!(0)), "r": p(json!(0)), "a": p(json!(0)), "p": p(json!(1))}
    });
    json!({
        "v": "5.9.0", "nm": "All", "fr": 29.97, "ip": 0, "op": 90, "w": 320, "h": 240, "ddd": 0,
        "meta": {"g": "test"},
        "markers": [{"cm": "start", "tm": 0, "dr": 10}],
        "fonts": {"list": [{"fName": "Sans-Bold", "fFamily": "Sans", "fStyle": "Bold", "ascent": 75.5, "origin": 0}]},
        "chars": [{"ch": "H", "fFamily": "Sans", "style": "Bold", "size": 24, "w": 70, "data": {"shapes": [{"ty": "gr", "it": [{"ty": "sh", "ks": p(json!({"c": true, "v": [], "i": [], "o": []}))}]}]}}],
        "assets": [
            {"id": "img", "nm": "Photo", "u": "images/", "p": "a.png", "w": 8, "h": 8, "e": 0},
            {"id": "pre", "nm": "Inner", "fr": 30, "layers": [{"ty": 3, "nm": "Null", "ind": 1, "ip": 0, "op": 90, "st": 0, "sr": 1, "ks": ks}]}
        ],
        "layers": [
            {"ty": 0, "nm": "Precomp", "ind": 1, "refId": "pre", "w": 320, "h": 240, "tm": p(json!(0)), "ip": 0, "op": 90, "st": 0, "sr": 1, "ks": ks, "tt": 1, "tp": 2},
            {"ty": 1, "nm": "Solid", "ind": 2, "sc": "#ff8000", "sw": 320, "sh": 240, "td": 1, "ip": 0, "op": 90, "st": 0, "sr": 1, "ks": ks, "hasMask": true,
             "masksProperties": [{"nm": "M", "mode": "s", "inv": true, "pt": p(json!({"c": true, "v": [], "i": [], "o": []})), "o": p(json!(100)), "x": p(json!(0))}],
             "ef": [{"ty": 29, "nm": "Blur", "np": 3, "en": 1, "ef": [{"ty": 0, "nm": "Blurriness", "v": p(json!(4))}, {"ty": 7, "nm": "Dimensions", "v": p(json!(1))}]}]},
            {"ty": 2, "nm": "Image", "ind": 3, "refId": "img", "parent": 5, "hd": false, "bm": 3, "ao": 0, "ip": 0, "op": 90, "st": 0, "sr": 1, "ks": ks},
            {"ty": 3, "nm": "Null", "ind": 5, "ddd": 1, "ip": 0, "op": 90, "st": 0, "sr": 1, "ks": {"p": {"s": true, "x": p(json!(1)), "y": p(json!(2))}, "rx": p(json!(0)), "ry": p(json!(0)), "rz": p(json!(0)), "or": p(json!([0, 0, 0]))}},
            {"ty": 4, "nm": "Shapes", "ind": 6, "ip": 0, "op": 90, "st": 0, "sr": 2.5, "ct": 0, "ks": ks, "shapes": shapes},
            {"ty": 5, "nm": "Text", "ind": 7, "ip": 0, "op": 90, "st": 0, "sr": 1, "ks": ks, "t": text},
            {"ty": 13, "nm": "Camera", "ind": 8, "pe": p(json!(500))}
        ]
    })
}

#[test]
fn every_object_reads_typed_and_writes_back_the_same() {
    let doc = everything();
    let a = read(&doc);
    assert_eq!((a.version.as_deref(), a.frame_rate, a.width, a.three_d), (Some("5.9.0"), Some(29.97), Some(320.0), Some(0)));
    assert!(a.extra.is_empty(), "{:?}", a.extra);
    assert_eq!(
        a.markers.as_deref(),
        Some(&[Marker { comment: Some("start".into()), time: Some(0.0), duration: Some(10.0), ..Marker::default() }][..])
    );
    assert_eq!(a.fonts.as_ref().unwrap().list.as_ref().unwrap()[0].ascent, Some(75.5));
    assert_eq!(a.chars.as_ref().unwrap()[0].data.as_ref().unwrap().shapes.as_ref().unwrap().len(), 1);
    assert_eq!(a.asset("img").unwrap().path.as_deref(), Some("a.png"));
    assert_eq!(a.asset("pre").unwrap().layers.as_ref().unwrap()[0].content, LayerContent::Null);
    assert!(a.asset("none").is_none());

    assert_eq!(a.to_json(), doc);
    assert_eq!(Animation::read(&a.to_vec()).unwrap(), a);
}

#[test]
fn every_layer_type_reads_typed() {
    let a = read(&everything());
    let layers = a.layers.as_deref().unwrap();
    let LayerContent::Precomp(pre) = &layers[0].content else { panic!("precomp") };
    assert_eq!((pre.ref_id.as_deref(), pre.width, layers[0].matte_mode, layers[0].matte_parent), (Some("pre"), Some(320.0), Some(1), Some(2)));
    assert_eq!(layers[1].content, LayerContent::Solid(SolidLayer { color: Some("#ff8000".into()), width: Some(320.0), height: Some(240.0) }));
    let mask = &layers[1].masks.as_ref().unwrap()[0];
    assert_eq!((mask.mode.as_deref(), mask.inverted), (Some("s"), Some(true)));
    let blur = &layers[1].effects.as_ref().unwrap()[0];
    assert_eq!((blur.kind, blur.values.as_ref().unwrap().len(), blur.values.as_ref().unwrap()[1].kind), (Some(29), 2, Some(7)));
    assert_eq!(layers[2].content, LayerContent::Image(Some("img".into())));
    assert_eq!((layers[2].parent, layers[2].hidden, layers[2].blend_mode), (Some(5), Some(false), Some(3)));
    let split = layers[3].transform.as_ref().unwrap().position.as_ref().unwrap().split().unwrap();
    assert!(split[0].is_some() && split[1].is_some() && split[2].is_none());
    assert!(layers[0].transform.as_ref().unwrap().position.as_ref().unwrap().split().is_none());
    assert_eq!(layers[4].stretch, Some(2.5));
    let LayerContent::Text(t) = &layers[5].content else { panic!("text") };
    let d = t.documents.as_ref().unwrap().keys.as_ref().unwrap()[0].document.as_ref().unwrap();
    assert_eq!(
        (d.text.as_deref(), d.justify, d.fill.as_deref(), d.stroke_over_fill),
        (Some("Hi\rthere"), Some(2), Some(&[1.0, 1.0, 1.0][..]), Some(true))
    );
    assert!(t.ranges.as_ref().unwrap()[0].style.as_ref().unwrap().tracking.is_some());
    assert_eq!(t.follow_path.as_ref().unwrap().mask, Some(0));
    assert_eq!(layers[6].content, LayerContent::Other(json!(13)));
    assert!(layers[6].extra.contains_key("pe"), "an unknown layer keeps its own keys");
}

#[test]
fn every_shape_item_reads_typed() {
    let a = read(&everything());
    let LayerContent::Shape(shapes) = &a.layers.as_deref().unwrap()[4].content else { panic!("shapes") };
    let ShapeKind::Group(g) = &shapes[0].kind else { panic!("group") };
    let tags: Vec<_> = g.items.as_ref().unwrap().iter().map(|s| s.kind.tag().unwrap()).collect();
    assert_eq!(tags, ["rc", "el", "sr", "sh", "fl", "st", "gf", "gs", "tm", "rd", "rp", "mm", "pb", "tw", "zz", "op", "tr"]);
    let items = g.items.as_ref().unwrap();
    assert!(items.iter().all(|s| s.extra.is_empty()), "every key of every item is modelled");
    let ShapeKind::Stroke(st) = &items[5].kind else { panic!("stroke") };
    assert_eq!((st.cap, st.miter_limit, st.dashes.as_ref().unwrap().len()), (Some(2), Some(4.0), 3));
    let ShapeKind::GradientFill(gf) = &items[6].kind else { panic!("gradient") };
    let colors = gf.colors.as_ref().unwrap();
    let PropValue::Static(stops) = colors.stops.as_ref().unwrap().read().value else { panic!("static") };
    assert_eq!(crate::Gradient::from_flat(stops, colors.count.unwrap() as usize).unwrap().colors.len(), 2);
    assert_eq!((items[16].name.as_deref(), matches!(items[16].kind, ShapeKind::Transform(_))), (Some("Transform"), true));
    assert_eq!(shapes[1].kind, ShapeKind::Other(json!("future")));
    assert_eq!(shapes[1].extra.get("q"), Some(&json!([1, 2])));
}

#[test]
fn what_the_model_does_not_know_is_kept() {
    // Unknown keys at every level, known keys of another type, entries that are not objects.
    let doc = json!({
        "w": "wide", "h": 10, "nm": 7, "tiny": {"x": 1}, "assets": {"not": "a list"},
        "layers": [
            {"ty": 4, "nm": ["n"], "ind": 1.5, "hd": 2, "ks": 5, "shapes": [{"ty": "rc", "d": "cw", "p": null, "zzz": 1}, {"nm": "no type"}], "custom": true},
            {"ty": 4, "nm": "no shapes"},
            {"ty": 5, "t": "not an object"},
            {"ty": "4", "shapes": []},
            {"nm": "no type at all"}
        ]
    });
    let a = read(&doc);
    assert_eq!((a.width, a.height, &a.name, &a.assets), (None, Some(10.0), &None, &None));
    assert_eq!(a.extra.get("w"), Some(&json!("wide")));
    let layers = a.layers.as_deref().unwrap();
    assert_eq!((&layers[0].name, layers[0].index, layers[0].hidden, &layers[0].transform), (&None, None, None, &None));
    let LayerContent::Shape(shapes) = &layers[0].content else { panic!("shapes") };
    let ShapeKind::Rect(rc) = &shapes[0].kind else { panic!("rect") };
    assert_eq!(rc, &Rect::default());
    assert_eq!(shapes[1].kind, ShapeKind::Other(Json::Null));
    assert_eq!(layers[1].content, LayerContent::Other(json!(4)));
    assert_eq!(layers[2].content, LayerContent::Other(json!(5)));
    assert_eq!(layers[3].content, LayerContent::Other(json!("4")));
    assert_eq!(layers[4].content, LayerContent::Other(Json::Null));
    assert_eq!(a.to_json(), doc);

    // List entries of another type are the one thing left out.
    let a = read(&json!({"layers": [1, "two", {"ty": 3}, null], "markers": [[], {"tm": 3}]}));
    assert_eq!(a.to_json(), json!({"layers": [{"ty": 3}], "markers": [{"tm": 3}]}));
}

#[test]
fn numbers_and_flags_are_read_however_written() {
    let a = read(
        &json!({"fr": 30.0, "ddd": 1.0, "w": 1e3, "layers": [{"ty": 3.0, "ind": 4.0, "hd": 1, "hasMask": 0, "sr": 0.5, "parent": 9007199254740993u64}]}),
    );
    let l = &a.layers.as_ref().unwrap()[0];
    assert_eq!((a.three_d, l.index, l.hidden, l.has_mask, l.stretch), (Some(1), Some(4), Some(true), Some(false), Some(0.5)));
    assert_eq!(l.content, LayerContent::Other(json!(3.0)), "a `ty` is an integer");
    // Written back: whole numbers without a fraction, flags as booleans.
    let out = a.to_json();
    assert_eq!(serde_json::to_string(&out["fr"]).unwrap(), "30");
    assert_eq!(serde_json::to_string(&out["w"]).unwrap(), "1000");
    assert_eq!(out["layers"][0]["hd"], json!(true));
    assert_eq!(out["layers"][0]["sr"], json!(0.5));
    assert_eq!(out["layers"][0]["parent"], json!(9007199254740993u64));
    // Numbers that are not finite have no JSON form.
    let a = Animation { frame_rate: Some(f64::NAN), width: Some(f64::INFINITY), ..Animation::default() };
    assert_eq!(a.to_json(), json!({"fr": null, "w": null}));
}

#[test]
fn a_document_built_in_code_reads_back() {
    let lin = Tangents { ox: 0.25, oy: 0.25, ix: 0.75, iy: 0.75 };
    let keys = [
        KeyOut { t: 0.0, s: json!([0]), hold: false, ease: vec![lin], spatial: None },
        KeyOut { t: 30.0, s: json!([100]), hold: false, ease: vec![], spatial: None },
    ];
    let rect = Shape {
        name: Some("R".into()),
        kind: ShapeKind::Rect(Rect { size: Some(Animated::fixed(json!([10, 10]))), ..Rect::default() }),
        ..Shape::default()
    };
    let fill = Shape { kind: ShapeKind::Fill(Fill { color: Some(Animated::fixed(json!([1, 0, 0, 1]))), ..Fill::default() }), ..Shape::default() };
    let group = Shape { kind: ShapeKind::Group(Group { count: Some(2.0), items: Some(vec![rect, fill]) }), ..Shape::default() };
    let layer = Layer {
        name: Some("L".into()),
        index: Some(1),
        in_point: Some(0.0),
        out_point: Some(30.0),
        transform: Some(Transform { opacity: Some(Animated::keyed(&keys, Some("time"))), ..Transform::default() }),
        content: LayerContent::Shape(vec![group]),
        ..Layer::default()
    };
    let a = Animation {
        version: Some("5.9.0".into()),
        frame_rate: Some(30.0),
        in_point: Some(0.0),
        out_point: Some(30.0),
        width: Some(100.0),
        height: Some(100.0),
        assets: Some(vec![Asset { id: Some("x".into()), ..Asset::default() }]),
        layers: Some(vec![layer]),
        ..Animation::default()
    };
    let back = Animation::read(&a.to_vec()).unwrap();
    assert_eq!(back, a);
    let out = a.to_json();
    assert_eq!(out["layers"][0]["ty"], json!(4));
    assert_eq!(out["layers"][0]["shapes"][0]["it"][1]["ty"], json!("fl"));
    let o = back.layers.as_ref().unwrap()[0].transform.as_ref().unwrap().opacity.as_ref().unwrap().read();
    assert_eq!(o.expression, Some("time"));
    let PropValue::Keys(k) = o.value else { panic!("keys") };
    assert_eq!((k.len(), k[1].t), (2, 30.0));
    // A layer with no type and nothing else is an empty object.
    assert_eq!(Animation { layers: Some(vec![Layer::default()]), ..Animation::default() }.to_json(), json!({"layers": [{}]}));
}

#[test]
fn fixtures_round_trip() {
    for name in ["bouncing-ball.json", "legacy-masks.json", "precomp-matte-text.json", "shapes-gradient.json"] {
        let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap();
        let doc: Json = serde_json::from_slice(&bytes).unwrap();
        let a = Animation::read(&bytes).unwrap();
        assert!(a.layers.as_ref().is_some_and(|l| !l.is_empty()), "{name}");
        assert_eq!(a.to_json(), doc, "{name}");
    }
}

#[test]
fn every_limit_is_enforced() {
    let too = |what, limit| Err(LottieError::TooLarge { what, limit });
    let at = |doc: &Json, limits: Limits| Animation::from_document(doc.clone(), &limits).map(|_| ());

    let bytes = br#"{"w":1,"h":1}"#;
    assert!(Animation::read_with_limits(bytes, &Limits { max_bytes: bytes.len(), ..Limits::default() }).is_ok());
    assert_eq!(
        Animation::read_with_limits(bytes, &Limits { max_bytes: bytes.len() - 1, ..Limits::default() }).map(|_| ()),
        too("bytes", bytes.len() - 1)
    );

    // Two layers at the top and two in a precomposition.
    let doc = json!({"assets": [{"id": "a", "layers": [{"ty": 3}, {"ty": 3}]}, {"id": "b"}], "layers": [{"ty": 3}, {"ty": 3}]});
    assert_eq!(at(&doc, Limits { max_layers: 4, max_assets: 2, ..Limits::default() }), Ok(()));
    assert_eq!(at(&doc, Limits { max_layers: 3, ..Limits::default() }), too("layers", 3));
    assert_eq!(at(&doc, Limits { max_assets: 1, ..Limits::default() }), too("assets", 1));

    // Shape items: a group, the two in it, one in a precomposition's layer and one in a glyph.
    let doc = json!({
        "chars": [{"data": {"shapes": [{"ty": "sh"}]}}],
        "assets": [{"layers": [{"ty": 4, "shapes": [{"ty": "el"}]}]}],
        "layers": [{"ty": 4, "shapes": [{"ty": "gr", "it": [{"ty": "rc"}, {"ty": "fl"}]}]}]
    });
    assert_eq!(at(&doc, Limits { max_shapes: 5, ..Limits::default() }), Ok(()));
    assert_eq!(at(&doc, Limits { max_shapes: 4, ..Limits::default() }), too("shape items", 4));

    // Nesting: `{"layers": [{"x": 1}]}` is three levels.
    let doc = json!({"layers": [{"x": 1}]});
    assert_eq!(at(&doc, Limits { max_depth: 3, ..Limits::default() }), Ok(()));
    assert_eq!(at(&doc, Limits { max_depth: 2, ..Limits::default() }), too("nesting levels", 2));
    // A value built in code, deeper than a parser lets through, is refused and not walked.
    let mut deep = json!(1);
    for _ in 0..300 {
        deep = json!({"it": [deep], "ty": "gr"});
    }
    assert_eq!(at(&json!({"layers": [{"ty": 4, "shapes": [deep]}]}), Limits::default()), too("nesting levels", 128));
    // The same from text: the parser stops it.
    let text = format!("{{\"layers\":{}{}}}", "[".repeat(300), "]".repeat(300));
    assert!(matches!(Animation::read(text.as_bytes()), Err(LottieError::Parse(_))));
}

#[test]
fn what_is_not_a_document_is_an_error() {
    for bad in [&b"[1, 2]"[..], b"5", b"null", b"\"text\"", b"", b"{\"w\":", b"PK\x03\x04"] {
        assert!(matches!(Animation::read(bad), Err(LottieError::Parse(_))), "{bad:?}");
    }
    assert_eq!(Animation::read(b"{}").unwrap(), Animation::default());
}

#[test]
fn damaged_documents_never_panic() {
    let doc = serde_json::to_vec(&everything()).unwrap();
    // Every prefix, and every byte replaced by a few that change the structure.
    for cut in 0..doc.len() {
        let _ = Animation::read(&doc[..cut]);
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..4000 {
        let mut d = doc.clone();
        for _ in 0..3 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed >> 8) as usize % d.len();
            d[at] = b"{}[]\",:0n-e. t"[(seed >> 40) as usize % 14];
        }
        if let Ok(a) = Animation::read(&d) {
            // Whatever read writes back to something that reads the same.
            assert_eq!(Animation::read(&a.to_vec()).unwrap(), a);
        }
    }
}

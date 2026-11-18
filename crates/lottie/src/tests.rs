use serde_json::json;

use crate::{
    Bezier, Ease, Gradient, Header, Json, KeyOut, LottieError, PropValue, Tangents, base64, color, hex_color, image_mime, is_animated, nums,
};
use crate::{parse_document, read_prop, round6, tangent, to_hex, write_keys, write_prop};

fn fixture(name: &str) -> Json {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    parse_document(&std::fs::read(path).unwrap()).unwrap()
}

/// Every animatable property (`{a, k}`) under `j`.
fn props<'a>(j: &'a Json, out: &mut Vec<&'a Json>) {
    match j {
        Json::Object(m) => {
            if m.contains_key("a") && m.contains_key("k") {
                out.push(j);
            }
            m.values().for_each(|v| props(v, out));
        }
        Json::Array(a) => a.iter().for_each(|v| props(v, out)),
        _ => {}
    }
}

#[test]
fn header_defaults_and_refusals() {
    let doc = json!({"w": 120, "h": 80, "fr": 25, "ip": 10, "op": 60, "nm": "Ball"});
    assert_eq!(
        Header::read(&doc).unwrap(),
        Header { width: 120.0, height: 80.0, frame_rate: 25.0, in_point: 10.0, out_point: 60.0, name: Some("Ball") }
    );
    // No rate: 30. No `op`: one second after `ip`. An empty name is no name.
    let doc = json!({"w": 1, "h": 1, "fr": 0, "ip": 5, "nm": ""});
    let h = Header::read(&doc).unwrap();
    assert_eq!((h.frame_rate, h.in_point, h.out_point, h.name), (30.0, 5.0, 35.0, None));
    // `op` before `ip`: one frame.
    assert_eq!(Header::read(&json!({"w": 1, "h": 1, "ip": 5, "op": 2})).unwrap().out_point, 6.0);
    for bad in [json!({"w": 1}), json!({"w": "1", "h": 1}), json!([1, 2]), json!(null)] {
        assert_eq!(Header::read(&bad), Err(LottieError::Parse("missing `w`/`h`".into())));
    }
    assert!(matches!(parse_document(b"{\"w\": 1"), Err(LottieError::Parse(_))));
    assert!(matches!(parse_document(b"PK\x03\x04"), Err(LottieError::Parse(_))));
}

#[test]
fn nesting_deeper_than_128_levels_is_refused() {
    let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    assert!(parse_document(deep(100).as_bytes()).is_ok());
    assert!(parse_document(deep(200).as_bytes()).is_err());
}

#[test]
fn numbers_and_colours() {
    assert_eq!(nums(&json!(5)), Some(vec![5.0]));
    assert_eq!(nums(&json!([5])), Some(vec![5.0]));
    assert_eq!(nums(&json!([[1, 2]])), Some(vec![1.0, 2.0]));
    assert_eq!(nums(&json!([1, "x"])), None);
    assert_eq!(nums(&json!("5")), None);
    assert_eq!(color(&json!([0.5, 0.25])), Some([0.5, 0.25, 0.0, 1.0]));
    // 0..=255 files: colour divided, alpha kept.
    assert_eq!(color(&json!([255, 0, 51, 1])), Some([1.0, 0.0, 0.2, 1.0]));
    assert_eq!(color(&json!([])), None);
    assert_eq!(hex_color("#e0c040"), [224.0 / 255.0, 192.0 / 255.0, 64.0 / 255.0]);
    // `+1` parses as a number but is not two hex digits; a short string has no blue.
    assert_eq!(hex_color("#+1+2+3"), [0.0, 0.0, 0.0]);
    assert_eq!(hex_color("ff80"), [1.0, 128.0 / 255.0, 0.0]);
    assert_eq!(to_hex([1.0, 0.5, -3.0]), "#ff8000");
    assert_eq!(hex_color(&to_hex([0.2, 0.4, 0.6])), [0.2, 0.4, 0.6]);
    assert_eq!(round6(1.234_567_89), 1.234_568);
    assert_eq!(round6(f64::NAN), 0.0);
    assert_eq!(round6(f64::INFINITY), 0.0);
}

#[test]
fn image_signatures_are_matched_in_full() {
    assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n\0"), Some("image/png"));
    assert_eq!(image_mime(b"GIF89a\0"), Some("image/gif"));
    assert_eq!(image_mime(b"GIF87a\0"), Some("image/gif"));
    assert_eq!(image_mime(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
    assert_eq!(image_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    // Near matches (a prefix of the signature, or text that starts with it) are not images.
    assert_eq!(image_mime(b"\x89PNG\x01\x02\x03\x04"), None);
    assert_eq!(image_mime(b"GIF8 is a text file"), None);
    assert_eq!(image_mime(b"RIFF\0\0\0\0WAVE"), None);
    assert_eq!(image_mime(b""), None);
}

#[test]
fn base64_and_data_uris() {
    for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
        assert_eq!(base64::decode(&base64::encode(s.as_bytes())).unwrap(), s.as_bytes());
    }
    assert_eq!(base64::encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64::encode(b"fo"), "Zm8=");
    // URL-safe letters and white space are read; anything else, or one stray letter, is not.
    assert_eq!(base64::decode("-_ 8=\n"), Some(vec![0xfb, 0xff]));
    assert_eq!(base64::decode("Zm9v!"), None);
    assert_eq!(base64::decode("Zm9vY"), None);
    let uri = base64::to_data_uri("image/png", b"\x89PNG");
    assert_eq!(uri, "data:image/png;base64,iVBORw==");
    assert_eq!(base64::data_uri(&uri), Some(("image/png".to_string(), b"\x89PNG".to_vec())));
    assert_eq!(base64::data_uri("data:image/png,raw"), None);
    assert_eq!(base64::data_uri("images/a.png"), None);
}

#[test]
fn paths_read_legacy_closed_flags_and_fit_tangents() {
    let j = json!({"v": [[0, 0], [10, 0], [10, 10]], "i": [[1, 1]], "o": [[2, 2], [3, 3], [4, 4], [5, 5]]});
    let p = Bezier::from_json(&j, true).unwrap();
    assert!(p.closed, "no `c`: the flag kept outside the path applies");
    assert_eq!(p.vertices.len(), 3);
    assert_eq!(p.in_tangents, [[1.0, 1.0], [0.0, 0.0], [0.0, 0.0]]);
    assert_eq!(p.out_tangents, [[2.0, 2.0], [3.0, 3.0], [4.0, 4.0]]);
    assert!(!Bezier::from_json(&json!({"c": false, "v": []}), true).unwrap().closed);
    // A key's `s` wraps the path in a list; a point that is not numbers is the origin.
    let wrapped = Bezier::from_json(&json!([{"c": true, "v": [[1, 2], "x", [3]]}]), false).unwrap();
    assert_eq!(wrapped.vertices, [[1.0, 2.0], [0.0, 0.0], [3.0, 0.0]]);
    assert_eq!(Bezier::from_json(&json!([]), false), None);
    assert_eq!(Bezier::from_json(&json!(5), false), Some(Bezier::default()));
    let back = Bezier::from_json(&p.to_json(), false).unwrap();
    assert_eq!(back, p);
    assert_eq!(Bezier { vertices: vec![[1.0 / 3.0, 2.0]], ..Bezier::default() }.to_json()["v"], json!([[0.333_333, 2.0]]));
}

#[test]
fn gradients_split_colour_and_alpha_stops() {
    let flat = json!([0.0, 0.1, 0.4, 0.9, 1.0, 0.9, 0.2, 0.5]);
    let g = Gradient::from_flat(&flat, 2).unwrap();
    assert_eq!(g.colors, [(0.0, [0.1, 0.4, 0.9]), (1.0, [0.9, 0.2, 0.5])]);
    assert_eq!(g.opacities, [(0.0, 1.0), (1.0, 1.0)]);
    assert_eq!(g.to_flat(), flat, "opaque: no alpha pairs written");
    let with_alpha = json!([0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.5, 0.25]);
    let g = Gradient::from_flat(&with_alpha, 2).unwrap();
    assert_eq!(g.opacities, [(0.0, 1.0), (0.5, 0.25)]);
    assert_eq!(g.to_flat(), with_alpha);
    // A count beyond the list is held to the groups there are; an odd tail is dropped.
    let g = Gradient::from_flat(&json!([0, 1, 1, 1, 0.5]), 99).unwrap();
    assert_eq!((g.colors.len(), g.opacities.len()), (1, 2));
    assert_eq!(Gradient::from_flat(&json!([]), usize::MAX).unwrap().colors, []);
    assert_eq!(Gradient::from_flat(&json!("x"), 2), None);
}

#[test]
fn properties_static_keyed_legacy_and_bare() {
    // A bare value.
    assert_eq!(read_prop(&json!(100)).value, PropValue::Static(&json!(100)));
    let p = json!({"a": 0, "k": [1, 2], "x": "wiggle(1, 2)"});
    let r = read_prop(&p);
    assert_eq!(r.expression, Some("wiggle(1, 2)"));
    assert_eq!(r.value, PropValue::Static(&json!([1, 2])));
    assert_eq!(read_prop(&json!({"a": 0, "k": 1, "x": "  "})).expression, None);
    assert_eq!(read_prop(&json!({"a": 1})).value, PropValue::None);
    assert_eq!(read_prop(&json!({"a": 1, "k": 5})).value, PropValue::None);
    // No `a`, but `k` is a list of keys.
    assert!(is_animated(&json!({"k": [{"t": 0, "s": [1]}]})));
    assert!(!is_animated(&json!({"k": [1, 2, 3]})));

    // Before 5.0: a segment ends with `e`, and the last key has no value of its own.
    let legacy = json!({"a": 1, "k": [
        {"t": 0, "s": [0], "e": [100], "i": {"x": [0.833], "y": [0.833]}, "o": {"x": [0.167], "y": [0.167]}},
        {"t": 20}
    ]});
    let PropValue::Keys(keys) = read_prop(&legacy).value else { panic!("keys") };
    assert_eq!(keys.len(), 2);
    assert_eq!((keys[0].t, keys[0].s), (0.0, Some(&json!([0]))));
    assert_eq!((keys[1].t, keys[1].s), (20.0, Some(&json!([100]))));
    assert!(keys[1].o.is_none() && !keys[1].hold);

    let spatial = json!({"a": 1, "k": [{"t": 0, "s": [0, 0], "h": 1, "to": [1, 2], "ti": [3, 4, 5, 6]}, {"s": [9, 9]}, {"t": 4}]});
    let PropValue::Keys(keys) = read_prop(&spatial).value else { panic!("keys") };
    assert!(keys[0].hold);
    assert_eq!((keys[0].to, keys[0].ti), ([1.0, 2.0, 0.0], [3.0, 4.0, 5.0]));
    assert_eq!(keys[1].t, 0.0, "no `t`: frame 0");
    assert_eq!(keys[2].s, None, "no `s` and no `e` before it");
}

#[test]
fn tangents_are_numbers_or_lists() {
    let o = json!({"x": [0.1, 0.2], "y": 0.5});
    assert_eq!(tangent(&o, "x", 1), Some(0.2));
    assert_eq!(tangent(&o, "x", 7), Some(0.1), "a short list gives its first");
    assert_eq!(tangent(&o, "y", 3), Some(0.5));
    assert_eq!(tangent(&o, "z", 0), None);
    assert_eq!(tangent(&json!({"x": "a"}), "x", 0), None);
    // Defaults: thirds, linear; `x` held to 0..=1.
    let t = Tangents::read(&json!({}), &json!({}), 0);
    assert_eq!(t, Tangents { ox: 1.0 / 3.0, oy: 1.0 / 3.0, ix: 2.0 / 3.0, iy: 2.0 / 3.0 });
    assert!(t.is_linear());
    let t = Tangents::read(&json!({"x": 5, "y": 2}), &json!({"x": -1}), 0);
    assert_eq!(t, Tangents { ox: 1.0, oy: 2.0, ix: 0.0, iy: 0.0 });
    assert!(!t.is_linear());
}

#[test]
fn ease_converts_exactly_both_ways() {
    // 0 to 180 in 0.8 s, leaving at 300/s with 60 % influence, arriving at -50/s with 20 %.
    let (out, into) = (Ease { speed: 300.0, influence: 0.6 }, Ease { speed: -50.0, influence: 0.2 });
    let t = Tangents::from_speed(out, into, 180.0, 0.8);
    assert!((t.ox - 0.6).abs() < 1e-9);
    assert!((t.oy - 300.0 * 0.6 * 0.8 / 180.0).abs() < 1e-5);
    assert!((t.ix - 0.8).abs() < 1e-9);
    assert!((t.iy - (1.0 + 50.0 * 0.2 * 0.8 / 180.0)).abs() < 1e-5);
    let (o, i) = t.to_speed(180.0, 0.8);
    assert!((o.speed - 300.0).abs() < 1e-3 && (o.influence - 0.6).abs() < 1e-6, "{o:?}");
    assert!((i.speed + 50.0).abs() < 1e-3 && (i.influence - 0.2).abs() < 1e-6, "{i:?}");
    // A falling value: 100 to 20 at 10/s out.
    let t = Tangents::from_speed(Ease { speed: 10.0, influence: 0.9 }, Ease { speed: 0.0, influence: 1.0 / 3.0 }, -80.0, 1.0);
    assert!((t.oy + 0.1125).abs() < 1e-9);
    assert!((t.ix - 2.0 / 3.0).abs() < 1e-6);
    // No value change: progress 0 and 1, nothing divided by zero; influences held to 0..=1.
    let t = Tangents::from_speed(Ease { speed: 9.0, influence: 7.0 }, Ease { speed: 9.0, influence: -1.0 }, 0.0, 1.0);
    assert_eq!(t, Tangents { ox: 1.0, oy: 0.0, ix: 1.0, iy: 1.0 });
    // A zero `x` still gives a finite speed.
    let (o, i) = Tangents { ox: 0.0, oy: 0.0, ix: 1.0, iy: 1.0 }.to_speed(10.0, 1.0);
    assert!(o.speed.is_finite() && i.speed.is_finite());
    assert_eq!((o.influence, i.influence), (0.001, 0.001));
}

#[test]
fn keys_are_written_and_read_back() {
    let lin = Tangents { ox: 0.25, oy: 0.5, ix: 0.75, iy: 1.0 };
    let keys = [
        KeyOut { t: 0.0, s: json!(5), hold: false, ease: vec![lin], spatial: None },
        KeyOut { t: 10.000_000_4, s: json!([1, 2]), hold: true, ease: vec![lin], spatial: None },
        KeyOut { t: 20.0, s: json!([3, 4]), hold: false, ease: vec![lin, lin], spatial: Some((vec![1.0, 2.0], vec![-1.0 / 3.0, 0.0])) },
        KeyOut { t: 30.0, s: json!([5, 6]), hold: false, ease: vec![lin], spatial: Some((vec![9.0], vec![9.0])) },
    ];
    let list = write_keys(&keys);
    assert_eq!(list[0], json!({"t": 0.0, "s": [5], "o": {"x": [0.25], "y": [0.5]}, "i": {"x": [0.75], "y": [1.0]}}));
    assert_eq!(list[1], json!({"t": 10.0, "s": [1, 2], "h": 1}));
    assert_eq!(list[2]["o"]["x"], json!([0.25, 0.25]));
    assert_eq!((&list[2]["to"], &list[2]["ti"]), (&json!([1.0, 2.0]), &json!([-0.333_333, 0.0])));
    assert_eq!(list[3], json!({"t": 30.0, "s": [5, 6]}), "the last key starts no segment");

    let prop = write_prop(json!(0), &keys, Some("time"));
    assert_eq!((&prop["a"], &prop["x"]), (&json!(1), &json!("time")));
    let PropValue::Keys(back) = read_prop(&prop).value else { panic!("keys") };
    assert_eq!(back.len(), 4);
    assert!(back[1].hold && !back[0].hold);
    assert_eq!(Tangents::read(back[0].o.unwrap(), back[0].i.unwrap(), 0), lin);
    assert_eq!(back[2].to, [1.0, 2.0, 0.0]);

    assert_eq!(write_prop(json!([1, 2]), &[], None), json!({"a": 0, "k": [1, 2]}));
    assert!(write_keys(&[]).is_empty());
}

#[test]
fn fixtures_read() {
    let mut keyed = 0;
    for name in ["bouncing-ball.json", "legacy-masks.json", "precomp-matte-text.json", "shapes-gradient.json"] {
        let doc = fixture(name);
        let h = Header::read(&doc).unwrap();
        assert!(h.width > 0.0 && h.out_point > h.in_point, "{name}");
        let mut all = vec![];
        props(&doc, &mut all);
        assert!(!all.is_empty(), "{name}");
        for p in all {
            match read_prop(p).value {
                PropValue::Keys(keys) => {
                    keyed += 1;
                    assert!(keys.iter().all(|k| k.s.is_some()), "{name}: a key without a value");
                    assert!(keys.windows(2).all(|w| w[0].t <= w[1].t), "{name}: keys out of order");
                }
                PropValue::Static(v) => {
                    assert!(nums(v).is_some() || Bezier::from_json(v, false).is_some_and(|b| !b.vertices.is_empty()), "{name}: {v}")
                }
                PropValue::None => panic!("{name}: a property without a value"),
            }
        }
    }
    assert!(keyed >= 3, "{keyed}");

    // The legacy file: opacity 0 to 100 through an `e`, and a closed four-point mask.
    let doc = fixture("legacy-masks.json");
    let layer = &doc["layers"][0];
    let PropValue::Keys(keys) = read_prop(&layer["ks"]["o"]).value else { panic!("keys") };
    assert_eq!(keys[1].s, Some(&json!([100])));
    assert_eq!(hex_color(layer["sc"].as_str().unwrap()), [224.0 / 255.0, 192.0 / 255.0, 64.0 / 255.0]);
    let PropValue::Static(pt) = read_prop(&layer["masksProperties"][0]["pt"]).value else { panic!("static") };
    let mask = Bezier::from_json(pt, false).unwrap();
    assert!(mask.closed);
    assert_eq!(mask.vertices, [[10.0, 10.0], [110.0, 10.0], [110.0, 110.0], [10.0, 110.0]]);

    // The gradient fill: two colour stops, opaque.
    let doc = fixture("shapes-gradient.json");
    let mut fills = vec![];
    fn find<'a>(j: &'a Json, out: &mut Vec<&'a Json>) {
        match j {
            Json::Object(m) => {
                if m.get("ty").and_then(Json::as_str) == Some("gf") {
                    out.push(j);
                }
                m.values().for_each(|v| find(v, out));
            }
            Json::Array(a) => a.iter().for_each(|v| find(v, out)),
            _ => {}
        }
    }
    find(&doc, &mut fills);
    let g = &fills[0]["g"];
    let PropValue::Static(stops) = read_prop(&g["k"]).value else { panic!("static") };
    let grad = Gradient::from_flat(stops, g["p"].as_u64().unwrap() as usize).unwrap();
    assert_eq!(grad.colors, [(0.0, [0.1, 0.4, 0.9]), (1.0, [0.9, 0.2, 0.5])]);
}

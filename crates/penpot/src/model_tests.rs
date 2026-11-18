//! The typed model on members written by hand in the shape Penpot exports them.
#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::{Value, json};

use super::model::*;
use super::*;

const FILE: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const PAGE: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
const SHAPE: &str = "cccccccc-cccc-cccc-cccc-cccccccccccc";

/// A rectangle as an export writes it: geometry, paints, effects, layout child fields, tokens,
/// and fields this model has no name for.
fn rect() -> Value {
    json!({
        "id": SHAPE, "name": "Card", "type": "rect", "parentId": ROOT_ID, "frameId": ROOT_ID, "pageId": PAGE,
        "x": 10.0, "y": 20.0, "width": 100.0, "height": 50.0, "rotation": 0.0,
        "selrect": {"x": 10.0, "y": 20.0, "width": 100.0, "height": 50.0, "x1": 10.0, "y1": 20.0, "x2": 110.0, "y2": 70.0},
        "points": [{"x": 10.0, "y": 20.0}, {"x": 110.0, "y": 20.0}, {"x": 110.0, "y": 70.0}, {"x": 10.0, "y": 70.0}],
        "transform": {"a": 1.0, "b": 0.0, "c": 0.0, "d": 1.0, "e": 0.0, "f": 0.0},
        "transformInverse": {"a": 1.0, "b": 0.0, "c": 0.0, "d": 1.0, "e": 0.0, "f": 0.0},
        "proportionLock": false, "hidden": false, "blocked": false, "opacity": 0.9, "blendMode": "multiply",
        "fills": [
            {"fillColor": "#ff0000", "fillOpacity": 1.0},
            {"fillColorGradient": {"type": "linear", "startX": 0.5, "startY": 0.0, "endX": 0.5, "endY": 1.0, "width": 1.0,
              "stops": [{"color": "#000000", "opacity": 1.0, "offset": 0.0}, {"color": "#ffffff", "opacity": 0.0, "offset": 1.0}]}},
            {"fillImage": {"id": "dddddddd-dddd-dddd-dddd-dddddddddddd", "width": 2.0, "height": 1.0, "mtype": "image/png", "keepAspectRatio": true}},
        ],
        "strokes": [{"strokeColor": "#000000", "strokeOpacity": 1.0, "strokeWidth": 2.0, "strokeAlignment": "inner", "strokeStyle": "dashed"}],
        "shadow": [{"id": "e1", "style": "drop-shadow", "offsetX": 0.0, "offsetY": 2.0, "blur": 4.0, "spread": 0.0, "hidden": false,
                    "color": {"color": "#000000", "opacity": 0.25}}],
        "blur": {"id": "e2", "type": "layer-blur", "value": 3.0, "hidden": false},
        "r1": 8.0, "r2": 8.0, "r3": 0.0, "r4": 0.0,
        "constraintsH": "leftright", "constraintsV": "top",
        "layoutItemHSizing": "fill", "layoutItemVSizing": "fix", "layoutItemMargin": {"m1": 4, "m2": 0, "m3": 4, "m4": 0},
        "appliedTokens": {"fill": "color.brand", "r1": "radius.md"},
        "interactions": [{"eventType": "click", "actionType": "navigate", "destination": PAGE}],
        "pluginData": {"anything": "kept"},
        "flipX": null,
    })
}

#[test]
fn a_shape_reads_into_the_typed_records() {
    let s: Shape = serde_json::from_value(rect()).unwrap();
    assert_eq!((s.kind.clone(), s.parent_id.as_deref()), (Some(ShapeType::Rect), Some(ROOT_ID)));
    assert_eq!(s.selrect.as_ref().unwrap().x2, Some(110.0));
    assert_eq!(s.points.as_ref().map(Vec::len), Some(4));
    assert_eq!(s.transform, Some(Matrix::default()));
    assert_eq!(s.blend_mode, Some(BlendMode::Multiply));
    let fills = s.fills.as_ref().unwrap();
    assert_eq!(fills[0].fill_color.as_deref(), Some("#ff0000"));
    let g = fills[1].fill_color_gradient.as_ref().unwrap();
    assert_eq!((g.kind.clone(), g.stops.as_ref().map(Vec::len)), (Some(GradientType::Linear), Some(2)));
    assert_eq!(fills[2].fill_image.as_ref().unwrap().keep_aspect_ratio, Some(true));
    assert_eq!(s.strokes.as_ref().unwrap()[0].stroke_style.as_deref(), Some("dashed"));
    assert_eq!(s.shadow.as_ref().unwrap()[0].color.as_ref().unwrap().opacity, Some(0.25));
    assert_eq!(s.blur.as_ref().unwrap().kind.as_deref(), Some("layer-blur"));
    assert_eq!((s.r1, s.r4), (Some(8.0), Some(0.0)));
    assert_eq!(s.applied_tokens.as_ref().unwrap()["fill"], "color.brand");
    assert_eq!(s.layout_item_margin.as_ref().unwrap()["m1"], 4);
    assert_eq!(s.extra["pluginData"]["anything"], "kept");
    assert_eq!(s.flip_x, None);
}

#[test]
fn nothing_but_nulls_is_lost_between_reading_and_writing() {
    let original = rect();
    let s: Shape = serde_json::from_value(original.clone()).unwrap();
    let mut want = original;
    want.as_object_mut().unwrap().remove("flipX");
    assert_eq!(serde_json::to_value(&s).unwrap(), want);
    // Values this model doesn't know stay as they were.
    let odd = json!({"type": "hologram", "blendMode": "plus-darker", "fills": [{"fillColorGradient": {"type": "conic"}}]});
    let s: Shape = serde_json::from_value(odd.clone()).unwrap();
    assert_eq!(s.kind, Some(ShapeType::Other("hologram".into())));
    assert_eq!(serde_json::to_value(&s).unwrap(), odd);
}

#[test]
fn pages_libraries_and_the_manifest_read() {
    let page: Page = serde_json::from_value(json!({"id": PAGE, "name": "Canvas", "index": 0, "background": "#f0f0f0",
        "guides": {"g1": {"id": "g1", "axis": "x", "position": 120.0}},
        "flows": {"f1": {"id": "f1", "name": "Flow 1", "startingFrame": SHAPE}}}))
    .unwrap();
    assert_eq!(page.guides.as_ref().unwrap()["g1"].axis.as_deref(), Some("x"));
    assert_eq!(page.flows.as_ref().unwrap()["f1"].starting_frame.as_deref(), Some(SHAPE));
    // Typography sizes come as numbers or as strings.
    let t: Typography =
        serde_json::from_value(json!({"id": "t", "name": "Body", "fontFamily": "Inter", "fontSize": "14", "fontWeight": 400, "lineHeight": "1.2"}))
            .unwrap();
    assert_eq!((t.font_size, t.font_weight), (Some(json!("14")), Some(json!(400))));
    let m: Manifest = serde_json::from_value(manifest("craft-penpot/test", &[(FILE, "File", json!(["components/v2"]))])).unwrap();
    assert_eq!((m.kind.as_deref(), m.version), (Some(MANIFEST_TYPE), Some(1.0)));
    assert_eq!(m.files.as_ref().unwrap()[0].features.as_deref(), Some(&["components/v2".to_string()][..]));
    let c: Component = serde_json::from_value(json!({"id": "k", "name": "Button", "mainInstanceId": SHAPE, "mainInstancePage": PAGE})).unwrap();
    assert_eq!(c.main_instance_id.as_deref(), Some(SHAPE));
    let media: Media =
        serde_json::from_value(json!({"id": "m", "mediaId": "o", "mtype": "image/png", "width": 2, "height": 1, "isLocal": true})).unwrap();
    assert_eq!((media.media_id.as_deref(), media.width), (Some("o"), Some(2.0)));
}

#[test]
fn a_field_of_the_wrong_type_fails_the_read_and_says_which() {
    for (bad, want) in [
        (json!({"x": "ten"}), "f64"),
        (json!({"shapes": "abc"}), "sequence"),
        (json!({"fills": [{"fillOpacity": []}]}), "f64"),
        (json!({"transform": {"a": 1}}), "missing field"),
        (json!({"appliedTokens": {"fill": 3}}), "string"),
    ] {
        let e = serde_json::from_value::<Shape>(bad).unwrap_err().to_string();
        assert!(e.contains(want), "{e}");
    }
}

#[test]
fn the_archive_reads_and_writes_typed_members() {
    let shape: Shape = serde_json::from_value(rect()).unwrap();
    let root = Shape { id: Some(ROOT_ID.into()), kind: Some(ShapeType::Frame), shapes: Some(vec![SHAPE.into()]), ..Shape::default() };
    let page = Page { id: Some(PAGE.into()), name: Some("Canvas".into()), index: Some(0.0), ..Page::default() };
    let members: Vec<(String, Vec<u8>)> = vec![
        ("manifest.json".into(), serde_json::to_vec(&manifest("craft-penpot/test", &[(FILE, "File", json!([]))])).unwrap()),
        (format!("files/{FILE}/pages/{PAGE}.json"), serde_json::to_vec(&page).unwrap()),
        (format!("files/{FILE}/pages/{PAGE}/{ROOT_ID}.json"), serde_json::to_vec(&root).unwrap()),
        (format!("files/{FILE}/pages/{PAGE}/{SHAPE}.json"), serde_json::to_vec(&shape).unwrap()),
        ("bad.json".into(), br#"{"x": "ten"}"#.to_vec()),
    ];
    let bytes = write_archive(members.iter().map(|(p, b)| (p.as_str(), b.as_slice()))).unwrap();
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(a.typed::<Page>(&format!("files/{FILE}/pages/{PAGE}.json")).unwrap(), page);
    assert_eq!(a.typed::<Shape>(&format!("files/{FILE}/pages/{PAGE}/{SHAPE}.json")).unwrap(), shape);
    let m: Manifest = a.typed("manifest.json").unwrap();
    assert_eq!(m.files.unwrap()[0].id.as_deref(), Some(FILE));
    // Within the archive's budgets, and failing on a wrong type with the member's name.
    let e = a.typed::<Shape>("bad.json").unwrap_err().to_string();
    assert!(e.starts_with("Penpot: bad.json: "), "{e}");
    assert!(a.typed::<Shape>("missing.json").is_err());
    assert!(a.check().is_ok());
}

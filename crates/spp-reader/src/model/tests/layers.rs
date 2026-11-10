//! Layer kinds, nesting, ordering, blending and robustness of the builder.

use super::doc::*;
use crate::model::*;

fn build(bytes: &[u8]) -> (ProjectDocument, ModelReport) {
    ProjectDocument::from_bytes(bytes).expect("model")
}

#[test]
fn reads_the_texture_set() {
    let (doc, report) = build(&single_set(vec![]));
    assert_eq!(doc.format_version, 78);
    let set = &doc.texture_sets[0];
    assert_eq!((set.name.as_str(), set.material_uid, set.uid), ("Vase", Some(77), Some(2)));
    assert_eq!(set.channels[0].channel, ProjectChannel::BaseColor);
    assert!(set.layers.is_empty());
    assert_eq!(report.raw(), 0);
    assert_eq!(report.not_modelled(), 0);
}

#[test]
fn layer_kinds_follow_the_first_content_effect() {
    let paint_group = obj(
        "DataActionGroup",
        vec![
            ("enabled", V::Bool(true)),
            ("perChannelBlending", V::List(vec![blending(u64::MAX, 25, 1.0)])),
            ("subStack", stack("DataStackActions", 41, vec![])),
        ],
    );
    let (doc, _) = build(&single_set(vec![
        layer(10, "Paint", vec![paint_group], vec![], None),
        layer(20, "Fill", vec![fill(25, vec![uniform(1, [0.5; 3])])], vec![], None),
        layer(30, "Empty", vec![], vec![], None),
    ]));
    let layers = &doc.texture_sets[0].layers;
    let kinds: Vec<&str> = layers.iter().map(|l| l.kind.name()).collect();
    assert_eq!(kinds, ["paint", "fill", "paint"]);
    assert_eq!(layers[0].name, "Paint", "stored order (bottom → top) is kept");
    let ProjectEffect::Group(g) = &layers[0].content[0] else { panic!("group") };
    assert_eq!(g.header.blending[0].mode, ProjectBlendMode::Passthrough);
    assert!(layers[1].mask.is_none());
}

#[test]
fn folders_nest_with_their_children() {
    let inner = folder(100, "Inner", vec![layer(110, "Deep", vec![], vec![], None)]);
    let outer = folder(200, "Outer", vec![layer(210, "A", vec![], vec![], None), inner]);
    let (doc, _) = build(&single_set(vec![outer]));
    let set = &doc.texture_sets[0];
    let outer = &set.layers[0];
    assert!(!outer.visible);
    assert_eq!(outer.children().len(), 2);
    assert_eq!(outer.children()[1].children()[0].name, "Deep");
    let names: Vec<&str> = set.all_layers().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Outer", "A", "Inner", "Deep"]);
    let summary = StackSummary::of(&doc);
    assert_eq!((summary.layers, summary.folders, summary.paint_layers), (4, 2, 2));
}

#[test]
fn blending_entries_keep_modes_opacity_and_masks() {
    let blend = vec![blending(1 | 16777216, 4, 0.5), blending(2, 11, 1.0), blending(4194304, 27, 1.0)];
    let (doc, _) = build(&single_set(vec![layer(10, "L", vec![], blend, None)]));
    let b = &doc.texture_sets[0].layers[0].blending;
    assert_eq!(b.len(), 3);
    assert_eq!(b[0].mode, ProjectBlendMode::Multiply);
    assert_eq!(b[0].opacity, 0.5);
    assert!(b[0].channels.contains(ProjectChannel::Diffuse));
    assert_eq!(b[1].mode, ProjectBlendMode::Overlay);
    assert!(!b[1].is_default());
    assert!(b[2].is_default());
    assert_eq!(blending_for(b, ProjectChannel::Height).map(|e| e.mode), Some(ProjectBlendMode::Overlay));
}

#[test]
fn unknown_classes_are_kept_raw_and_counted() {
    let future_layer = obj("DataLayerInstance", vec![("label", V::Str("I")), ("uid", V::Uid(5))]);
    let future_effect = obj("DataActionHologram", vec![("uid", V::Uid(6))]);
    let future_source = obj("DataSourceVideo", vec![("uid", V::Uid(7))]);
    let (doc, report) = build(&single_set(vec![future_layer, layer(10, "L", vec![future_effect, fill(20, vec![future_source])], vec![], None)]));
    let layers = &doc.texture_sets[0].layers;
    assert_eq!(layers[0].kind, LayerKind::Other("DataLayerInstance".into()));
    assert_eq!(layers[0].name, "I");
    let ProjectEffect::Other(raw) = &layers[1].content[0] else { panic!("raw effect") };
    assert_eq!((raw.class.as_str(), raw.uid), ("DataActionHologram", Some(6)));
    let ProjectEffect::Fill(f) = &layers[1].content[1] else { panic!("fill") };
    assert!(matches!(&f.sources[0], ProjectSource::Other(r) if r.class == "DataSourceVideo"));
    assert_eq!(report.raw(), 3);
    let raw_classes: Vec<&str> = report.raw_classes().iter().map(|(c, _)| *c).collect();
    assert_eq!(raw_classes, ["DataActionHologram", "DataLayerInstance", "DataSourceVideo"]);
}

#[test]
fn another_root_class_gives_an_empty_model() {
    let bytes = document(2, obj("DataLayersStackState", vec![("uid", V::Uid(1))]));
    let (doc, report) = build(&bytes);
    assert!(doc.texture_sets.is_empty());
    assert_eq!(report.raw(), 1);
}

#[test]
fn deep_folder_nesting_is_cut_not_followed() {
    let mut v = layer(1, "bottom", vec![], vec![], None);
    for _ in 0..70 {
        v = folder(9, "f", vec![v]);
    }
    let (doc, report) = build(&single_set(vec![v]));
    assert_eq!(report.depth_limited, 1);
    let deepest = doc.texture_sets[0].all_layers().len();
    assert_eq!(deepest, 65, "64 nested folders plus the one kept raw");
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let bytes = single_set(vec![layer(10, "L", vec![fill(20, vec![uniform(1, [0.1, 0.2, 0.3])])], vec![], None)]);
    for len in 0..bytes.len() {
        assert!(ProjectDocument::from_bytes(&bytes[..len]).is_err(), "length {len}");
    }
    assert!(ProjectDocument::from_bytes(&bytes).is_ok());
}

/// A texture set in the compact layout: channels by index, blending as a value list, mask as a wide value.
#[test]
fn compact_layout_builds_the_same_model() {
    use crate::decode::document::test_writer::CompactWriter;
    let mut w = CompactWriter::new(92, 7);
    w.object("DataDocument", &[("materials", 0x13)]).u32(1);
    w.object("DataMaterial", &[("sceneMaterialName", 0x10), ("resolutionLog2", 0x06), ("stacks", 0x13)]);
    w.string("Set").u32(9).u32(9).u32(1);
    w.object("DataMaterialStack", &[("channels", 0x13), ("stack", 0x12)]).u32(2);
    let channel: &[(&str, u32)] = &[("format", 0x09), ("type", 0x09), ("userName", 0x10)];
    w.object("DataChannel", channel).u32(1).u32(7).string("").close();
    w.object("DataChannel", channel).u32(6).u32(64).string("mine").close();
    w.object("DataStackLayers", &[("items", 0x13)]).u32(1);
    w.object("DataLayerColor", &[("label", 0x10), ("perChannelBlending", 0x11), ("enabled", 0x0a)]);
    w.string("Top");
    w.value_list(1, "DataBlending", &[("blendingMode", 0x09), ("channelTypes", 0x16), ("opacity", 0x01)]);
    w.u32(11).raw(&128u128.to_le_bytes()).raw(&0.5f32.to_le_bytes());
    w.raw(&[0xff]);
    w.close().close().close().close().close();
    let (doc, report) = crate::model::ProjectDocument::from_bytes(&w.bytes).unwrap();
    assert_eq!(doc.format_version, 92);
    assert_eq!(report.unknown_tags, 0);
    let set = &doc.texture_sets[0];
    assert_eq!(set.name, "Set");
    assert_eq!(set.resolution_log2, Some([9, 9]));
    assert_eq!(set.channels[0].channel, crate::model::ProjectChannel::Roughness);
    assert_eq!(set.channels[1].channel.bit(), 1 << 14);
    assert_eq!(set.channels[1].user_name, "mine");
    let layer = &set.layers[0];
    assert_eq!(layer.name, "Top");
    let blend = &layer.blending[0];
    assert!(blend.channels.contains(crate::model::ProjectChannel::Roughness));
    assert_eq!(blend.mode.raw(), 11);
    assert_eq!(blend.opacity, 0.5);
    let scanned = crate::decode::document::DocumentMaterials::scan(&w.bytes).unwrap();
    assert_eq!(scanned[0].channels[0].type_bit, 128);
    assert_eq!(scanned[0].channels[1].format, 6);
}

/// Malformed compact documents reach the model builder without a panic.
#[test]
fn compact_layout_corruption_never_panics_the_model() {
    let data = crate::decode::document::test_writer::compact_sample();
    for at in 12..data.len() {
        for byte in [0x00, 0x13, 0x7f, 0xff] {
            let mut bad = data.clone();
            bad[at] = byte;
            let _ = crate::model::ProjectDocument::from_bytes(&bad);
        }
    }
}

/// Stroke key rows are packed: their enum field must take the same room as in the tagged layout.
#[test]
fn compact_stroke_keys_are_read() {
    use crate::decode::document::test_writer::CompactWriter;
    use crate::model::raw::RawTree;
    let key: &[(&str, u32)] = &[("type", 0x09), ("value", 0x01), ("x", 0x01)];
    let mut w = CompactWriter::new(92, 2);
    w.object("DataStroke", &[("pressure", 0x11)]);
    w.value_list(2, "DataTimelineKey", key);
    for (value, x) in [(0.25f32, 0.0f32), (0.5, 1.0)] {
        w.u32(0).raw(&value.to_le_bytes()).raw(&x.to_le_bytes());
    }
    w.close();
    let tree = RawTree::parse(&w.bytes).unwrap();
    let stroke = tree.root.unwrap();
    let rows: Vec<_> = stroke.packed("pressure").unwrap().rows().collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].f32("value"), Some(0.5));
    assert_eq!(rows[1].f32("x"), Some(1.0));
}

#[test]
fn a_channel_stored_as_an_enum_is_the_bit_before_the_compact_layout() {
    let channel = |ty: u64| obj("DataChannel", vec![("type", V::Enum(ty)), ("format", V::Enum(0))]);
    let stack = obj("DataMaterialStack", vec![("channels", V::List(vec![channel(128), channel(1), channel(0)]))]);
    let material = obj("DataMaterial", vec![("stacks", V::List(vec![stack]))]);
    let root = obj("DataDocument", vec![("materials", V::List(vec![material]))]);
    let (doc, _) = ProjectDocument::from_bytes(&document(55, root)).expect("model");
    let declared: Vec<ProjectChannel> = doc.texture_sets[0].channels.iter().map(|c| c.channel).collect();
    assert_eq!(declared, [ProjectChannel::Roughness, ProjectChannel::BaseColor, ProjectChannel::Bit(0)]);
}

#[test]
fn geometry_masks_keep_the_mesh_list() {
    let mut eyes = folder(300, "Eyes", vec![layer(310, "Base Metal", vec![], vec![], None)]);
    let box_of = |name: &'static str| obj("DataByteArrayBox", vec![("value", V::Str(name))]);
    if let V::Obj(_, fields) = &mut eyes {
        fields.push(("enabledGeometryMask", V::Bool(true)));
        fields.push(("enabledMeshDefault", V::Bool(true)));
        fields.push(("enabledMeshList", V::List(vec![box_of("Body_low"), box_of("Fin_low")])));
        fields.push(("enabledUVTileDefault", V::Bool(true)));
        fields.push(("enabledUVTileList", V::List(vec![])));
        fields.push(("geometryMaskType", V::Enum(1)));
    }
    let (doc, _) = build(&single_set(vec![eyes]));
    let layers = &doc.texture_sets[0].layers;
    let g = layers[0].geometry.as_ref().expect("a geometry mask");
    assert_eq!(g.kind, GeometryMaskKind::Meshes);
    assert_eq!(g.mesh_list, ["Body_low", "Fin_low"]);
    assert!(g.restricts() && g.includes_mesh("Eyes_low") && !g.includes_mesh("Body_low"));
    assert!(layers[0].children()[0].geometry.is_none(), "no fields, no mask");
}

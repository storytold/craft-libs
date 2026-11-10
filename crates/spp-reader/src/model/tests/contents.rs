//! Layer contents and masks: uniform values, bitmaps, procedurals with tweaks, strokes, filters.

use super::doc::*;
use crate::format_names::PACKAGE_SUFFIX;
use crate::model::*;

fn only_layer(content: Vec<V>, mask: Option<Vec<V>>) -> (ProjectLayer, ModelReport) {
    let bytes = single_set(vec![layer(10, "L", content, vec![], mask)]);
    let (mut doc, report) = ProjectDocument::from_bytes(&bytes).expect("model");
    (doc.texture_sets.remove(0).layers.remove(0), report)
}

fn procedural_source(channels: u64, url: &'static str, tweaks: Vec<V>) -> V {
    let procedural = obj(
        "DataProcedural",
        vec![
            ("outputs", V::List(vec![obj("DataProceduralOutput", vec![("channelTypes", V::Uid(1)), ("identifier", V::Str("basecolor"))])])),
            ("tweaks", V::List(tweaks)),
            ("urlToSbsRes", V::Str(url)),
        ],
    );
    obj("DataSourceProcedural", vec![("channelTypes", V::UInt(channels as u32)), ("mode", V::Enum(1)), ("procedural", procedural)])
}

#[test]
fn uniform_values_are_kept_per_channel_as_stored() {
    let sources = vec![uniform(1, [0.27583286, 0.288816, 0.30663478]), uniform(128, [0.3; 3]), uniform(4194304, [0.5, 0.5, 1.0])];
    let (layer, _) = only_layer(vec![fill(20, sources)], None);
    assert_eq!(layer.kind, LayerKind::Fill);
    let ProjectEffect::Fill(f) = &layer.content[0] else { panic!("fill") };
    assert!(f.is_uniform());
    let values: Vec<_> = f.uniform_values().collect();
    assert_eq!(values[0], (ProjectChannel::BaseColor, [0.27583286, 0.288816, 0.30663478]));
    assert_eq!(values[1], (ProjectChannel::Roughness, [0.3; 3]));
    assert_eq!(values[2].0, ProjectChannel::Normal);
    assert_eq!(ContentKind::of(&layer.content), ContentKind::Uniform);
}

#[test]
fn bitmap_and_procedural_sources_keep_resources_and_tweaks() {
    let bitmap =
        obj("DataSourceBitmap", vec![("bitmap", obj("DataBitmap", vec![("urlToBitmapRes", V::Str("/Moth?version=abc"))])), ("channelTypes", V::Uid(2))]);
    let tweaks = vec![
        obj("DataTweakFloat3", vec![("identifier", V::Str("color")), ("value", V::Vec3([0.5, 0.25, 0.0]))]),
        obj("DataTweakInt", vec![("identifier", V::Str("$randomseed")), ("value", V::Int(-3))]),
        obj("DataTweakMystery", vec![("identifier", V::Str("x"))]),
    ];
    let procedural = procedural_source(1 | 128, format!("resource://library/concrete/concrete?version=1{PACKAGE_SUFFIX}").leak(), tweaks);
    let (layer, report) = only_layer(vec![fill(20, vec![bitmap]), fill(30, vec![procedural])], None);
    let ProjectEffect::Fill(f) = &layer.content[0] else { panic!("fill") };
    let ProjectSource::Bitmap(b) = &f.sources[0] else { panic!("bitmap") };
    assert_eq!((b.url.as_str(), b.channels), ("/Moth?version=abc", ChannelMask(2)));
    let ProjectEffect::Fill(f) = &layer.content[1] else { panic!("fill") };
    let ProjectSource::Procedural(p) = &f.sources[0] else { panic!("procedural") };
    let proc = p.procedural.as_ref().expect("procedural");
    assert_eq!(proc.name(), "concrete");
    assert_eq!(proc.outputs[0].channels, ChannelMask(1));
    assert_eq!(proc.tweaks[0].value, TweakValue::Floats(vec![0.5, 0.25, 0.0]));
    assert_eq!(proc.tweaks[1].value, TweakValue::Int(-3));
    assert_eq!(proc.tweaks[2].value, TweakValue::Other("DataTweakMystery".into()));
    assert_eq!(report.raw(), 1);
    assert_eq!(ContentKind::of(&layer.content), ContentKind::Mixed, "bitmap and procedural");
}

#[test]
fn references_point_at_anchors() {
    let anchor = obj("DataActionAnchor", vec![("enabled", V::Bool(true)), ("label", V::Str("Edges")), ("uid", V::Uid(90))]);
    let reference = obj(
        "DataSourceReference",
        vec![
            ("channelTypes", V::Uid(0)),
            ("levels", obj("DataLevels", vec![("gamma", V::Vec3([2.0; 3]))])),
            ("reference", obj("DataReference", vec![("source", V::Ref(90)), ("sourceLabel", V::Str("Edges"))])),
        ],
    );
    let (layer, report) = only_layer(vec![anchor], Some(vec![fill(20, vec![reference])]));
    let ProjectEffect::Anchor(a) = &layer.content[0] else { panic!("anchor") };
    assert_eq!((a.label.as_str(), a.uid), ("Edges", Some(90)));
    let ProjectEffect::Fill(f) = &layer.mask.as_ref().expect("mask").effects[0] else { panic!("fill") };
    let ProjectSource::Reference(r) = &f.sources[0] else { panic!("reference") };
    assert_eq!((r.anchor_uid, r.anchor_label.as_str()), (Some(90), "Edges"));
    assert_eq!(report.not_modelled(), 0);
}

#[test]
fn masks_hold_generators_strokes_and_filters() {
    let stroke = obj("DataStroke", vec![("points", V::List(vec![]))]);
    let paint = obj(
        "DataActionPaint",
        vec![("enabled", V::Bool(true)), ("opacity", V::Float(0.5)), ("sourceColor", V::List(vec![uniform(0, [1.0; 3])])), ("strokes", V::List(vec![stroke]))],
    );
    let group = obj("DataActionGroup", vec![("enabled", V::Bool(true)), ("subStack", stack("DataStackActions", 50, vec![paint]))]);
    let generator = fill(60, vec![procedural_source(0, format!("resource://generators/mg_dirt?version=1{PACKAGE_SUFFIX}").leak(), vec![])]);
    let filter = obj("DataActionFilterProcedural", vec![("enabled", V::Bool(false)), ("procedural", V::Absent)]);
    let (layer, report) = only_layer(vec![], Some(vec![group, generator, filter]));
    let mask = layer.mask.as_ref().expect("mask");
    assert_eq!(mask.initial, MaskInitial::Black);
    assert!(mask.enabled);
    let ProjectEffect::Group(g) = &mask.effects[0] else { panic!("group") };
    let ProjectEffect::Paint(p) = &g.effects[0] else { panic!("paint") };
    assert_eq!((p.tool, p.stroke_count(), p.opacity), (PaintTool::Brush, 1, 0.5));
    assert!(p.colors[0].channels().is_empty(), "mask channel");
    let ProjectEffect::Filter(f) = &mask.effects[2] else { panic!("filter") };
    assert!(!f.header.enabled && f.procedural.is_none());
    assert_eq!(ContentKind::of(&mask.effects), ContentKind::Mixed);
    assert_eq!(report.classes["DataStroke"].recognised, 1);
    assert_eq!(report.raw(), 0);
}

#[test]
fn strokes_are_read_with_points_pressure_and_camera() {
    let key = |x: f32, t: f32| V::Inline("DataStrokeKey", vec![("position", V::Floats(0x02, vec![x, -x])), ("time", V::Float(t)), ("type", V::Enum(0))]);
    let pressure = |t: f32, v: f32| V::Inline("DataTimelineKey", vec![("type", V::Enum(0)), ("value", V::Float(v)), ("x", V::Float(t))]);
    let view: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let stroke = obj(
        "DataStroke",
        vec![
            ("duplicateCount", V::UInt(1)),
            ("duplicateMatrix", V::Floats(0x0e, vec![-1.0; 16])),
            ("points", V::Values(vec![key(0.25, 0.0), key(0.5, 0.1), key(0.75, 0.2)])),
            ("pressure", V::Values(vec![pressure(0.0, 0.2), pressure(0.2, 0.6)])),
            ("projectionMatrix", V::Floats(0x0e, vec![2.0; 16])),
            ("randomSeed", V::UInt(4_000_000_000)),
            ("tilt", V::Values(vec![])),
            ("uid", V::Uid(77)),
            ("viewMatrix", V::Floats(0x0e, view.clone())),
            ("viewType", V::Enum(0)),
        ],
    );
    let brush = obj("DataBrush", vec![("uid", V::Uid(12))]);
    let paint = obj(
        "DataActionEraser",
        vec![
            ("brush", brush),
            ("strokes", V::List(vec![stroke, obj("DataStroke", vec![])])),
            ("strokes3D", V::List(vec![obj("DataStroke3D", vec![("points", V::Values(vec![]))])])),
        ],
    );
    let (layer, report) = only_layer(vec![paint], None);
    assert_eq!(layer.kind, LayerKind::Paint);
    let ProjectEffect::Paint(p) = &layer.content[0] else { panic!("paint") };
    assert_eq!((p.tool, p.stroke_count(), p.path_strokes(), p.brush_uid), (PaintTool::Eraser, 3, 1, Some(12)));
    let s = &p.strokes[0];
    assert_eq!(s.uid, Some(77));
    assert_eq!(s.points, [(0.25, 0.0), (0.5, 0.1), (0.75, 0.2)].map(|(x, t)| StrokeKey { position: [x, -x], time: t }));
    assert_eq!(s.pressure, [TimelineKey { time: 0.0, value: 0.2 }, TimelineKey { time: 0.2, value: 0.6 }]);
    assert_eq!((s.tilt_keys, s.complete, s.view), (0, true, StrokeView::Uv));
    assert_eq!(s.view_matrix.to_vec(), view);
    assert_eq!((s.projection_matrix[15], s.duplicate_matrix[0]), (2.0, -1.0));
    assert_eq!((s.duplicate_count, s.random_seed), (1, 4_000_000_000));
    let empty = &p.strokes[1];
    assert!(empty.points.is_empty() && empty.complete && empty.view == StrokeView::Mesh);
    assert_eq!(empty.view_matrix, [0.0; 16]);
    assert_eq!(report.classes["DataStrokeKey"].total, 3);
    assert_eq!(report.classes["DataStrokeKey"].recognised, 3);
    assert_eq!(report.classes["DataTimelineKey"].not_modelled(), 0);
    assert_eq!(report.classes["DataStroke"].recognised, 2);
    assert_eq!(report.classes["DataStroke3D"].recognised, 1);
    assert_eq!(ContentKind::of(&layer.content), ContentKind::Strokes);
}

#[test]
fn path_strokes_are_read_as_bezier_splines_in_both_formats() {
    // format 92: size and opacity per key; type 1 anchor, 3 out-handle, 2 in-handle
    let key = |t: u64, p: [f32; 3], w: f32| {
        V::Inline(
            "DataStroke3DKey",
            vec![
                ("normal", V::Vec3(if t == 1 { [0.0, 0.0, 1.0] } else { [0.0; 3] })),
                ("opacity", V::Float(w)),
                ("position", V::Vec3(p)),
                ("size", V::Float(w * 0.5)),
                ("type", V::Enum(t)),
            ],
        )
    };
    // before 92: one pressure
    let old = |t: u64, p: [f32; 3], w: f32| {
        V::Inline("DataStroke3DKey", vec![("normal", V::Vec3([0.0; 3])), ("position", V::Vec3(p)), ("pressure", V::Float(w)), ("type", V::Enum(t))])
    };
    let path = |keys: Vec<V>, closed: bool| {
        obj(
            "DataStroke3D",
            vec![
                ("duplicateCount", V::UInt(0)),
                ("isClosed", V::Bool(closed)),
                ("label", V::Str("Path 1")),
                ("points", V::Values(keys)),
                ("randomSeed", V::UInt(123)),
                ("uid", V::Uid(305)),
            ],
        )
    };
    let new = path(vec![key(1, [-0.3, 0.0, 0.0], 1.0), key(3, [-0.1, 0.0, 0.0], 0.0), key(2, [0.1, 0.0, 0.0], 0.0), key(1, [0.3, 0.0, 0.0], 1.0)], false);
    let older = path(vec![old(1, [0.0; 3], 0.5), old(1, [0.0, 1.0, 0.0], 1.0)], true);
    let paint = obj("DataActionPaint", vec![("strokes", V::List(vec![])), ("strokes3D", V::List(vec![new, older]))]);
    let (layer, report) = only_layer(vec![paint], None);
    let ProjectEffect::Paint(p) = &layer.content[0] else { panic!("paint") };
    assert_eq!(p.path_strokes(), 2);
    let a = &p.paths[0];
    assert_eq!((a.uid, a.label.as_str(), a.closed, a.random_seed, a.complete), (Some(305), "Path 1", false, 123, true));
    assert_eq!(a.points.iter().map(|k| k.kind).collect::<Vec<_>>(), [PathKeyKind::Anchor, PathKeyKind::Out, PathKeyKind::In, PathKeyKind::Anchor]);
    assert_eq!((a.points[0].size, a.points[0].opacity, a.points[0].normal), (0.5, 1.0, [0.0, 0.0, 1.0]));
    let segments = a.segments();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].at(0.5), [0.0, 0.0, 0.0]);
    assert_eq!(segments[0].at(1.0), [0.3, 0.0, 0.0]);
    let b = &p.paths[1];
    assert!(b.closed);
    assert_eq!((b.points[0].size, b.points[0].opacity), (0.5, 0.5));
    // no handles: a straight piece
    let q = b.segments()[0].at(0.25);
    assert!(q[0] == 0.0 && q[2] == 0.0 && q[1] > 0.0 && q[1] < 1.0);
    assert_eq!(report.classes["DataStroke3DKey"].recognised, 6);
}

#[test]
fn unexpected_point_layouts_keep_what_was_read() {
    let good = V::Inline("DataStrokeKey", vec![("position", V::Floats(0x02, vec![1.0, 2.0])), ("time", V::Float(0.5))]);
    let nested = V::Inline("DataStrokeKey", vec![("position", V::Inline("Odd", vec![("x", V::Float(1.0))]))]);
    let stroke = obj("DataStroke", vec![("points", V::Values(vec![good, nested]))]);
    let paint = obj("DataActionPaint", vec![("strokes", V::List(vec![stroke]))]);
    let (layer, report) = only_layer(vec![paint], None);
    let ProjectEffect::Paint(p) = &layer.content[0] else { panic!("paint") };
    assert_eq!(p.strokes[0].points.len(), 1);
    assert!(!p.strokes[0].complete);
    assert_eq!(report.classes["DataStrokeKey"].total, 2);
    assert_eq!(report.classes["Odd"].total, 1);
}

#[test]
fn particle_brushes_keep_their_emitter_fields() {
    let tweak = |id: &'static str, v: f32| obj("DataTweakFloat", vec![("identifier", V::Str(id)), ("value", V::Float(v))]);
    let emitter = obj(
        "DataEmitterPhysics",
        vec![
            ("deltaTime", V::Float(0.02)),
            ("source", V::Enum(1)),
            ("time", V::Float(1.5)),
            ("tweaksEmitter", V::List(vec![tweak("SpawnSpeed", 10.0)])),
            ("tweaksReceiver", V::List(vec![tweak("ParticleLife", 4.0), tweak("Spread", 0.5)])),
            ("urlEmitter", V::Str("resource://a/EmitSingle")),
            ("urlReceiver", V::Str("resource://a/RcvLaser")),
        ],
    );
    let brush = obj("DataBrush", vec![("stampsEmitter", emitter)]);
    let paint = obj("DataActionPaint", vec![("brush", brush)]);
    let (layer, report) = only_layer(vec![paint], None);
    let ProjectEffect::Paint(p) = &layer.content[0] else { panic!("not paint") };
    let StampEmitter::Physics(e) = &p.brush.as_ref().expect("brush").emitter else { panic!("not physics") };
    assert_eq!((e.emitter.as_str(), e.receiver.as_str()), ("resource://a/EmitSingle", "resource://a/RcvLaser"));
    assert_eq!((e.delta_time, e.time, e.source), (Some(0.02), Some(1.5), Some(1)));
    assert_eq!(e.emitter_tweaks.len(), 1);
    assert_eq!(e.receiver_tweaks[0].identifier, "ParticleLife");
    assert_eq!(e.receiver_tweaks[1].value, TweakValue::Floats(vec![0.5]));
    assert_eq!(report.classes["DataEmitterPhysics"].not_modelled(), 0);
}

//! Round trip: a v2 document with N:M layer membership saves and reloads unchanged.

use craft_stencilcraft_document::{Document, FORMAT_VERSION, Layer, Operation, OperationSettings};
use kurbo::BezPath;

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x, y + h));
    p.close_path();
    p
}

#[test]
fn v2_round_trip_keeps_one_geometry_shared_by_two_layers() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document {
        layers: vec![
            Layer::new("cut", Some(Operation::Cut(OperationSettings::default()))),
            Layer::new("engrave", Some(Operation::Engrave(OperationSettings::default()))),
            Layer::new("guide", Some(Operation::Guide)),
        ],
        ..Document::default()
    };
    let first = doc.push_shape(0, rect_path(0.0, 0.0, 10.0, 10.0))?;
    let second = doc.push_shape(0, rect_path(20.0, 0.0, 10.0, 10.0))?;
    // N:M membership: the first cut shape is also engraved, the second is the guide's member.
    doc.layers[1].members.push(first.id);
    doc.layers[2].members.push(second.id);

    let json = doc.save_json()?;
    let header: serde_json::Value = serde_json::from_str(&json)?;
    assert_eq!(header["stencilcraft"]["version"].as_u64(), Some(FORMAT_VERSION), "save_json writes the v2 header");

    let back = Document::load_json(&json)?;
    assert_eq!(back.shapes, doc.shapes, "shapes survive the round trip");
    assert_eq!(back.layers, doc.layers, "per-layer members survive the round trip");
    assert_eq!(back.shapes.len(), 2, "the shared geometry is stored once in shapes");
    assert_eq!(back.layers[1].members, vec![1], "shape 1 is shared with the engrave layer");
    let guide = back.layers.iter().find(|l| l.operation == Some(Operation::Guide)).expect("the Guide layer is present");
    assert_eq!(guide.members, vec![2], "the Guide layer keeps its member");
    Ok(())
}

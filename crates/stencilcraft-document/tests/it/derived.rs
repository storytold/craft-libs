//! Derived queries: unassigned shapes, production order over operations, layer bounds,
//! layer deletion that preserves geometry, and membership pruning on load
//! (D-03/D-04/D-17/D-19, MODEL-07).

use craft_stencilcraft_document::{DocError, Document, Layer, Operation, OperationKind, OperationSettings};
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

fn op(id: u64, name: &str, kind: OperationKind, members: Vec<u64>) -> Operation {
    Operation {
        id,
        name: name.to_string(),
        kind,
        settings: OperationSettings::default(),
        color: None,
        visible: true,
        locked: false,
        members,
        icon: None,
    }
}

/// Layers: 0 cut, 1 engrave, 2 guide, 3 plain container. Operations: cut (members 1, 2),
/// engrave (empty), guide (empty). Shapes 1 and 2 are pushed onto the cut layer; shape 3
/// is a member of no layer at all.
fn fixture() -> Result<Document, Box<dyn std::error::Error>> {
    let mut doc =
        Document { layers: vec![Layer::new("cut"), Layer::new("engrave"), Layer::new("guide"), Layer::new("plain")], ..Document::default() };
    doc.operations = vec![
        op(1, "cut", OperationKind::Cut, vec![1, 2]),
        op(2, "engrave", OperationKind::Engrave, vec![]),
        op(3, "guide", OperationKind::Guide, vec![]),
    ];
    doc.push_shape(0, rect_path(0.0, 0.0, 10.0, 10.0))?;
    doc.push_shape(0, rect_path(20.0, 0.0, 10.0, 10.0))?;
    doc.shapes.push(craft_stencilcraft_document::Shape {
        id: 3,
        path: rect_path(40.0, 0.0, 5.0, 5.0),
        hidden: false,
        locked: false,
        text: None,
        production: Default::default(),
    });
    doc.next_shape_id = 4;
    Ok(doc)
}

#[test]
fn shape_in_no_operation_is_unassigned() -> Result<(), Box<dyn std::error::Error>> {
    let doc = fixture()?;
    let unassigned: Vec<u64> = doc.unassigned_shapes().map(|s| s.id).collect();
    assert_eq!(unassigned, vec![3], "only the operation-less shape is unassigned");
    Ok(())
}

#[test]
fn multi_operation_member_is_assigned_and_appears_in_both_passes() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = fixture()?;
    doc.operations[1].members.push(1);
    assert!(doc.unassigned_shapes().all(|s| s.id != 1), "a shape in a Cut and an Engrave operation is assigned");
    let passes: Vec<(&str, Vec<u64>)> =
        doc.production_sequence().map(|(o, shapes)| (o.name.as_str(), shapes.iter().map(|s| s.id).collect())).collect();
    assert_eq!(passes, vec![("cut", vec![1, 2]), ("engrave", vec![1])], "one shape appears in each of its operations' passes");
    Ok(())
}

#[test]
fn guide_membership_counts_as_assigned_but_produces_no_pass() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = fixture()?;
    doc.operations[2].members.push(2);
    assert!(doc.unassigned_shapes().all(|s| s.id != 2), "a Guide member is assigned (D-04)");
    let names: Vec<&str> = doc.production_sequence().map(|(o, _)| o.name.as_str()).collect();
    assert_eq!(names, vec!["cut", "engrave"], "the Guide operation is excluded from production (D-17)");
    Ok(())
}

#[test]
fn production_sequence_follows_operations_order() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = fixture()?;
    doc.layers[3].members.push(3);
    let names: Vec<&str> = doc.production_sequence().map(|(o, _)| o.name.as_str()).collect();
    assert_eq!(names, vec!["cut", "engrave"], "pass order is the operations order");
    assert_eq!(doc.unassigned_shapes().count(), 1, "a shape in a geometry layer only is still unassigned");
    // Move engrave first: pass order follows.
    doc.operations.swap(0, 1);
    let names: Vec<&str> = doc.production_sequence().map(|(o, _)| o.name.as_str()).collect();
    assert_eq!(names, vec!["engrave", "cut"]);
    Ok(())
}

#[test]
fn layer_bounds_unions_member_bounds() -> Result<(), Box<dyn std::error::Error>> {
    let doc = fixture()?;
    let b = doc.layer_bounds(0).expect("the cut layer has bounds");
    assert!((b.x0 - 0.0).abs() < 1e-9 && (b.y0 - 0.0).abs() < 1e-9 && (b.x1 - 30.0).abs() < 1e-9 && (b.y1 - 10.0).abs() < 1e-9);
    assert!(doc.layer_bounds(2).is_none(), "the empty Guide layer has no bounds");
    assert!(doc.layer_bounds(99).is_none(), "an unknown layer has no bounds");
    Ok(())
}

#[test]
fn remove_layer_preserves_geometry_and_makes_members_unassigned() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = fixture()?;
    let removed = doc.remove_layer(0)?;
    assert_eq!(removed.name, "cut");
    assert_eq!(doc.shape_count(), 3, "remove_layer never touches Document.shapes");
    assert_eq!(doc.layers.len(), 3);
    // Operations are production, not geometry: they survive the layer removal.
    assert_eq!(doc.unassigned_shapes().count(), 1, "only the layer-less shape is unassigned; operation membership still assigns");
    assert!(matches!(doc.remove_layer(9), Err(DocError::Model(_))), "out-of-range removal is a calm Model error");
    Ok(())
}

#[test]
fn removing_the_last_layer_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::default();
    doc.push_shape(0, rect_path(0.0, 0.0, 5.0, 5.0))?;
    doc.remove_layer(0)?;
    assert!(doc.layers.is_empty(), "the model has no minimum-layer invariant");
    assert_eq!(doc.shape_count(), 1, "the shape survives its layer's deletion");
    Ok(())
}

#[test]
fn loading_prunes_dangling_and_duplicate_member_ids() -> Result<(), Box<dyn std::error::Error>> {
    let doc = fixture()?;
    let json = doc.save_json()?;
    let mut v: serde_json::Value = serde_json::from_str(&json)?;
    let members = v["operations"][0]["members"].as_array_mut().expect("members written");
    members.push(serde_json::json!(999));
    members.push(serde_json::json!(1));
    let back = Document::load_json(&serde_json::to_string(&v)?)?;
    assert_eq!(back.operations[0].members, vec![1, 2], "dangling and duplicate member ids are pruned on load, never fatal");
    Ok(())
}

#[test]
fn saving_repairs_stale_members_without_failing() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = fixture()?;
    doc.operations[0].members.push(999);
    doc.operations[0].members.push(1);
    let json = doc.save_json()?;
    let v: serde_json::Value = serde_json::from_str(&json)?;
    let members = v["operations"][0]["members"].as_array().expect("members written");
    assert_eq!(members.len(), 2, "save_json serialises the pruned clone");
    let _ = Document::load_json(&json)?;
    Ok(())
}

#[test]
fn validate_rejects_bad_model_state() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = fixture()?;
    doc.shapes.push(doc.shapes[0].clone());
    assert!(matches!(doc.validate(), Err(DocError::Model(_))), "duplicate shape ids rejected");
    let mut doc = fixture()?;
    doc.next_shape_id = 2;
    assert!(matches!(doc.validate(), Err(DocError::Model(_))), "next_shape_id not above all ids rejected");
    let mut doc = fixture()?;
    doc.operations[0].members.push(999);
    assert!(matches!(doc.validate_model(), Err(DocError::Model(_))), "missing operation member reference rejected by validate_model");
    let mut doc = fixture()?;
    doc.operations.push(doc.operations[0].clone());
    assert!(matches!(doc.validate_model(), Err(DocError::Model(_))), "duplicate operation ids rejected");
    Ok(())
}

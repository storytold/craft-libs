//! Operations: all ten kinds round-trip as first-class entities with non-default
//! settings, the process/strategy axes match D-06, there is no per-shape operation
//! field (D-05), and invalid settings are rejected with `DocError::Model`.

use craft_stencilcraft_document::{DocError, Document, Operation, OperationKind, OperationSettings, Process, Strategy, Tab, Tip};
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

fn settings() -> OperationSettings {
    OperationSettings {
        power: 80.0,
        speed_mm_per_min: 1200.0,
        passes: 2,
        focus_mm: -0.5,
        kerf_mm: 0.15,
        tabs: vec![Tab { offset_mm: 3.0, width_mm: 1.5 }],
        tip: Some(Tip::Wide),
        deboss: true,
        punch_diameter_mm: Some(4.0),
    }
}

fn all_kinds() -> Vec<OperationKind> {
    vec![
        OperationKind::Cut,
        OperationKind::KissCut,
        OperationKind::DieCut,
        OperationKind::Draw,
        OperationKind::Score,
        OperationKind::Engrave,
        OperationKind::Perforate,
        OperationKind::PrintThenCut,
        OperationKind::Punch,
        OperationKind::Guide,
    ]
}

fn operation(id: u64, kind: OperationKind, settings: OperationSettings) -> Operation {
    Operation {
        id,
        name: kind.kind_name().to_string(),
        kind,
        settings,
        color: Some("#102030".to_string()),
        visible: true,
        locked: false,
        members: vec![1],
        icon: Some("star".to_string()),
    }
}

#[test]
fn every_operation_kind_round_trips_with_non_default_params() -> Result<(), Box<dyn std::error::Error>> {
    for kind in all_kinds() {
        let mut doc = Document::default();
        doc.push_shape(0, rect_path(0.0, 0.0, 8.0, 8.0))?;
        let op = operation(doc.next_operation_id(), kind, settings());
        doc.operations.push(op.clone());
        let back = Document::load_json(&doc.save_json()?)?;
        assert_eq!(back.operations.as_slice(), &[op], "{} round-trips through save/load", kind.kind_name());
    }
    Ok(())
}

#[test]
fn process_and_strategy_axes_match_the_kind() {
    assert_eq!(OperationKind::Cut.process(), Some(Process::Laser));
    assert_eq!(OperationKind::Engrave.process(), Some(Process::Laser));
    assert_eq!(OperationKind::Score.process(), Some(Process::Creaser));
    assert_eq!(OperationKind::Guide.process(), None);
    assert_eq!(OperationKind::Guide.strategy(), Strategy::Guide);
    assert_eq!(OperationKind::Cut.strategy(), Strategy::Line);
    assert_eq!(OperationKind::Engrave.strategy(), Strategy::Fill);
    assert_eq!(OperationKind::Score.strategy(), Strategy::Score);
}

#[test]
fn a_shape_has_no_per_shape_operation_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::default();
    let shape = doc.push_shape(0, rect_path(0.0, 0.0, 4.0, 4.0))?;
    let v = serde_json::to_value(&shape)?;
    assert!(v.get("operation").is_none(), "operation authority lives on Document.operations only (D-05)");
    // An operation reaches a shape only through its members (ids only, N:M).
    doc.operations.push(operation(doc.next_operation_id(), OperationKind::Cut, OperationSettings::default()));
    let passes: Vec<u64> = doc.production_sequence().flat_map(|(_, ss)| ss.iter().map(|s| s.id).collect::<Vec<_>>()).collect();
    assert_eq!(passes, vec![shape.id], "the shape is produced by the operation holding its id");
    Ok(())
}

#[test]
fn invalid_settings_are_rejected_with_model_errors() -> Result<(), Box<dyn std::error::Error>> {
    let mut non_finite = settings();
    non_finite.power = f64::NAN;
    let mut zero_passes = settings();
    zero_passes.passes = 0;
    let mut negative_tab = settings();
    negative_tab.tabs[0].width_mm = -1.0;
    for (kind, s) in [(OperationKind::Cut, non_finite), (OperationKind::Engrave, zero_passes), (OperationKind::Perforate, negative_tab)] {
        let mut doc = Document::default();
        doc.push_shape(0, rect_path(0.0, 0.0, 4.0, 4.0))?;
        doc.operations.push(operation(doc.next_operation_id(), kind, s));
        assert!(matches!(doc.save_json(), Err(DocError::Model(_))), "save_json rejects invalid settings with DocError::Model");
    }
    Ok(())
}

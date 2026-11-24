//! Operations: all ten variants round-trip with non-default settings, the process/strategy
//! axes match D-06, there is no per-shape operation field (D-05), and invalid settings are
//! rejected with `DocError::Model`.

use craft_stencilcraft_document::{DocError, Document, Layer, Operation, OperationSettings, Process, Strategy, Tab, Tip};
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

fn all_variants() -> Vec<Operation> {
    let s = settings();
    vec![
        Operation::Cut(s.clone()),
        Operation::KissCut(s.clone()),
        Operation::DieCut(s.clone()),
        Operation::Draw(s.clone()),
        Operation::Score(s.clone()),
        Operation::Engrave(s.clone()),
        Operation::Perforate(s.clone()),
        Operation::PrintThenCut(s.clone()),
        Operation::Punch(s),
        Operation::Guide,
    ]
}

#[test]
fn every_operation_variant_round_trips_with_non_default_params() -> Result<(), Box<dyn std::error::Error>> {
    for op in all_variants() {
        let mut doc = Document { layers: vec![Layer::new("pass", Some(op.clone()))], ..Document::default() };
        doc.push_shape(0, rect_path(0.0, 0.0, 8.0, 8.0))?;
        let back = Document::load_json(&doc.save_json()?)?;
        assert_eq!(back.layers[0].operation.as_ref(), Some(&op), "{} round-trips through save/load", op.kind_name());
    }
    Ok(())
}

#[test]
fn process_and_strategy_axes_match_the_operation() {
    let s = OperationSettings::default();
    assert_eq!(Operation::Cut(s.clone()).process(), Some(Process::Laser));
    assert_eq!(Operation::Engrave(s.clone()).process(), Some(Process::Laser));
    assert_eq!(Operation::Score(s.clone()).process(), Some(Process::Creaser));
    assert_eq!(Operation::Guide.process(), None);
    assert_eq!(Operation::Guide.strategy(), Strategy::Guide);
    assert_eq!(Operation::Cut(s.clone()).strategy(), Strategy::Line);
    assert_eq!(Operation::Engrave(s.clone()).strategy(), Strategy::Fill);
    assert_eq!(Operation::Score(s).strategy(), Strategy::Score);
}

#[test]
fn a_shape_has_no_per_shape_operation_field() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::default();
    let shape = doc.push_shape(0, rect_path(0.0, 0.0, 4.0, 4.0))?;
    let v = serde_json::to_value(&shape)?;
    assert!(v.get("operation").is_none(), "operation authority lives on layers only (D-05)");
    // An operation reaches a shape only through a one-shape operation layer.
    doc.layers[0].operation = Some(Operation::Cut(OperationSettings::default()));
    doc.layers[0].members = vec![shape.id];
    let passes: Vec<u64> = doc.production_sequence().flat_map(|(_, ss)| ss.iter().map(|s| s.id).collect::<Vec<_>>()).collect();
    assert_eq!(passes, vec![shape.id], "the shape is produced by its layer's pass");
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
    for op in [Operation::Cut(non_finite), Operation::Engrave(zero_passes), Operation::Perforate(negative_tab)] {
        let doc = Document { layers: vec![Layer::new("pass", Some(op))], ..Document::default() };
        assert!(matches!(doc.save_json(), Err(DocError::Model(_))), "save_json rejects invalid settings with DocError::Model");
    }
    Ok(())
}

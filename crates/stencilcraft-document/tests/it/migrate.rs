//! Migration: v1 fixtures load in memory and re-save as v2 with no legacy wire fields (D-13/D-14/D-16).

use craft_stencilcraft_document::{Document, FORMAT_VERSION, Operation, Tip};

const STARTER: &str = include_str!("../fixtures/starter.stencilcraft");
const POSTER: &str = include_str!("../fixtures/poster.stencilcraft");
const STENCIL_FRAME: &str = include_str!("../fixtures/stencil-frame.stencilcraft");
const TOOL_PASSES: &str = include_str!("../fixtures/tool-passes.stencilcraft");

#[test]
fn existing_fixtures_load_and_resave_as_v2_without_legacy_fields() -> Result<(), Box<dyn std::error::Error>> {
    for (name, json) in [("starter", STARTER), ("poster", POSTER), ("stencil-frame", STENCIL_FRAME)] {
        let doc = Document::load_json(json)?;
        let saved = doc.save_json()?;
        let v: serde_json::Value = serde_json::from_str(&saved)?;
        assert_eq!(v["stencilcraft"]["version"].as_u64(), Some(FORMAT_VERSION), "{name} re-saves with the v2 header");
        let shapes = v["shapes"].as_array().expect("{name}: a v2 file has a top-level shapes array");
        assert!(shapes.iter().all(|s| s.get("style").is_none()), "{name}: no legacy style key on shapes");
        let layers = v["layers"].as_array().expect("{name}: a v2 file has layers");
        assert!(layers.iter().all(|l| l.get("kind").is_none()), "{name}: no legacy kind key on layers");
        assert_eq!(Document::load_json(&saved)?, doc, "{name}: the re-saved v2 file reloads equal");
    }
    Ok(())
}

#[test]
fn starter_migrates_to_three_shapes_and_two_operation_layers() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(STARTER)?;
    assert_eq!(doc.shape_count(), 3, "starter yields three document-level shapes");
    assert_eq!(doc.next_shape_id, 4, "starter keeps its stored next-shape-id");
    assert_eq!(doc.layers.len(), 2, "one operation layer per v1 layer");
    let (cut, engrave) = (&doc.layers[0], &doc.layers[1]);
    assert!(matches!(cut.operation, Some(Operation::Cut(_))), "the cut layer keeps the Cut pass");
    assert_eq!(cut.members, vec![1, 2], "the Cut layer holds both cut shapes");
    assert!(matches!(engrave.operation, Some(Operation::Engrave(_))), "the engrave layer keeps the Engrave pass");
    assert_eq!(engrave.members, vec![3], "the Engrave layer holds the one engrave shape");
    Ok(())
}

#[test]
fn poster_migrates_to_three_shapes_with_cut_and_engrave_layers() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(POSTER)?;
    assert_eq!(doc.shape_count(), 3, "poster yields three document-level shapes");
    assert_eq!(doc.next_shape_id, 4, "poster keeps its stored next-shape-id");
    assert_eq!(doc.layers.len(), 2, "one operation layer per v1 layer");
    let (cut, engrave) = (&doc.layers[0], &doc.layers[1]);
    assert!(matches!(cut.operation, Some(Operation::Cut(_))), "the cut layer keeps the Cut pass");
    assert_eq!(cut.members, vec![1, 2], "the Cut layer holds both cut shapes");
    assert!(matches!(engrave.operation, Some(Operation::Engrave(_))), "the engrave layer keeps the Engrave pass");
    assert_eq!(engrave.members, vec![3], "the Engrave layer holds the one engrave shape");
    Ok(())
}

#[test]
fn stencil_frame_migrates_to_two_shapes_and_preserves_kerf() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(STENCIL_FRAME)?;
    assert_eq!(doc.shape_count(), 2, "stencil-frame yields two document-level shapes");
    assert_eq!(doc.next_shape_id, 3, "stencil-frame keeps its stored next-shape-id");
    assert_eq!(doc.layers.len(), 1, "both shapes were in one v1 layer");
    assert!(matches!(doc.layers[0].operation, Some(Operation::Cut(_))), "the layer keeps the Cut pass");
    assert_eq!(doc.layers[0].members, vec![1, 2], "the Cut layer holds both shapes");
    assert_eq!(doc.kerf.width_mm, 0.2, "the 0.2 mm kerf setting survives migration");
    Ok(())
}

#[test]
fn distinct_tool_passes_migrate_to_separate_layers() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(TOOL_PASSES)?;
    assert_eq!(doc.shape_count(), 4);
    assert_eq!(doc.next_shape_id, 5);
    assert_eq!(doc.layers.len(), 4, "each distinct tool pass becomes its own layer, first-appearance order (D-14)");

    let ops: Vec<&Operation> = doc.layers.iter().map(|l| l.operation.as_ref().expect("a migrated layer carries an operation")).collect();
    assert!(matches!(&ops[0], Operation::Score(s) if s.tip == Some(Tip::Fine) && s.passes == 1), "score/fine keeps its settings");
    assert!(matches!(&ops[1], Operation::Score(s) if s.tip == Some(Tip::Wide) && s.passes == 3), "score/wide-3 is not merged with score/fine");
    assert!(
        matches!(&ops[2], Operation::Engrave(s) if s.tip == Some(Tip::Fine) && s.passes == 2 && !s.deboss),
        "emboss maps to Engrave carrying tip/passes/deboss"
    );
    assert!(matches!(&ops[3], Operation::Engrave(s) if s.tip.is_none() && !s.deboss), "power-engrave maps to Engrave and is not merged with emboss");

    let mut memberships: Vec<u64> = doc.layers.iter().flat_map(|l| l.members.iter().copied()).collect();
    memberships.sort_unstable();
    assert_eq!(memberships, vec![1, 2, 3, 4], "each shape is a member of exactly one layer");

    let saved = doc.save_json()?;
    let v: serde_json::Value = serde_json::from_str(&saved)?;
    assert_eq!(v["stencilcraft"]["version"].as_u64(), Some(FORMAT_VERSION), "the migrated fixture re-saves as v2");
    Ok(())
}

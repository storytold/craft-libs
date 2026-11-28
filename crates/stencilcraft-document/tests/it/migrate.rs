//! Migration: v1 fixtures load in memory and re-save as v2 with no legacy wire fields,
//! landing directly on the revised model — geometry-only layers plus first-class
//! operations (D-11/D-13/D-14/D-16).

use craft_stencilcraft_document::{Document, FORMAT_VERSION, OperationKind, Tip};

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
        assert!(layers.iter().all(|l| l.get("operation").is_none()), "{name}: layers are geometry-only now");
        assert_eq!(Document::load_json(&saved)?, doc, "{name}: the re-saved v2 file reloads equal");
    }
    Ok(())
}

#[test]
fn starter_migrates_to_three_shapes_and_two_operations() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(STARTER)?;
    assert_eq!(doc.shape_count(), 3, "starter yields three document-level shapes");
    assert_eq!(doc.next_shape_id, 4, "starter keeps its stored next-shape-id");
    assert_eq!(doc.layers.len(), 2, "one geometry layer per v1 layer");
    assert_eq!(doc.operations.len(), 2, "one operation per v1 layer pass");
    let (cut, engrave) = (&doc.operations[0], &doc.operations[1]);
    assert_eq!(cut.kind, OperationKind::Cut, "the cut layer keeps the Cut pass");
    assert_eq!(cut.members, vec![1, 2], "the Cut operation holds both cut shapes");
    assert_eq!(engrave.kind, OperationKind::Engrave, "the engrave layer keeps the Engrave pass");
    assert_eq!(engrave.members, vec![3], "the Engrave operation holds the one engrave shape");
    assert_eq!(doc.layers[0].members, vec![1, 2], "the geometry layer holds all of its shapes");
    Ok(())
}

#[test]
fn poster_migrates_to_three_shapes_with_cut_and_engrave_operations() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(POSTER)?;
    assert_eq!(doc.shape_count(), 3, "poster yields three document-level shapes");
    assert_eq!(doc.next_shape_id, 4, "poster keeps its stored next-shape-id");
    assert_eq!(doc.layers.len(), 2, "one geometry layer per v1 layer");
    let (cut, engrave) = (&doc.operations[0], &doc.operations[1]);
    assert_eq!(cut.kind, OperationKind::Cut, "the cut layer keeps the Cut pass");
    assert_eq!(cut.members, vec![1, 2], "the Cut operation holds both cut shapes");
    assert_eq!(engrave.kind, OperationKind::Engrave, "the engrave layer keeps the Engrave pass");
    assert_eq!(engrave.members, vec![3], "the Engrave operation holds the one engrave shape");
    Ok(())
}

#[test]
fn stencil_frame_migrates_to_two_shapes_and_preserves_kerf() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(STENCIL_FRAME)?;
    assert_eq!(doc.shape_count(), 2, "stencil-frame yields two document-level shapes");
    assert_eq!(doc.next_shape_id, 3, "stencil-frame keeps its stored next-shape-id");
    assert_eq!(doc.layers.len(), 1, "both shapes were in one v1 layer");
    assert_eq!(doc.operations.len(), 1);
    assert_eq!(doc.operations[0].kind, OperationKind::Cut, "the layer keeps the Cut pass");
    assert_eq!(doc.operations[0].members, vec![1, 2], "the Cut operation holds both shapes");
    assert_eq!(doc.kerf.width_mm, 0.2, "the 0.2 mm kerf setting survives migration");
    Ok(())
}

#[test]
fn distinct_tool_passes_migrate_to_separate_operations() -> Result<(), Box<dyn std::error::Error>> {
    let doc = Document::load_json(TOOL_PASSES)?;
    assert_eq!(doc.shape_count(), 4);
    assert_eq!(doc.next_shape_id, 5);
    assert_eq!(doc.layers.len(), 1, "the v1 layer becomes one geometry layer");
    assert_eq!(doc.layers[0].members, vec![1, 2, 3, 4]);
    assert_eq!(doc.operations.len(), 4, "each distinct tool pass becomes its own operation, first-appearance order (D-14)");

    let ops: Vec<OperationKind> = doc.operations.iter().map(|o| o.kind).collect();
    assert_eq!(ops[0], OperationKind::Score);
    assert_eq!(doc.operations[0].settings.tip, Some(Tip::Fine));
    assert_eq!(doc.operations[0].settings.passes, 1, "score/fine keeps its settings");
    assert_eq!(ops[1], OperationKind::Score);
    assert_eq!(doc.operations[1].settings.tip, Some(Tip::Wide));
    assert_eq!(doc.operations[1].settings.passes, 3, "score/wide-3 is not merged with score/fine");
    assert_eq!(ops[2], OperationKind::Engrave);
    assert!(
        doc.operations[2].settings.tip == Some(Tip::Fine) && doc.operations[2].settings.passes == 2 && !doc.operations[2].settings.deboss,
        "emboss maps to Engrave carrying tip/passes/deboss"
    );
    assert_eq!(ops[3], OperationKind::Engrave);
    assert!(
        doc.operations[3].settings.tip.is_none() && !doc.operations[3].settings.deboss,
        "power-engrave maps to Engrave and is not merged with emboss"
    );

    let mut memberships: Vec<u64> = doc.operations.iter().flat_map(|o| o.members.iter().copied()).collect();
    memberships.sort_unstable();
    assert_eq!(memberships, vec![1, 2, 3, 4], "each shape is a member of exactly one operation");

    let saved = doc.save_json()?;
    let v: serde_json::Value = serde_json::from_str(&saved)?;
    assert_eq!(v["stencilcraft"]["version"].as_u64(), Some(FORMAT_VERSION), "the migrated fixture re-saves as v2");
    assert_eq!(Document::load_json(&saved)?, doc, "geometry and production data round-trip losslessly");
    Ok(())
}

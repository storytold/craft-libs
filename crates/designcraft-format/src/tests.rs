use super::*;
use serde_json::json;

#[test]
fn native_bundle_roundtrips_document_assets_and_preview() -> Result<(), Box<dyn std::error::Error>> {
    let document = json!({"title":"A page", "assets":{"7":{"mime":"image/png"}}});
    let image = [1u8, 2, 3, 4];
    let assets = [Asset { id: "7", mime: "image/png", data: &image }];
    let preview = b"preview png";
    let bytes = save(&document, "DesignCraft test", Some(preview), &assets)?;

    assert!(is_designcraft(&bytes));
    assert!(starts_designcraft(&bytes[..bytes.len().min(80)]));
    let loaded = load::<Value>(&bytes)?;
    assert_eq!(loaded.document, document);
    assert_eq!(loaded.assets, [("7".to_owned(), image.to_vec())]);
    assert_eq!(read_preview(std::io::Cursor::new(&bytes)).as_deref(), Some(&preview[..]));
    Ok(())
}

#[test]
fn preview_reader_rejects_an_entry_over_its_declared_read_budget() -> Result<(), Box<dyn std::error::Error>> {
    let oversized = vec![0u8; usize::try_from(MAX_PREVIEW_BYTES + 1)?];
    let bytes = save(&json!({}), "DesignCraft test", Some(&oversized), &[])?;
    assert_eq!(read_preview(std::io::Cursor::new(bytes)), None);
    Ok(())
}

#[test]
fn required_version_preserves_the_reader_capability_contract() {
    assert_eq!(required_version(&json!({"title":"plain"}), &[]), 2);
    assert_eq!(required_version(&json!({}), &[Asset { id: "1", mime: "application/postscript", data: b"%!PS-Adobe-3.0 EPSF-3.0" }]), 3);
    assert_eq!(required_version(&json!({"layerOptions":{}}), &[]), 4);
    assert_eq!(required_version(&json!({"pdfLayerOverride":true,"pdfHiddenLayers":[]}), &[]), 4);
    assert_eq!(required_version(&json!({"pdfLayerOverride":true,"pdfHiddenLayers":["guide"]}), &[]), 2);

    let mut nested = json!(null);
    for _ in 0..65 {
        nested = json!([nested]);
    }
    assert_eq!(required_version(&nested, &[]), 4);
}

#[test]
fn legacy_json_extracts_assets_and_applies_v1_migrations() -> Result<(), Box<dyn std::error::Error>> {
    let legacy = json!({
        "assetData":{"7":"AQID"},
        "frames":[{"id":8,"content":{"type":"text","options":{"vertical":true}}}],
        "stories":{"vertical":{"frames":[8]},"explicit":{"frames":[8],"vertical":false}},
        "styles":{"paragraph":[{"name":"Base"},{"name":"Child","basedOn":"Base","para":{"keepAllLines":false}}]},
        "paragraphs":[{"style":"Missing"}]
    });
    let loaded = load::<Value>(&serde_json::to_vec(&legacy)?)?;

    assert_eq!(loaded.assets, [("7".to_owned(), vec![1, 2, 3])]);
    assert_eq!(loaded.document.get("assetData"), None);
    assert_eq!(loaded.document["stories"]["vertical"]["vertical"], true);
    assert_eq!(loaded.document["stories"]["explicit"]["vertical"], false);
    assert_eq!(loaded.document["styles"]["paragraph"][0]["para"]["keepAllLines"], true);
    assert_eq!(loaded.document["styles"]["paragraph"][0]["para"]["dropCapAlignLeft"], false);
    assert_eq!(loaded.document["styles"]["paragraph"][1]["para"]["keepAllLines"], false);
    assert_eq!(loaded.document["paragraphs"][0]["para"]["keepAllLines"], true);
    Ok(())
}

#[test]
fn newer_metadata_precedes_malformed_document_json() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = craft_document_bundle::write(b"{", VERSION + 1, "future", None, &[], BUNDLE)?;
    assert!(matches!(load::<Value>(&bytes), Err(Error::TooNew(version)) if version == u64::from(VERSION + 1)));
    assert!(matches!(load::<Value>(b"not a package"), Err(Error::NotOurs(_))));
    Ok(())
}

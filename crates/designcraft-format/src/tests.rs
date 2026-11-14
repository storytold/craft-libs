use super::*;
use serde_json::json;

#[test]
fn native_bundle_roundtrips_document_assets_and_preview() -> Result<(), Box<dyn std::error::Error>> {
    let document = json!({"title":"A page", "assets":{"7":{"mime":"image/png"}}});
    let image = [1u8, 2, 3, 4];
    let assets = [Asset { id: "7", mime: "image/png", data: &image }];
    let preview = b"preview png";
    let bytes = save(&document, "DesignCraft test", Some(preview), &assets, &[])?;

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
    let bytes = save(&json!({}), "DesignCraft test", Some(&oversized), &[], &[])?;
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
    let bytes = craft_document_bundle::write(b"{", VERSION + 1, "future", None, &[], &[], BUNDLE)?;
    assert!(matches!(load::<Value>(&bytes), Err(Error::TooNew(version)) if version == u64::from(VERSION + 1)));
    assert!(matches!(load::<Value>(b"not a package"), Err(Error::NotOurs(_))));
    Ok(())
}

fn linked_doc() -> Value {
    json!({"assets":{"7":{"storage":"linked","mime":"image/png"},"8":{"mime":"image/png"}}})
}

#[test]
fn embedded_documents_keep_their_version() {
    assert_eq!(required_version(&json!({"assets":{"7":{"storage":"embedded"},"8":{}}}), &[]), 2);
}

#[test]
fn a_linked_asset_requires_version_five_and_old_readers_refuse_it() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(required_version(&linked_doc(), &[]), 5);
    let bytes = save(&linked_doc(), "t", None, &[], &[])?;
    assert_eq!(craft_document_bundle::read(&bytes, 4, BUNDLE), Err(craft_document_bundle::Error::TooNew(5)));
    let loaded = load::<Value>(&bytes)?;
    assert_eq!(loaded.document, linked_doc());
    assert!(loaded.assets.is_empty());
    Ok(())
}

#[test]
fn proxies_round_trip_and_unknown_or_oversized_ones_are_skipped() -> Result<(), Box<dyn std::error::Error>> {
    let big = vec![0u8; usize::try_from(craft_document_bundle::PROXY_LIMIT)? + 1];
    let proxies = [
        Asset { id: "7", mime: "image/jpeg", data: b"jpeg" },
        Asset { id: "8", mime: "image/png", data: &big },
        Asset { id: "99", mime: "image/png", data: b"unknown" },
    ];
    let bytes = save(&linked_doc(), "t", None, &[], &proxies)?;
    assert_eq!(load::<Value>(&bytes)?.proxies, [("7".to_owned(), b"jpeg".to_vec())]);
    Ok(())
}

#[test]
fn proxy_paths_must_be_exactly_proxies_id_ext() -> Result<(), Box<dyn std::error::Error>> {
    let proxies = [
        Asset { id: "../x", mime: "image/png", data: b"a" },
        Asset { id: "a/b", mime: "image/png", data: b"b" },
        Asset { id: "7/8", mime: "image/png", data: b"c" },
    ];
    let bytes = save(&linked_doc(), "t", None, &[], &proxies)?;
    assert!(load::<Value>(&bytes)?.proxies.is_empty());
    Ok(())
}

#[test]
fn write_to_stops_when_the_writer_fails() {
    struct Failing {
        inner: Cursor<Vec<u8>>,
        budget: usize,
        failed: bool,
        calls_after_failure: std::rc::Rc<std::cell::Cell<u32>>,
    }
    impl Write for Failing {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.failed {
                self.calls_after_failure.set(self.calls_after_failure.get() + 1);
            }
            if self.budget < buf.len() {
                self.failed = true;
                return Err(std::io::Error::other("cancelled"));
            }
            self.budget -= buf.len();
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.failed {
                self.calls_after_failure.set(self.calls_after_failure.get() + 1);
            }
            self.inner.flush()
        }
    }
    impl Seek for Failing {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            if self.failed {
                self.calls_after_failure.set(self.calls_after_failure.get() + 1);
            }
            self.inner.seek(pos)
        }
    }
    let data = vec![1u8; 4096];
    let assets = [Asset { id: "8", mime: "image/png", data: &data }];
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let out = Failing { inner: Cursor::new(Vec::new()), budget: 1024, failed: false, calls_after_failure: calls.clone() };
    let result = write_to(out, &linked_doc(), "t", None, &assets, &[]);
    assert_eq!(calls.get(), 0, "no write, flush or seek reaches the writer after it failed");
    assert!(matches!(result, Err(Error::Io(message)) if message.contains("cancelled")));
}

#[test]
fn bytes_supplied_for_a_linked_asset_are_not_written() -> Result<(), Box<dyn std::error::Error>> {
    let assets = [Asset { id: "7", mime: "image/png", data: b"linked bytes" }, Asset { id: "8", mime: "image/png", data: b"kept" }];
    let bytes = save(&linked_doc(), "t", None, &assets, &[])?;
    assert_eq!(load::<Value>(&bytes)?.assets, [("8".to_owned(), b"kept".to_vec())]);
    Ok(())
}

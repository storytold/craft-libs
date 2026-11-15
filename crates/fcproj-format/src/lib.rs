//! FilmCraft `.fcproj` JSON envelope, version detection and historical migrations.
//! Project and editor-view types are supplied by the caller.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// The `format` tag written in every envelope.
pub const FORMAT_ID: &str = "filmcraft.project";

/// A migration upgrades a whole document from schema `n` to `n + 1`.
pub type Migration = fn(Value) -> Result<Value, String>;

/// `MIGRATIONS[i]` upgrades schema `i + 1` to `i + 2`. Append one function per schema bump; never
/// edit a shipped one.
pub const MIGRATIONS: &[Migration] = &[
    v1_to_v2, v2_to_v3, v3_to_v4, v4_to_v5, v5_to_v6, v6_to_v7, v7_to_v8, v8_to_v9, v9_to_v10, v10_to_v11, v11_to_v12, v12_to_v13, v13_to_v14,
    v14_to_v15, v15_to_v16, v16_to_v17,
];

/// The schema version this build writes (and the newest it reads).
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32 + 1;

/// Oldest schema this build can still upgrade.
pub const OLDEST_SCHEMA: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a FilmCraft project: {0}")]
    NotAProject(String),
    #[error(
        "this project was saved by a newer version of FilmCraft (project schema v{found}); this build reads up to v{supported}. Update FilmCraft to open it."
    )]
    TooNew { found: u32, supported: u32 },
    #[error("project schema v{found} is too old for this build (oldest supported: v{oldest})")]
    TooOld { found: u32, oldest: u32 },
    #[error("upgrading project from schema v{from} to v{}: {msg}", from + 1)]
    Migration { from: u32, msg: String },
    #[error("project file is damaged: {0}")]
    Corrupt(String),
}

/// Serialized current-schema envelope.
#[derive(Serialize)]
pub struct EnvelopeOut<'a, P, V> {
    pub format: &'static str,
    pub schema_version: u32,
    pub generator: String,
    pub project: &'a P,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<&'a V>,
}
/// Parsed envelope. View parsing remains optional to preserve forward compatibility.
#[derive(Deserialize)]
pub struct EnvelopeIn<P> {
    #[serde(default)]
    pub generator: Option<String>,
    pub project: P,
    #[serde(default)]
    pub view: Option<Value>,
}
/// Serialize the current envelope, with optional editor view and caller-provided generator.
pub fn encode<P: Serialize, V: Serialize>(project: &P, view: Option<&V>, generator: String, pretty: bool) -> Vec<u8> {
    let envelope = EnvelopeOut { format: FORMAT_ID, schema_version: SCHEMA_VERSION, generator, project, view };
    let result = if pretty { serde_json::to_vec_pretty(&envelope) } else { serde_json::to_vec(&envelope) };
    result.unwrap_or_default()
}

/// Decode an envelope, migrating older documents while retaining the current-version fast path.
pub fn decode<P: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<(EnvelopeIn<P>, u32), FormatError> {
    let found = detect_bytes(bytes)?;
    if found == SCHEMA_VERSION {
        let env = serde_json::from_slice(bytes).map_err(|e| FormatError::Corrupt(e.to_string()))?;
        return Ok((env, found));
    }
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let doc = migrate_with(MIGRATIONS, doc, found)?;
    let env = serde_json::from_value(doc).map_err(|e| FormatError::Corrupt(format!("after upgrading from schema v{found}: {e}")))?;
    Ok((env, found))
}

/// Probe and validate the schema directly from bytes without building a JSON tree.
pub fn detect_bytes(bytes: &[u8]) -> Result<u32, FormatError> {
    let probe: Probe = serde_json::from_slice(bytes)
        .map_err(|e| if e.is_syntax() || e.is_eof() { FormatError::Corrupt(e.to_string()) } else { FormatError::NotAProject(e.to_string()) })?;
    let found = probe_version(&probe)?;
    check_supported(found)?;
    Ok(found)
}
/// Just enough of a document to know which schema it is.
#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    schema_version: Option<u32>,
    /// Bare (pre-envelope) project: its `version` field.
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    items: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    root: Option<serde::de::IgnoredAny>,
}

/// Detect the schema version of a parsed document.
pub fn detect_version(doc: &Value) -> Result<u32, FormatError> {
    let probe: Probe = serde_json::from_value(doc.clone()).map_err(|e| FormatError::NotAProject(e.to_string()))?;
    probe_version(&probe)
}

fn probe_version(p: &Probe) -> Result<u32, FormatError> {
    if let Some(v) = p.schema_version {
        if let Some(f) = &p.format
            && f != FORMAT_ID
        {
            return Err(FormatError::NotAProject(format!("format is `{f}`, expected `{FORMAT_ID}`")));
        }
        return Ok(v);
    }
    // Before the envelope (v1) a file was the bare Project object with `version`.
    // Its `version` was 1, or 2 on builds that briefly bumped it when caption tracks were added
    // (same shape; the new field loads with its default). Anything higher is from the future.
    if p.items.is_some() && p.root.is_some() {
        return Ok(match p.version.unwrap_or(1) {
            0..=2 => 1,
            v => v,
        });
    }
    Err(FormatError::NotAProject("no `schema_version` and not a legacy project".into()))
}

pub fn check_supported(found: u32) -> Result<(), FormatError> {
    if found > SCHEMA_VERSION {
        return Err(FormatError::TooNew { found, supported: SCHEMA_VERSION });
    }
    if found < OLDEST_SCHEMA {
        return Err(FormatError::TooOld { found, oldest: OLDEST_SCHEMA });
    }
    Ok(())
}

/// Run `table` from schema `from` up to `table.len() + 1`, stamping `schema_version` after each step.
pub fn migrate_with(table: &[Migration], mut doc: Value, from: u32) -> Result<Value, FormatError> {
    let target = table.len() as u32 + 1;
    if from > target {
        return Err(FormatError::TooNew { found: from, supported: target });
    }
    if from == 0 {
        return Err(FormatError::TooOld { found: 0, oldest: 1 });
    }
    for v in from..target {
        let step = table[(v - 1) as usize];
        doc = step(doc).map_err(|msg| FormatError::Migration { from: v, msg })?;
        match doc.as_object_mut() {
            Some(o) => {
                o.insert("schema_version".into(), Value::from(v + 1));
            }
            None => return Err(FormatError::Migration { from: v, msg: "migration produced a non-object document".into() }),
        }
    }
    Ok(doc)
}

/// v1 → v2: wrap the bare project in the envelope and drop the per-project `version` field.
fn v1_to_v2(doc: Value) -> Result<Value, String> {
    let Value::Object(mut project) = doc else { return Err("expected a JSON object".into()) };
    project.remove("version");
    Ok(serde_json::json!({
        "format": FORMAT_ID,
        "schema_version": 2,
        "generator": "FilmCraft (schema v1)",
        "project": Value::Object(project),
    }))
}

/// v2 → v3: graphic clips added; existing data needs no change.
fn v2_to_v3(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v3 → v4: Essential Sound fields added; existing data needs no change.
fn v3_to_v4(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v4 → v5: colour-management fields added; existing data needs no change.
fn v4_to_v5(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v5 → v6: media identity and ingest settings added; existing data needs no change.
fn v5_to_v6(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v6 → v7: effect masks (Bézier mask paths as keyframable parameters); existing data needs no change.
fn v6_to_v7(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v7 → v8: multi-camera source sequences / clips and merged clips added; existing data needs no
/// change.
fn v7_to_v8(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v8 → v9: per-media-item transcripts (`project.transcripts`) added; existing data needs no change.
fn v8_to_v9(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v9 → v10: clip time interpolation, Hold Filters, Field Options and audio source channels; the
/// Modify ▸ Audio Channels map; subclips' Restrict Trims flag. Existing data needs no change.
fn v9_to_v10(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v10 → v11: search bins (`project.search_bins`), Flash Cue markers, Project Settings safe areas,
/// capture format and scratch disks. Existing data needs no change.
fn v10_to_v11(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v11 → v12: graphics templates, rolls / crawls, responsive design and per-character text styles
/// on graphic clips, and source graphics. Existing data needs no change.
fn v11_to_v12(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v12 → v13: container audio streams and per-clip stream selection. `MediaInfo` reads the old
/// `audio` field as stream zero; `TrackItem::audio_stream` defaults to zero.
fn v12_to_v13(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v13 → v14: narrations (Text to Speech). Existing data needs no change.
fn v13_to_v14(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v14 → v15: project notes. Existing data needs no change.
fn v14_to_v15(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v15 → v16: existing colors remain presets; authors and the named custom palette default empty.
fn v15_to_v16(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

/// v16 → v17: marker URLs default empty. Older builds must not silently drop saved URLs.
fn v16_to_v17(doc: Value) -> Result<Value, String> {
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_and_current_envelopes_preserve_values() {
        let bare = json!({"version": 2, "items": {}, "root": {}, "extra": [1, 2]});
        assert_eq!(detect_version(&bare).unwrap(), 1);
        let migrated = migrate_with(MIGRATIONS, bare, 1).unwrap();
        assert_eq!(migrated["schema_version"], 17);
        assert_eq!(migrated["project"]["extra"], json!([1, 2]));
        assert_eq!(migrated["generator"], "FilmCraft (schema v1)");
        assert_eq!(detect_bytes(serde_json::to_string(&migrated).unwrap().as_bytes()).unwrap(), 17);
        let parsed: EnvelopeIn<Value> = serde_json::from_value(migrated).unwrap();
        assert!(parsed.view.is_none());
        assert_eq!(parsed.project["extra"], json!([1, 2]));
    }
    #[test]
    fn typed_encode_and_decode_current_and_legacy() {
        let project = json!({"items": {}, "root": {}, "name": "Cut"});
        let view = json!({"active": 3});
        for pretty in [false, true] {
            let bytes = encode(&project, Some(&view), "FilmCraft test".into(), pretty);
            let (file, version): (EnvelopeIn<Value>, _) = decode(&bytes).unwrap();
            assert_eq!(version, SCHEMA_VERSION);
            assert_eq!(file.project, project);
            assert_eq!(file.view, Some(view.clone()));
        }
        let old = br#"{"version":1,"items":{},"root":{},"name":"Cut"}"#;
        let (file, version): (EnvelopeIn<Value>, _) = decode(old).unwrap();
        assert_eq!(version, 1);
        assert_eq!(file.project["name"], "Cut");
    }
    #[test]
    fn rejects_wrong_future_and_damaged_envelopes() {
        assert!(matches!(detect_bytes(br#"{"format":"other","schema_version":1}"#), Err(FormatError::NotAProject(_))));
        assert!(matches!(detect_bytes(br#"{"schema_version":18}"#), Err(FormatError::TooNew { found: 18, supported: 17 })));
        assert!(matches!(detect_bytes(br#"{"schema_version":0}"#), Err(FormatError::TooOld { found: 0, oldest: 1 })));
        assert!(matches!(detect_bytes(b"{"), Err(FormatError::Corrupt(_))));
        assert!(matches!(migrate_with(MIGRATIONS, json!([]), 1), Err(FormatError::Migration { from: 1, .. })));
    }
}

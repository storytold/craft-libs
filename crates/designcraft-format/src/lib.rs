//! Versioned JSON and bundle codec for DesignCraft's native document format.
//!
//! The application owns its document model. This crate owns the on-disk schema version,
//! serialization, legacy JSON upgrades, version inference and the shared ZIP envelope.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, Write};

pub use craft_document_bundle::Asset;
use craft_document_bundle::Spec;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub const MIME: &str = "application/vnd.designcraft+zip";
pub const VERSION: u32 = 5;
pub const EXTENSION: &str = "designcraft";
pub const PREVIEW: &str = "preview.png";
pub const PROXIES_DIRECTORY: &str = "proxies";
pub const MAX_PREVIEW_BYTES: u64 = 4 << 20;

const BUNDLE: Spec<'static> = Spec {
    mimetype_entry: "mimetype",
    mimetype: MIME,
    metadata_entry: "meta.json",
    document_entry: "document.json",
    preview_entry: PREVIEW,
    assets_directory: "assets",
    proxies_directory: PROXIES_DIRECTORY,
    format_name: "designcraft",
    legacy_assets_field: Some("assetData"),
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a DesignCraft document: {0}")]
    NotOurs(String),
    #[error("document is from a newer DesignCraft (format {0}); please update")]
    TooNew(u64),
    #[error("{0}")]
    Io(String),
    #[error("document cannot be saved: {0}")]
    Unsaveable(String),
}

impl From<craft_document_bundle::Error> for Error {
    fn from(error: craft_document_bundle::Error) -> Self {
        match error {
            craft_document_bundle::Error::TooNew(version) => Error::TooNew(version),
            craft_document_bundle::Error::NotBundle(message) => Error::NotOurs(message),
            craft_document_bundle::Error::Read(message) | craft_document_bundle::Error::Write(message) => Error::Io(message),
        }
    }
}

/// A decoded native document and the separately stored embedded assets.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded<T> {
    pub document: T,
    pub assets: Vec<(String, Vec<u8>)>,
    /// Proxy previews (JPEG or PNG bytes) of linked assets, keyed by asset id.
    pub proxies: Vec<(String, Vec<u8>)>,
}

/// The reader capability required by `document`, inferred from its neutral JSON schema and
/// embedded assets. Traversal beyond 64 levels conservatively requires version 4. A linked
/// asset (`/assets/<id>/storage == "linked"`) requires version 5.
pub fn required_version(document: &Value, assets: &[Asset<'_>]) -> u32 {
    if has_linked_asset(document) {
        return 5;
    }
    if has_version_four_feature(document) {
        return 4;
    }
    if assets.iter().any(|asset| craft_eps::is_eps(asset.data)) { 3 } else { 2 }
}

/// Serialize a document, its embedded assets and the proxy previews of its linked assets as a
/// `.designcraft` ZIP bundle. Linked assets have a record in the document and no `assets` entry.
pub fn save<T: Serialize>(
    document: &T,
    generator: &str,
    preview: Option<&[u8]>,
    assets: &[Asset<'_>],
    proxies: &[Asset<'_>],
) -> Result<Vec<u8>, Error> {
    let mut out = Cursor::new(Vec::new());
    write_to(&mut out, document, generator, preview, assets, proxies)?;
    Ok(out.into_inner())
}

/// Like [`save`], streaming the ZIP into `out`. A writer that returns an error (to cancel a
/// long save, say) stops the write and the error is returned as [`Error::Io`].
pub fn write_to<T: Serialize, W: Write + Seek>(
    out: W,
    document: &T,
    generator: &str,
    preview: Option<&[u8]>,
    assets: &[Asset<'_>],
    proxies: &[Asset<'_>],
) -> Result<(), Error> {
    let value = serde_json::to_value(document).map_err(|error| Error::Io(error.to_string()))?;
    let embedded: Vec<Asset<'_>> = assets.iter().filter(|asset| !is_linked(&value, asset.id)).copied().collect();
    let assets = embedded.as_slice();
    let version = required_version(&value, assets);
    let json = serde_json::to_vec_pretty(&value).map_err(|error| Error::Io(error.to_string()))?;
    craft_document_bundle::write_to(out, &json, version, generator, preview, assets, proxies, BUNDLE).map_err(Into::into)
}

/// Read a `.designcraft` ZIP or legacy single-file JSON, applying all versioned schema migrations
/// before deserializing the caller's model.
pub fn load<T: DeserializeOwned>(bytes: &[u8]) -> Result<Loaded<T>, Error> {
    let mut package = craft_document_bundle::read(bytes, VERSION, BUNDLE)?;
    upgrade_legacy_json(&mut package.document, package.version);
    let document = serde_json::from_value(package.document).map_err(|error| Error::NotOurs(error.to_string()))?;
    Ok(Loaded { document, assets: package.assets, proxies: package.proxies })
}

/// Whether `bytes` are a native DesignCraft ZIP package.
pub fn is_designcraft(bytes: &[u8]) -> bool {
    craft_document_bundle::has_document(bytes, BUNDLE)
}

/// Whether `prefix` begins with this format's stored mimetype entry.
pub fn starts_designcraft(prefix: &[u8]) -> bool {
    craft_document_bundle::starts_with_mimetype(prefix, BUNDLE)
}

/// Read the first-page PNG preview from a ZIP package, with a fixed 4 MiB cap.
pub fn read_preview<R: Read + Seek>(reader: R) -> Option<Vec<u8>> {
    craft_document_bundle::read_preview_from(reader, PREVIEW)
}

fn is_linked(document: &Value, id: &str) -> bool {
    document.get("assets").and_then(|assets| assets.get(id)).and_then(|asset| asset.get("storage")).and_then(Value::as_str) == Some("linked")
}

fn has_linked_asset(document: &Value) -> bool {
    document
        .get("assets")
        .and_then(Value::as_object)
        .is_some_and(|assets| assets.values().any(|asset| asset.get("storage").and_then(Value::as_str) == Some("linked")))
}

fn has_version_four_feature(document: &Value) -> bool {
    let mut stack = vec![(document, 0usize)];
    let mut visited = 0usize;
    while let Some((value, depth)) = stack.pop() {
        if depth > 64 || visited >= 1_000_000 {
            return true;
        }
        visited += 1;
        match value {
            Value::Object(object) => {
                if object.get("layerOptions").is_some_and(|options| !options.is_null())
                    || (object.get("pdfLayerOverride").and_then(Value::as_bool) == Some(true)
                        && object.get("pdfHiddenLayers").and_then(Value::as_array).is_none_or(Vec::is_empty))
                {
                    return true;
                }
                stack.extend(object.values().map(|child| (child, depth + 1)));
            }
            Value::Array(children) => stack.extend(children.iter().map(|child| (child, depth + 1))),
            _ => {}
        }
    }
    false
}

fn upgrade_legacy_json(document: &mut Value, from: u64) {
    let mut vertical = HashSet::new();
    vertical_frames(document, &mut vertical, 0);
    if let Some(stories) = document.get_mut("stories").and_then(Value::as_object_mut) {
        for story in stories.values_mut() {
            let first = story.get("frames").and_then(|frames| frames.get(0)).and_then(Value::as_u64);
            if let (Some(frame), Some(story)) = (first, story.as_object_mut())
                && vertical.contains(&frame)
                && !story.contains_key("vertical")
            {
                story.insert("vertical".into(), Value::Bool(true));
            }
        }
    }
    if from < 2 {
        pin_paragraph_defaults(document);
    }
}

fn vertical_frames(value: &Value, out: &mut HashSet<u64>, depth: usize) {
    if depth > 256 {
        return;
    }
    match value {
        Value::Object(object) => {
            if let (Some(id), Some(content)) = (object.get("id").and_then(Value::as_u64), object.get("content"))
                && content.get("type").and_then(Value::as_str) == Some("text")
                && content.get("options").and_then(|options| options.get("vertical")).and_then(Value::as_bool) == Some(true)
            {
                out.insert(id);
            }
            for child in object.values() {
                vertical_frames(child, out, depth + 1);
            }
        }
        Value::Array(children) => {
            for child in children {
                vertical_frames(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn pin_paragraph_defaults(document: &mut Value) {
    const MAX_STYLE_DEPTH: usize = 64;
    let styles = document.pointer("/styles/paragraph").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut by_name: HashMap<String, (Option<String>, bool, bool)> = HashMap::new();
    for style in &styles {
        let Some(name) = style.get("name").and_then(Value::as_str) else { continue };
        let based_on = style.get("basedOn").and_then(Value::as_str).map(str::to_string);
        let attrs = style.get("para");
        let missing_keep = attrs.and_then(|attrs| attrs.get("keepAllLines")).is_none();
        let missing_drop_cap = attrs.and_then(|attrs| attrs.get("dropCapAlignLeft")).is_none();
        by_name.entry(name.to_string()).or_insert((based_on, missing_keep, missing_drop_cap));
    }

    let mut roots = HashSet::new();
    for name in by_name.keys() {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut current = Some(name.as_str());
        while let Some(style_name) = current {
            let Some((based_on, missing_keep, missing_drop_cap)) = by_name.get(style_name) else { break };
            if chain.len() >= MAX_STYLE_DEPTH || !seen.insert(style_name.to_string()) {
                break;
            }
            chain.push((style_name, *missing_keep, *missing_drop_cap));
            current = based_on.as_deref();
        }
        if !chain.is_empty()
            && chain.iter().all(|(_, keep, drop_cap)| *keep && *drop_cap)
            && let Some((root, _, _)) = chain.last()
        {
            roots.insert((*root).to_string());
        }
    }
    if let Some(styles) = document.pointer_mut("/styles/paragraph").and_then(Value::as_array_mut) {
        for style in styles {
            if let Some(style) = style.as_object_mut()
                && style.get("name").and_then(Value::as_str).is_some_and(|name| roots.contains(name))
            {
                let attrs = style.entry("para").or_insert_with(|| Value::Object(Default::default()));
                if let Some(attrs) = attrs.as_object_mut() {
                    attrs.entry("keepAllLines").or_insert(Value::Bool(true));
                    attrs.entry("dropCapAlignLeft").or_insert(Value::Bool(false));
                }
            }
        }
    }
    let known_styles: HashSet<String> = by_name.into_keys().collect();
    pin_unstyled_paragraphs(document, &known_styles, 0);
}

fn pin_unstyled_paragraphs(value: &mut Value, known_styles: &HashSet<String>, depth: usize) {
    if depth > 256 {
        return;
    }
    match value {
        Value::Object(object) => {
            if let Some(style) = object.get("style").and_then(Value::as_str)
                && !known_styles.contains(style)
            {
                let attrs = object.entry("para").or_insert_with(|| Value::Object(Default::default()));
                if let Some(attrs) = attrs.as_object_mut() {
                    attrs.entry("keepAllLines").or_insert(Value::Bool(true));
                    attrs.entry("dropCapAlignLeft").or_insert(Value::Bool(false));
                }
            }
            for child in object.values_mut() {
                pin_unstyled_paragraphs(child, known_styles, depth + 1);
            }
        }
        Value::Array(children) => {
            for child in children {
                pin_unstyled_paragraphs(child, known_styles, depth + 1);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;

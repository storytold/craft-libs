//! PhotoCraft `.pcraft` bundles: a ZIP archive holding `manifest.json` (the document tree, with
//! `format_version`), tiles and blobs, and optionally the previews the manifest names
//! (`thumbnail`, `composite`). Only the ZIP form is read; a bundle saved as a folder is not. The
//! manifest's envelope is [`craft_pcraft_format::Manifest`]'s.

use craft_pcraft_format::{self as format, COMPOSITE, FORMAT_VERSION, MANIFEST, THUMB};
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};

use crate::zip::{Zip, local_names_include};
use crate::{Budget, Canvas, Error, Kind, Layer, MAX_DEPTH, Outline, Rendition, RenditionKind, counted, malformed, parse, parse_versioned, too_new};

pub(crate) fn sniff(head: &[u8]) -> bool {
    head.starts_with(b"PK\x03\x04") && (local_names_include(head, MANIFEST) || Zip::open(head).is_ok_and(|z| z.find(MANIFEST).is_some()))
}

/// The archive and its `manifest.json`.
fn manifest<'a>(bytes: &'a [u8], budget: &mut Budget) -> Result<(Zip<'a>, Vec<u8>), Error> {
    let zip = Zip::open(bytes)?;
    let entry = zip.find(MANIFEST).ok_or_else(|| malformed("the bundle has no manifest.json"))?;
    let max = budget.limits.max_json.min(budget.limits.max_entry);
    let json = zip.read(entry, max, budget, MANIFEST)?;
    Ok((zip, json))
}

/// The version and the writer, read without the rest of the manifest, so that a manifest of a
/// newer format still reports them.
#[derive(Deserialize)]
struct Stamp {
    format_version: u32,
    #[serde(default)]
    generator: Option<String>,
}

fn checked(version: u32) -> Result<u32, Error> {
    if version == 0 {
        return Err(malformed("format_version 0 is invalid"));
    }
    Ok(version)
}

/// The members of [`craft_pcraft_format::Manifest`], with the document as `D`, read without
/// building the manifest as a `serde_json::Value` first.
#[derive(Deserialize)]
struct Envelope<D> {
    format_version: u32,
    /// Required, and a string, as in the manifest; [`info`] reports it.
    #[serde(rename = "generator")]
    _generator: String,
    document: D,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    composite: Option<String>,
}

/// The manifest's envelope, checked as [`craft_pcraft_format::read_manifest`] checks it: the
/// size, nesting and item count with [`craft_pcraft_format::check_manifest`], a version of 0 or
/// newer than [`FORMAT_VERSION`] refused, and a preview that names an entry other than [`THUMB`]
/// or [`COMPOSITE`] refused. A member that appears twice is refused.
fn read<D: DeserializeOwned>(json: &[u8], budget: &Budget) -> Result<Envelope<D>, Error> {
    let mut limits = format::Limits::default();
    limits.max_manifest_bytes = json.len();
    limits.max_depth = budget.limits.max_depth.min(MAX_DEPTH) as usize;
    format::check_manifest(json, &limits).map_err(|e| match e {
        format::Error::LimitExceeded(m) => Error::LimitExceeded(m),
        e => malformed(e),
    })?;
    let m: Envelope<D> = parse_versioned(json, budget.limits, Kind::PhotoCraft)?;
    let version = checked(m.format_version)?;
    if version > FORMAT_VERSION {
        return Err(too_new(Kind::PhotoCraft, version));
    }
    for (member, entry, expected) in [("thumbnail", &m.thumbnail, THUMB), ("composite", &m.composite, COMPOSITE)] {
        if let Some(entry) = entry
            && entry != expected
        {
            return Err(malformed(format!("`{member}` names {entry:?}, not {expected:?}")));
        }
    }
    Ok(m)
}

pub(crate) fn info(bytes: &[u8], budget: &mut Budget) -> Result<(u32, Option<String>), Error> {
    let (_, json) = manifest(bytes, budget)?;
    let stamp: Stamp = parse(&json, budget.limits)?;
    Ok((checked(stamp.format_version)?, stamp.generator))
}

pub(crate) fn renditions(bytes: &[u8], budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let (zip, json) = manifest(bytes, budget)?;
    let m: Envelope<IgnoredAny> = read(&json, budget)?;
    let mut out = vec![];
    for (kind, path) in [(RenditionKind::Thumbnail, m.thumbnail), (RenditionKind::Composite, m.composite)] {
        let Some(path) = path else { continue };
        let entry = zip.find(&path).ok_or_else(|| malformed(format!("the manifest names `{path}`, which the bundle does not have")))?;
        let max = budget.limits.max_entry;
        let bytes = zip.read(entry, max, budget, &path)?;
        out.push(Rendition { kind, mime: "image/png".into(), bytes });
    }
    Ok(out)
}

#[derive(Deserialize)]
struct DocM {
    size: SizeM,
    resolution_dpi: f32,
    mode: String,
    depth: String,
    /// Bottom to top.
    #[serde(deserialize_with = "counted")]
    layers: Vec<LayerM>,
}

#[derive(Deserialize)]
struct SizeM {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct LayerM {
    name: String,
    visible: bool,
    blend: String,
    opacity: f32,
    content: ContentM,
}

#[derive(Deserialize)]
struct ContentM {
    kind: String,
    #[serde(default, deserialize_with = "counted")]
    children: Vec<LayerM>,
}

impl From<LayerM> for Layer {
    fn from(l: LayerM) -> Self {
        Layer {
            name: l.name,
            kind: l.content.kind,
            visible: l.visible,
            opacity: Some(l.opacity),
            blend: Some(l.blend),
            children: l.content.children.into_iter().map(Layer::from).collect(),
        }
    }
}

pub(crate) fn outline(bytes: &[u8], budget: &mut Budget) -> Result<Outline, Error> {
    let (_, json) = manifest(bytes, budget)?;
    let d = read::<DocM>(&json, budget)?.document;
    Ok(Outline {
        canvas: Some(Canvas { width: d.size.width, height: d.size.height, resolution_dpi: d.resolution_dpi, mode: d.mode, depth: d.depth }),
        pages: vec![],
        layers: d.layers.into_iter().map(Layer::from).collect(),
    })
}

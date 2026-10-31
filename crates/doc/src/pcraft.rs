//! PhotoCraft `.pcraft` bundles: a ZIP archive holding `manifest.json` (the document tree, with
//! `format_version`), tiles and blobs, and optionally the previews the manifest names
//! (`thumbnail`, `composite`). Only the ZIP form is read; a bundle saved as a folder is not.

use serde::Deserialize;

use crate::zip::{Zip, local_names_include};
use crate::{Budget, Canvas, Error, Kind, Layer, Outline, Rendition, RenditionKind, check_version, malformed, parse, parse_versioned};

/// The newest `format_version` read.
pub(crate) const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";

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

#[derive(Deserialize)]
struct Head {
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

pub(crate) fn info(bytes: &[u8], budget: &mut Budget) -> Result<(u32, Option<String>), Error> {
    let (_, json) = manifest(bytes, budget)?;
    let head: Head = parse(&json, budget.limits)?;
    Ok((checked(head.format_version)?, head.generator))
}

#[derive(Deserialize)]
struct Previews {
    format_version: u32,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    composite: Option<String>,
}

pub(crate) fn renditions(bytes: &[u8], budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let (zip, json) = manifest(bytes, budget)?;
    let m: Previews = parse_versioned(&json, budget.limits, Kind::PhotoCraft)?;
    check_version(Kind::PhotoCraft, checked(m.format_version)?)?;
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
struct Manifest {
    format_version: u32,
    document: DocM,
}

#[derive(Deserialize)]
struct DocM {
    size: SizeM,
    resolution_dpi: f32,
    mode: String,
    depth: String,
    /// Bottom to top.
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
    #[serde(default)]
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
    let m: Manifest = parse_versioned(&json, budget.limits, Kind::PhotoCraft)?;
    check_version(Kind::PhotoCraft, checked(m.format_version)?)?;
    let d = m.document;
    Ok(Outline {
        canvas: Some(Canvas { width: d.size.width, height: d.size.height, resolution_dpi: d.resolution_dpi, mode: d.mode, depth: d.depth }),
        pages: vec![],
        layers: d.layers.into_iter().map(Layer::from).collect(),
    })
}

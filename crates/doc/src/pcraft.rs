//! PhotoCraft `.pcraft` bundles: a ZIP archive holding `manifest.json` (the document tree, with
//! `format_version`), tiles and blobs, and optionally the previews the manifest names
//! (`thumbnail`, `composite`, `full_composite`). Only the ZIP form is read; a bundle saved as a folder is not. The
//! manifest's envelope is [`craft_pcraft_format::Manifest`]'s.

use std::fmt;
use std::marker::PhantomData;

use craft_pcraft_format::{self as format, COMPOSITE, FORMAT_VERSION, FULL_COMPOSITE, MANIFEST, MAX_GROUP_DEPTH, THUMB, manifest_depth};
use serde::Deserialize;
use serde::de::value::MapAccessDeserializer;
use serde::de::{DeserializeOwned, Deserializer, IgnoredAny, MapAccess, Visitor};

use crate::zip::{Zip, local_names_include};
use crate::{Budget, Canvas, Error, Kind, Layer, Outline, Rendition, RenditionKind, counted, lenient, malformed, parse, too_new};

/// The reader recognizes a ZIP archive with a `manifest.json` entry. When the archive opens,
/// [`Zip::find`] checks its central-directory names. Otherwise the reader checks the local headers
/// that `head` holds.
pub(crate) fn sniff(head: &[u8]) -> bool {
    sniff_checked(head, None).unwrap_or(false)
}

pub(crate) fn sniff_checked(head: &[u8], control: Option<&crate::cancel::Control<'_>>) -> Result<bool, Error> {
    if !head.starts_with(b"PK\x03\x04") {
        return Ok(false);
    }
    match Zip::open(head, control) {
        Ok(zip) => Ok(zip.find(MANIFEST, control)?.is_some()),
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Err(_) => local_names_include(head, MANIFEST, control),
    }
}

/// The archive and its `manifest.json`.
fn manifest<'a>(bytes: &'a [u8], budget: &mut Budget) -> Result<(Zip<'a>, Vec<u8>), Error> {
    let zip = Zip::open(bytes, budget.control)?;
    let entry = zip.find(MANIFEST, budget.control)?.ok_or_else(|| malformed("the bundle has no manifest.json"))?;
    let max = budget.limits.max_json.min(budget.limits.max_entry);
    let json = zip.read(entry, max, budget, MANIFEST)?;
    Ok((zip, json))
}

/// A `T` read only from a JSON object, as [`craft_pcraft_format::read_manifest`] reads the
/// manifest. serde's derived `Deserialize` also reads a struct from a JSON array.
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("the manifest's JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Object<T>, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(Object)
            }
        }
        d.deserialize_map(ObjectVisitor(PhantomData))
    }
}

/// The version and the writer, read without the rest of the manifest, so that a manifest of a
/// newer format still reports them. A `generator` that is not a string reads as `None`, as
/// [`craft_pcraft_format::migrate`] reads it.
#[derive(Deserialize)]
struct Stamp {
    /// `None` only when the member is missing; a `null` is not a version.
    #[serde(default, deserialize_with = "present")]
    format_version: Option<u32>,
    #[serde(default, deserialize_with = "lenient")]
    generator: Option<String>,
}

/// A member that is present, read as `T`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// The manifest's version and writer. A manifest without `format_version` is not PhotoCraft's:
/// [`Error::Unrecognized`]. A version of 0 is refused.
fn stamp(json: &[u8], budget: &Budget) -> Result<(u32, Option<String>), Error> {
    let Object(stamp) = parse::<Object<Stamp>>(json, budget)?;
    let version = stamp.format_version.ok_or(Error::Unrecognized)?;
    if version == 0 {
        return Err(malformed("format_version 0 is invalid"));
    }
    Ok((version, stamp.generator))
}

/// The members of [`craft_pcraft_format::Manifest`] after `format_version`, with the document as
/// `D`, read without building the manifest as a `serde_json::Value` first.
#[derive(Deserialize)]
struct Envelope<D> {
    /// Required, and a string, as in the manifest; [`info`] reports it.
    #[serde(rename = "generator")]
    _generator: String,
    document: D,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    composite: Option<String>,
    #[serde(default)]
    full_composite: Option<String>,
}

/// The manifest's envelope, checked as [`craft_pcraft_format::read_manifest`] checks it: the
/// size, nesting and item count with [`craft_pcraft_format::check_manifest`] and PhotoCraft's
/// own nesting limit, a manifest that is not a JSON object refused, the version read first (0 or
/// newer than [`FORMAT_VERSION`] refused), and a preview that names an entry other than [`THUMB`]
/// or [`COMPOSITE`] refused. `full_composite` may name only [`FULL_COMPOSITE`]. A member that
/// appears twice is refused.
fn read<D: DeserializeOwned>(json: &[u8], budget: &Budget) -> Result<Envelope<D>, Error> {
    read_with_items(json, budget, format::MAX_MANIFEST_ITEMS)
}

/// Check the envelope with a caller-supplied structural item cap before deserializing `D`.
fn read_with_items<D: DeserializeOwned>(json: &[u8], budget: &Budget, max_items: usize) -> Result<Envelope<D>, Error> {
    let mut limits = format::Limits::default();
    limits.max_manifest_bytes = json.len();
    limits.max_depth = (budget.limits.max_depth as usize).min(manifest_depth(MAX_GROUP_DEPTH));
    limits.max_items = max_items.min(format::MAX_MANIFEST_ITEMS);
    let checked = match budget.control {
        Some(control) => format::check_manifest_with_cancel(json, &limits, &|| control.check().is_err()),
        None => format::check_manifest(json, &limits),
    };
    checked.map_err(|e| match e {
        format::Error::Cancelled => Error::Cancelled,
        format::Error::LimitExceeded(m) => Error::LimitExceeded(m),
        e => malformed(e),
    })?;
    let (version, _) = stamp(json, budget)?;
    if version > FORMAT_VERSION {
        return Err(too_new(Kind::PhotoCraft, version));
    }
    budget.check_cancelled()?;
    let Object(m) = parse::<Object<Envelope<D>>>(json, budget)?;
    for (member, entry, expected) in
        [("thumbnail", &m.thumbnail, THUMB), ("composite", &m.composite, COMPOSITE), ("full_composite", &m.full_composite, FULL_COMPOSITE)]
    {
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
    stamp(&json, budget)
}

/// Extract declared previews, retaining whether the selected composite names the full member.
pub(crate) fn renditions(bytes: &[u8], wanted: Option<RenditionKind>, budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let (zip, json) = manifest(bytes, budget)?;
    let m: Envelope<IgnoredAny> = read(&json, budget)?;
    let composite_is_full = m.full_composite.is_some();
    let mut out = vec![];
    for (kind, path) in [(RenditionKind::Thumbnail, m.thumbnail), (RenditionKind::Composite, m.full_composite.or(m.composite))] {
        if wanted.is_some_and(|wanted| wanted != kind) {
            continue;
        }
        let Some(path) = path else { continue };
        let entry =
            zip.find(&path, budget.control)?.ok_or_else(|| malformed(format!("the manifest names `{path}`, which the bundle does not have")))?;
        let max = budget.limits.max_entry;
        let bytes = zip.read(entry, max, budget, &path)?;
        out.push(Rendition { kind, mime: "image/png".into(), bytes, is_full_composite: kind == RenditionKind::Composite && composite_is_full });
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

fn layer(l: LayerM, budget: &Budget) -> Result<Layer, Error> {
    budget.check_cancelled()?;
    Ok(Layer {
        name: l.name,
        kind: l.content.kind,
        visible: l.visible,
        opacity: Some(l.opacity),
        blend: Some(l.blend),
        children: l.content.children.into_iter().map(|l| layer(l, budget)).collect::<Result<_, _>>()?,
    })
}

pub(crate) fn outline(bytes: &[u8], budget: &mut Budget) -> Result<Outline, Error> {
    let (_, json) = manifest(bytes, budget)?;
    let d = read::<DocM>(&json, budget)?.document;
    document_outline(d, budget)
}

fn document_outline(d: DocM, budget: &Budget) -> Result<Outline, Error> {
    Ok(Outline {
        canvas: Some(Canvas { width: d.size.width, height: d.size.height, resolution_dpi: d.resolution_dpi, mode: d.mode, depth: d.depth }),
        pages: vec![],
        layers: d.layers.into_iter().map(|l| layer(l, budget)).collect::<Result<_, _>>()?,
    })
}

/// The archive and a document head, with structural items checked before deserialization.
#[cfg(feature = "pcraft-pixels")]
pub(crate) fn document_with_items<'a, D: DeserializeOwned>(bytes: &'a [u8], budget: &mut Budget) -> Result<(Zip<'a>, D), Error> {
    let (zip, json) = manifest(bytes, budget)?;
    let max_items = usize::try_from(budget.limits.max_items).unwrap_or(usize::MAX);
    Ok((zip, read_with_items::<D>(&json, budget, max_items)?.document))
}

/// The archive and the manifest's document read as `D`, checked as [`outline`] reads it.
#[cfg(feature = "pcraft-pixels")]
pub(crate) fn document<'a, D: DeserializeOwned>(bytes: &'a [u8], budget: &mut Budget) -> Result<(Zip<'a>, D), Error> {
    let (zip, json) = manifest(bytes, budget)?;
    let document = read::<D>(&json, budget)?.document;
    Ok((zip, document))
}

/// The caller's document head, with its structural item limit checked before deserialization.
pub(crate) fn head<D: DeserializeOwned>(bytes: &[u8], budget: &mut Budget) -> Result<D, Error> {
    let (_, json) = manifest(bytes, budget)?;
    let max_items = usize::try_from(budget.limits.max_items).unwrap_or(usize::MAX);
    Ok(read_with_items::<D>(&json, budget, max_items)?.document)
}

#[cfg(test)]
mod cancellation_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::cell::Cell;

    #[test]
    fn conversion_stops_inside_group_children() {
        let child = r#"{"name":"child","visible":true,"blend":"Normal","opacity":1,"content":{"kind":"Raster"}}"#;
        let parent = format!(
            r#"{{"name":"group","visible":true,"blend":"Normal","opacity":1,"content":{{"kind":"Group","children":[{child},{child},{child},{child},{child}]}}}}"#
        );
        let node: LayerM = serde_json::from_str(&parent).unwrap();
        let polls = Cell::new(0);
        let cancel = || {
            polls.set(polls.get() + 1);
            polls.get() == 4
        };
        let result = crate::cancel::run(&cancel, |control| layer(node, &Budget::controlled(&crate::Limits::default(), control)));
        assert_eq!(result, Err(Error::Cancelled));
        assert_eq!(polls.get(), 4);
    }
}

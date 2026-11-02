//! VectorCraft `.vectorcraft` documents: one JSON object, gzip-compressed when saved with Use
//! Compression:
//!
//! ```text
//! { "format": "vectorcraft", "version": 4, "generator": "VectorCraft 0.1.0",
//!   "preview": { "mime": "image/png", "data": "<base64>" },
//!   "pdf": { "mime": "application/pdf", "blob": [offset, length] },
//!   "document": { "artboards": [...], "layers": [...], ... }, ... }
//! ```
//!
//! Since version 4, large members may follow the JSON after the marker `\n\0VCBLOBS\0`; such a
//! member gives `"blob": [offset, length]` (from the end of the marker) instead of `data`. In a
//! compressed file, the marker and the data after it are inside the gzip stream too. Files
//! written before the rename say `"format": "drawcraft"`. The envelope's members are those of
//! [`craft_vectorcraft_format::File`], read as [`craft_vectorcraft_format::File::read`] reads
//! them, and [`craft_vectorcraft_format::Image`].

use std::borrow::Cow;
use std::fmt;
use std::io::Read;

use craft_vectorcraft_format::{BLOB_MAGIC, Image, entry, is_compressed, marker_at};
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};

use crate::{Budget, Error, Kind, Layer, Lenient, Outline, Page, Rendition, RenditionKind, counted, malformed, parse, parse_versioned, too_new};

/// Bytes decompressed at a time.
const CHUNK: u64 = 64 << 10;

pub(crate) fn sniff(head: &[u8]) -> bool {
    craft_vectorcraft_format::sniff(head)
}

/// Where the JSON ends and the data after the marker starts, when `text` holds a NUL. JSON holds
/// no raw NUL and the marker's second byte is one, so the JSON ends right before the first NUL.
/// The marker must follow the JSON's closing `}`, `]` or `"`, as
/// [`craft_vectorcraft_format::marker_at`] requires; any other NUL makes the file malformed.
fn marker(text: &[u8]) -> Result<(usize, usize), Error> {
    marker_at(text).map(|at| (at, at + BLOB_MAGIC.len())).ok_or_else(|| malformed("the file holds a NUL byte outside the marker after its JSON"))
}

fn damaged(e: std::io::Error) -> Error {
    malformed(format!("damaged compressed data: {e}"))
}

/// A file's JSON, and the data after it.
struct Unpacked<'a> {
    json: Cow<'a, [u8]>,
    tail: Tail<'a>,
}

/// The data after the marker.
enum Tail<'a> {
    /// In an uncompressed file.
    Slice(&'a [u8]),
    /// Still in the gzip stream: `read` holds what was decompressed past the marker already, not
    /// yet counted against the budget.
    Stream { dec: Box<GzDecoder<&'a [u8]>>, read: Vec<u8> },
}

impl Tail<'_> {
    /// The bytes each `[offset, length]` in `ranges` names, or `None` for a range the file ends
    /// before. A compressed file is decompressed only as far as the last range ends, and that
    /// offset is checked against the budget before anything past the marker is decompressed.
    /// Every byte decompressed is counted; only the bytes inside the ranges are kept.
    fn ranges(self, ranges: &[[u64; 2]], budget: &mut Budget) -> Result<Vec<Option<Vec<u8>>>, Error> {
        const WHAT: &str = "the decompressed document";
        let (mut dec, read) = match self {
            Tail::Slice(s) => return Ok(ranges.iter().map(|&r| entry(s, r).map(<[u8]>::to_vec)).collect()),
            Tail::Stream { dec, read } => (dec, read),
        };
        let end = ranges.iter().map(|&[offset, len]| offset.saturating_add(len)).max().unwrap_or(0);
        if end > budget.left() {
            return Err(Error::LimitExceeded(format!(
                "the file states data up to byte {end} after its JSON, more than max_total ({} bytes) in all",
                budget.limits.max_total
            )));
        }
        // The ranges in file order, overlapping ones merged.
        let mut spans: Vec<[u64; 2]> = ranges.iter().map(|&[offset, len]| [offset, offset.saturating_add(len)]).collect();
        spans.sort_unstable();
        let mut merged: Vec<[u64; 2]> = vec![];
        for [start, end] in spans {
            match merged.last_mut() {
                Some(last) if start <= last[1] => last[1] = last[1].max(end),
                _ => merged.push([start, end]),
            }
        }
        let mut src = read.as_slice().chain(&mut *dec);
        let (mut at, mut kept) = (0u64, Vec::<(u64, Vec<u8>)>::new());
        for [start, end] in merged {
            let skipped = std::io::copy(&mut (&mut src).take(start.saturating_sub(at)), &mut std::io::sink()).map_err(damaged)?;
            budget.take(skipped, WHAT)?;
            at += skipped;
            if at < start {
                break;
            }
            let mut bytes = vec![];
            let n = (&mut src).take(end - start).read_to_end(&mut bytes).map_err(damaged)?;
            budget.take(n as u64, WHAT)?;
            at += n as u64;
            kept.push((start, bytes));
            if at < end {
                break;
            }
        }
        Ok(ranges
            .iter()
            .map(|&[offset, len]| kept.iter().find_map(|(start, bytes)| entry(bytes, [offset.checked_sub(*start)?, len])).map(<[u8]>::to_vec))
            .collect())
    }
}

/// A file's JSON and where the data after it is. A compressed file is decompressed only as far
/// as the end of the marker, every byte used counted against the budget, and its JSON is
/// checked against `max_json` as it grows.
fn unpack<'a>(bytes: &'a [u8], budget: &mut Budget) -> Result<Unpacked<'a>, Error> {
    const JSON: &str = "the document's JSON";
    let too_long = |budget: &Budget| Error::LimitExceeded(format!("{JSON} is more than max_json ({} bytes)", budget.limits.max_json));
    if !is_compressed(bytes) {
        let (json, tail) = if bytes.contains(&0) {
            let (end, tail) = marker(bytes)?;
            (bytes.get(..end).unwrap_or_default(), bytes.get(tail..).unwrap_or_default())
        } else {
            (bytes, &[][..])
        };
        budget.check_json(json.len() as u64, JSON)?;
        return Ok(Unpacked { json: Cow::Borrowed(json), tail: Tail::Slice(tail) });
    }
    let mut dec = Box::new(GzDecoder::new(bytes));
    let mut buf = Vec::new();
    let mut split: Option<(usize, usize)> = None;
    let mut nul: Option<usize> = None;
    loop {
        let start = buf.len();
        let want = CHUNK.min(budget.left().saturating_add(1));
        let n = (&mut *dec).take(want).read_to_end(&mut buf).map_err(damaged)?;
        // The JSON ends before the first NUL: until there is one, all of it is JSON. The marker
        // may end in a later chunk than its first NUL.
        if nul.is_none() {
            nul = buf.get(start..).and_then(|b| b.iter().position(|&c| c == 0)).map(|p| p + start);
        }
        if nul.unwrap_or(buf.len()).saturating_sub(1) as u64 > budget.limits.max_json {
            return Err(too_long(budget));
        }
        let mut used = n;
        if let Some(nul) = nul
            && buf.len() >= nul + BLOB_MAGIC.len() - 1
        {
            let s = marker(&buf)?;
            split = Some(s);
            // The data after the marker in this chunk is counted when it is read.
            used = s.1.saturating_sub(start).min(n);
        }
        budget.take(used as u64, "the decompressed document")?;
        if (n as u64) < want || split.is_some() {
            break;
        }
    }
    let (json_len, tail_at) = match (split, nul) {
        (Some(s), _) => s,
        // The stream ended inside the marker.
        (None, Some(_)) => marker(&buf)?,
        (None, None) => (buf.len(), buf.len()),
    };
    budget.check_json(json_len as u64, JSON)?;
    let read = if tail_at <= buf.len() { buf.split_off(tail_at) } else { vec![] };
    buf.truncate(json_len);
    Ok(Unpacked { json: Cow::Owned(buf), tail: Tail::Stream { dec, read } })
}

/// The format name, the version, the writer and, when `IMAGES` is true, the preview and the PDF,
/// read without the rest of the file, so that a file of a newer version still reports them. The
/// members are read as [`craft_vectorcraft_format::File::read`] reads them: a missing or repeated
/// `format` or `version` is refused, and a `generator`, `preview` or `pdf` of the wrong shape
/// reads as `None` and a repeated one as its last value.
struct Stamp<const IMAGES: bool> {
    format: String,
    version: u32,
    generator: Option<String>,
    preview: Option<Image>,
    pdf: Option<Image>,
}

/// Reads the value of member `name` into `slot`, refusing a second one.
fn once<'de, A: MapAccess<'de>, T: Deserialize<'de>>(map: &mut A, slot: &mut Option<T>, name: &'static str) -> Result<(), A::Error> {
    if slot.is_some() {
        return Err(de::Error::duplicate_field(name));
    }
    *slot = Some(map.next_value()?);
    Ok(())
}

impl<'de, const IMAGES: bool> Deserialize<'de> for Stamp<IMAGES> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct StampVisitor<const IMAGES: bool>;
        impl<'de, const IMAGES: bool> Visitor<'de> for StampVisitor<IMAGES> {
            type Value = Stamp<IMAGES>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a VectorCraft file's JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Stamp<IMAGES>, A::Error> {
                let (mut format, mut version, mut generator, mut preview, mut pdf) = (None, None, None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => once(&mut map, &mut format, "format")?,
                        "version" => once(&mut map, &mut version, "version")?,
                        "generator" => generator = map.next_value::<Lenient<String>>()?.0,
                        "preview" if IMAGES => preview = map.next_value::<Lenient<Image>>()?.0,
                        "pdf" if IMAGES => pdf = map.next_value::<Lenient<Image>>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(Stamp {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    generator,
                    preview,
                    pdf,
                })
            }
        }
        d.deserialize_map(StampVisitor::<IMAGES>)
    }
}

/// The file's [`Stamp`], refused when the format is not VectorCraft's, and when the version is
/// newer than this crate reads unless `newer` is true.
fn stamp<const IMAGES: bool>(json: &[u8], budget: &Budget, newer: bool) -> Result<Stamp<IMAGES>, Error> {
    let s: Stamp<IMAGES> = if newer { parse(json, budget.limits)? } else { parse_versioned(json, budget.limits, Kind::VectorCraft)? };
    match craft_vectorcraft_format::check(&s.format, s.version) {
        Ok(()) => Ok(s),
        Err(craft_vectorcraft_format::Error::Format(format)) => Err(malformed(format!("format is `{format}`"))),
        Err(craft_vectorcraft_format::Error::TooNew(_)) if newer => Ok(s),
        Err(craft_vectorcraft_format::Error::TooNew(version)) => Err(too_new(Kind::VectorCraft, version)),
        Err(e) => Err(malformed(e)),
    }
}

pub(crate) fn info(bytes: &[u8], budget: &mut Budget) -> Result<(u32, Option<String>), Error> {
    let json = unpack(bytes, budget)?.json;
    let s = stamp::<false>(&json, budget, true)?;
    Ok((s.version, s.generator))
}

pub(crate) fn renditions(bytes: &[u8], budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let Unpacked { json, tail } = unpack(bytes, budget)?;
    let head = stamp::<true>(&json, budget, false)?;
    // A member without a MIME type gives no rendition: a preview may be PNG or JPEG.
    let members: Vec<_> = [(RenditionKind::Thumbnail, head.preview, "preview"), (RenditionKind::Pdf, head.pdf, "PDF")]
        .into_iter()
        .filter_map(|(kind, m, what)| Some((kind, m.filter(|m| !m.mime.is_empty())?, what)))
        .collect();
    // Every size is checked before any data after the JSON is decompressed.
    for (_, m, what) in &members {
        budget.take_entry(m.len(), what)?;
    }
    let ranges: Vec<[u64; 2]> = members.iter().filter_map(|(_, m, _)| m.blob).collect();
    let mut blobs = if ranges.is_empty() { vec![] } else { tail.ranges(&ranges, budget)? }.into_iter();
    members
        .into_iter()
        .map(|(kind, m, what)| {
            let bytes = match m.blob {
                Some(_) => blobs.next().flatten().ok_or_else(|| malformed(format!("the file ends before its {what}")))?,
                None => m.bytes_at_most(&[], u64::MAX).ok_or_else(|| malformed(format!("the {what} is not valid base64")))?,
            };
            Ok(Rendition { kind, mime: m.mime, bytes })
        })
        .collect()
}

#[derive(Deserialize)]
struct WithDocument {
    document: DocM,
}

#[derive(Deserialize)]
struct DocM {
    #[serde(default, deserialize_with = "counted")]
    artboards: Vec<ArtboardM>,
    /// Bottom first.
    #[serde(default, deserialize_with = "counted")]
    layers: Vec<NodeM>,
}

#[derive(Deserialize)]
struct ArtboardM {
    #[serde(default)]
    name: String,
    rect: RectM,
}

#[derive(Deserialize)]
struct RectM {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// A node, written without the fields at their defaults.
#[derive(Deserialize)]
struct NodeM {
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    blend: Option<String>,
    kind: KindM,
}

#[derive(Deserialize)]
struct KindM {
    #[serde(rename = "type")]
    kind: String,
}

pub(crate) fn outline(bytes: &[u8], budget: &mut Budget) -> Result<Outline, Error> {
    let json = unpack(bytes, budget)?.json;
    stamp::<false>(&json, budget, false)?;
    let f: WithDocument = parse(&json, budget.limits)?;
    let pages = f
        .document
        .artboards
        .into_iter()
        .map(|a| Page {
            name: a.name,
            x: a.rect.x0.min(a.rect.x1),
            y: a.rect.y0.min(a.rect.y1),
            width: (a.rect.x1 - a.rect.x0).abs(),
            height: (a.rect.y1 - a.rect.y0).abs(),
            timing: None,
            layers: vec![],
        })
        .collect();
    let layers = f
        .document
        .layers
        .into_iter()
        .map(|n| Layer {
            name: n.name.unwrap_or_default(),
            kind: n.kind.kind,
            visible: n.visible,
            opacity: Some(n.opacity),
            blend: Some(n.blend.unwrap_or_else(|| "Normal".into())),
            children: vec![],
        })
        .collect();
    Ok(Outline { canvas: None, pages, layers })
}

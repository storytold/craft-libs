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
//! written before the rename say `"format": "drawcraft"`.

use std::borrow::Cow;
use std::io::Read;

use flate2::read::GzDecoder;
use serde::Deserialize;

use crate::{Budget, Error, Kind, Layer, Outline, Page, Rendition, RenditionKind, b64, check_version, malformed, parse, parse_versioned};

/// The newest `version` read.
pub(crate) const VERSION: u32 = 4;
const BLOB_MAGIC: &[u8] = b"\n\0VCBLOBS\0";
/// Bytes decompressed at a time.
const CHUNK: u64 = 64 << 10;

fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

pub(crate) fn sniff(head: &[u8]) -> bool {
    const HEAD: u64 = 256;
    let head: Cow<'_, [u8]> = if is_gzip(head) {
        let mut out = Vec::with_capacity(HEAD as usize);
        // A head cut short still decompresses its start.
        let _ = GzDecoder::new(head).take(HEAD).read_to_end(&mut out);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(head.get(..HEAD as usize).unwrap_or(head))
    };
    head.trim_ascii_start().starts_with(b"{") && format_member(&head).is_some_and(|f| f == b"vectorcraft" || f == b"drawcraft")
}

/// The string value of the first `"format"` member in `text`.
fn format_member(text: &[u8]) -> Option<&[u8]> {
    const KEY: &[u8] = b"\"format\"";
    let at = text.windows(KEY.len()).position(|w| w == KEY)?;
    let rest = text.get(at + KEY.len()..)?.trim_ascii_start().strip_prefix(b":")?.trim_ascii_start().strip_prefix(b"\"")?;
    rest.get(..rest.iter().position(|&b| b == b'"')?)
}

/// Where the JSON ends and the data after the marker starts, when `text` holds a NUL. JSON holds
/// no raw NUL and the marker's second byte is one, so the JSON ends right before the first NUL;
/// a NUL that is not the marker's makes the file malformed.
fn marker(text: &[u8], nul: usize) -> Result<(usize, usize), Error> {
    match nul.checked_sub(1) {
        Some(at) if text.get(at..).is_some_and(|r| r.starts_with(BLOB_MAGIC)) => Ok((at, at + BLOB_MAGIC.len())),
        _ => Err(malformed("the file holds a NUL byte outside the marker after its JSON")),
    }
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
    /// Still in the gzip stream: `read` holds what was decompressed past the marker already,
    /// of which `counted` bytes are counted against the budget.
    Stream { dec: Box<GzDecoder<&'a [u8]>>, read: Vec<u8>, counted: usize },
}

impl Tail<'_> {
    /// The data up to `end` (less where the file ends first), decompressing only that far.
    fn up_to(&mut self, end: u64, budget: &mut Budget) -> Result<&[u8], Error> {
        match self {
            Tail::Slice(s) => Ok(s),
            Tail::Stream { dec, read, counted } => {
                let end = usize::try_from(end).map_err(|_| Error::LimitExceeded("the data after the JSON does not fit in memory".into()))?;
                let had = read.len().min(end);
                if had > *counted {
                    budget.take((had - *counted) as u64, "the decompressed document")?;
                    *counted = had;
                }
                if read.len() < end {
                    let want = ((end - read.len()) as u64).min(budget.left().saturating_add(1));
                    let n = (&mut **dec).take(want).read_to_end(read).map_err(damaged)?;
                    budget.take(n as u64, "the decompressed document")?;
                    *counted += n;
                }
                Ok(read.as_slice())
            }
        }
    }
}

/// A file's JSON and where the data after it is. A compressed file is decompressed only as far
/// as the end of the marker, every byte used counted against the budget, and its JSON is
/// checked against `max_json` as it grows.
fn unpack<'a>(bytes: &'a [u8], budget: &mut Budget) -> Result<Unpacked<'a>, Error> {
    const JSON: &str = "the document's JSON";
    let too_long = |budget: &Budget| Error::LimitExceeded(format!("{JSON} is more than max_json ({} bytes)", budget.limits.max_json));
    if !is_gzip(bytes) {
        let (json, tail) = match bytes.iter().position(|&b| b == 0) {
            Some(nul) => {
                let (end, tail) = marker(bytes, nul)?;
                (bytes.get(..end).unwrap_or_default(), bytes.get(tail..).unwrap_or_default())
            }
            None => (bytes, &[][..]),
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
            let s = marker(&buf, nul)?;
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
        (None, Some(nul)) => marker(&buf, nul)?,
        (None, None) => (buf.len(), buf.len()),
    };
    budget.check_json(json_len as u64, JSON)?;
    let read = if tail_at <= buf.len() { buf.split_off(tail_at) } else { vec![] };
    buf.truncate(json_len);
    Ok(Unpacked { json: Cow::Owned(buf), tail: Tail::Stream { dec, read, counted: 0 } })
}

fn check_format(format: &str) -> Result<(), Error> {
    if format == "vectorcraft" || format == "drawcraft" { Ok(()) } else { Err(malformed(format!("format is `{format}`"))) }
}

#[derive(Deserialize)]
struct Head {
    format: String,
    version: u32,
    #[serde(default)]
    generator: Option<String>,
}

pub(crate) fn info(bytes: &[u8], budget: &mut Budget) -> Result<(u32, Option<String>), Error> {
    let json = unpack(bytes, budget)?.json;
    let head: Head = parse(&json, budget.limits)?;
    check_format(&head.format)?;
    Ok((head.version, head.generator))
}

/// A member holding bytes: inline as base64 `data`, or after the JSON at `blob`.
#[derive(Deserialize)]
struct Member {
    #[serde(default)]
    mime: String,
    #[serde(default)]
    data: String,
    #[serde(default)]
    blob: Option<[u64; 2]>,
}

impl Member {
    /// Check the member's size against `max_entry` and count it; for a member after the JSON,
    /// the offset where it ends.
    fn take(&self, budget: &mut Budget, what: &str) -> Result<Option<u64>, Error> {
        match self.blob {
            Some([offset, len]) => {
                budget.take_entry(len, what)?;
                Ok(Some(offset.saturating_add(len)))
            }
            None => {
                budget.take_entry(b64::decoded_len(&self.data), what)?;
                Ok(None)
            }
        }
    }

    /// The member's bytes, after [`Member::take`].
    fn bytes(&self, tail: &[u8], what: &str) -> Result<Vec<u8>, Error> {
        match self.blob {
            Some([offset, len]) => {
                let range = usize::try_from(offset).ok().zip(usize::try_from(len).ok()).and_then(|(a, n)| Some(a..a.checked_add(n)?));
                range.and_then(|r| tail.get(r)).map(<[u8]>::to_vec).ok_or_else(|| malformed(format!("the file ends before its {what}")))
            }
            None => b64::decode(&self.data).ok_or_else(|| malformed(format!("the {what} is not valid base64"))),
        }
    }
}

#[derive(Deserialize)]
struct WithMembers {
    format: String,
    version: u32,
    #[serde(default)]
    preview: Option<Member>,
    #[serde(default)]
    pdf: Option<Member>,
}

pub(crate) fn renditions(bytes: &[u8], budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let Unpacked { json, mut tail } = unpack(bytes, budget)?;
    let f: WithMembers = parse_versioned(&json, budget.limits, Kind::VectorCraft)?;
    check_format(&f.format)?;
    check_version(Kind::VectorCraft, f.version)?;
    let members: Vec<_> = [(RenditionKind::Thumbnail, f.preview, "preview", "image/png"), (RenditionKind::Pdf, f.pdf, "PDF", "application/pdf")]
        .into_iter()
        .filter_map(|(kind, m, what, mime)| Some((kind, m?, what, mime)))
        .collect();
    // Every size is checked before any data after the JSON is decompressed.
    let mut end = 0u64;
    for (_, m, what, _) in &members {
        end = end.max(m.take(budget, what)?.unwrap_or(0));
    }
    let tail = if end > 0 { tail.up_to(end, budget)? } else { &[] };
    members
        .into_iter()
        .map(|(kind, m, what, mime)| {
            let bytes = m.bytes(tail, what)?;
            let mime = if m.mime.is_empty() { mime.to_string() } else { m.mime };
            Ok(Rendition { kind, mime, bytes })
        })
        .collect()
}

#[derive(Deserialize)]
struct WithDocument {
    format: String,
    version: u32,
    document: DocM,
}

#[derive(Deserialize)]
struct DocM {
    #[serde(default)]
    artboards: Vec<ArtboardM>,
    /// Bottom first.
    #[serde(default)]
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
    let f: WithDocument = parse_versioned(&json, budget.limits, Kind::VectorCraft)?;
    check_format(&f.format)?;
    check_version(Kind::VectorCraft, f.version)?;
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

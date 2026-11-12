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
use std::marker::PhantomData;

use craft_vectorcraft_format::{BLOB_MAGIC, Image, entry, is_compressed, marker_at};
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::cancel::Reader;
use crate::{
    Budget, Error, Kind, Layer, Lenient, Outline, Page, Rendition, RenditionKind, Scalar, Shaped, counted, malformed, parse, parse_versioned, too_new,
};

/// Bytes decompressed at a time.
const CHUNK: u64 = 64 << 10;

pub(crate) fn sniff(head: &[u8]) -> bool {
    if craft_vectorcraft_format::sniff(head) {
        return true;
    }
    if !is_compressed(head) {
        return false;
    }
    // A decoder can inspect broken compressed data past the requested output. Retry the
    // bounded sniff with one compressed byte at a time, retaining any intact prefix.
    let mut unpacked = Vec::new();
    let head = head.get(..64 * 1024).unwrap_or(head);
    let _ = gzip_decoder(head, 1, None).take(256).read_to_end(&mut unpacked);
    !is_compressed(&unpacked) && craft_vectorcraft_format::sniff(&unpacked)
}

struct GzipInput<'a> {
    bytes: &'a [u8],
    step: usize,
}

impl Read for GzipInput<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let len = out.len().min(self.step);
        self.bytes.read(&mut out[..len])
    }
}

fn gzip_decoder<'a, 'c>(bytes: &'a [u8], step: usize, control: Option<&'c crate::cancel::Control<'c>>) -> GzDecoder<Reader<'c, GzipInput<'a>>> {
    GzDecoder::new(Reader::new(GzipInput { bytes, step }, control))
}

pub(crate) fn sniff_checked(head: &[u8], control: Option<&crate::cancel::Control<'_>>) -> Result<bool, Error> {
    if control.is_none() || !is_compressed(head) {
        return Ok(sniff(head));
    }
    // Match the format crate's 256-byte probe, including truncated gzip input. Bound input
    // reads too: optional gzip header fields can exceed the decoded probe's length.
    let mut prefix = Vec::new();
    let _ = GzDecoder::new(Reader::new(head, control)).take(256).read_to_end(&mut prefix);
    crate::cancel::check(control)?;
    if !is_compressed(&prefix) && sniff(&prefix) {
        return Ok(true);
    }
    prefix.clear();
    let head = head.get(..64 * 1024).unwrap_or(head);
    let _ = gzip_decoder(head, 1, control).take(256).read_to_end(&mut prefix);
    crate::cancel::check(control)?;
    Ok(!is_compressed(&prefix) && sniff(&prefix))
}

/// Where the JSON ends and the data after the marker starts, when `text` holds a NUL. JSON holds
/// no raw NUL and the marker's second byte is one, so the JSON ends right before the first NUL.
/// The marker must follow the JSON's closing `}`, `]` or `"`, as
/// [`craft_vectorcraft_format::marker_at`] requires; any other NUL makes the file malformed.
fn marker(text: &[u8], nul: usize) -> Result<(usize, usize), Error> {
    // Start at the byte before the marker so the shared check inspects a bounded suffix,
    // rather than scanning the already searched JSON again.
    let found = nul.checked_sub(2).and_then(|start| marker_at(text.get(start..)?).map(|at| start + at));
    found.map(|at| (at, at + BLOB_MAGIC.len())).ok_or_else(|| malformed("the file holds a NUL byte outside the marker after its JSON"))
}

fn first_nul(bytes: &[u8], budget: &Budget) -> Result<Option<usize>, Error> {
    for (i, chunk) in bytes.chunks(crate::cancel::CHUNK).enumerate() {
        budget.check_cancelled()?;
        if chunk.contains(&0) {
            return Ok(chunk.iter().position(|&b| b == 0).map(|at| i * crate::cancel::CHUNK + at));
        }
    }
    Ok(None)
}

fn damaged(e: std::io::Error) -> Error {
    malformed(format!("damaged compressed data: {e}"))
}

/// A file's JSON, and the data after it.
struct Unpacked<'a, 'c> {
    json: Cow<'a, [u8]>,
    tail: Tail<'a, 'c>,
}

/// The data after the marker.
enum Tail<'a, 'c> {
    /// In an uncompressed file.
    Slice(&'a [u8]),
    /// Still in the gzip stream: `read` holds what was decompressed past the marker already, not
    /// yet counted against the budget.
    Stream { dec: Box<GzDecoder<Reader<'c, GzipInput<'a>>>>, read: Vec<u8> },
}

impl Tail<'_, '_> {
    /// Each `[offset, length]` in `ranges` gives its bytes, or `None` if the file ends before it.
    /// The final offset and duplicate-copy cost are checked against the remaining budget before
    /// reading the tail. Bytes consumed from the tail and additional copies count against the
    /// budget. Only bytes inside the ranges are kept.
    fn ranges(self, ranges: &[[u64; 2]], budget: &mut Budget) -> Result<Vec<Option<Vec<u8>>>, Error> {
        const WHAT: &str = "the decompressed document";
        let (mut dec, read) = match self {
            Tail::Slice(s) => return ranges.iter().map(|&r| entry(s, r).map(|bytes| budget.copy(bytes)).transpose()).collect(),
            Tail::Stream { dec, read } => (dec, read),
        };
        let end = ranges.iter().map(|&[offset, len]| offset.saturating_add(len)).max().unwrap_or(0);
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
        // Each range is returned as its own copy: bytes that more than one range names are
        // decompressed once and counted again here for each further copy.
        let over_budget = || Error::LimitExceeded(format!("{WHAT}: more than max_total ({} bytes) in all", budget.limits.max_total));
        let stated = ranges.iter().try_fold(0u64, |n, &[_, len]| n.checked_add(len)).ok_or_else(over_budget)?;
        let spanned = merged.iter().fold(0u64, |n, &[start, end]| n.saturating_add(end - start));
        let copies = stated.saturating_sub(spanned);
        if end.checked_add(copies).is_none_or(|required| required > budget.left()) {
            return Err(over_budget());
        }
        budget.take(copies, WHAT)?;
        let mut src = Reader::new(read.as_slice().chain(&mut *dec), budget.control);
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
        ranges
            .iter()
            .map(|&[offset, len]| {
                kept.iter().find_map(|(start, bytes)| entry(bytes, [offset.checked_sub(*start)?, len])).map(|bytes| budget.copy(bytes)).transpose()
            })
            .collect()
    }
}

/// The reader extracts a file's JSON. For gzip input, it buffers any tail bytes in the same chunk of up to 64 KiB.
/// JSON and marker bytes count against the budget. The JSON is checked against `max_json` as it grows.
fn unpack<'a, 'c>(bytes: &'a [u8], budget: &mut Budget<'c>) -> Result<Unpacked<'a, 'c>, Error> {
    const JSON: &str = "the document's JSON";
    let too_long = |budget: &Budget| Error::LimitExceeded(format!("{JSON} is more than max_json ({} bytes)", budget.limits.max_json));
    if !is_compressed(bytes) {
        let (json, tail) = if let Some(nul) = first_nul(bytes, budget)? {
            let (end, tail) = marker(bytes, nul)?;
            (bytes.get(..end).unwrap_or_default(), bytes.get(tail..).unwrap_or_default())
        } else {
            (bytes, &[][..])
        };
        budget.check_json(json.len() as u64, JSON)?;
        return Ok(Unpacked { json: Cow::Borrowed(json), tail: Tail::Slice(tail) });
    }
    let mut dec = Box::new(gzip_decoder(bytes, usize::MAX, budget.control));
    let mut step = CHUNK;
    let mut buf = Vec::new();
    let mut split: Option<(usize, usize)> = None;
    let mut nul: Option<usize> = None;
    loop {
        let start = buf.len();
        let want = step.min(budget.left().saturating_add(1));
        let n = match Reader::new(&mut *dec, budget.control).take(want).read_to_end(&mut buf) {
            Ok(n) => n,
            Err(_) if step > 1 => {
                budget.check_cancelled()?;
                // Keep the fast path for intact files. On damage, replay the already-counted
                // prefix and read only through the marker, with no compressed read-ahead.
                let mut retry = Box::new(gzip_decoder(bytes, 1, budget.control));
                let skipped =
                    std::io::copy(&mut Reader::new(&mut *retry, budget.control).take(start as u64), &mut std::io::sink()).map_err(damaged)?;
                if skipped != start as u64 {
                    return Err(malformed("the compressed document ends before its previously read prefix"));
                }
                dec = retry;
                buf.truncate(start);
                step = 1;
                continue;
            }
            Err(e) => return Err(damaged(e)),
        };
        // The JSON ends before the first NUL: until there is one, all of it is JSON. The marker
        // may end in a later chunk than its first NUL.
        if nul.is_none() {
            nul = first_nul(buf.get(start..).unwrap_or_default(), budget)?.map(|p| p + start);
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
    Ok(Unpacked { json: Cow::Owned(buf), tail: Tail::Stream { dec, read } })
}

/// A value of type `Self` that is a JSON array or object, read by [`ContainerVisitor`] as it is
/// parsed; any other value has the wrong shape, apart from `null` where [`Container::null`] takes
/// it.
trait Container: Sized {
    fn seq<'de, A: SeqAccess<'de>>(seq: A) -> Result<Option<Self>, A::Error>;
    fn map<'de, A: MapAccess<'de>>(map: A) -> Result<Option<Self>, A::Error>;
    fn null() -> Option<Self> {
        None
    }
}

/// Reads a [`Container`], or `None` when the value has the wrong shape.
struct ContainerVisitor<T>(PhantomData<T>);

impl<'de, T: Container> Visitor<'de> for ContainerVisitor<T> {
    type Value = Option<T>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Option<T>, E> {
        Ok(T::null())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<T>, A::Error> {
        T::seq(seq)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<T>, A::Error> {
        T::map(map)
    }
}

/// Sets `slot` to `value`; `false` when `value` is `None` (the wrong shape) or `slot` is already
/// set (a repeated member).
fn fill<T>(slot: &mut Option<T>, value: Option<T>) -> bool {
    match value {
        Some(v) if slot.is_none() => {
            *slot = Some(v);
            true
        }
        _ => false,
    }
}

/// Skips the rest of `seq`; `true` when nothing was left.
fn ended<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<bool, A::Error> {
    let mut ended = true;
    while seq.next_element::<IgnoredAny>()?.is_some() {
        ended = false;
    }
    Ok(ended)
}

/// [`Image::blob`]: `null`, or an array of two non-negative integers.
struct Blob(Option<[u64; 2]>);

impl Container for Blob {
    fn seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<Option<Blob>, A::Error> {
        let (mut range, mut n, mut ok) = ([0u64; 2], 0usize, true);
        while let Some(v) = seq.next_element::<Scalar>()? {
            match (range.get_mut(n), v.unsigned()) {
                (Some(slot), Some(v)) => *slot = v,
                _ => ok = false,
            }
            n += 1;
        }
        Ok((ok && n == 2).then_some(Blob(Some(range))))
    }

    fn map<'de, A: MapAccess<'de>>(mut map: A) -> Result<Option<Blob>, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }

    fn null() -> Option<Blob> {
        Some(Blob(None))
    }
}

/// An [`Image`] read as its derived `Deserialize` reads it, from a JSON object or from an array of
/// its fields in order, without keeping a value of the wrong shape: an unknown member is skipped,
/// and a missing `mime`, a repeated member or a member of the wrong type gives `None`.
impl Container for Image {
    fn seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<Option<Image>, A::Error> {
        let mut image = Image::default();
        let Some(mime) = seq.next_element::<Scalar>()? else { return Ok(None) };
        let mut ok = mime.string().map(|m| image.mime = m).is_some();
        let Some(data) = seq.next_element::<Scalar>()? else { return Ok(ok.then_some(image)) };
        ok &= data.string().map(|d| image.data = d).is_some();
        let Some(blob) = seq.next_element_seed(Contained::<Blob>(PhantomData))? else { return Ok(ok.then_some(image)) };
        ok &= blob.map(|b| image.blob = b.0).is_some();
        let Some(proxy) = seq.next_element::<Scalar>()? else { return Ok(ok.then_some(image)) };
        ok &= proxy.bool().map(|p| image.proxy = p).is_some();
        ok &= ended(seq)?;
        Ok(ok.then_some(image))
    }

    fn map<'de, A: MapAccess<'de>>(mut map: A) -> Result<Option<Image>, A::Error> {
        let (mut mime, mut data, mut blob, mut proxy) = (None, None, None, None);
        let mut ok = true;
        while let Some(key) = map.next_key::<String>()? {
            ok &= match key.as_str() {
                "mime" => fill(&mut mime, map.next_value::<Scalar>()?.string()),
                "data" => fill(&mut data, map.next_value::<Scalar>()?.string()),
                "blob" => fill(&mut blob, map.next_value_seed(Contained::<Blob>(PhantomData))?),
                "proxy" => fill(&mut proxy, map.next_value::<Scalar>()?.bool()),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                    true
                }
            };
        }
        let (Some(mime), true) = (mime, ok) else { return Ok(None) };
        let mut image = Image::default();
        image.mime = mime;
        image.data = data.unwrap_or_default();
        image.blob = blob.and_then(|b| b.0);
        image.proxy = proxy.unwrap_or_default();
        Ok(Some(image))
    }
}

impl Shaped for Image {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Image>, D::Error> {
        d.deserialize_any(ContainerVisitor(PhantomData))
    }
}

/// Reads a [`Container`] as an element or member value.
struct Contained<T>(PhantomData<T>);

impl<'de, T: Container> de::DeserializeSeed<'de> for Contained<T> {
    type Value = Option<T>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Option<T>, D::Error> {
        d.deserialize_any(ContainerVisitor(PhantomData))
    }
}

/// The format name, the version, the writer and, when `IMAGES` is true, the preview and the PDF,
/// read without the rest of the file, so that a file of a newer version still reports them. The
/// members are read as [`craft_vectorcraft_format::File::read`] reads them: a missing or repeated
/// `format` or `version` is refused, and a `generator`, `preview` or `pdf` of the wrong shape
/// reads as `None` and a repeated one as its last value. When `IMAGES` is true, a missing or
/// repeated `document` is refused too; its value is skipped. `PREVIEW` and `PDF` choose which
/// image values to read; unrequested values are skipped without copying their strings.
struct Stamp<const IMAGES: bool, const PREVIEW: bool, const PDF: bool> {
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

impl<'de, const IMAGES: bool, const PREVIEW: bool, const PDF: bool> Deserialize<'de> for Stamp<IMAGES, PREVIEW, PDF> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct StampVisitor<const IMAGES: bool, const PREVIEW: bool, const PDF: bool>;
        impl<'de, const IMAGES: bool, const PREVIEW: bool, const PDF: bool> Visitor<'de> for StampVisitor<IMAGES, PREVIEW, PDF> {
            type Value = Stamp<IMAGES, PREVIEW, PDF>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a VectorCraft file's JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Stamp<IMAGES, PREVIEW, PDF>, A::Error> {
                let (mut format, mut version, mut generator, mut preview, mut pdf) = (None, None, None, None, None);
                let mut document: Option<IgnoredAny> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => once(&mut map, &mut format, "format")?,
                        "version" => once(&mut map, &mut version, "version")?,
                        "document" if IMAGES => once(&mut map, &mut document, "document")?,
                        "generator" => generator = map.next_value::<Lenient<String>>()?.0,
                        "preview" if IMAGES && PREVIEW => preview = map.next_value::<Lenient<Image>>()?.0,
                        "pdf" if IMAGES && PDF => pdf = map.next_value::<Lenient<Image>>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                if IMAGES && document.is_none() {
                    return Err(de::Error::missing_field("document"));
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
        d.deserialize_map(StampVisitor::<IMAGES, PREVIEW, PDF>)
    }
}

/// The file's [`Stamp`], refused when the format is not VectorCraft's, and when the version is
/// newer than this crate reads unless `newer` is true.
fn stamp<const IMAGES: bool, const PREVIEW: bool, const PDF: bool>(
    json: &[u8],
    budget: &Budget,
    newer: bool,
) -> Result<Stamp<IMAGES, PREVIEW, PDF>, Error> {
    let s: Stamp<IMAGES, PREVIEW, PDF> = if newer { parse(json, budget)? } else { parse_versioned(json, budget, Kind::VectorCraft)? };
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
    let s = stamp::<false, false, false>(&json, budget, true)?;
    Ok((s.version, s.generator))
}

fn inline_bytes(image: &Image, budget: &Budget, what: &str) -> Result<Vec<u8>, Error> {
    let bytes = if budget.control.is_some() {
        image.bytes_at_most_with_check(&[], u64::MAX, || budget.check_cancelled())?
    } else {
        image.bytes_at_most(&[], u64::MAX)
    };
    bytes.ok_or_else(|| malformed(format!("the {what} is not valid base64")))
}

/// Skip unrequested image fields during deserialization, including their inline base64 strings.
fn images<const PREVIEW: bool, const PDF: bool>(json: &[u8], budget: &Budget) -> Result<(Option<Image>, Option<Image>), Error> {
    let head = stamp::<true, PREVIEW, PDF>(json, budget, false)?;
    Ok((head.preview, head.pdf))
}

pub(crate) fn renditions(bytes: &[u8], wanted: Option<RenditionKind>, budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let Unpacked { json, tail } = unpack(bytes, budget)?;
    let (preview, pdf) = match wanted {
        None => images::<true, true>(&json, budget),
        Some(RenditionKind::Thumbnail) => images::<true, false>(&json, budget),
        Some(RenditionKind::Pdf) => images::<false, true>(&json, budget),
        Some(RenditionKind::Composite) => images::<false, false>(&json, budget),
    }?;
    // A member without a MIME type gives no rendition: a preview may be PNG or JPEG.
    let members: Vec<_> = [(RenditionKind::Thumbnail, preview, "preview"), (RenditionKind::Pdf, pdf, "PDF")]
        .into_iter()
        .filter(|(kind, _, _)| wanted.is_none_or(|wanted| wanted == *kind))
        .filter_map(|(kind, m, what)| Some((kind, m.filter(|m| !m.mime.is_empty())?, what)))
        .collect();
    // Every size is checked before any data after the JSON is decompressed. A member after the
    // JSON of a compressed file is counted against max_total as it is decompressed; any other
    // member is counted here.
    let compressed = matches!(tail, Tail::Stream { .. });
    for (_, m, what) in &members {
        let len = if budget.control.is_some() { m.len_with_check(|| budget.check_cancelled())? } else { m.len() };
        if compressed && m.blob.is_some() {
            budget.check_entry(len, what)?;
        } else {
            budget.take_entry(len, what)?;
        }
    }
    let ranges: Vec<[u64; 2]> = members.iter().filter_map(|(_, m, _)| m.blob).collect();
    let mut blobs = if ranges.is_empty() { vec![] } else { tail.ranges(&ranges, budget)? }.into_iter();
    members
        .into_iter()
        .map(|(kind, m, what)| {
            budget.check_cancelled()?;
            let bytes = match m.blob {
                Some(_) => blobs.next().flatten().ok_or_else(|| malformed(format!("the file ends before its {what}")))?,
                None => inline_bytes(&m, budget, what)?,
            };
            budget.check_cancelled()?;
            Ok(Rendition { kind, mime: m.mime, bytes, is_full_composite: false })
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
    stamp::<false, false, false>(&json, budget, false)?;
    let f: WithDocument = parse(&json, budget)?;
    document_outline(f.document, budget)
}

fn document_outline(document: DocM, budget: &Budget) -> Result<Outline, Error> {
    let pages = document
        .artboards
        .into_iter()
        .map(|a| {
            budget.check_cancelled()?;
            Ok(Page {
                name: a.name,
                x: a.rect.x0.min(a.rect.x1),
                y: a.rect.y0.min(a.rect.y1),
                width: (a.rect.x1 - a.rect.x0).abs(),
                height: (a.rect.y1 - a.rect.y0).abs(),
                timing: None,
                layers: vec![],
            })
        })
        .collect::<Result<_, Error>>()?;
    let layers = document
        .layers
        .into_iter()
        .map(|n| {
            budget.check_cancelled()?;
            Ok(Layer {
                name: n.name.unwrap_or_default(),
                kind: n.kind.kind,
                visible: n.visible,
                opacity: Some(n.opacity),
                blend: Some(n.blend.unwrap_or_else(|| "Normal".into())),
                children: vec![],
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Outline { canvas: None, pages, layers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{unread_array, unread_object};
    use serde::de::value::{Error as ValueError, MapDeserializer, SeqDeserializer, StrDeserializer};

    /// An ignored field must use IgnoredAny, which can skip this sentinel without asking it
    /// to deserialize an image or materialize any strings.
    #[test]
    fn unrequested_images_are_skipped_before_their_strings_are_read() {
        fn skipped<const PREVIEW: bool, const PDF: bool>(field: &str) {
            let map = MapDeserializer::<_, ValueError>::new([(field, crate::tests::Unread), ("document", crate::tests::Unread)].into_iter());
            let err = Stamp::<true, PREVIEW, PDF>::deserialize(map).err().unwrap();
            assert!(err.to_string().contains("missing field `format`"), "{err}");
        }
        skipped::<false, true>("preview");
        skipped::<true, false>("pdf");
        skipped::<false, false>("preview");
        skipped::<false, false>("pdf");
    }

    #[test]
    fn duplicate_copy_cost_is_checked_before_the_stream_is_read() {
        let limits = crate::Limits { max_total: 12, ..Default::default() };
        let mut budget = Budget::new(&limits);
        let tail = Tail::Stream { dec: Box::new(gzip_decoder(&b"not a gzip stream"[..], usize::MAX, None)), read: vec![] };
        let result = tail.ranges(&[[0, 8], [0, 8]], &mut budget);
        assert!(matches!(result, Err(Error::LimitExceeded(_))), "{result:?}");
        assert_eq!(budget.left(), 12);
    }

    #[test]
    fn duplicate_copies_can_use_exactly_the_remaining_budget() {
        let limits = crate::Limits { max_total: 16, ..Default::default() };
        let mut budget = Budget::new(&limits);
        let tail = Tail::Stream { dec: Box::new(gzip_decoder(&b"not a gzip stream"[..], usize::MAX, None)), read: b"12345678".to_vec() };
        let result = tail.ranges(&[[0, 8], [0, 8]], &mut budget).unwrap();
        assert_eq!(result, [Some(b"12345678".to_vec()), Some(b"12345678".to_vec())]);
        assert_eq!(budget.left(), 0);
    }

    #[test]
    fn rendition_copy_size_overflow_is_rejected_before_reading() {
        let limits = crate::Limits { max_total: u64::MAX, ..Default::default() };
        let mut budget = Budget::new(&limits);
        let tail = Tail::Stream { dec: Box::new(gzip_decoder(&b"not a gzip stream"[..], usize::MAX, None)), read: vec![] };
        assert!(matches!(tail.ranges(&[[0, u64::MAX], [0, 1]], &mut budget), Err(Error::LimitExceeded(_))));
        assert_eq!(budget.left(), u64::MAX);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn conversion_stops_inside_document_layers() {
        use std::cell::Cell;
        let node = r#"{"kind":{"type":"Path"}}"#;
        let json = format!(r#"{{"layers":[{node},{node},{node},{node},{node}]}}"#);
        let document: DocM = serde_json::from_str(&json).unwrap();
        let polls = Cell::new(0);
        let cancel = || {
            polls.set(polls.get() + 1);
            polls.get() == 4
        };
        let result = crate::cancel::run(&cancel, |control| document_outline(document, &Budget::controlled(&crate::Limits::default(), control)));
        assert_eq!(result, Err(Error::Cancelled));
        assert_eq!(polls.get(), 4);
    }

    #[test]
    fn inline_cancellation_interrupts_extraction() {
        let image = Image::inline("application/pdf", &vec![1; 2 << 20]);
        let polls = std::cell::Cell::new(0);
        let cancelled = || {
            polls.set(polls.get() + 1);
            polls.get() >= 10
        };
        let result = crate::cancel::run(&cancelled, |control| inline_bytes(&image, &Budget::controlled(&crate::Limits::default(), control), "PDF"));
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(polls.get(), 10);
    }

    #[test]
    fn an_image_of_the_wrong_shape_is_skipped_without_reading_it_whole() {
        assert!(Lenient::<Image>::deserialize(unread_object(1000)).unwrap().0.is_none());
        // A `mime` that is an array, in the array form and in an object, and a `blob` that is an
        // object.
        let array_form = SeqDeserializer::<_, ValueError>::new([unread_array(1000), unread_array(1000)].into_iter());
        assert!(Lenient::<Image>::deserialize(array_form).unwrap().0.is_none());
        let mime_array = MapDeserializer::<_, ValueError>::new([("mime", unread_array(1000))].into_iter());
        assert!(Lenient::<Image>::deserialize(mime_array).unwrap().0.is_none());
        let blob_object = MapDeserializer::<_, ValueError>::new([("blob", unread_object(1000))].into_iter());
        assert!(Lenient::<Image>::deserialize(blob_object).unwrap().0.is_none());
        let fine: Lenient<Image> =
            Lenient::deserialize(MapDeserializer::<_, ValueError>::new([("mime", StrDeserializer::<ValueError>::new("image/png"))].into_iter()))
                .unwrap();
        assert_eq!(fine.0.map(|i| i.mime).as_deref(), Some("image/png"));
    }

    #[test]
    fn the_preview_and_pdf_read_as_file_read_reads_them() {
        let shapes = [
            r#"{"mime":"image/png","data":"AAAA"}"#,
            r#"{"mime":"image/png","data":"AAAA","extra":[1,{"a":2}]}"#,
            r#"{"mime":"image/png","blob":[1,2],"proxy":true}"#,
            r#"{"mime":"image/png","blob":null}"#,
            r#"{"mime":"","data":""}"#,
            r#"{"data":"AAAA"}"#,
            r#"{"mime":7}"#,
            r#"{"mime":"image/png","data":null}"#,
            r#"{"mime":"image/png","proxy":1}"#,
            r#"{"mime":"image/png","proxy":null}"#,
            r#"{"mime":"image/png","blob":[1,2,3]}"#,
            r#"{"mime":"image/png","blob":[1]}"#,
            r#"{"mime":"image/png","blob":[-1,2]}"#,
            r#"{"mime":"image/png","blob":[1.0,2]}"#,
            r#"{"mime":"image/png","blob":[1,18446744073709551616]}"#,
            r#"{"mime":"image/png","blob":{"0":1}}"#,
            r#"{"mime":"image/png","blob":"1,2"}"#,
            r#"{"mime":"image/png","mime":"image/jpeg"}"#,
            r#"{"mime":"image/png","blob":[1,2],"blob":[1,2]}"#,
            r#"["image/png"]"#,
            r#"["image/png","AAAA"]"#,
            r#"["image/png","",[3,4]]"#,
            r#"["image/png","",null,true]"#,
            r#"["image/png","",null,true,0]"#,
            r#"["image/png",[0,0]]"#,
            r#"[7]"#,
            r#"[]"#,
            r#"[[0,0,0]]"#,
            r#""image/png""#,
            r#"7"#,
            r#"-7"#,
            r#"1.5"#,
            r#"true"#,
            r#"null"#,
        ];
        for shape in shapes {
            let json = format!(r#"{{"format":"vectorcraft","version":4,"generator":{shape},"preview":{shape},"pdf":{shape},"document":0}}"#);
            let file = craft_vectorcraft_format::File::<IgnoredAny>::read(json.as_bytes()).unwrap();
            let stamp: Stamp<true, true, true> = crate::parse(json.as_bytes(), &crate::Budget::new(&crate::Limits::default())).unwrap();
            assert_eq!((&stamp.generator, &stamp.preview, &stamp.pdf), (&file.generator, &file.preview, &file.pdf), "{shape}");
        }
    }
}

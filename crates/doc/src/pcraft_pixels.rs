//! A PhotoCraft layer's pixels (feature `pcraft-pixels`): the tiles of its surface, decoded with
//! `ruzstd` and laid out as RGBA over the canvas, and its layer mask.
//!
//! A surface is a pixel format, a default pixel and a list of 256 × 256 tiles on a grid in
//! document pixels, so tiles may sit at negative columns and rows. Each tile is stored at
//! [`craft_pcraft_format::tile_path`] of its hash as one zstd frame of interleaved little-endian
//! samples, row by row, the color channels first and then the alpha channel when the format has
//! one; the hash is the BLAKE3 hash of those samples. Pixels without a tile hold the default
//! pixel.
//!
//! The same bounded blob decoder reads a document's declared ICC payload without interpreting it.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;

use craft_pcraft_format::TileRef;
use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::zip::{Entry, Zip};
use crate::{Budget, Error, Kind, Limits, counted, malformed, pcraft, sniff};

/// The side of a tile, in pixels.
const TILE: usize = 256;

/// The largest zstd window a tile or ICC frame may declare: 8 MiB, eight times the largest tile
/// (256 × 256 pixels of four 32-bit samples). The decoder's buffer grows as it decodes, up to the
/// window the frame declares; a frame that declares a larger window is refused before decoding
/// starts.
const MAX_WINDOW: u64 = 8 << 20;

/// Samples of one type, interleaved.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Samples {
    /// 8-bit samples, 0 to 255.
    U8(Vec<u8>),
    /// 16-bit samples, 0 to 65535.
    U16(Vec<u16>),
    /// 32-bit floating-point samples as stored, nominally 0 to 1.
    F32(Vec<f32>),
}

impl Samples {
    /// The number of samples.
    pub fn len(&self) -> usize {
        match self {
            Samples::U8(v) => v.len(),
            Samples::U16(v) => v.len(),
            Samples::F32(v) => v.len(),
        }
    }

    /// `true` when there are no samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A layer's pixels over the canvas, from [`pcraft_layer_rgba`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LayerPixels {
    /// The canvas width in pixels.
    pub width: u32,
    /// The canvas height in pixels.
    pub height: u32,
    /// `width` × `height` pixels of red, green, blue and alpha, row by row from the top-left
    /// corner of the canvas, in the layer's sample type. Grayscale pixels repeat the gray value in
    /// red, green and blue. A surface without alpha is opaque (255, 65535 or 1.0). Alpha is
    /// straight, not premultiplied.
    pub rgba: Samples,
    /// The layer mask, when the layer has one.
    pub mask: Option<LayerMask>,
}

/// A layer mask as the manifest stores it. The pixels are the mask's surface; `density` and
/// `feather` are not applied to them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LayerMask {
    /// `false` when the mask is turned off.
    pub enabled: bool,
    /// The mask's density, 0 to 1.
    pub density: f32,
    /// The mask's feather radius in pixels.
    pub feather: f32,
    /// One sample per canvas pixel, row by row, in the mask's sample type: the minimum hides the
    /// layer and the maximum shows it.
    pub alpha: Samples,
}

/// The pixels of the PhotoCraft layer at `layer`, a path of indexes through [`crate::Outline`]'s
/// `layers` (bottom first) and then each group's `children`, over the canvas.
///
/// A raster layer gives its surface, and a text, shape or smart object layer the rendered copy
/// PhotoCraft saves with it. A video layer is a raster layer whose frames are in its `video`
/// member; it gives its surface, which holds the frame at the playhead, and the other frames are
/// not read. `Ok(None)` when `layer` names no layer, or a layer
/// without pixels of its own: a group, an adjustment or fill layer, or a text, shape or smart
/// object layer saved without its rendered copy. The mask of such a layer is not returned either.
/// Tiles outside the canvas are not read.
///
/// Surfaces stored as RGB or grayscale are read at 8, 16 and 32 bits. PhotoCraft stores the
/// layers of bitmap, duotone, indexed and multichannel documents that way too. A layer or mask
/// stored in another color mode, such as CMYK or Lab, gives [`Error::Unsupported`]. A mask stored
/// as RGB gives its first channel. A file other than a `.pcraft` bundle gives
/// [`Error::Unrecognized`] or [`Error::Unsupported`], and a canvas of zero width or height gives
/// [`Error::Malformed`].
///
/// The decoded layer and the decoded mask each count as one entry against
/// [`Limits::max_entry`], and together with each distinct tile read and decompressed against
/// [`Limits::max_total`]. A tile that several references name is read and decoded once per call.
/// Each is checked before it is allocated. Tile references in parsed surfaces, rendered caches
/// and masks throughout the layer tree count against [`Limits::max_items`], including other
/// layers. Video frames and other unrecognized members are skipped.
pub fn pcraft_layer_rgba(bytes: &[u8], layer: &[usize], limits: &Limits) -> Result<Option<LayerPixels>, Error> {
    match sniff(bytes).ok_or(Error::Unrecognized)? {
        Kind::PhotoCraft => {}
        kind => return Err(Error::Unsupported(format!("layer pixels are read from .pcraft bundles, not .{}", kind.extension()))),
    }
    let mut budget = Budget::new(limits);
    let (zip, doc) = pcraft::document::<DocP>(bytes, &mut budget)?;
    if doc.size.width == 0 || doc.size.height == 0 {
        return Err(malformed(format!("canvas size {}x{} is empty", doc.size.width, doc.size.height)));
    }
    let Some(found) = find(&doc.layers, layer) else { return Ok(None) };
    let surface = match found.content.kind.as_str() {
        "raster" => found.content.surface.as_ref(),
        "text" | "shape" | "smart" => found.content.cache.as_ref(),
        _ => None,
    };
    let Some(surface) = surface else { return Ok(None) };
    check_mode(surface)?;
    if let Some(m) = &found.mask {
        check_mode(&m.surface)?;
    }
    let canvas = Canvas { width: doc.size.width, height: doc.size.height };
    let mut tiles = Tiles::new(&zip);
    let rgba = decode(&mut tiles, surface, canvas, Out::Rgba, &mut budget)?;
    let mask = match &found.mask {
        Some(m) => {
            let alpha = decode(&mut tiles, &m.surface, canvas, Out::Gray, &mut budget)?;
            Some(LayerMask { enabled: m.enabled, density: m.density, feather: m.feather, alpha })
        }
        None => None,
    };
    Ok(Some(LayerPixels { width: canvas.width, height: canvas.height, rgba, mask }))
}

/// The raw ICC payload declared by a PhotoCraft document, verified against its BLAKE3 hash.
///
/// A missing or null `document.icc_profile` gives `Ok(None)`. A declared profile whose hash,
/// entry or compressed payload is invalid gives an error. These bytes are not checked for ICC
/// semantics, interpreted, converted or rendered; even an empty verified blob is returned.
/// Previews, tiles and other blobs are not read. The normal manifest envelope is still checked.
///
/// The manifest, its selected zstd frame and the decoded payload count against `max_total`;
/// each is bounded by `max_entry`, and the manifest also by `max_json`. The frame window is at
/// most 8 MiB. One frame is accepted, with no trailing bytes. Unlike the layer reader's counted
/// lists, this call checks `max_items` against structural JSON items throughout the manifest,
/// including skipped values: objects/arrays opened and commas outside strings, at most
/// [`craft_pcraft_format::MAX_MANIFEST_ITEMS`]. The check precedes metadata deserialization.
pub fn pcraft_icc_profile(bytes: &[u8], limits: &Limits) -> Result<Option<Vec<u8>>, Error> {
    match sniff(bytes).ok_or(Error::Unrecognized)? {
        Kind::PhotoCraft => {}
        kind => return Err(Error::Unsupported(format!("ICC payloads are read from .pcraft bundles, not .{}", kind.extension()))),
    }
    let mut budget = Budget::new(limits);
    let (zip, head) = pcraft::document_with_items::<ProfileHead>(bytes, &mut budget)?;
    let Some(ProfileHash(hash)) = head.profile else { return Ok(None) };
    let path = craft_pcraft_format::blob_path(&hash).ok_or_else(|| malformed("invalid ICC blob hash"))?;
    let entry = zip.find(&path, budget.control)?.ok_or_else(|| malformed(format!("the manifest names `{path}`, which the bundle does not have")))?;
    let frame = zip.read(entry, limits.max_entry, &mut budget, &path)?;
    let cap = limits.max_entry.min(budget.left());
    let cap = usize::try_from(cap).unwrap_or(usize::MAX);
    let profile = unzstd_bounded(&frame, DecodedSize::AtMost(cap), &path)?;
    budget.take_entry(profile.len() as u64, &path)?;
    if blake3::hash(&profile).to_hex().as_str() != hash {
        return Err(malformed(format!("{path}: the payload does not match its hash")));
    }
    Ok(Some(profile))
}

/// A document object that retains only its declared profile reference.
struct ProfileHead {
    profile: Option<ProfileHash>,
}

impl<'de> Deserialize<'de> for ProfileHead {
    /// Read an object while skipping all metadata unrelated to the profile reference.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            IccProfile,
            #[serde(other)]
            Other,
        }
        struct Head;
        impl<'de> Visitor<'de> for Head {
            type Value = ProfileHead;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a PhotoCraft document object")
            }
            /// Track presence separately from value so null cannot conceal a duplicate field.
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ProfileHead, A::Error> {
                let mut seen = false;
                let mut profile = None;
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::IccProfile => {
                            if seen {
                                return Err(de::Error::duplicate_field("icc_profile"));
                            }
                            seen = true;
                            profile = map.next_value()?;
                        }
                        Field::Other => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(ProfileHead { profile })
            }
        }
        d.deserialize_map(Head)
    }
}

/// A canonical blob hash; invalid references are refused before building an entry name.
struct ProfileHash(String);

impl<'de> Deserialize<'de> for ProfileHash {
    /// Retain only a canonical reference, without accepting a caller-supplied archive path.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Hash;
        impl Visitor<'_> for Hash {
            type Value = ProfileHash;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a 64-digit lowercase BLAKE3 hash")
            }
            /// Check the decoded string before copying the 64 hash digits into owned metadata.
            fn visit_str<E: de::Error>(self, value: &str) -> Result<ProfileHash, E> {
                if !craft_pcraft_format::is_valid_hash(value) {
                    return Err(E::custom("invalid ICC blob hash"));
                }
                Ok(ProfileHash(value.to_owned()))
            }
        }
        d.deserialize_str(Hash)
    }
}

/// [`Error::Unsupported`] unless `surface` is stored as RGB or grayscale.
fn check_mode(surface: &SurfaceP) -> Result<(), Error> {
    let mode = surface.format.mode.as_str();
    if matches!(mode, "Rgb" | "Grayscale") { Ok(()) } else { Err(Error::Unsupported(format!("{mode} pixels are not read; only Rgb and Grayscale"))) }
}

/// The bundle's tiles for one call: its entries by name, and each tile decoded so far by its
/// hash.
struct Tiles<'z, 'a> {
    zip: &'z Zip<'a>,
    entries: HashMap<&'a [u8], &'z Entry>,
    decoded: HashMap<String, Vec<u8>>,
}

impl<'z, 'a> Tiles<'z, 'a> {
    fn new(zip: &'z Zip<'a>) -> Self {
        Tiles { zip, entries: zip.index(), decoded: HashMap::new() }
    }

    /// The `len` bytes of samples of the tile `t` names, read, decoded and checked against its
    /// hash the first time a call asks for that hash.
    fn get(&mut self, t: &TileRef, len: usize, budget: &mut Budget) -> Result<&[u8], Error> {
        if !self.decoded.contains_key(&t.hash) {
            let path = t.path().ok_or_else(|| malformed(format!("tile ({}, {}) has an invalid hash", t.tx, t.ty)))?;
            let entry =
                self.entries.get(path.as_bytes()).ok_or_else(|| malformed(format!("the manifest names `{path}`, which the bundle does not have")))?;
            // A frame larger than its tile plus an eighth and 4 KiB is refused, as PhotoCraft reads it.
            let max_frame = (len + len / 8 + 4096) as u64;
            let frame = self.zip.read(entry, max_frame.min(budget.limits.max_entry), budget, &path)?;
            budget.take_entry(len as u64, &path)?;
            let samples = unzstd(&frame, len, &path)?;
            if blake3::hash(&samples).to_hex().as_str() != t.hash {
                return Err(malformed(format!("{path}: the samples do not match their hash")));
            }
            self.decoded.insert(t.hash.clone(), samples);
        }
        match self.decoded.get(&t.hash) {
            // One hash names one string of bytes, so a tile of another length is a reference from
            // a format of another pixel size.
            Some(samples) if samples.len() == len => Ok(samples),
            _ => Err(malformed(format!("tile {} is referenced with different sizes", t.hash))),
        }
    }
}

/// The layer at `path`.
fn find<'a>(layers: &'a [LayerP], path: &[usize]) -> Option<&'a LayerP> {
    let (first, rest) = path.split_first()?;
    let layer = layers.get(*first)?;
    if rest.is_empty() { Some(layer) } else { find(&layer.content.children, rest) }
}

#[derive(Deserialize)]
struct DocP {
    size: SizeP,
    #[serde(deserialize_with = "counted")]
    layers: Vec<LayerP>,
}

#[derive(Deserialize)]
struct SizeP {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct LayerP {
    #[serde(default)]
    mask: Option<MaskP>,
    content: ContentP,
}

#[derive(Deserialize)]
struct MaskP {
    surface: SurfaceP,
    enabled: bool,
    density: f32,
    feather: f32,
}

#[derive(Deserialize)]
struct ContentP {
    kind: String,
    /// A raster layer's surface.
    #[serde(default)]
    surface: Option<SurfaceP>,
    /// A text, shape or smart object layer's rendered copy.
    #[serde(default)]
    cache: Option<SurfaceP>,
    #[serde(default, deserialize_with = "counted")]
    children: Vec<LayerP>,
}

#[derive(Deserialize)]
struct SurfaceP {
    format: FormatP,
    /// The default pixel, as hex of its little-endian samples.
    default: String,
    #[serde(deserialize_with = "counted")]
    tiles: Vec<TileRef>,
}

#[derive(Deserialize)]
struct FormatP {
    mode: String,
    sample: SampleP,
    alpha: bool,
}

#[derive(Deserialize, Clone, Copy)]
enum SampleP {
    U8,
    U16,
    F32,
}

#[derive(Clone, Copy)]
struct Canvas {
    width: u32,
    height: u32,
}

/// What [`decode`] writes per pixel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Out {
    /// Red, green, blue and alpha.
    Rgba,
    /// The first channel.
    Gray,
}

impl Out {
    fn channels(self) -> usize {
        match self {
            Out::Rgba => 4,
            Out::Gray => 1,
        }
    }
}

/// One sample type, read from little-endian bytes.
trait Sample: Copy {
    const SIZE: usize;
    const OPAQUE: Self;
    fn read(bytes: &[u8]) -> Vec<Self>;
    fn at(bytes: &[u8], index: usize) -> Option<Self>;
}

#[cfg(test)]
thread_local! { static SAMPLE_BUFFER_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

impl Sample for u8 {
    const SIZE: usize = 1;
    const OPAQUE: u8 = u8::MAX;
    fn read(bytes: &[u8]) -> Vec<u8> {
        #[cfg(test)]
        SAMPLE_BUFFER_BYTES.with(|n| n.set(n.get() + bytes.len()));
        bytes.to_vec()
    }
    fn at(bytes: &[u8], index: usize) -> Option<u8> {
        bytes.get(index).copied()
    }
}

impl Sample for u16 {
    const SIZE: usize = 2;
    const OPAQUE: u16 = u16::MAX;
    fn read(bytes: &[u8]) -> Vec<u16> {
        #[cfg(test)]
        SAMPLE_BUFFER_BYTES.with(|n| n.set(n.get() + bytes.len()));
        bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()
    }
    fn at(bytes: &[u8], index: usize) -> Option<u16> {
        Some(u16::from_le_bytes(*bytes.get(index.checked_mul(2)?..)?.first_chunk::<2>()?))
    }
}

impl Sample for f32 {
    const SIZE: usize = 4;
    const OPAQUE: f32 = 1.0;
    fn read(bytes: &[u8]) -> Vec<f32> {
        #[cfg(test)]
        SAMPLE_BUFFER_BYTES.with(|n| n.set(n.get() + bytes.len()));
        bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
    }
    fn at(bytes: &[u8], index: usize) -> Option<f32> {
        Some(f32::from_le_bytes(*bytes.get(index.checked_mul(4)?..)?.first_chunk::<4>()?))
    }
}

/// `surface` over the canvas, in its own sample type.
fn decode(tiles: &mut Tiles, surface: &SurfaceP, canvas: Canvas, out: Out, budget: &mut Budget) -> Result<Samples, Error> {
    Ok(match surface.format.sample {
        SampleP::U8 => Samples::U8(decode_as::<u8>(tiles, surface, canvas, out, budget)?),
        SampleP::U16 => Samples::U16(decode_as::<u16>(tiles, surface, canvas, out, budget)?),
        SampleP::F32 => Samples::F32(decode_as::<f32>(tiles, surface, canvas, out, budget)?),
    })
}

/// The channels of one source pixel, written as `out` asks.
fn put<S: Sample>(src: &[S], gray: bool, alpha: bool, out: Out, dst: &mut [S]) {
    let (Some(&first), Some(&last)) = (src.first(), src.last()) else { return };
    match out {
        Out::Gray => {
            if let Some(d) = dst.first_mut() {
                *d = first;
            }
        }
        Out::Rgba => {
            let a = if alpha { last } else { S::OPAQUE };
            let rgb = if gray { [first; 3] } else { [first, src.get(1).copied().unwrap_or(first), src.get(2).copied().unwrap_or(first)] };
            for (d, s) in dst.iter_mut().zip(rgb.into_iter().chain([a])) {
                *d = s;
            }
        }
    }
}

fn decode_as<S: Sample>(tiles: &mut Tiles, surface: &SurfaceP, canvas: Canvas, out: Out, budget: &mut Budget) -> Result<Vec<S>, Error> {
    let format = &surface.format;
    let gray = format.mode == "Grayscale";
    let channels = if gray { 1 } else { 3 } + usize::from(format.alpha);
    let pixel_bytes = channels * S::SIZE;

    if surface.default.len() != pixel_bytes * 2 {
        return Err(malformed(format!("the default pixel has {} hex digits, not {}", surface.default.len(), pixel_bytes * 2)));
    }
    let default = S::read(&unhex(&surface.default)?);

    let (w, h) = (canvas.width as usize, canvas.height as usize);
    let oc = out.channels();
    let len = u64::from(canvas.width)
        .checked_mul(u64::from(canvas.height))
        .and_then(|n| n.checked_mul((oc * S::SIZE) as u64))
        .ok_or_else(|| Error::LimitExceeded("the canvas is too large".into()))?;
    let what = match out {
        Out::Rgba => "the layer's pixels",
        Out::Gray => "the layer mask",
    };
    budget.take_entry(len, what)?;
    // The samples, counted in `usize`; on a 32-bit target a canvas within the limits may still
    // hold more than `usize` counts.
    let too_large = || Error::LimitExceeded(format!("{what} do not fit in memory"));
    let count = usize::try_from(len / S::SIZE as u64).map_err(|_| too_large())?;
    let mut fill = vec![S::OPAQUE; oc];
    put(&default, gray, format.alpha, out, &mut fill);
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).map_err(|_| too_large())?;
    for _ in 0..count / oc {
        pixels.extend_from_slice(&fill);
    }

    let tile_bytes = TILE * TILE * pixel_bytes;
    let mut seen = BTreeSet::new();
    for t in &surface.tiles {
        // Both edges of the tile must fit in i32, as PhotoCraft reads them.
        let fits = |i: i32| i.checked_mul(TILE as i32).is_some() && i.checked_add(1).and_then(|e| e.checked_mul(TILE as i32)).is_some();
        if !fits(t.tx) || !fits(t.ty) {
            return Err(malformed(format!("tile ({}, {}) is outside the coordinate range", t.tx, t.ty)));
        }
        if !seen.insert((t.tx, t.ty)) {
            return Err(malformed(format!("duplicate tile ({}, {})", t.tx, t.ty)));
        }
        // Columns and rows of the canvas the tile covers; negative tiles cover none.
        let (Ok(tx), Ok(ty)) = (usize::try_from(t.tx), usize::try_from(t.ty)) else { continue };
        let (x0, y0) = (tx.saturating_mul(TILE), ty.saturating_mul(TILE));
        if x0 >= w || y0 >= h {
            continue;
        }
        let tile = tiles.get(t, tile_bytes, budget)?;
        for y in y0..(y0 + TILE).min(h) {
            for x in x0..(x0 + TILE).min(w) {
                let s = ((y - y0) * TILE + (x - x0)) * channels;
                let d = (y * w + x) * oc;
                let mut src = [S::OPAQUE; 4];
                for (c, sample) in src.iter_mut().enumerate().take(channels) {
                    *sample = S::at(tile, s + c).ok_or_else(|| malformed("tile pixel is incomplete"))?;
                }
                if let Some(dst) = pixels.get_mut(d..d + oc) {
                    put(src.get(..channels).ok_or_else(|| malformed("too many pixel channels"))?, gray, format.alpha, out, dst);
                }
            }
        }
    }
    Ok(pixels)
}

/// One zstd frame that decodes to exactly `len` bytes. Decoding stops one byte past `len`, so a
/// frame that expands further is refused without being decoded.
fn unzstd(frame: &[u8], len: usize, what: &str) -> Result<Vec<u8>, Error> {
    unzstd_bounded(frame, DecodedSize::Exact(len), what)
}

/// An exact tile length or an admitted upper bound for a variable-length payload.
#[derive(Clone, Copy)]
enum DecodedSize {
    Exact(usize),
    AtMost(usize),
}

/// Decode one bounded frame without allocating the destination from an untrusted header.
/// Exact lengths retain the tile reader's malformed-length errors; upper bounds admit only
/// remaining profile bytes. Destination growth stays within that bound, and the window is
/// limited before the decoder allocates it. Neither mode accepts a second frame or trailing data.
fn unzstd_bounded(frame: &[u8], size: DecodedSize, what: &str) -> Result<Vec<u8>, Error> {
    let zstd = |e: &dyn std::fmt::Display| malformed(format!("{what}: zstd: {e}"));
    let mut dec = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(frame, MAX_WINDOW).map_err(|e| {
        if matches!(size, DecodedSize::AtMost(_)) && matches!(e, ruzstd::decoding::errors::FrameDecoderError::WindowSizeTooBig { .. }) {
            Error::LimitExceeded(format!("{what}: zstd window exceeds {MAX_WINDOW} bytes"))
        } else {
            zstd(&e)
        }
    })?;
    let len = match size {
        DecodedSize::Exact(n) | DecodedSize::AtMost(n) => n,
    };
    let declared = dec.decoder.content_size();
    // After header validation, descriptor bits 7-6 (FCS flag) or 5 (single segment) mean
    // the size is present, even when zero. See RFC 8878, section 3.1.1.1.1.1.
    let has_content_size = frame.get(4).is_some_and(|descriptor| descriptor & 0xe0 != 0);
    if matches!(size, DecodedSize::AtMost(_)) && declared > len as u64 {
        return Err(Error::LimitExceeded(format!("{what}: declared payload exceeds {len} bytes")));
    }
    let mut out = Vec::new();
    if let DecodedSize::Exact(n) = size {
        out.try_reserve_exact(n).map_err(|_| Error::LimitExceeded(format!("{what} does not fit in memory")))?;
    }
    let mut chunk = [0u8; 8192];
    loop {
        let n = len.saturating_sub(out.len()).saturating_add(1).min(chunk.len());
        let read = dec.read(&mut chunk[..n]).map_err(|e| zstd(&e))?;
        if read == 0 {
            break;
        }
        let next = out.len().checked_add(read).ok_or_else(|| Error::LimitExceeded(format!("{what} does not fit in memory")))?;
        if next > len {
            return Err(match size {
                DecodedSize::Exact(_) => malformed(format!("{what} decodes to more than {len} bytes")),
                DecodedSize::AtMost(_) => Error::LimitExceeded(format!("{what} decodes to more than {len} admitted bytes")),
            });
        }
        if next > out.capacity() {
            let target = out.capacity().max(chunk.len()).saturating_mul(2).min(len).max(next);
            out.try_reserve_exact(target - out.len()).map_err(|_| Error::LimitExceeded(format!("{what} does not fit in memory")))?;
        }
        out.extend_from_slice(&chunk[..read]);
    }
    if matches!(size, DecodedSize::Exact(_)) && out.len() < len {
        return Err(malformed(format!("{what} decodes to {} bytes, not {len}", out.len())));
    }
    if matches!(size, DecodedSize::AtMost(_)) && has_content_size && declared != out.len() as u64 {
        return Err(malformed(format!("{what}: declared payload size {declared} differs from {} decoded bytes", out.len())));
    }
    if !dec.decoder.is_finished() {
        return Err(malformed(format!("{what} has an incomplete zstd frame")));
    }
    if !dec.get_ref().is_empty() {
        return Err(malformed(format!("{what} has bytes after its zstd frame")));
    }
    Ok(out)
}

/// Bytes from hex digits, either case.
fn unhex(s: &str) -> Result<Vec<u8>, Error> {
    let bad = || malformed(format!("bad hex `{s}`"));
    if !s.len().is_multiple_of(2) {
        return Err(bad());
    }
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[hi, lo]| {
            let digit = |c: u8| char::from(c).to_digit(16);
            u8::try_from(digit(*hi).ok_or_else(bad)? * 16 + digit(*lo).ok_or_else(bad)?).map_err(|_| bad())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_both_cases_and_refuses_the_rest() {
        assert_eq!(unhex("00ffA0").ok(), Some(vec![0, 255, 0xa0]));
        assert!(unhex("0").is_err());
        assert!(unhex("+5").is_err());
        assert!(unhex("zz").is_err());
    }

    #[test]
    fn a_frame_that_expands_past_the_tile_is_refused() {
        let frame = ruzstd::encoding::compress_to_vec(&[7u8; 100][..], ruzstd::encoding::CompressionLevel::Fastest);
        assert_eq!(unzstd(&frame, 100, "t").ok(), Some(vec![7u8; 100]));
        assert!(matches!(unzstd(&frame, 99, "t"), Err(Error::Malformed(m)) if m.contains("more than 99 bytes")));
        assert!(matches!(unzstd(&frame, 101, "t"), Err(Error::Malformed(_))));
    }

    #[test]
    fn bytes_after_a_zstd_frame_are_malformed() {
        let frame = ruzstd::encoding::compress_to_vec(&[7u8; 100][..], ruzstd::encoding::CompressionLevel::Fastest);
        for extra in [&b"junk"[..], frame.as_slice()] {
            let mut trailing = frame.clone();
            trailing.extend_from_slice(extra);
            assert!(matches!(unzstd(&trailing, 100, "t"), Err(Error::Malformed(_))));
        }
    }

    /// A ZIP archive of STORE entries.
    fn stored(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let (mut out, mut dir) = (vec![], vec![]);
        for (name, data) in entries {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let (offset, size) = (out.len() as u32, data.len() as u32);
            let fields = |sig: u32| [sig.to_le_bytes(), [20, 0, 0, 0]].concat();
            out.extend(fields(0x0403_4b50));
            out.extend([0u8; 6]);
            out.extend([crc.sum().to_le_bytes(), size.to_le_bytes(), size.to_le_bytes()].concat());
            out.extend([(name.len() as u16).to_le_bytes(), [0, 0]].concat());
            out.extend(name.as_bytes());
            out.extend(data);
            dir.extend(fields(0x0201_4b50));
            dir.extend([0u8; 8]);
            dir.extend([crc.sum().to_le_bytes(), size.to_le_bytes(), size.to_le_bytes()].concat());
            dir.extend([(name.len() as u16).to_le_bytes(), [0; 2], [0; 2], [0; 2], [0; 2]].concat());
            dir.extend([0u8; 4]);
            dir.extend(offset.to_le_bytes());
            dir.extend(name.as_bytes());
        }
        let (start, count) = (out.len() as u32, entries.len() as u16);
        out.extend(&dir);
        out.extend([0x0605_4b50u32.to_le_bytes(), [0; 4]].concat());
        out.extend([count.to_le_bytes(), count.to_le_bytes()].concat());
        out.extend([(dir.len() as u32).to_le_bytes(), start.to_le_bytes()].concat());
        out.extend([0u8; 2]);
        out
    }

    #[test]
    fn tiles_are_found_with_one_walk_of_the_central_directory() {
        // 40 distinct grayscale tiles side by side, behind 1,000 entries that are not tiles.
        let n = 40;
        let mut entries: Vec<(String, Vec<u8>)> = (0..1000).map(|i| (format!("other/{i:060}.bin"), vec![])).collect();
        let mut refs = vec![];
        for i in 0..n {
            let tile = vec![i as u8; TILE * TILE];
            let hash = blake3::hash(&tile).to_hex().to_string();
            refs.push(format!(r#"{{"tx":{i},"ty":0,"hash":"{hash}"}}"#));
            entries.push((format!("tiles/{hash}.zst"), ruzstd::encoding::compress_to_vec(&tile[..], ruzstd::encoding::CompressionLevel::Fastest)));
        }
        let surface = format!(r#"{{"format":{{"mode":"Grayscale","sample":"U8","alpha":false}},"default":"00","tiles":[{}]}}"#, refs.join(","));
        let manifest = format!(
            r#"{{"format_version":1,"generator":"photocraft-format 0.1.0","document":{{"size":{{"width":{},"height":1}},"mode":"Grayscale","layers":[{{"mask":null,"content":{{"kind":"raster","surface":{surface}}}}}]}}}}"#,
            n * TILE
        );
        entries.push(("manifest.json".into(), manifest.into_bytes()));
        let file = stored(&entries);
        let before = crate::zip::SCANS.with(|s| s.get());
        let pixels = pcraft_layer_rgba(&file, &[0], &Limits::default());
        let scans = crate::zip::SCANS.with(|s| s.get()) - before;
        let Ok(Some(pixels)) = pixels else { panic!("not read: {pixels:?}") };
        let Samples::U8(rgba) = &pixels.rgba else { panic!("not 8-bit") };
        assert_eq!(rgba.get((39 * TILE + 5) * 4), Some(&39));
        assert!(scans <= 4, "{scans} walks of the central directory");
    }

    #[test]
    fn shared_tiles_do_not_allocate_a_converted_tile_for_each_reference() {
        let tile = vec![7u8; TILE * TILE];
        let hash = blake3::hash(&tile).to_hex().to_string();
        let refs: Vec<String> = (0..64).map(|i| format!(r#"{{"tx":{i},"ty":0,"hash":"{hash}"}}"#)).collect();
        let manifest = format!(
            r#"{{"format_version":1,"generator":"test","document":{{"size":{{"width":{},"height":1}},"layers":[{{"content":{{"kind":"raster","surface":{{"format":{{"mode":"Grayscale","sample":"U8","alpha":false}},"default":"00","tiles":[{}]}}}}}}]}}}}"#,
            64 * TILE,
            refs.join(",")
        );
        let file = stored(&[
            ("manifest.json".into(), manifest.into_bytes()),
            (format!("tiles/{hash}.zst"), ruzstd::encoding::compress_to_vec(&tile[..], ruzstd::encoding::CompressionLevel::Fastest)),
        ]);
        let before = SAMPLE_BUFFER_BYTES.with(|n| n.get());
        let p = pcraft_layer_rgba(&file, &[0], &Limits::default()).unwrap().unwrap();
        assert!(matches!(p.rgba, Samples::U8(ref rgba) if rgba.len() == 64 * TILE * 4 && rgba[7 * 4] == 7));
        let allocated = SAMPLE_BUFFER_BYTES.with(|n| n.get()) - before;
        assert!(allocated <= 64 * TILE + 1, "{allocated} bytes allocated as converted samples");
    }

    #[test]
    fn a_frame_that_declares_a_window_past_the_limit_is_refused() {
        // Magic, a header without a content size, then a window of 2^(10 + exponent) bytes and
        // one empty last block.
        let frame = |exponent: u8| [0x28, 0xb5, 0x2f, 0xfd, 0x00, exponent << 3, 0x01, 0x00, 0x00];
        assert_eq!(unzstd(&frame(13), 0, "t").ok(), Some(vec![]));
        assert!(matches!(unzstd(&frame(14), 0, "t"), Err(Error::Malformed(m)) if m.contains("zstd")));
    }
}

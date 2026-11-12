#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! GIMP XCF (`.xcf`, `.xcf.gz`), read. Layout from the public specification
//! (<https://developer.gimp.org/core/standards/xcf/>, versions 0 to 26): a header, image
//! properties, then pointers to layer, channel and path structures; each drawable's pixels sit in
//! 64×64 tiles, uncompressed, RLE-coded or zlib-coded. Written from the specification and from
//! files GIMP 3.2 writes; no GIMP code.
//!
//! - [`Xcf::parse`] reads the header, the image properties and every layer's structure into an
//!   [`Image`]: no pixel is decoded.
//! - [`Xcf::layer_pixels`], [`Xcf::layer_mask`] and [`Xcf::channel`] decode one drawable each
//!   into [`Pixels`] (normalised `f32` samples), charging a byte budget the caller owns.
//! - [`Image::tree`] arranges the layers by their item paths.
//! - [`is_xcf`], [`is_gzipped_xcf`] and [`gunzip`] recognise and inflate files.
//!
//! Layer modes, colour tags and composite settings are kept as the numbers GIMP wrote; what
//! they mean to an application is the application's to decide.

#[cfg(test)]
mod tests;
mod tiles;

use std::cell::Cell;
use std::collections::HashMap;
use std::io::Read;

/// Why an XCF file was not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is not XCF, is damaged, or exceeds a limit.
    #[error("invalid XCF file: {0}")]
    Invalid(String),
    /// The caller's `keep_going` returned `false`.
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

const MAGIC: &[u8] = b"gimp xcf ";
/// Canvas and drawable side limit.
pub const MAX_SIDE: u32 = 300_000;
/// The canvas may hold at most this many bytes at 16 bytes per pixel (RGBA `f32`). Also the
/// budget a caller would usually give the decoders.
pub const MAX_PIXEL_BYTES: usize = 1 << 30;
/// Layers and groups in one image.
pub const MAX_LAYERS: usize = 8192;
/// Extra channels in one image.
pub const MAX_CHANNELS: usize = 1024;
/// Group nesting [`Image::tree`] builds.
pub const MAX_DEPTH: usize = 64;
/// Total copied layer and channel metadata (names and item paths). A single reused record with a large
/// name is bounded by [`MAX_STRING`], but repeated layer offsets referencing it must not multiply
/// into unbounded retained metadata.
pub const MAX_METADATA_BYTES: usize = 32 << 20;
/// A string field (layer names, parasite names).
pub const MAX_STRING: usize = 1 << 20;
/// A property payload.
pub const MAX_PAYLOAD: usize = 64 << 20;
/// The largest file a gzipped XCF may inflate to.
pub const MAX_INFLATED: usize = 1 << 30;
/// Highest version this reader was written against; newer files still open (unknown properties
/// are skipped by their length).
pub const KNOWN_VERSION: u32 = 26;
/// The side of a pixel tile.
pub const TILE: u32 = 64;

/// `true` for an XCF file (by its signature).
pub fn is_xcf(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// `true` for a gzip stream named `.xcf.gz` / `.xcfgz` (GIMP writes and reads compressed XCF).
pub fn is_gzipped_xcf(name: &str, bytes: &[u8]) -> bool {
    let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    bytes.starts_with(&[0x1f, 0x8b]) && (lower.ends_with(".xcf.gz") || lower.ends_with(".xcfgz"))
}

/// Inflates a gzipped XCF, refusing one that inflates past [`MAX_INFLATED`].
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut dec = flate2::read::GzDecoder::new(bytes).take(MAX_INFLATED as u64 + 1);
    dec.read_to_end(&mut out).map_err(|e| invalid(format!("gzip: {e}")))?;
    if out.len() > MAX_INFLATED {
        return Err(invalid("the compressed file inflates past the 1 GiB limit"));
    }
    Ok(out)
}

/// How samples are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Precision {
    /// Bytes per sample: 1, 2, 4 or 8.
    pub bytes: usize,
    /// Floating point (16, 32 or 64 bit) rather than unsigned integer.
    pub float: bool,
    /// The values are linear light; otherwise gamma-encoded (the profile's own curve).
    pub linear: bool,
}

impl Precision {
    /// 8-bit gamma integer: every file before version 4.
    pub const GAMMA8: Precision = Precision { bytes: 1, float: false, linear: false };

    /// The precision a header's code means in a file of `version` (the codes changed twice).
    pub fn from_header(version: u32, code: u32) -> Result<Precision> {
        let p = |bytes, float, linear| Ok(Precision { bytes, float, linear });
        match version {
            0..=3 => Ok(Self::GAMMA8),
            4 => match code {
                0 => p(1, false, false),
                1 => p(2, false, false),
                2 => p(4, false, true),
                3 => p(2, true, true),
                4 => p(4, true, true),
                c => Err(invalid(format!("unknown precision {c} in an XCF version 4 file"))),
            },
            5 | 6 => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                400 => p(2, true, true),
                450 => p(2, true, false),
                500 => p(4, true, true),
                550 => p(4, true, false),
                c => Err(invalid(format!("unknown precision {c} in an XCF version {version} file"))),
            },
            _ => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                500 => p(2, true, true),
                550 => p(2, true, false),
                600 => p(4, true, true),
                650 => p(4, true, false),
                700 => p(8, true, true),
                750 => p(8, true, false),
                c => Err(invalid(format!("unknown precision {c}"))),
            },
        }
    }

    /// One sample, big-endian, as a normalised float (integers map to 0..1).
    pub fn sample(self, b: &[u8]) -> f32 {
        match (self.float, self.bytes, b) {
            (false, 1, [v]) => f32::from(*v) / 255.0,
            (false, 2, [a, b]) => f32::from(u16::from_be_bytes([*a, *b])) / 65535.0,
            (false, 4, [a, b, c, d]) => (u32::from_be_bytes([*a, *b, *c, *d]) as f64 / f64::from(u32::MAX)) as f32,
            (true, 2, [a, b]) => half_to_f32(u16::from_be_bytes([*a, *b])),
            (true, 4, [a, b, c, d]) => f32::from_be_bytes([*a, *b, *c, *d]),
            (true, 8, [a, b, c, d, e, f, g, h]) => f64::from_be_bytes([*a, *b, *c, *d, *e, *f, *g, *h]) as f32,
            _ => 0.0,
        }
    }
}

/// An IEEE 754 binary16 value as `f32` (exact: every half is representable).
fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from(h >> 10) & 0x1f;
    let frac = u32::from(h) & 0x3ff;
    let bits = match (exp, frac) {
        (0, 0) => 0,
        (0, f) => {
            // Subnormal: shift the leading one up to the implicit position.
            let shift = f.leading_zeros() - 21;
            ((113 - shift) << 23) | ((f << shift) & 0x3ff) << 13
        }
        (31, f) => 0x7f80_0000 | f << 13,
        (e, f) => ((e + 112) << 23) | f << 13,
    };
    f32::from_bits(sign | bits)
}

/// How tiles are stored (`PROP_COMPRESSION`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Rle,
    Zlib,
}

/// The image's colour model (the header's base type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseType {
    Rgb,
    Gray,
    /// One colormap index per pixel. [`Pixels`] of an indexed image are expanded to RGB.
    Indexed,
}

/// Big-endian reads with bounds checks; every failure is an error, never a panic.
#[derive(Clone, Copy)]
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
    /// XCF 11 and later store pointers as 64-bit.
    wide: bool,
}

impl<'a> Cursor<'a> {
    fn at(&self, pos: usize) -> Cursor<'a> {
        Cursor { b: self.b, pos, wide: self.wide }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| invalid("offset overflow"))?;
        let s = self.b.get(self.pos..end).ok_or_else(|| invalid(format!("the file ends early (wanted {n} bytes at {})", self.pos)))?;
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(be_u32(self.take(4)?).unwrap_or(0))
    }

    fn ptr(&mut self) -> Result<usize> {
        if self.wide {
            let s = self.take(8)?;
            let v = s.first_chunk::<8>().map(|b| u64::from_be_bytes(*b)).unwrap_or(0);
            usize::try_from(v).map_err(|_| invalid("pointer does not fit in memory"))
        } else {
            self.u32().map(|v| v as usize)
        }
    }

    /// A STRING: `n+1` then `n` bytes and a NUL (0 for the empty string).
    fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string_bytes()?).into_owned())
    }

    /// The same bounded STRING read without allocating or decoding an unused field.
    fn string_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > MAX_STRING {
            return Err(invalid(format!("a string of {n} bytes exceeds the limit")));
        }
        let s = self.take(n)?;
        Ok(s.strip_suffix(&[0]).unwrap_or(s))
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// One property record: its type and payload.
struct Prop<'a> {
    kind: u32,
    data: &'a [u8],
}

/// Reads a property list, measuring known payloads whose length words GIMP writes incorrectly.
/// Ends at PROP_END (type 0).
fn props<'a>(c: &mut Cursor<'a>) -> Result<Vec<Prop<'a>>> {
    let mut out = Vec::new();
    loop {
        let kind = c.u32()?;
        let len = c.u32()? as usize;
        if kind == 0 {
            return Ok(out);
        }
        // PROP_COLORMAP's length word was wrong in some releases (n+4 for 3n+4 bytes); its real
        // size follows from the count. PROP_USER_UNIT's length is wrong in every release.
        // PROP_LINK_LAYER's length can be four bytes short; read all its fields.
        let len = match kind {
            1 => {
                let n = c.at(c.pos).u32()? as usize;
                4usize.checked_add(n.checked_mul(3).ok_or_else(|| invalid("colormap overflow"))?).ok_or_else(|| invalid("colormap overflow"))?
            }
            24 => user_unit_len(c)?,
            48 => link_layer_len(c)?,
            _ => len,
        };
        if len > MAX_PAYLOAD {
            return Err(invalid(format!("property {kind} declares {len} bytes")));
        }
        let data = c.take(len)?;
        out.push(Prop { kind, data });
        if out.len() > 4096 {
            return Err(invalid("too many properties"));
        }
    }
}

/// A PROP_LINK_LAYER payload: flags, a path STRING, then the source width and height.
/// The dimensions are present for raster links too, where their values are unused.
fn link_layer_len(c: &Cursor<'_>) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(4)?;
    p.string_bytes()?;
    p.skip(8)?;
    Ok(p.pos - c.pos)
}

/// The real size of a PROP_USER_UNIT payload: a float, a count and three (XCF 21+) or five
/// strings. Measured without consuming.
fn user_unit_len(c: &Cursor<'_>) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(8)?;
    for _ in 0..3 {
        p.string_bytes()?;
    }
    // The two name forms of older files, when present: another string pair before the next
    // property. Decide by whether a plausible property header follows.
    let three = p.pos - c.pos;
    if p.at(p.pos).u32().map(|k| k <= 64).unwrap_or(false) {
        return Ok(three);
    }
    p.string_bytes()?;
    p.string_bytes()?;
    Ok(p.pos - c.pos)
}

fn be_u32(b: &[u8]) -> Option<u32> {
    b.first_chunk::<4>().map(|s| u32::from_be_bytes(*s))
}

fn be_i32(b: &[u8]) -> Option<i32> {
    be_u32(b).map(|v| v as i32)
}

fn be_f32(b: &[u8]) -> Option<f32> {
    be_u32(b).map(f32::from_bits)
}

/// The parasites of a PROP_PARASITES payload as (name, data), in first-seen order; a later
/// duplicate's data wins. A record cut short ends the list.
fn parasites(data: &[u8]) -> Vec<(String, &[u8])> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    let mut out: Vec<(String, &[u8])> = Vec::new();
    // Name to output slot, so a payload of many distinct names is not rescanned per record.
    let mut index: HashMap<String, usize> = HashMap::new();
    while c.pos < data.len() {
        let Ok(name) = c.string() else { break };
        let (Ok(_flags), Ok(len)) = (c.u32(), c.u32()) else { break };
        let Ok(payload) = c.take(len as usize) else { break };
        match index.get(&name).and_then(|&i| out.get_mut(i)) {
            Some(slot) => slot.1 = payload,
            None => {
                index.insert(name.clone(), out.len());
                out.push((name, payload));
            }
        }
    }
    out
}

/// A guide's direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// A horizontal line: `position` is a y coordinate.
    Horizontal,
    /// A vertical line: `position` is an x coordinate.
    Vertical,
}

/// A guide (`PROP_GUIDES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guide {
    pub position: i32,
    pub orientation: Orientation,
}

/// A layer structure, with its properties as GIMP wrote them.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The size its pixels are declared to have.
    pub width: u32,
    pub height: u32,
    /// The layer type: 0 RGB, 1 RGBA, 2 gray, 3 gray + alpha, 4 indexed, 5 indexed + alpha.
    pub layer_type: u32,
    pub name: String,
    /// `PROP_FLOATING_SELECTION`: the layer is a floating selection, not part of the stack.
    pub floating_selection: bool,
    /// 0 to 1: `PROP_FLOAT_OPACITY` when present (it wins whatever the order), else
    /// `PROP_OPACITY` / 255.
    pub opacity: f32,
    /// `PROP_MODE`: GIMP's layer mode number (0 to 22 legacy, 23 and up since 2.10; 61 is
    /// pass-through).
    pub mode: u32,
    pub visible: bool,
    /// `PROP_LOCK_ALPHA`.
    pub lock_alpha: bool,
    /// `PROP_LOCK_CONTENT`.
    pub lock_content: bool,
    /// `PROP_LOCK_POSITION`.
    pub lock_position: bool,
    /// `PROP_APPLY_MASK`: the layer mask is in effect. `true` when the property is absent.
    pub apply_mask: bool,
    /// `PROP_OFFSETS`: where the layer's (and its mask's) top-left pixel sits on the canvas.
    pub x: i32,
    pub y: i32,
    /// `PROP_TEXT_LAYER_FLAGS` or a `gimp-text-layer` parasite: a text layer (its pixels are the
    /// rendered text).
    pub text: bool,
    /// `PROP_VECTOR_LAYER`.
    pub vector: bool,
    /// `PROP_LINK_LAYER`.
    pub link: bool,
    /// `PROP_GROUP_ITEM`: a group; its members follow it with longer item paths.
    pub group: bool,
    /// `PROP_GROUP_ITEM_FLAGS` bit 0. `true` when the property is absent.
    pub expanded: bool,
    /// `PROP_ITEM_PATH`: the layer's index at each level of the tree; empty at the top level of
    /// a file without groups. See [`Image::tree`].
    pub item_path: Vec<u32>,
    /// `PROP_COLOR_TAG`: 0 none, 1 blue, 2 green, 3 yellow, 4 orange, 5 brown, 6 red, 7 violet,
    /// 8 gray.
    pub color_tag: u32,
    /// `PROP_COMPOSITE_MODE`: 0 or ±1 union, ±2 clip to backdrop, ±3 clip to layer,
    /// ±4 intersection (negative: GIMP's automatic choice).
    pub composite_mode: i32,
    /// `PROP_COMPOSITE_SPACE`: ±1 linear, ±2 perceptual.
    pub composite_space: i32,
    /// `PROP_BLEND_SPACE`: ±1 linear, ±2 perceptual.
    pub blend_space: i32,
    /// The layer has a `gimphoto-clipped` parasite (a clipping-mask convention).
    pub clipped: bool,
    /// Non-destructive layer effects (version 20 and later); they are not read.
    pub effects: usize,
    hierarchy: usize,
    mask: usize,
}

impl Layer {
    /// Does the layer type carry an alpha channel?
    pub fn has_alpha(&self) -> bool {
        matches!(self.layer_type, 1 | 3 | 5)
    }
    /// Does the layer point at pixels? Groups usually do not.
    pub fn has_pixels(&self) -> bool {
        self.hierarchy != 0
    }
    /// Does the layer have a mask?
    pub fn has_mask(&self) -> bool {
        self.mask != 0
    }
}

/// A channel structure: a layer mask, the selection or an extra channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    /// `PROP_SELECTION`: this channel is the image's selection.
    pub selection: bool,
}

/// Pixel data of a drawable: `channels` normalised samples per pixel, row-major. A layer has 1
/// (gray), 2 (gray, alpha), 3 (RGB) or 4 (RGBA) channels; a channel has 1. Indexed pixels are
/// expanded to RGB through the colormap.
#[derive(Debug, Clone, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<f32>,
}

/// A layer in [`Image::tree`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TreeNode {
    /// Index into [`Image::layers`].
    pub layer: usize,
    /// The group's members, topmost first.
    pub children: Vec<TreeNode>,
}

/// The layers arranged by their item paths.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    /// The top level, topmost first.
    pub roots: Vec<TreeNode>,
    /// Layers (by index) nested deeper than [`MAX_DEPTH`], moved to the top level.
    pub too_deep: Vec<usize>,
    /// Layers (by index) whose item path leads to no group, put at the top level.
    pub misplaced: Vec<usize>,
}

/// The header and structure of an XCF file.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// The file version: 0 to [`KNOWN_VERSION`], or newer.
    pub version: u32,
    /// Canvas size, 1 to [`MAX_SIDE`].
    pub width: u32,
    pub height: u32,
    pub base_type: BaseType,
    pub precision: Precision,
    pub compression: Compression,
    /// `PROP_COLORMAP`: at most 256 RGB entries.
    pub colormap: Vec<[u8; 3]>,
    /// `PROP_RESOLUTION`: horizontal and vertical pixels per inch, as stored.
    pub resolution: Option<(f32, f32)>,
    /// `PROP_GUIDES`. Files before version 15 could hold stray negative guides; those are
    /// left out, as are guides of an unknown orientation.
    pub guides: Vec<Guide>,
    /// The `icc-profile` parasite, when not empty.
    pub icc_profile: Option<Vec<u8>>,
    /// The `gimp-comment` parasite.
    pub comment: Option<String>,
    /// Paths the file holds (they are not read).
    pub path_count: usize,
    /// Every layer structure in file order, topmost first, floating selections included.
    pub layers: Vec<Layer>,
    /// The number of channel structures (the selection and the extras); read each with
    /// [`Xcf::channel`].
    pub channel_count: usize,
}

impl Image {
    /// Arranges the layers by their item paths, skipping floating selections. A layer without a
    /// path sits at the top level in file order; one whose path leads to no group (at any
    /// depth) is put at the top level, so later siblings keep their indices.
    pub fn tree(&self) -> Tree {
        let mut tree = Tree::default();
        let stack = self.layers.iter().enumerate().filter(|(_, l)| !l.floating_selection);
        for (position, (index, layer)) in stack.enumerate() {
            // Default path: the item's position at the top level, as a file without groups has it.
            let default = [position as u32];
            let mut path: &[u32] = if layer.item_path.is_empty() { &default } else { &layer.item_path };
            if path.len() > MAX_DEPTH {
                tree.too_deep.push(index);
                path = path.get(..1).unwrap_or(path);
            }
            let (parents, _) = path.split_at(path.len().saturating_sub(1));
            let mut probe = &tree.roots;
            let reachable = parents.iter().all(|&i| match probe.get(i as usize) {
                Some(n) if self.layers.get(n.layer).is_some_and(|l| l.group) => {
                    probe = &n.children;
                    true
                }
                _ => false,
            });
            let group = if reachable { parents } else { &[] };
            if !reachable {
                tree.misplaced.push(index);
            }
            if let Some(level) = group.iter().try_fold(&mut tree.roots, |level, &i| level.get_mut(i as usize).map(|n| &mut n.children)) {
                level.push(TreeNode { layer: index, children: Vec::new() });
            }
        }
        tree
    }
}

/// A parsed XCF file: its [`Image`] and the pixels behind it.
pub struct Xcf<'a> {
    c: Cursor<'a>,
    image: Image,
    channels: Vec<usize>,
    /// What is left of [`MAX_METADATA_BYTES`] after the layers; channel names are charged too.
    meta_budget: Cell<usize>,
}

impl<'a> Xcf<'a> {
    /// Reads the header, the image properties and every layer structure of an XCF file
    /// (already inflated when it was gzipped).
    pub fn parse(bytes: &'a [u8]) -> Result<Xcf<'a>> {
        if !is_xcf(bytes) {
            return Err(invalid("not an XCF file"));
        }
        let mut c = Cursor { b: bytes, pos: MAGIC.len(), wide: false };
        let version = match c.take(5)? {
            b"file\0" => 0,
            [b'v', a, b, d, 0] if a.is_ascii_digit() && b.is_ascii_digit() && d.is_ascii_digit() => {
                u32::from(a - b'0') * 100 + u32::from(b - b'0') * 10 + u32::from(d - b'0')
            }
            _ => return Err(invalid("unknown XCF version tag")),
        };
        c.wide = version >= 11;
        let (width, height, base_type) = (c.u32()?, c.u32()?, c.u32()?);
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(invalid(format!("canvas {width}x{height} must be 1 to {MAX_SIDE} pixels a side")));
        }
        if (width as usize).saturating_mul(height as usize).saturating_mul(16) > MAX_PIXEL_BYTES {
            return Err(invalid("the canvas exceeds the 1 GiB pixel limit"));
        }
        let precision = if version >= 4 { Precision::from_header(version, c.u32()?)? } else { Precision::GAMMA8 };
        let base_type = match base_type {
            0 => BaseType::Rgb,
            1 => BaseType::Gray,
            2 => BaseType::Indexed,
            t => return Err(invalid(format!("unknown image type {t}"))),
        };
        let mut image = Image {
            version,
            width,
            height,
            base_type,
            precision,
            compression: Compression::None,
            colormap: Vec::new(),
            resolution: None,
            guides: Vec::new(),
            icc_profile: None,
            comment: None,
            path_count: 0,
            layers: Vec::new(),
            channel_count: 0,
        };
        image_props(&mut c, &mut image)?;

        let pointers = |c: &mut Cursor<'_>, max: usize, what: &str| -> Result<Vec<usize>> {
            let mut out = Vec::new();
            loop {
                let p = c.ptr()?;
                if p == 0 {
                    return Ok(out);
                }
                out.push(p);
                if out.len() > max {
                    return Err(invalid(format!("more than {max} {what}")));
                }
            }
        };
        let layer_ptrs = pointers(&mut c, MAX_LAYERS, "layers")?;
        let channels = pointers(&mut c, MAX_CHANNELS, "channels")?;
        if version >= 18 {
            let mut n = 0;
            while c.ptr().ok().is_some_and(|p| p != 0) {
                n += 1;
                if n > MAX_CHANNELS {
                    break;
                }
            }
            image.path_count += n;
        }
        image.channel_count = channels.len();
        let c = c.at(0);
        let mut meta_budget = MAX_METADATA_BYTES;
        for ptr in layer_ptrs {
            let layer = layer(c.at(ptr), &image, &mut meta_budget)?;
            image.layers.push(layer);
        }
        Ok(Xcf { c, image, channels, meta_budget: Cell::new(meta_budget) })
    }

    /// The file's header and structure.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// Decodes the pixels of layer `index` of [`Image::layers`]; `None` for a layer that has
    /// none (a group). The decoded samples (4 bytes each) are taken from `budget` before
    /// anything is allocated. `keep_going` is asked every 64 tiles.
    pub fn layer_pixels(&self, index: usize, budget: &mut usize, keep_going: &dyn Fn() -> bool) -> Result<Option<Pixels>> {
        let layer = self.image.layers.get(index).ok_or_else(|| invalid("no such layer"))?;
        if layer.hierarchy == 0 {
            return Ok(None);
        }
        let color = if self.image.base_type == BaseType::Rgb { 3 } else { 1 };
        let source_channels = color + usize::from(layer.has_alpha());
        let indexed = self.image.base_type == BaseType::Indexed;
        let px = self.hierarchy(layer.hierarchy, source_channels, indexed, budget, keep_going)?;
        if (px.width, px.height) != (layer.width, layer.height) {
            let (name, lw, lh) = (&layer.name, layer.width, layer.height);
            return Err(invalid(format!("layer '{name}' pixels are {}x{} where {lw}x{lh} were declared", px.width, px.height)));
        }
        Ok(Some(px))
    }

    /// Decodes the mask of layer `index`: one channel, placed at the layer's offset. `None`
    /// for a layer without one.
    pub fn layer_mask(&self, index: usize, budget: &mut usize, keep_going: &dyn Fn() -> bool) -> Result<Option<(Channel, Pixels)>> {
        let layer = self.image.layers.get(index).ok_or_else(|| invalid("no such layer"))?;
        if layer.mask == 0 {
            return Ok(None);
        }
        self.channel_at(layer.mask, budget, keep_going).map(Some)
    }

    /// Decodes channel `index` (below [`Image::channel_count`]): the selection or an extra
    /// channel, the size of the canvas.
    pub fn channel(&self, index: usize, budget: &mut usize, keep_going: &dyn Fn() -> bool) -> Result<(Channel, Pixels)> {
        let ptr = *self.channels.get(index).ok_or_else(|| invalid("no such channel"))?;
        self.channel_at(ptr, budget, keep_going)
    }

    fn channel_at(&self, ptr: usize, budget: &mut usize, keep_going: &dyn Fn() -> bool) -> Result<(Channel, Pixels)> {
        let mut c = self.c.at(ptr);
        let (_w, _h) = (c.u32()?, c.u32()?);
        let name = c.string()?;
        // As for layers: repeated channel pointers must not retain unbounded copies of one name.
        self.meta_budget.set(self.meta_budget.get().checked_sub(name.len()).ok_or_else(|| invalid("channel metadata exceeds the budget"))?);
        let selection = props(&mut c)?.iter().any(|p| p.kind == 4);
        let hptr = c.ptr()?;
        // Channels are always one plain sample per pixel, even in indexed images.
        let px = self.hierarchy(hptr, 1, false, budget, keep_going)?;
        Ok((Channel { name, selection }, px))
    }

    /// Reads a hierarchy: the first level's tiles into floats, `source_channels` as stored
    /// (indexed pixels expand to RGB here).
    fn hierarchy(&self, hptr: usize, source_channels: usize, indexed: bool, budget: &mut usize, keep_going: &dyn Fn() -> bool) -> Result<Pixels> {
        let precision = self.image.precision;
        let mut c = self.c.at(hptr);
        let (w, h, bpp) = (c.u32()?, c.u32()?, c.u32()? as usize);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(invalid(format!("drawable size {w}x{h} is out of range")));
        }
        let expected_bpp = source_channels.checked_mul(precision.bytes).ok_or_else(|| invalid("bpp overflow"))?;
        if bpp != expected_bpp {
            return Err(invalid(format!("{bpp} bytes per pixel where {expected_bpp} were expected")));
        }
        let channels = if indexed { source_channels + 2 } else { source_channels };
        let count = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(channels)).ok_or_else(|| invalid("drawable overflow"))?;
        let bytes = count.checked_mul(4).ok_or_else(|| invalid("drawable overflow"))?;
        *budget = budget.checked_sub(bytes).ok_or_else(|| invalid("the layers exceed the pixel limit"))?;
        let lptr = c.ptr()?;
        let mut l = self.c.at(lptr);
        if (l.u32()?, l.u32()?) != (w, h) {
            return Err(invalid("the level does not match its hierarchy"));
        }
        let across = w.div_ceil(TILE) as usize;
        let down = h.div_ceil(TILE) as usize;
        let mut tiles = Vec::with_capacity(across * down);
        for _ in 0..across * down {
            let p = l.ptr()?;
            if p == 0 {
                return Err(invalid("the tile list ends before every tile"));
            }
            tiles.push(p);
        }
        let mut data = vec![0.0f32; count];
        let stride = w as usize * channels;
        let side = TILE as usize;
        for (i, &at) in tiles.iter().enumerate() {
            if i % 64 == 0 && !keep_going() {
                return Err(Error::Cancelled);
            }
            let (tx, ty) = (i % across, i / across);
            let (tw, th) = ((w as usize - tx * side).min(side), (h as usize - ty * side).min(side));
            let end = tiles.get(i + 1).copied().filter(|&n| n > at).unwrap_or(self.c.b.len());
            let input = self.c.b.get(at..end.min(self.c.b.len())).ok_or_else(|| invalid("a tile pointer is outside the file"))?;
            let raw = tiles::decode(input, self.image.compression, bpp, tw * th)?;
            for (py, row) in raw.chunks_exact(tw * bpp).enumerate() {
                let start = (ty * side + py) * stride + tx * side * channels;
                let Some(out) = data.get_mut(start..start + tw * channels) else { continue };
                for (pixel, dst) in row.chunks_exact(bpp).zip(out.chunks_exact_mut(channels)) {
                    if indexed {
                        let idx = pixel.first().copied().unwrap_or(0);
                        let rgb = self.image.colormap.get(usize::from(idx)).copied().unwrap_or([idx; 3]);
                        let alpha = pixel.get(1).copied();
                        for (d, v) in dst.iter_mut().zip(rgb.into_iter().chain(alpha)) {
                            *d = f32::from(v) / 255.0;
                        }
                    } else {
                        for (d, s) in dst.iter_mut().zip(pixel.chunks_exact(precision.bytes)) {
                            *d = precision.sample(s);
                        }
                    }
                }
            }
        }
        Ok(Pixels { width: w, height: h, channels, data })
    }
}

fn image_props(c: &mut Cursor<'_>, image: &mut Image) -> Result<()> {
    for p in &props(c)? {
        match p.kind {
            1 => {
                let n = be_u32(p.data).unwrap_or(0) as usize;
                image.colormap = p.data.get(4..).unwrap_or(&[]).as_chunks::<3>().0.iter().take(n.min(256)).copied().collect();
            }
            17 => {
                image.compression = match p.data.first() {
                    Some(0) => Compression::None,
                    Some(1) => Compression::Rle,
                    Some(2) => Compression::Zlib,
                    Some(k) => return Err(invalid(format!("unknown compression {k}"))),
                    None => return Err(invalid("empty compression property")),
                }
            }
            18 => {
                for g in p.data.as_chunks::<5>().0 {
                    let position = i32::from_be_bytes([g[0], g[1], g[2], g[3]]);
                    let orientation = match g[4] {
                        1 => Orientation::Horizontal,
                        2 => Orientation::Vertical,
                        _ => continue,
                    };
                    if position >= 0 || image.version >= 15 {
                        image.guides.push(Guide { position, orientation });
                    }
                }
            }
            19 => {
                if let (Some(x), Some(y)) = (be_f32(p.data), p.data.get(4..).and_then(be_f32)) {
                    image.resolution = Some((x, y));
                }
            }
            21 => {
                for (name, data) in parasites(p.data) {
                    match name.as_str() {
                        "icc-profile" if !data.is_empty() => image.icc_profile = Some(data.to_vec()),
                        "gimp-comment" => image.comment = Some(String::from_utf8_lossy(data).trim_end_matches('\0').to_string()),
                        _ => {}
                    }
                }
            }
            23 => image.path_count += be_u32(p.data.get(4..).unwrap_or(&[])).unwrap_or(0) as usize,
            25 => image.path_count += be_u32(p.data.get(8..).unwrap_or(&[])).unwrap_or(0) as usize,
            _ => {}
        }
    }
    Ok(())
}

/// Reads a layer structure at `c`.
fn layer(mut c: Cursor<'_>, image: &Image, meta_budget: &mut usize) -> Result<Layer> {
    let (width, height, layer_type) = (c.u32()?, c.u32()?, c.u32()?);
    let name = c.string()?;
    // Charge the owned name against the image-wide metadata budget: repeated layer offsets
    // referencing one large name must not retain unbounded copies.
    *meta_budget = meta_budget.checked_sub(name.len()).ok_or_else(|| invalid("layer metadata exceeds the budget"))?;
    let lprops = props(&mut c)?;
    let hierarchy = c.ptr()?;
    let mask = c.ptr()?;
    let mut effects = 0usize;
    if image.version >= 20 {
        while c.ptr()? != 0 {
            effects += 1;
            if effects > 1024 {
                return Err(invalid("too many layer effects"));
            }
        }
    }
    let type_rgb = match layer_type {
        0 | 1 => true,
        2..=5 => false,
        k => return Err(invalid(format!("unknown layer type {k} on '{name}'"))),
    };
    if type_rgb != (image.base_type != BaseType::Gray) && image.base_type != BaseType::Indexed {
        return Err(invalid(format!("layer '{name}' does not match the image's colour model")));
    }
    let mut l = Layer {
        width,
        height,
        layer_type,
        name,
        floating_selection: false,
        opacity: 1.0,
        mode: 0,
        visible: true,
        lock_alpha: false,
        lock_content: false,
        lock_position: false,
        apply_mask: true,
        x: 0,
        y: 0,
        text: false,
        vector: false,
        link: false,
        group: false,
        expanded: true,
        item_path: Vec::new(),
        color_tag: 0,
        composite_mode: 0,
        composite_space: 0,
        blend_space: 0,
        clipped: false,
        effects,
        hierarchy,
        mask,
    };
    // PROP_FLOAT_OPACITY wins over PROP_OPACITY whatever their order in the file.
    let mut float_opacity = None;
    for p in &lprops {
        let v = be_u32(p.data).unwrap_or(0);
        match p.kind {
            5 => l.floating_selection = true,
            6 => l.opacity = (v.min(255) as f32) / 255.0,
            33 => float_opacity = be_f32(p.data).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).or(float_opacity),
            7 => l.mode = v,
            8 => l.visible = v != 0,
            10 => l.lock_alpha = v != 0,
            11 => l.apply_mask = v != 0,
            15 => {
                l.x = be_i32(p.data).unwrap_or(0);
                l.y = p.data.get(4..).and_then(be_i32).unwrap_or(0);
            }
            26 => l.text = true,
            28 => l.lock_content = v != 0,
            29 => l.group = true,
            30 => {
                l.item_path = p.data.as_chunks::<4>().0.iter().map(|s| u32::from_be_bytes(*s)).collect();
                *meta_budget =
                    meta_budget.checked_sub(l.item_path.len().saturating_mul(4)).ok_or_else(|| invalid("layer metadata exceeds the budget"))?;
            }
            31 => l.expanded = v & 1 != 0,
            32 => l.lock_position = v != 0,
            34 => l.color_tag = v,
            35 => l.composite_mode = be_i32(p.data).unwrap_or(0),
            36 => l.composite_space = be_i32(p.data).unwrap_or(0),
            37 => l.blend_space = be_i32(p.data).unwrap_or(0),
            47 => l.vector = true,
            48 => l.link = true,
            21 => {
                for (pname, _) in parasites(p.data) {
                    match pname.as_str() {
                        "gimphoto-clipped" => l.clipped = true,
                        "gimp-text-layer" => l.text = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    l.opacity = float_opacity.unwrap_or(l.opacity);
    Ok(l)
}

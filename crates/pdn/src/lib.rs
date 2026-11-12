#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Paint.NET documents (`.pdn`), read. Only the PDN3 layout: the `PDN3` signature, an XML
//! header (skipped), a .NET binary serialization stream ([`nrbf`]) holding a
//! `PaintDotNet.Document` with its bitmap layers, then one block of pixels per surface, in
//! numbered chunks that are each a gzip stream or raw bytes.
//!
//! The layout follows the reader of the pypdn project (MIT); the serialization records follow
//! Microsoft's public MS-NRBF specification.
//!
//! - [`Document::parse`] reads the structure: canvas size, [`Layer`]s with their name,
//!   visibility, opacity and [`BlendMode`], and which pixel block each layer shows. No pixel is
//!   decoded, and every allocation the pixels will need is checked first.
//! - [`Document::read_surfaces`] then decodes the pixel blocks in file order, handing each
//!   [`Surface`] (rows of B, G, R, A bytes) to a callback.

pub mod nrbf;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Read;

use nrbf::{Graph, Reader, Value};

/// Why a Paint.NET document was not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is not a PDN3 document, is damaged, uses something this crate does not read, or
    /// exceeds a limit.
    #[error("invalid Paint.NET document: {0}")]
    Invalid(String),
    /// The caller's `keep_going` returned `false`.
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn check(keep_going: &dyn Fn() -> bool) -> Result<()> {
    if keep_going() { Ok(()) } else { Err(Error::Cancelled) }
}

/// The canvas, and all pixel blocks together, may hold at most this many bytes.
pub const MAX_PIXEL_BYTES: usize = 1 << 30;
/// Layers in one document.
pub const MAX_LAYERS: usize = 1024;
/// Longest layer name, in bytes.
pub const MAX_NAME_BYTES: usize = 64 * 1024;
/// Most bytes of layer names in one document. The graph holds a shared string once; every layer
/// that refers to it gets its own copy.
pub const MAX_TOTAL_NAME_BYTES: usize = 4 * 1024 * 1024;

/// `true` for a PDN3 document (by its signature).
pub fn is_pdn(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PDN3")
}

/// A layer's blend mode: Paint.NET's `LayerBlendMode`, in its numeric order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlendMode {
    Normal,
    Multiply,
    Additive,
    /// Paint.NET's own formula, not Photoshop's Color Burn.
    ColorBurn,
    /// Paint.NET's own formula, not Photoshop's Color Dodge.
    ColorDodge,
    Reflect,
    Glow,
    Overlay,
    Difference,
    Negation,
    Lighten,
    Darken,
    Screen,
    Xor,
}

impl BlendMode {
    /// Every mode, indexed by its `LayerBlendMode` value.
    pub const ALL: [BlendMode; 14] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Additive,
        BlendMode::ColorBurn,
        BlendMode::ColorDodge,
        BlendMode::Reflect,
        BlendMode::Glow,
        BlendMode::Overlay,
        BlendMode::Difference,
        BlendMode::Negation,
        BlendMode::Lighten,
        BlendMode::Darken,
        BlendMode::Screen,
        BlendMode::Xor,
    ];

    /// The name older files use in `PaintDotNet.UserBlendOps+<name>BlendOp`.
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Additive => "Additive",
            BlendMode::ColorBurn => "ColorBurn",
            BlendMode::ColorDodge => "ColorDodge",
            BlendMode::Reflect => "Reflect",
            BlendMode::Glow => "Glow",
            BlendMode::Overlay => "Overlay",
            BlendMode::Difference => "Difference",
            BlendMode::Negation => "Negation",
            BlendMode::Lighten => "Lighten",
            BlendMode::Darken => "Darken",
            BlendMode::Screen => "Screen",
            BlendMode::Xor => "Xor",
        }
    }
}

/// A bitmap layer, bottom first in [`Document::layers`]. Every layer is the size of the canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    /// 0 to 255.
    pub opacity: u8,
    pub blend: BlendMode,
    /// Which surface [`Document::read_surfaces`] hands out holds this layer's pixels. Layers may
    /// share one.
    pub surface: usize,
}

/// One decoded pixel block: `height` rows of `stride` bytes, each starting with `width` pixels
/// of B, G, R, A bytes (straight alpha).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    pub width: usize,
    pub stride: usize,
    pub data: Vec<u8>,
}

impl Surface {
    /// The rows, top first, each cut to its `width × 4` pixel bytes (B, G, R, A).
    pub fn rows(&self) -> impl Iterator<Item = &[u8]> {
        self.data.chunks_exact(self.stride.max(1)).filter_map(|row| row.get(..self.width * 4))
    }
}

struct Block {
    length: usize,
    stride: usize,
}

/// The structure of a PDN3 document. Parse first, then read the pixels.
pub struct Document<'a> {
    /// Canvas size in pixels.
    pub width: usize,
    pub height: usize,
    /// The layers, bottom first.
    pub layers: Vec<Layer>,
    /// The document carries metadata items (`userMetadataItems`); they are not read.
    pub has_user_metadata: bool,
    bytes: &'a [u8],
    /// Where the pixel blocks start.
    pixels_at: usize,
    blocks: Vec<Block>,
}

fn count(graph: &Graph, object: &Value, field: &str) -> Result<usize> {
    usize::try_from(graph.integer(graph.field(object, field)?)?).map_err(|_| invalid(format!("invalid {field}")))
}

fn blend(graph: &Graph, layer: &Value, props: &Value) -> Result<BlendMode> {
    if let Some(mode) = graph.optional_field(props, "blendMode")? {
        let mode = match graph.resolve(mode)? {
            Value::Int(n) => *n,
            _ => graph.integer(graph.field(mode, "value__")?)?,
        };
        let mode = usize::try_from(mode).map_err(|_| invalid("invalid blend mode"))?;
        return BlendMode::ALL.get(mode).copied().ok_or_else(|| invalid(format!("unsupported blend mode {mode}")));
    }
    let props = graph.field(layer, "properties")?;
    let mode = graph.class_name(graph.field(props, "blendOp")?)?;
    let mode = mode
        .strip_prefix("PaintDotNet.UserBlendOps+")
        .and_then(|s| s.strip_suffix("BlendOp"))
        .ok_or_else(|| invalid(format!("unsupported blend operation {mode}")))?;
    BlendMode::ALL.iter().copied().find(|m| m.name() == mode).ok_or_else(|| invalid(format!("unsupported blend operation {mode}")))
}

impl<'a> Document<'a> {
    /// Reads the header and the serialized document. `keep_going` is asked throughout; `false`
    /// ends the read with [`Error::Cancelled`].
    pub fn parse(bytes: &'a [u8], keep_going: &dyn Fn() -> bool) -> Result<Document<'a>> {
        let mut reader = Reader { bytes, pos: 0 };
        if reader.take(4)? != b"PDN3" {
            return Err(invalid("unsupported document signature (expected PDN3)"));
        }
        let header_size = match reader.take(3)? {
            [a, b, c] => u32::from_le_bytes([*a, *b, *c, 0]) as usize,
            _ => return Err(invalid("invalid header")),
        };
        reader.take(header_size)?;
        if reader.take(2)? != [0, 1] {
            return Err(invalid("unsupported PDN3 serialization version"));
        }
        let graph = nrbf::parse_from(&mut reader, keep_going)?;
        let root = &graph.root;
        if graph.class_name(root)? != "PaintDotNet.Document" {
            return Err(invalid("root object is not a Paint.NET document"));
        }
        let width = count(&graph, root, "width")?;
        let height = count(&graph, root, "height")?;
        let row_bytes = width.checked_mul(4).ok_or_else(|| invalid("canvas size overflow"))?;
        let canvas_bytes = row_bytes.checked_mul(height).ok_or_else(|| invalid("canvas size overflow"))?;
        if width == 0 || height == 0 || width > i32::MAX as usize || height > i32::MAX as usize || canvas_bytes > MAX_PIXEL_BYTES {
            return Err(invalid("canvas is empty or exceeds the 1 GiB pixel limit"));
        }
        let list = graph.field(root, "layers")?;
        let items = graph.array(graph.field(list, "ArrayList+_items")?)?;
        let len = count(&graph, list, "ArrayList+_size")?;
        if len == 0 || len > MAX_LAYERS {
            return Err(invalid("document must have 1..1024 bitmap layers"));
        }
        let items = items.get(..len).ok_or_else(|| invalid("layer list count exceeds its array"))?;
        // Memory block id to the stride of the surfaces that use it.
        let mut layouts = HashMap::new();
        let mut layers = Vec::with_capacity(len);
        let mut name_bytes = 0usize;
        for layer in items {
            check(keep_going)?;
            if graph.class_name(layer)? != "PaintDotNet.BitmapLayer" {
                return Err(invalid("only PDN3 bitmap layers are supported"));
            }
            if count(&graph, layer, "Layer+width")? != width || count(&graph, layer, "Layer+height")? != height {
                return Err(invalid("layer dimensions do not match the canvas"));
            }
            let surface = graph.field(layer, "surface")?;
            if graph.class_name(surface)? != "PaintDotNet.Surface"
                || count(&graph, surface, "width")? != width
                || count(&graph, surface, "height")? != height
            {
                return Err(invalid("invalid bitmap surface"));
            }
            let stride = count(&graph, surface, "stride")?;
            if stride < row_bytes {
                return Err(invalid("surface stride is too short for BGRA32 pixels"));
            }
            let Value::Ref(id) = graph.field(surface, "scan0")? else { return Err(invalid("missing pixel memory reference")) };
            if let Some(old) = layouts.insert(*id, stride)
                && old != stride
            {
                return Err(invalid("shared pixel memory has inconsistent strides"));
            }
            let props = graph.field(layer, "Layer+properties")?;
            let opacity = u8::try_from(count(&graph, props, "opacity")?).map_err(|_| invalid("opacity exceeds 255"))?;
            let name = graph.string(graph.field(props, "name")?)?;
            if name.len() > MAX_NAME_BYTES {
                return Err(invalid("layer name is too long"));
            }
            name_bytes = name_bytes.saturating_add(name.len());
            if name_bytes > MAX_TOTAL_NAME_BYTES {
                return Err(invalid("layer names exceed the total metadata budget"));
            }
            let visible = graph.boolean(graph.field(props, "visible")?)?;
            layers.push((*id, Layer { name: name.to_string(), visible, opacity, blend: blend(&graph, layer, props)?, surface: 0 }));
        }
        // Validate every allocation before any block is decoded.
        let mut blocks = Vec::new();
        let mut index_of = HashMap::new();
        let mut total = 0usize;
        for (id, block) in graph.objects_of_class("PaintDotNet.MemoryBlock") {
            let length = count(&graph, block, "length64")?;
            let stride = *layouts.get(&id).ok_or_else(|| invalid("unreferenced pixel memory block"))?;
            if length != stride.checked_mul(height).ok_or_else(|| invalid("pixel length overflow"))? {
                return Err(invalid("pixel memory length does not match the surface"));
            }
            if graph.boolean(graph.field(block, "hasParent")?)? || !graph.boolean(graph.field(block, "deferred")?)? {
                return Err(invalid("unsupported non-deferred or parented pixel memory"));
            }
            total = total.checked_add(length).ok_or_else(|| invalid("pixel length overflow"))?;
            if total > MAX_PIXEL_BYTES {
                return Err(invalid("decoded pixels exceed the 1 GiB limit"));
            }
            index_of.insert(id, blocks.len());
            blocks.push(Block { length, stride });
        }
        let layers = layers
            .into_iter()
            .map(|(id, layer)| Ok(Layer { surface: *index_of.get(&id).ok_or_else(|| invalid("missing pixel memory block"))?, ..layer }))
            .collect::<Result<Vec<_>>>()?;
        let has_user_metadata = match graph.optional_field(root, "userMetadataItems")? {
            Some(meta) if !matches!(graph.resolve(meta)?, Value::Null) => !graph.array(meta)?.is_empty(),
            _ => false,
        };
        Ok(Document { width, height, layers, has_user_metadata, bytes, pixels_at: reader.pos, blocks })
    }

    /// How many surfaces [`Self::read_surfaces`] hands out.
    pub fn surface_count(&self) -> usize {
        self.blocks.len()
    }

    /// Bytes all surfaces take once decoded: at most [`MAX_PIXEL_BYTES`].
    pub fn decoded_bytes(&self) -> usize {
        self.blocks.iter().map(|b| b.length).sum()
    }

    /// Decodes the pixel blocks in file order and hands each to `each` with its index (what
    /// [`Layer::surface`] names). Only one decoded surface is held at a time unless `each` keeps
    /// them. Data after the last block is an error.
    pub fn read_surfaces(&self, keep_going: &dyn Fn() -> bool, each: &mut dyn FnMut(usize, Surface)) -> Result<()> {
        let mut reader = Reader { bytes: self.bytes, pos: self.pixels_at };
        for (i, block) in self.blocks.iter().enumerate() {
            let data = pixels(&mut reader, block.length, keep_going)?;
            each(i, Surface { width: self.width, stride: block.stride, data });
        }
        if reader.pos != self.bytes.len() {
            return Err(invalid("unexpected trailing data"));
        }
        Ok(())
    }
}

/// Reads a memory block's numbered chunks in their on-disk order; each chunk has its own gzip
/// stream and checksum. Inflates directly into the bounded destination, checking its exact size.
fn pixels(reader: &mut Reader<'_>, length: usize, keep_going: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let version = reader.byte()?;
    if version > 1 {
        return Err(invalid(format!("unsupported pixel compression {version}")));
    }
    let chunk_size = reader.be_u32()?;
    if chunk_size == 0 {
        return Err(invalid("zero pixel chunk size"));
    }
    let chunks = length.div_ceil(chunk_size);
    if chunks > 1_000_000 {
        return Err(invalid("too many pixel chunks"));
    }
    // Uncompressed chunks hold every pixel byte: a file shorter than that is truncated, and is
    // rejected before the buffer is allocated.
    if version == 1 && reader.bytes.len().saturating_sub(reader.pos) < length {
        return Err(invalid("uncompressed pixel data is truncated"));
    }
    let mut seen = vec![false; chunks];
    let mut data = vec![0; length];
    for _ in 0..chunks {
        check(keep_going)?;
        let index = reader.be_u32()?;
        let found = seen.get_mut(index).ok_or_else(|| invalid("pixel chunk index out of bounds"))?;
        if *found {
            return Err(invalid("duplicate pixel chunk"));
        }
        *found = true;
        let size = reader.be_u32()?;
        let raw = reader.take(size)?;
        let start = index.checked_mul(chunk_size).ok_or_else(|| invalid("pixel offset overflow"))?;
        let end = start.checked_add(chunk_size.min(length.saturating_sub(start))).ok_or_else(|| invalid("pixel offset overflow"))?;
        let dst = data.get_mut(start..end).ok_or_else(|| invalid("invalid pixel chunk range"))?;
        if version == 0 {
            let mut decoder = flate2::bufread::GzDecoder::new(raw);
            for band in dst.chunks_mut(1 << 20) {
                check(keep_going)?;
                decoder.read_exact(band).map_err(|e| invalid(format!("invalid gzip pixel chunk: {e}")))?;
            }
            let mut extra = [0];
            if decoder.read(&mut extra).map_err(|e| invalid(format!("invalid gzip pixel checksum: {e}")))? != 0 || !decoder.into_inner().is_empty() {
                return Err(invalid("pixel chunk has excess data"));
            }
        } else {
            if raw.len() != dst.len() {
                return Err(invalid("incorrect uncompressed pixel chunk size"));
            }
            dst.copy_from_slice(raw);
        }
    }
    Ok(data)
}

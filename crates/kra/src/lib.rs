#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Krita documents (`.kra`, `.krz`), read. Layout observed from files Krita writes (no Krita code
//! used): a ZIP whose first, stored entry is `mimetype` = `application/x-krita`, a `maindoc.xml`
//! node tree, one tiled pixel file per paint layer and mask under `<image name>/layers/`, the
//! image's ICC profile in `<image name>/annotations/icc`, and Krita's rendering in
//! `mergedimage.png`.
//!
//! - [`Archive::open`] reads the archive and `maindoc.xml` into an [`Image`]: the node tree with
//!   each node's attributes as the file has them ([`Node`]). No pixel is read.
//! - [`Archive::layer_file`] reads one pixel file, [`tiles::TileFile`] parses it and decodes a
//!   tile at a time, and [`tiles::Space`] says how a pixel's bytes are laid out (RGB pixels are
//!   in Krita's BGRA order, 16-bit samples little-endian).
//! - [`Archive::default_pixel`], [`Archive::icc_profile`] and [`Archive::merged_image`] read the
//!   rest.
//!
//! What a node kind or a composite operation means to an application is the application's to
//! decide: `nodetype` and `compositeop` are kept as the strings Krita wrote.

#[cfg(test)]
mod tests;
pub mod tiles;

use craft_xml::El;
use tiles::Space;

/// Why a Krita document was not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is not a Krita archive, is damaged, or exceeds a limit.
    #[error("invalid Krita document: {0}")]
    Invalid(String),
    /// The file is a kind of Krita data this crate does not read.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// The `mimetype` entry of a Krita archive.
pub const MIMETYPE: &[u8] = b"application/x-krita";
/// File extensions of Krita documents.
pub const EXTENSIONS: [&str; 2] = ["kra", "krz"];
/// Canvas side limit; node offsets and tile positions may be up to four times this.
pub const MAX_SIDE: u32 = 300_000;
/// The canvas may hold at most this many bytes at 8 bytes per pixel (16-bit RGBA).
pub const MAX_PIXEL_BYTES: usize = 1 << 30;
/// `<layer>` nodes in one document.
pub const MAX_NODES: usize = 8192;
/// Largest `maindoc.xml`.
pub const MAX_MAINDOC: usize = 32 << 20;
/// Largest pixel file, inflated.
pub const MAX_LAYER_FILE: usize = 1 << 30;
/// Most entries the archive may declare; checked before the archive is indexed.
pub const MAX_ENTRIES: u64 = 100_000;
/// Largest ICC profile.
pub const MAX_ICC: usize = 16 << 20;
/// Deepest element nesting of `maindoc.xml`.
pub const MAX_XML_DEPTH: usize = 512;

/// The payload of a ZIP's first entry when it is a stored file named `mimetype` of at most 256
/// bytes, which is how Krita (and OpenRaster, OpenDocument, EPUB) mark their archives so the
/// type is readable without unzipping.
pub fn zip_mimetype(bytes: &[u8]) -> Option<&[u8]> {
    craft_zip::stored_mimetype(bytes)
}

/// `true` for a Krita archive (by its `mimetype` entry).
pub fn is_kra(bytes: &[u8]) -> bool {
    zip_mimetype(bytes).is_some_and(|mime| mime.trim_ascii() == MIMETYPE)
}

/// A `<layer>` or `<mask>` element of `maindoc.xml`, with the attributes Krita writes on it.
/// Text attributes are trimmed; one the element does not have is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// `nodetype`: `paintlayer`, `grouplayer`, `shapelayer`, `adjustmentlayer`, `generatorlayer`,
    /// `filelayer`, `clonelayer`; for masks `transparencymask`, `selectionmask`, `filtermask`,
    /// `transformmask`, `colorizemask`. Empty when the attribute is missing.
    pub node_type: String,
    pub name: Option<String>,
    /// `visible`: everything but `0` is visible.
    pub visible: bool,
    /// `opacity`, 0 to 255 as stored (not clamped). 255 when missing.
    pub opacity: f64,
    pub locked: bool,
    /// `compositeop`: Krita's composite operation id (`normal`, `multiply`, `soft_light_svg`, ...).
    pub composite_op: Option<String>,
    /// `channelflags`: one `0` or `1` per channel; a `0` locks that channel.
    pub channel_flags: Option<String>,
    pub inherit_alpha: bool,
    /// `keyframes`: the name of the node's animation file, when it is animated.
    pub keyframes: Option<String>,
    /// `x`, `y`: where the node's pixel file sits. A group's own offset is not applied to its
    /// children: Krita moves a group by moving their pixels.
    pub x: i32,
    pub y: i32,
    /// `colorspacename` of a paint layer; see [`Space::from_name`].
    pub colorspace_name: Option<String>,
    /// `filename`: the pixel file's name under `<image name>/layers/`. A mask's pixels are in
    /// `<filename>.pixelselection`.
    pub filename: Option<String>,
    pub passthrough: bool,
    pub collapsed: bool,
    /// The `<layer>` children, topmost first as the file lists them.
    pub layers: Vec<Node>,
    /// The `<mask>` children, in file order.
    pub masks: Vec<Node>,
}

/// The `<IMAGE>` element of `maindoc.xml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    /// `name`: also the archive directory the image's files are under.
    pub name: String,
    /// Canvas size in pixels, 1 to [`MAX_SIDE`].
    pub width: u32,
    pub height: u32,
    /// `colorspacename`: `RGBA`, `RGBA16`, `GRAYA`, `GRAYAU16`, `CMYKA`, `LABA`, `RGBAF32`, ...
    pub colorspace_name: String,
    /// `x-res`, `y-res` in pixels per inch; 72 when missing.
    pub x_res: f64,
    pub y_res: f64,
    /// The top-level `<layer>` nodes, topmost first.
    pub layers: Vec<Node>,
    /// The image has an `<assistants>` element (drawing assistants).
    pub assistants: bool,
    /// The image has a `<ReferenceImages>` element.
    pub reference_images: bool,
}

fn attr<'a>(el: &'a El, name: &str) -> Option<&'a str> {
    el.attr(name).map(str::trim)
}

fn number(el: &El, name: &str, default: f64) -> Result<f64> {
    match attr(el, name) {
        None | Some("") => Ok(default),
        Some(v) => v.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| invalid(format!("invalid {name} attribute `{v}`"))),
    }
}

fn offset(el: &El, name: &str) -> Result<i32> {
    let v = number(el, name, 0.0)?.round();
    if v.abs() > f64::from(MAX_SIDE) * 4.0 {
        return Err(invalid(format!("{name} offset {v} is out of range")));
    }
    Ok(v as i32)
}

fn truthy(el: &El, name: &str) -> bool {
    matches!(attr(el, name), Some("1" | "true"))
}

/// Child elements named `tag` inside the element's `<container>` elements (`layers` or `masks`).
fn children<'a>(el: &'a El, container: &'static str, tag: &'static str) -> impl Iterator<Item = &'a El> {
    el.children(container).flat_map(move |c| c.children(tag))
}

fn node(el: &El, is_mask: bool, count: &mut usize) -> Result<Node> {
    let mut out = node_fields(el, is_mask)?;
    if !is_mask {
        out.layers = layers(el, count)?;
        out.masks = children(el, "masks", "mask").map(|m| node(m, true, count)).collect::<Result<_>>()?;
    }
    Ok(out)
}

/// A node's own attributes, without its children.
fn node_fields(el: &El, is_mask: bool) -> Result<Node> {
    let owned = |name: &str| attr(el, name).map(str::to_string);
    let node_type = attr(el, "nodetype").unwrap_or("").to_string();
    // Only the kinds whose pixels are read need sound numbers; a node an application will leave
    // out is not a reason to refuse the document.
    let read = matches!(node_type.as_str(), "paintlayer" | "grouplayer" | "transparencymask");
    let lenient = |v: Result<f64>, default: f64| if read { v } else { Ok(v.unwrap_or(default)) };
    let (x, y) = if is_mask && !read { (offset(el, "x").unwrap_or(0), offset(el, "y").unwrap_or(0)) } else { (offset(el, "x")?, offset(el, "y")?) };
    let out = Node {
        node_type,
        name: owned("name"),
        visible: attr(el, "visible") != Some("0"),
        opacity: lenient(number(el, "opacity", 255.0), 255.0)?,
        locked: truthy(el, "locked"),
        composite_op: owned("compositeop"),
        channel_flags: owned("channelflags"),
        inherit_alpha: truthy(el, "inheritalpha"),
        keyframes: owned("keyframes"),
        x,
        y,
        colorspace_name: owned("colorspacename"),
        filename: owned("filename"),
        passthrough: truthy(el, "passthrough"),
        collapsed: truthy(el, "collapsed"),
        layers: Vec::new(),
        masks: Vec::new(),
    };
    Ok(out)
}

/// The `<layers>` of `el`, walked with an explicit stack: groups can nest up to [`MAX_XML_DEPTH`]
/// deep, more than a small thread stack can recurse.
fn layers(el: &El, count: &mut usize) -> Result<Vec<Node>> {
    struct Frame<'a> {
        node: Node,
        rest: std::vec::IntoIter<&'a El>,
    }
    let mut root = children(el, "layers", "layer").collect::<Vec<_>>().into_iter();
    let mut stack: Vec<Frame> = Vec::new();
    let mut out = Vec::new();
    loop {
        let next = match stack.last_mut() {
            Some(frame) => frame.rest.next(),
            None => root.next(),
        };
        let Some(child) = next else {
            let Some(frame) = stack.pop() else { break };
            match stack.last_mut() {
                Some(parent) => parent.node.layers.push(frame.node),
                None => out.push(frame.node),
            }
            continue;
        };
        *count += 1;
        if *count > MAX_NODES {
            return Err(invalid(format!("more than {MAX_NODES} layers")));
        }
        let mut built = node_fields(child, false)?;
        built.masks = children(child, "masks", "mask").map(|m| node(m, true, count)).collect::<Result<_>>()?;
        stack.push(Frame { node: built, rest: children(child, "layers", "layer").collect::<Vec<_>>().into_iter() });
    }
    Ok(out)
}

impl Image {
    /// Reads `maindoc.xml`.
    pub fn parse(maindoc: &[u8]) -> Result<Image> {
        if std::str::from_utf8(maindoc).is_err() {
            return Err(invalid("maindoc.xml is not UTF-8"));
        }
        // maindoc.xml declares Krita's DTD by public id; Krita writes no internal subset, and one
        // is refused (nothing in it would be processed).
        let options = craft_xml::Options {
            doctype: craft_xml::Doctype::RejectInternalSubset,
            limits: craft_xml::Limits { max_input: MAX_MAINDOC, max_depth: MAX_XML_DEPTH, ..craft_xml::Limits::DEFAULT },
            ..craft_xml::Options::local().strict()
        };
        let root = options.parse(maindoc).map_err(|e| invalid(format!("maindoc.xml: {e}")))?;
        let image = root.child("IMAGE").ok_or_else(|| invalid("maindoc.xml has no IMAGE"))?;
        let side = |a: &str| -> Result<u32> {
            let v = number(image, a, 0.0)?;
            if !(1.0..=f64::from(MAX_SIDE)).contains(&v) || v.fract() != 0.0 {
                return Err(invalid(format!("image {a} {v} must be a whole number of pixels from 1 to {MAX_SIDE}")));
            }
            Ok(v as u32)
        };
        let (width, height) = (side("width")?, side("height")?);
        if (width as usize).saturating_mul(height as usize).saturating_mul(8) > MAX_PIXEL_BYTES {
            return Err(invalid("the canvas exceeds the 1 GiB pixel limit"));
        }
        let mut count = 0;
        Ok(Image {
            name: attr(image, "name").unwrap_or("").to_string(),
            width,
            height,
            colorspace_name: attr(image, "colorspacename").unwrap_or("").to_string(),
            x_res: number(image, "x-res", 72.0)?,
            y_res: number(image, "y-res", 72.0)?,
            layers: layers(image, &mut count)?,
            assistants: image.child("assistants").is_some(),
            reference_images: image.child("ReferenceImages").is_some(),
        })
    }
}

/// An open Krita archive: the parsed `maindoc.xml` and access to the archive's other entries.
pub struct Archive<'a> {
    zip: craft_zip::Archive<'a>,
    image: Image,
}

impl<'a> Archive<'a> {
    /// Opens the archive, checks its `mimetype` and reads `maindoc.xml`.
    pub fn open(bytes: &'a [u8]) -> Result<Archive<'a>> {
        let mut zip =
            craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: u64::MAX }).map_err(zip_error)?;
        // `zip` keeps one entry of a repeated name; which one the writer meant is not known.
        if zip.has_duplicate_names() {
            return Err(invalid("two entries of the archive share a name"));
        }
        let mime = read(&mut zip, "mimetype", 256)?.ok_or_else(|| invalid("the archive has no mimetype"))?;
        if mime.trim_ascii() != MIMETYPE {
            return Err(invalid("the archive's mimetype is not application/x-krita"));
        }
        let xml = read(&mut zip, "maindoc.xml", MAX_MAINDOC)?.ok_or_else(|| invalid("the archive has no maindoc.xml"))?;
        Ok(Archive { zip, image: Image::parse(&xml)? })
    }

    /// The document's `<IMAGE>`.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// An entry by its full name, refusing one that inflates past `max` bytes. `None` when the
    /// archive has no such entry.
    pub fn entry(&mut self, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
        read(&mut self.zip, name, max)
    }

    /// The file `name` under `<image name>/layers/`: a node's `filename` for a paint layer's
    /// tiles, `<filename>.pixelselection` for a mask's. Parse it with [`tiles::TileFile`].
    pub fn layer_file(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = format!("{}/layers/{name}", self.image.name);
        read(&mut self.zip, &path, MAX_LAYER_FILE)
    }

    /// The default pixel Krita stores next to the tiled file `name` (`<name>.defaultpixel`), as
    /// straight RGBA ([`Space::rgba`]): what the layer shows wherever it has no tile.
    pub fn default_pixel(&mut self, name: &str, space: Space) -> Result<Option<[f32; 4]>> {
        // One pixel: a member declaring the generic layer-file limit must not be inflated first.
        let path = format!("{}/layers/{name}.defaultpixel", self.image.name);
        let Some(bytes) = read(&mut self.zip, &path, space.pixel_size())? else { return Ok(None) };
        if bytes.len() != space.pixel_size() {
            return Err(invalid(format!("{name}.defaultpixel has {} bytes, expected {}", bytes.len(), space.pixel_size())));
        }
        Ok(Some(space.rgba(&bytes)))
    }

    /// The image's ICC profile (`<image name>/annotations/icc`); `None` when absent or empty.
    pub fn icc_profile(&mut self) -> Result<Option<Vec<u8>>> {
        let path = format!("{}/annotations/icc", self.image.name);
        Ok(read(&mut self.zip, &path, MAX_ICC)?.filter(|p| !p.is_empty()))
    }

    /// Krita's rendering of the whole image (`mergedimage.png`).
    pub fn merged_image(&mut self) -> Result<Option<Vec<u8>>> {
        read(&mut self.zip, "mergedimage.png", MAX_LAYER_FILE)
    }
}

fn read(zip: &mut craft_zip::Archive<'_>, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
    zip.read(name, max as u64).map_err(zip_error)
}

fn zip_error(e: craft_zip::Error) -> Error {
    match e {
        craft_zip::Error::NotZip(m) => invalid(m),
        craft_zip::Error::EntryTooLarge { name, limit } => invalid(format!("{name}: larger than {limit} bytes")),
        e => invalid(e.to_string()),
    }
}

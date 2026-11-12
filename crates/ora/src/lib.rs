#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! OpenRaster (`.ora`) read and write. Layout from the public OpenRaster specification
//! (<https://www.openraster.org/>, `stack.xml` 0.0.x): a ZIP whose first, stored entry is
//! `mimetype` = `image/openraster`, a `stack.xml` layer tree, one PNG per layer, and a
//! `mergedimage.png` / `Thumbnails/thumbnail.png` rendering.
//!
//! - [`Archive::open`] checks the mimetype and reads `stack.xml` into an [`Image`]: the tree of
//!   [`Node`]s with the attributes the file has. [`Archive::entry`] reads a layer's image.
//! - [`Writer`] builds an archive: the mimetype first, then the entries it is given and the
//!   `stack.xml` of an [`Image`].
//!
//! Layer images are PNG files; decoding and encoding them is the caller's. So is what a
//! composite operation means: `composite-op` is kept as written ([`SVG_COMPOSITE_OPS`] lists the
//! specification's names).

#[cfg(test)]
mod tests;

use craft_xml::El;

/// Why an OpenRaster document was not read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is not an OpenRaster archive, is damaged, or exceeds a limit.
    #[error("invalid OpenRaster document: {0}")]
    Invalid(String),
    /// The archive could not be written.
    #[error("OpenRaster archive not written: {0}")]
    Write(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// The `mimetype` entry of an OpenRaster archive.
pub const MIMETYPE: &[u8] = b"image/openraster";
/// The `stack.xml` version [`Image::to_xml`] writes when the image names none.
pub const VERSION: &str = "0.0.6";
/// Canvas side limit; layer offsets may be up to four times this.
pub const MAX_SIDE: u32 = 300_000;
/// The canvas may hold at most this many bytes at 8 bytes per pixel (16-bit RGBA).
pub const MAX_PIXEL_BYTES: usize = 1 << 30;
/// Elements under the root stack: layers, stacks and unknown ones.
pub const MAX_NODES: usize = 8192;
/// Largest `stack.xml`.
pub const MAX_STACK_XML: usize = 16 << 20;
/// Largest layer image file a reader should ask [`Archive::entry`] for.
pub const MAX_ENTRY: usize = 512 << 20;
/// Most entries the archive may declare; checked before the archive is indexed.
pub const MAX_ENTRIES: u64 = 100_000;
/// Deepest element nesting of `stack.xml`.
pub const MAX_XML_DEPTH: usize = 512;

/// The compositing operations the specification names for `composite-op`. Applications also
/// write `svg:plus`, `svg:exclusion` and their own (`krita:<id>`).
pub const SVG_COMPOSITE_OPS: [&str; 15] = [
    "svg:src-over",
    "svg:multiply",
    "svg:screen",
    "svg:overlay",
    "svg:darken",
    "svg:lighten",
    "svg:color-dodge",
    "svg:color-burn",
    "svg:hard-light",
    "svg:soft-light",
    "svg:difference",
    "svg:color",
    "svg:luminosity",
    "svg:hue",
    "svg:saturation",
];

/// The payload of a ZIP's first entry when it is a stored file named `mimetype` of at most 256
/// bytes, which is how OpenRaster (and Krita, OpenDocument, EPUB) mark their archives so the
/// type is readable without unzipping.
pub fn zip_mimetype(bytes: &[u8]) -> Option<&[u8]> {
    craft_zip::stored_mimetype(bytes)
}

/// `true` for an OpenRaster archive (by its `mimetype` entry).
pub fn is_ora(bytes: &[u8]) -> bool {
    zip_mimetype(bytes) == Some(MIMETYPE)
}

/// The attributes a `<layer>` and a `<stack>` share.
#[derive(Clone, Debug, PartialEq)]
pub struct Props {
    /// `name`, exactly as written (surrounding whitespace kept).
    pub name: Option<String>,
    /// `visibility`: everything but `hidden` is visible.
    pub visible: bool,
    /// `opacity`, as stored (not clamped). 1 when missing.
    pub opacity: f32,
    /// `composite-op`, trimmed. Missing means `svg:src-over`.
    pub composite_op: Option<String>,
    /// `edit-locked`.
    pub edit_locked: bool,
    /// `alpha-preserve`.
    pub alpha_preserve: bool,
}

impl Default for Props {
    fn default() -> Props {
        Props { name: None, visible: true, opacity: 1.0, composite_op: None, edit_locked: false, alpha_preserve: false }
    }
}

/// A `<layer>`: one image at a position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub props: Props,
    /// `x`, `y`: where the image's top-left pixel sits (fractions are rounded).
    pub x: i32,
    pub y: i32,
    /// `src`: the archive entry of the layer's image, trimmed.
    pub src: Option<String>,
}

/// A `<stack>`: a group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stack {
    pub props: Props,
    /// `isolation`, trimmed: `isolate` or `auto`. See [`Image::explicit_group_isolation`].
    pub isolation: Option<String>,
    /// The children, topmost first as the file lists them.
    pub children: Vec<Node>,
}

/// An element of a stack.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Layer(Layer),
    Stack(Stack),
    /// An element that is neither (`<text>`, ...): its tag and `name` attribute. Not written.
    Other {
        tag: String,
        name: Option<String>,
    },
}

/// The `<image>` root of `stack.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// `version`, as written.
    pub version: Option<String>,
    /// Canvas size in pixels (`w`, `h`), 1 to [`MAX_SIDE`].
    pub width: u32,
    pub height: u32,
    /// `xres`, `yres` in pixels per inch: 72 when `xres` is missing, `yres` defaults to `xres`.
    pub xres: f64,
    pub yres: f64,
    /// The children of the root `<stack>`, topmost first. The root stack's own attributes are
    /// not read.
    pub stack: Vec<Node>,
}

/// A number attribute; a missing one is `default`, an unparsable one an error.
fn num(el: &El, name: &str, default: f64) -> Result<f64> {
    match el.attr(name) {
        None => Ok(default),
        Some(v) => v.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| invalid(format!("invalid {name} attribute `{v}`"))),
    }
}

fn int(el: &El, name: &str) -> Result<i32> {
    let v = num(el, name, 0.0)?.round();
    if v.abs() > f64::from(MAX_SIDE) * 4.0 {
        return Err(invalid(format!("{name} offset {v} is out of range")));
    }
    Ok(v as i32)
}

fn flag(el: &El, name: &str) -> bool {
    el.attr(name).is_some_and(|v| matches!(v.trim(), "true" | "1"))
}

fn props(el: &El) -> Result<Props> {
    Ok(Props {
        name: el.attr("name").map(str::to_string),
        visible: el.attr("visibility").is_none_or(|v| v.trim() != "hidden"),
        opacity: num(el, "opacity", 1.0)? as f32,
        composite_op: el.attr("composite-op").map(|v| v.trim().to_string()),
        edit_locked: flag(el, "edit-locked"),
        alpha_preserve: flag(el, "alpha-preserve"),
    })
}

/// The elements of `stack`. Recursion is bounded by [`MAX_XML_DEPTH`].
fn stack(el: &El, count: &mut usize) -> Result<Vec<Node>> {
    let mut out = Vec::new();
    for child in el.elements() {
        *count += 1;
        if *count > MAX_NODES {
            return Err(invalid(format!("more than {MAX_NODES} layers")));
        }
        out.push(match child.name.as_str() {
            "layer" => Node::Layer(Layer {
                props: props(child)?,
                x: int(child, "x")?,
                y: int(child, "y")?,
                src: child.attr("src").map(|s| s.trim().to_string()),
            }),
            "stack" => Node::Stack(Stack {
                props: props(child)?,
                isolation: child.attr("isolation").map(|s| s.trim().to_string()),
                children: stack(child, count)?,
            }),
            other => Node::Other { tag: other.to_string(), name: child.attr("name").map(str::to_string) },
        });
    }
    Ok(out)
}

impl Image {
    /// Reads `stack.xml`.
    pub fn parse(xml: &[u8]) -> Result<Image> {
        if std::str::from_utf8(xml).is_err() {
            return Err(invalid("stack.xml is not UTF-8"));
        }
        let options = craft_xml::Options {
            doctype: craft_xml::Doctype::Reject,
            limits: craft_xml::Limits { max_input: MAX_STACK_XML, max_depth: MAX_XML_DEPTH, ..craft_xml::Limits::DEFAULT },
            ..craft_xml::Options::local().strict()
        };
        let image = options.parse(xml).map_err(|e| invalid(format!("stack.xml: {e}")))?;
        if image.name != "image" {
            return Err(invalid("stack.xml has no <image> root"));
        }
        let side = |attr: &str| -> Result<u32> {
            let v = num(&image, attr, 0.0)?;
            if !(1.0..=f64::from(MAX_SIDE)).contains(&v) || v.fract() != 0.0 {
                return Err(invalid(format!("image {attr} {v} must be a whole number of pixels from 1 to {MAX_SIDE}")));
            }
            Ok(v as u32)
        };
        let (width, height) = (side("w")?, side("h")?);
        if (width as usize).saturating_mul(height as usize).saturating_mul(8) > MAX_PIXEL_BYTES {
            return Err(invalid("the canvas exceeds the 1 GiB pixel limit"));
        }
        let root = image.child("stack").ok_or_else(|| invalid("stack.xml has no root <stack>"))?;
        let nodes = stack(root, &mut 0)?;
        let xres = num(&image, "xres", 72.0)?;
        let yres = num(&image, "yres", xres)?;
        Ok(Image { version: image.attr("version").map(str::to_string), width, height, xres, yres, stack: nodes })
    }

    /// Does the file's version make group isolation explicit? OpenRaster 0.0.6 defaults to
    /// isolated groups and makes `isolation="auto"` pass-through whatever the group's
    /// `composite-op`. Older files (and ones with no or an unrecognised version) leave it to the
    /// reader, by convention: a group passes through unless it says `isolate` or has a
    /// composite operation other than `svg:src-over`. The numeric core is compared, not the
    /// version string.
    pub fn explicit_group_isolation(&self) -> bool {
        let Some(core) = self.version.as_deref().and_then(|v| v.trim().split(['-', '+']).next()) else { return false };
        let mut parts = core.split('.');
        let number = |part: Option<&str>| part.and_then(|s| s.parse::<u32>().ok());
        match (number(parts.next()), number(parts.next()), number(parts.next()), parts.next()) {
            (Some(major), Some(minor), Some(patch), None) => [major, minor, patch] >= [0, 0, 6],
            _ => false,
        }
    }

    /// `stack.xml` for this image: `version` (or [`VERSION`]), the canvas, the resolution, and
    /// the layers and stacks with every attribute of [`Props`]. A stack with no `isolation`
    /// is written `isolate`; [`Node::Other`] is left out.
    pub fn to_xml(&self) -> String {
        let mut xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"{}\" w=\"{}\" h=\"{}\" xres=\"{}\" yres=\"{}\"><stack>",
            craft_xml::esc_attr(self.version.as_deref().unwrap_or(VERSION)),
            self.width,
            self.height,
            self.xres,
            self.yres
        );
        // Iterative, so a tree built by hand may be as deep as its builder made it.
        let mut open: Vec<std::slice::Iter<'_, Node>> = vec![self.stack.iter()];
        while let Some(level) = open.last_mut() {
            match level.next() {
                Some(Node::Layer(l)) => {
                    xml.push_str("<layer");
                    push_props(&mut xml, &l.props);
                    if let Some(src) = &l.src {
                        xml.push_str(&format!(" src=\"{}\"", craft_xml::esc_attr(src)));
                    }
                    xml.push_str(&format!(" x=\"{}\" y=\"{}\"/>", l.x, l.y));
                }
                Some(Node::Stack(s)) => {
                    xml.push_str("<stack");
                    push_props(&mut xml, &s.props);
                    xml.push_str(&format!(" isolation=\"{}\">", craft_xml::esc_attr(s.isolation.as_deref().unwrap_or("isolate"))));
                    open.push(s.children.iter());
                }
                Some(Node::Other { .. }) => {}
                None => {
                    open.pop();
                    xml.push_str(if open.is_empty() { "</stack></image>\n" } else { "</stack>" });
                }
            }
        }
        xml
    }
}

fn push_props(xml: &mut String, p: &Props) {
    if let Some(name) = &p.name {
        xml.push_str(&format!(" name=\"{}\"", craft_xml::esc_attr(name)));
    }
    xml.push_str(&format!(" visibility=\"{}\" opacity=\"{}\"", if p.visible { "visible" } else { "hidden" }, p.opacity));
    xml.push_str(&format!(" composite-op=\"{}\"", craft_xml::esc_attr(p.composite_op.as_deref().unwrap_or("svg:src-over"))));
    if p.edit_locked {
        xml.push_str(" edit-locked=\"true\"");
    }
    if p.alpha_preserve {
        xml.push_str(" alpha-preserve=\"true\"");
    }
}

/// An open OpenRaster archive: the parsed `stack.xml` and access to the archive's entries.
pub struct Archive<'a> {
    zip: craft_zip::Archive<'a>,
    image: Image,
}

impl<'a> Archive<'a> {
    /// Opens the archive, checks its `mimetype` and reads `stack.xml`.
    pub fn open(bytes: &'a [u8]) -> Result<Archive<'a>> {
        let mut zip =
            craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: u64::MAX }).map_err(zip_error)?;
        // `zip` keeps one entry of a repeated name; which one the writer meant is not known.
        if zip.has_duplicate_names() {
            return Err(invalid("two entries of the archive share a name"));
        }
        let mime = read(&mut zip, "mimetype", 256)?.ok_or_else(|| invalid("the archive has no mimetype"))?;
        if mime.trim_ascii() != MIMETYPE {
            return Err(invalid("the archive's mimetype is not image/openraster"));
        }
        let xml = read(&mut zip, "stack.xml", MAX_STACK_XML)?.ok_or_else(|| invalid("the archive has no stack.xml"))?;
        Ok(Archive { zip, image: Image::parse(&xml)? })
    }

    /// The document's `<image>`.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// An entry by name (a layer's `src`, `mergedimage.png`, `Thumbnails/thumbnail.png`),
    /// refusing one that inflates past `max` bytes ([`MAX_ENTRY`] for a layer image). `None`
    /// when the archive has no such entry.
    pub fn entry(&mut self, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
        read(&mut self.zip, name, max)
    }
}

fn read(zip: &mut craft_zip::Archive<'_>, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
    zip.read(name, max as u64).map_err(zip_error)
}

fn write_error(e: craft_zip::Error) -> Error {
    match e {
        craft_zip::Error::Write(m) => Error::Write(m),
        e => Error::Write(e.to_string()),
    }
}

fn zip_error(e: craft_zip::Error) -> Error {
    match e {
        craft_zip::Error::NotZip(m) => invalid(m),
        craft_zip::Error::EntryTooLarge { name, limit } => invalid(format!("{name}: larger than {limit} bytes")),
        e => invalid(e.to_string()),
    }
}

/// Writes an OpenRaster archive. Entries are stored (PNG is already compressed), in the order
/// they are added, after the `mimetype`.
pub struct Writer {
    zip: craft_zip::Writer,
    failed: Option<Error>,
}

impl Default for Writer {
    fn default() -> Writer {
        Writer::new()
    }
}

impl Writer {
    /// An archive holding the `mimetype`: it must be the first entry, stored, so the type is
    /// readable at a fixed offset.
    pub fn new() -> Writer {
        let mut w = Writer { zip: craft_zip::Writer::new(), failed: None };
        w.failed = w.add("mimetype", MIMETYPE).err();
        w
    }

    /// Adds an entry: a layer image under the name its [`Layer::src`] gives, `mergedimage.png`
    /// (the whole image, which the specification requires) or `Thumbnails/thumbnail.png` (at
    /// most 256 pixels a side, also required).
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<()> {
        self.zip.add(name, data, craft_zip::Compression::Stored).map_err(write_error)
    }

    /// Adds `stack.xml` for `image`.
    pub fn add_stack(&mut self, image: &Image) -> Result<()> {
        self.add("stack.xml", image.to_xml().as_bytes())
    }

    /// The finished archive.
    pub fn finish(self) -> Result<Vec<u8>> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        self.zip.finish().map_err(write_error)
    }
}

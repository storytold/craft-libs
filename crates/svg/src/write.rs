//! Writes SVG documents: the declaration and root element, elements and attributes with XML
//! escaping (through `craft-xml`), path data and numbers.
//!
//! The caller decides what to draw and how the file is laid out; this module makes what it
//! writes well-formed. Attribute values and text are escaped, so no value can inject markup.
//!
//! ```
//! use craft_svg::write::{SvgWriter, num, path_data};
//!
//! let mut path = kurbo::BezPath::new();
//! path.move_to((1.0, 2.0));
//! path.line_to((3.5, 2.0));
//! let mut w = SvgWriter::new();
//! w.root().attr("viewBox", format_args!("0 0 {} {}", num(10.0, 1), num(5.0, 1))).open();
//! w.line(1);
//! w.tag("path").attr("d", path_data(&path)).empty();
//! w.line(0);
//! w.close();
//! assert_eq!(
//!     w.finish(),
//!     "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10.0 5.0\">\n  <path d=\"M1,2 L3.5,2\"/>\n</svg>"
//! );
//! ```

use std::fmt::{self, Display, Write as _};

pub use craft_xml::Tag;
use kurbo::{BezPath, PathEl, Point};

/// The SVG namespace.
pub const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// A structured SVG scene. Applications provide geometry, styles and attributes; this module
/// owns the SVG element tree and serializes it with XML escaping.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub width_mm: f64,
    pub height_mm: f64,
    pub view_box: [f64; 4],
    pub elements: Vec<Element>,
}

/// Structured SVG elements supported by the path-document writer.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Group { attributes: Vec<Attribute>, children: Vec<Element> },
    Path { attributes: Vec<Attribute>, path: BezPath },
    Rect { attributes: Vec<Attribute>, x: f64, y: f64, width: f64, height: f64 },
    Text { attributes: Vec<Attribute>, x: f64, y: f64, text: String },
    Title(String),
    Style(String),
    NestedSvg { attributes: Vec<Attribute>, children: Vec<Element> },
}

/// An escaped SVG attribute. Element names and attribute names are API constants and must be
/// valid XML names; values are always escaped by the shared writer.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: Value,
}

/// Attribute values written by the shared serializer.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Millimetres(f64),
    ViewBox([f64; 4]),
    TranslateScale { x: f64, y: f64, scale_x: f64, scale_y: f64 },
}

impl Attribute {
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self { name: name.into(), value: Value::Text(value.into()) }
    }
    pub fn number(name: impl Into<String>, value: f64) -> Self {
        Self { name: name.into(), value: Value::Number(value) }
    }
    pub fn millimetres(name: impl Into<String>, value: f64) -> Self {
        Self { name: name.into(), value: Value::Millimetres(value) }
    }
    pub fn view_box(bounds: [f64; 4]) -> Self {
        Self { name: "viewBox".into(), value: Value::ViewBox(bounds) }
    }
    pub fn translate_scale(x: f64, y: f64, scale_x: f64, scale_y: f64) -> Self {
        Self { name: "transform".into(), value: Value::TranslateScale { x, y, scale_x, scale_y } }
    }
}

impl Scene {
    /// Serialize the scene as an SVG 1.1 document.
    pub fn write(&self) -> String {
        let mut writer = SvgWriter::new();
        writer
            .root()
            .attr("version", "1.1")
            .attr("width", format!("{}mm", num(self.width_mm, 3)))
            .attr("height", format!("{}mm", num(self.height_mm, 3)))
            .attr("viewBox", view_box_data(self.view_box))
            .open();
        write_elements(&mut writer, &self.elements, 1);
        writer.line(0);
        writer.close();
        writer.finish()
    }
}

fn write_attributes<'a>(mut tag: craft_xml::Tag<'a>, attributes: &'a [Attribute]) -> craft_xml::Tag<'a> {
    // SVG consumers disagree about duplicate attributes; make the last supplied value win.
    for (index, attribute) in attributes.iter().enumerate() {
        if attributes[index + 1..].iter().any(|later| later.name == attribute.name) {
            continue;
        }
        tag = match &attribute.value {
            Value::Text(value) => tag.attr(&attribute.name, value),
            Value::Number(value) => tag.attr(&attribute.name, num(*value, 3)),
            Value::Millimetres(value) => tag.attr(&attribute.name, format!("{}mm", num(*value, 3))),
            Value::ViewBox(bounds) => tag.attr(&attribute.name, view_box_data(*bounds)),
            Value::TranslateScale { x, y, scale_x, scale_y } => {
                tag.attr(&attribute.name, format!("translate({} {}) scale({} {})", num(*x, 3), num(*y, 3), shortest(*scale_x), shortest(*scale_y)))
            }
        };
    }
    tag
}

fn shortest(value: f64) -> String {
    if value.is_finite() { value.to_string() } else { "0".into() }
}

fn write_elements(writer: &mut SvgWriter, elements: &[Element], depth: usize) {
    for element in elements {
        writer.line(depth);
        match element {
            Element::Group { attributes, children } => {
                write_attributes(writer.tag("g"), attributes).open();
                write_elements(writer, children, depth + 1);
                writer.line(depth);
                writer.close();
            }
            Element::Path { attributes, path } => {
                let mut attrs = vec![Attribute::text("d", path_data(path))];
                attrs.extend(attributes.iter().cloned());
                write_attributes(writer.tag("path"), &attrs).empty();
            }
            Element::Rect { attributes, x, y, width, height } => {
                let mut attrs = vec![
                    Attribute::number("x", *x),
                    Attribute::number("y", *y),
                    Attribute::number("width", *width),
                    Attribute::number("height", *height),
                ];
                attrs.extend(attributes.iter().cloned());
                write_attributes(writer.tag("rect"), &attrs).empty();
            }
            Element::Text { attributes, x, y, text } => {
                let mut attrs = vec![Attribute::number("x", *x), Attribute::number("y", *y)];
                attrs.extend(attributes.iter().cloned());
                write_attributes(writer.tag("text"), &attrs).open();
                writer.text(text);
                writer.close();
            }
            Element::Title(text) => {
                writer.tag("title").open();
                writer.text(text);
                writer.close();
            }
            Element::Style(css) => {
                writer.tag("style").open();
                writer.text(css);
                writer.close();
            }
            Element::NestedSvg { attributes, children } => {
                write_attributes(writer.tag("svg"), attributes).open();
                write_elements(writer, children, depth + 1);
                writer.line(depth);
                writer.close();
            }
        }
    }
}

fn view_box_data(bounds: [f64; 4]) -> String {
    format!("{} {} {} {}", shortest(bounds[0]), shortest(bounds[1]), shortest(bounds[2]), shortest(bounds[3]))
}

/// A number written with a fixed count of decimals. A value that is not finite is written as
/// zero, because SVG has no spelling for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num(f64, usize);

/// `value` with `decimals` decimals: `num(1.5, 3)` is written `1.500`.
pub fn num(value: f64, decimals: usize) -> Num {
    Num(value, decimals)
}

impl Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = if self.0.is_finite() { self.0 } else { 0.0 };
        write!(f, "{value:.*}", self.1)
    }
}

/// Path data (the `d` attribute) with every coordinate in its shortest form that reads back
/// as the same number: `M1,2 L3.5,2 Z`.
pub fn path_data(path: &BezPath) -> String {
    path.to_svg()
}

/// Path data with `decimals` decimals per coordinate, written compactly: `M1.00 2.00L3.50 2.00Z`.
/// Coordinates that are not finite are written as zero.
pub fn path_data_fixed(path: &BezPath, decimals: usize) -> String {
    let mut d = String::new();
    let mut command = |letter: char, points: &[Point]| {
        d.push(letter);
        for (i, p) in points.iter().enumerate() {
            let sep = if i == 0 { "" } else { " " };
            let _ = write!(d, "{sep}{} {}", num(p.x, decimals), num(p.y, decimals));
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => command('M', &[p]),
            PathEl::LineTo(p) => command('L', &[p]),
            PathEl::QuadTo(a, p) => command('Q', &[a, p]),
            PathEl::CurveTo(a, b, p) => command('C', &[a, b, p]),
            PathEl::ClosePath => command('Z', &[]),
        }
    }
    d
}

/// Builds an SVG document or fragment. Elements are closed for the caller and attribute
/// values and text are escaped. Element and attribute names are written as given: they are
/// the caller's own constants, not checked.
///
/// Whitespace between elements is the caller's: nothing is added unless [`SvgWriter::line`]
/// is called.
#[derive(Clone, Debug, Default)]
pub struct SvgWriter(craft_xml::Writer);

impl SvgWriter {
    /// A document: starts with the XML declaration.
    pub fn new() -> SvgWriter {
        let mut xml = craft_xml::Writer::new();
        xml.decl(false);
        SvgWriter(xml)
    }
    /// A fragment, or a document without the XML declaration (for embedding in HTML or JSON).
    pub fn fragment() -> SvgWriter {
        SvgWriter::default()
    }
    /// Starts the root tag, `<svg xmlns="http://www.w3.org/2000/svg"`: add attributes and end
    /// it with [`Tag::open`].
    pub fn root(&mut self) -> Tag<'_> {
        self.0.tag("svg").attr("xmlns", SVG_NS)
    }
    /// Starts a tag: `w.tag("rect").attr("width", num(w, 3)).empty()`.
    pub fn tag<'w>(&'w mut self, name: &'w str) -> Tag<'w> {
        self.0.tag(name)
    }
    /// Closes the innermost open element. Nothing happens when none is open.
    pub fn close(&mut self) {
        self.0.close();
    }
    /// Escaped text.
    pub fn text(&mut self, text: &str) {
        self.0.text(text);
    }
    /// Starts a new line, indented `depth` levels of two spaces.
    pub fn line(&mut self, depth: usize) {
        self.0.raw("\n");
        (0..depth).for_each(|_| self.0.raw("  "));
    }
    /// Markup another [`SvgWriter`] has already written, as it is.
    pub fn raw(&mut self, svg: &str) {
        self.0.raw(svg);
    }
    /// How many bytes have been written so far.
    pub fn len(&self) -> usize {
        self.0.as_str().len()
    }
    /// Has nothing been written yet?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Closes every element still open and returns the text.
    pub fn finish(self) -> String {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn sample() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((1.0, 2.0));
        p.line_to((3.5, -2.0));
        p.quad_to((4.0, 5.0), (6.0, 7.25));
        p.curve_to((1.0, 1.0), (2.0, 2.0), (3.0, 1.0 / 3.0));
        p.close_path();
        p
    }

    #[test]
    fn numbers_have_fixed_decimals_and_are_never_non_finite() {
        assert_eq!(num(1.5, 3).to_string(), "1.500");
        assert_eq!(num(-2.0, 0).to_string(), "-2");
        assert_eq!(num(0.126, 2).to_string(), "0.13");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(num(bad, 2).to_string(), "0.00");
        }
    }

    #[test]
    fn path_data_reads_back_as_the_same_path() {
        let d = path_data(&sample());
        assert_eq!(d, "M1,2 L3.5,-2 Q4,5 6,7.25 C1,1 2,2 3,0.3333333333333333 Z");
        assert_eq!(BezPath::from_svg(&d).unwrap(), sample());
    }

    #[test]
    fn fixed_path_data_is_compact_and_parses() {
        let d = path_data_fixed(&sample(), 2);
        assert_eq!(d, "M1.00 2.00L3.50 -2.00Q4.00 5.00 6.00 7.25C1.00 1.00 2.00 2.00 3.00 0.33Z");
        assert_eq!(BezPath::from_svg(&d).unwrap().elements().len(), 5);
        let mut bad = BezPath::new();
        bad.move_to((f64::NAN, 1.0));
        assert_eq!(path_data_fixed(&bad, 1), "M0.0 1.0");
        assert_eq!(path_data_fixed(&BezPath::new(), 2), "");
    }

    #[test]
    fn writes_a_document_the_reader_reads() {
        let mut w = SvgWriter::new();
        assert!(!w.is_empty());
        w.root().attr("width", format_args!("{}mm", num(20.0, 3))).attr("height", "10mm").attr("viewBox", "0 0 20 10").open();
        w.line(1);
        w.tag("title").text("a < b & \"c\"");
        w.line(1);
        w.tag("g").attr("class", "x\"><script>").open();
        w.line(2);
        w.tag("path").attr("d", path_data(&sample())).empty();
        w.line(1);
        w.close();
        w.line(0);
        let svg = w.finish();
        assert_eq!(
            svg,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20.000mm\" height=\"10mm\" viewBox=\"0 0 20 10\">\n  \
             <title>a &lt; b &amp; \"c\"</title>\n  \
             <g class=\"x&quot;&gt;&lt;script&gt;\">\n    \
             <path d=\"M1,2 L3.5,-2 Q4,5 6,7.25 C1,1 2,2 3,0.3333333333333333 Z\"/>\n  \
             </g>\n</svg>"
        );
        #[derive(Clone, Default)]
        struct NoKind;
        impl crate::paths::Kind for NoKind {
            const PROPERTIES: &'static [&'static str] = &[];
            fn of(_: &crate::paths::StyleMap, _: &str) -> Option<Self> {
                None
            }
        }
        let read = crate::paths::import::<NoKind>(&svg, "written.svg").unwrap();
        assert_eq!((read.width_mm, read.height_mm), (20.0, 10.0));
        assert_eq!(read.shapes.len(), 1);
        assert_eq!(read.shapes.first().map(|s| &s.path), Some(&sample()));
    }

    #[test]
    fn fragments_have_no_declaration_and_embed_as_written() {
        let mut inner = SvgWriter::fragment();
        assert!(inner.is_empty());
        inner.tag("path").attr("d", "M0 0").empty();
        let inner = inner.finish();
        let mut w = SvgWriter::fragment();
        w.root().open();
        let before = w.len();
        w.raw(&inner);
        assert_eq!(w.len(), before + inner.len());
        assert_eq!(w.finish(), "<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M0 0\"/></svg>");
    }
}

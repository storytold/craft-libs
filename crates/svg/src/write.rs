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
    /// A translate/scale transform with three decimal places for positions and four for scales.
    pub fn translate_scale_fixed(x: f64, y: f64, scale_x: f64, scale_y: f64) -> Self {
        Self::text("transform", format!("translate({} {}) scale({} {})", num(x, 3), num(y, 3), num(scale_x, 4), num(scale_y, 4)))
    }
}

impl Scene {
    /// Serialize the scene as an SVG 1.1 document.
    pub fn write(&self) -> String {
        self.write_impl(false)
    }

    /// Serialize the root viewBox with three decimal places for stable fixed-width output.
    pub fn write_fixed_view_box(&self) -> String {
        self.write_impl(true)
    }

    fn write_impl(&self, fixed: bool) -> String {
        let mut writer = SvgWriter::new();
        let view_box = if fixed {
            format!("{} {} {} {}", num(self.view_box[0], 3), num(self.view_box[1], 3), num(self.view_box[2], 3), num(self.view_box[3], 3))
        } else {
            view_box_data(self.view_box)
        };
        writer
            .root()
            .attr("version", "1.1")
            .attr("width", format!("{}mm", num(self.width_mm, 3)))
            .attr("height", format!("{}mm", num(self.height_mm, 3)))
            .attr("viewBox", view_box)
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
    path_data_with_format(path, |value| num(value, decimals))
}

/// Compact absolute path commands with caller-owned coordinate formatting.
///
/// The formatter is called once per coordinate, in path order. It must produce a valid SVG
/// number (including handling nonfinite inputs); it controls rounding and negative zero.
/// Command letters and separators are shared, while the application's precision policy stays
/// with the caller. No intermediate geometry or coordinate collection is allocated.
pub fn path_data_with_format<T: Display>(path: &BezPath, mut format: impl FnMut(f64) -> T) -> String {
    let mut d = String::new();
    let mut command = |letter: char, points: &[Point]| {
        d.push(letter);
        for (i, p) in points.iter().enumerate() {
            let sep = if i == 0 { "" } else { " " };
            let _ = write!(d, "{sep}{} {}", format(p.x), format(p.y));
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

// A subset ends with `] S? >`; quoted entity values, comments and PIs may contain that text.
// Scan once without allocations; malformed unterminated constructs remain the caller's error.
fn closes_internal_subset(internal: &str) -> bool {
    let bytes = internal.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") || bytes[i..].starts_with(b"<?") {
            let (start, end): (usize, &[u8]) = if bytes[i..].starts_with(b"<!--") { (4, b"-->") } else { (2, b"?>") };
            let Some(offset) = bytes[i + start..].windows(end.len()).position(|part| part == end) else {
                return false;
            };
            i += start + offset + end.len();
        } else if matches!(bytes[i], b'\'' | b'"') {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else if bytes[i] == b']' {
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                i += 1;
            }
            if bytes.get(i) == Some(&b'>') {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

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
    /// A document that starts with an XML declaration naming `encoding` (for example `UTF-8` or
    /// `ISO-8859-1`) and a newline. Nothing is written for an invalid XML encoding name.
    pub fn with_encoding(encoding: &str) -> SvgWriter {
        let mut w = SvgWriter::fragment();
        w.decl(encoding, "\n");
        w
    }
    /// Writes `<?xml version="1.0" encoding="{encoding}"?>` followed by `newline`. Nothing is
    /// written unless `encoding` starts with an ASCII letter and continues with ASCII letters,
    /// digits, `.`, `_`, or `-` (XML EncName).
    pub fn decl(&mut self, encoding: &str, newline: &str) {
        let mut bytes = encoding.bytes();
        if bytes.next().is_some_and(|b| b.is_ascii_alphabetic()) && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            self.0.raw(&format!("<?xml version=\"1.0\" encoding=\"{encoding}\"?>{newline}"));
        }
    }
    /// Writes `<!DOCTYPE name [` + `newline` + `internal` + `newline` + `]>` + `newline`. When
    /// `internal` is empty, the inner newline is omitted, so a minified document can be
    /// `<!DOCTYPE svg []>` with no extra breaks. Nothing is written if `name` is not an XML name
    /// or `internal` closes the subset outside a quoted value, comment or processing instruction.
    /// The caller still supplies valid DTD declarations; this is not a DTD validator.
    pub fn doctype_internal(&mut self, name: &str, internal: &str, newline: &str) {
        if !craft_xml::is_name(name) || closes_internal_subset(internal) {
            return;
        }
        let mut out = format!("<!DOCTYPE {name} [");
        if !internal.is_empty() {
            out.push_str(newline);
            out.push_str(internal);
        }
        out.push_str(newline);
        out.push_str("]>");
        out.push_str(newline);
        self.0.raw(&out);
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
    /// What has been written so far, including any element that is still open.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    /// How many bytes have been written so far.
    pub fn len(&self) -> usize {
        self.as_str().len()
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
    fn custom_path_format_keeps_commands_and_coordinate_order() {
        let mut path = BezPath::new();
        path.move_to((1.0, 2.0));
        path.line_to((3.0, 4.0));
        path.quad_to((5.0, 6.0), (7.0, 8.0));
        path.curve_to((9.0, 10.0), (11.0, 12.0), (13.0, 14.0));
        path.close_path();
        let mut seen = Vec::new();
        let output = path_data_with_format(&path, |v| {
            seen.push(v);
            v
        });
        assert_eq!(output, "M1 2L3 4Q5 6 7 8C9 10 11 12 13 14Z");
        assert_eq!(seen, (1..=14).map(f64::from).collect::<Vec<_>>());
        assert_eq!(BezPath::from_svg(&output).unwrap(), path);
        assert_eq!(path_data_with_format(&BezPath::new(), |_: f64| -> String { panic!("empty path") }), "");
    }

    #[test]
    fn scene_can_keep_fixed_view_box_without_changing_default_output() {
        let scene = Scene { width_mm: 100.0, height_mm: 150.0, view_box: [0.0, 0.0, 100.0, 150.0], elements: vec![] };
        assert!(scene.write().contains("viewBox=\"0 0 100 150\""));
        assert!(scene.write_fixed_view_box().contains("viewBox=\"0.000 0.000 100.000 150.000\""));
        assert_eq!(Attribute::translate_scale_fixed(6.0, 46.0, 0.5, -0.5).value, Value::Text("translate(6.000 46.000) scale(0.5000 -0.5000)".into()));
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
    #[cfg(feature = "paths")]
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

    #[test]
    fn declaration_names_the_encoding_and_rejects_a_bad_name() {
        assert_eq!(SvgWriter::with_encoding("UTF-8").finish(), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        assert_eq!(SvgWriter::with_encoding("ISO-8859-1").finish(), "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n");
        assert!(SvgWriter::with_encoding("UTF-8\"").is_empty());
        assert!(SvgWriter::with_encoding("").is_empty());
        for valid in ["A", "a0._-"] {
            assert_eq!(SvgWriter::with_encoding(valid).finish(), format!("<?xml version=\"1.0\" encoding=\"{valid}\"?>\n"));
        }
        for invalid in [":UTF-8", "_UTF8", "1UTF8", "é", "UTF:8", "UTFé", "UTF 8"] {
            assert!(SvgWriter::with_encoding(invalid).is_empty(), "{invalid}");
        }
        let mut compact = SvgWriter::fragment();
        compact.decl("ISO-8859-1", "");
        compact.root().open();
        assert_eq!(compact.as_str(), "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><svg xmlns=\"http://www.w3.org/2000/svg\">");
        compact.close();
        assert_eq!(compact.finish(), "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><svg xmlns=\"http://www.w3.org/2000/svg\"></svg>");
    }

    #[test]
    fn doctype_internal_subset_keeps_caller_newlines() {
        let mut pretty = SvgWriter::fragment();
        pretty.doctype_internal("svg", "  <!ENTITY st1 \"fill:red\">", "\n");
        pretty.root().open();
        pretty.close();
        assert_eq!(pretty.finish(), "<!DOCTYPE svg [\n  <!ENTITY st1 \"fill:red\">\n]>\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>");
        let mut compact = SvgWriter::fragment();
        compact.doctype_internal("svg", "<!ENTITY st1 \"fill:red\">", "");
        assert_eq!(compact.finish(), "<!DOCTYPE svg [<!ENTITY st1 \"fill:red\">]>");
        let mut empty = SvgWriter::fragment();
        empty.doctype_internal("svg", "", "\n");
        assert_eq!(empty.finish(), "<!DOCTYPE svg [\n]>\n");
        let mut refused = SvgWriter::fragment();
        refused.doctype_internal("svg", "]]>", "\n");
        refused.doctype_internal("svg", "]><svg/><!--", "\n");
        refused.doctype_internal("svg", "] \t\r\n><svg/><!--", "\n");
        refused.doctype_internal("not a name", "<!ENTITY st1 \"x\">", "\n");
        assert!(refused.is_empty());
    }
    #[test]
    fn doctype_keeps_terminators_inside_values_comments_and_processing_instructions() {
        for internal in [r#"<!ENTITY a "]>">"#, "<!ENTITY a '] >'>", "<!-- ]> -->", "<?note ] >?>"] {
            let mut writer = SvgWriter::fragment();
            writer.doctype_internal("svg", internal, "");
            assert_eq!(writer.finish(), format!("<!DOCTYPE svg [{internal}]>"));
        }
        for internal in ["<!-- ok -->]>", "<?note ?>] >", "<!ENTITY a 'ok'>]>"] {
            let mut writer = SvgWriter::fragment();
            writer.doctype_internal("svg", internal, "");
            assert!(writer.is_empty());
        }
    }
}

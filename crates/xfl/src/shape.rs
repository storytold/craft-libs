//! Borrowed views of native XFL shape and style elements.
//!
//! These views recognize XFL's element names and attribute grammar without depending on an
//! application's geometry, paint, or animation types. They work with `craft_xml::El` and with
//! compatible borrowed element adapters implemented by consumers.

use crate::edges::{self, ParsedEdges, Segment};
use std::borrow::Cow;

/// An element source used by the borrowed XFL schema views.
///
/// Consumers with an existing XML facade can implement this small interface and use the same
/// XFL parser without converting or reparsing their tree. Attribute lookup must return the last
/// occurrence when an XML document repeats an attribute, matching the native XFL reader's
/// historical behavior.
pub trait XflElement {
    /// Iterator type for direct child elements.
    type Elements<'a>: Iterator<Item = &'a Self>
    where
        Self: 'a;

    /// The element's local name, with any namespace prefix removed.
    fn local_name(&self) -> &str;

    /// The last attribute with this exact name.
    fn attr(&self, name: &str) -> Option<&str>;

    /// Direct child elements in document order.
    fn elements(&self) -> Self::Elements<'_>;

    /// This element's direct character data, with XML references resolved.
    fn text_content(&self) -> Cow<'_, str>;

    /// The first direct child whose local name matches.
    fn child(&self, name: &str) -> Option<&Self> {
        self.elements().find(|element| element.local_name() == name)
    }

    /// Direct children whose local name matches, in document order.
    fn elements_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> + 'a {
        self.elements().filter(move |element| element.local_name() == name)
    }
}

/// Direct child elements of `craft_xml::El`, used by its `XflElement` implementation.
pub struct XmlElements<'a> {
    nodes: std::slice::Iter<'a, craft_xml::Node>,
}

impl<'a> Iterator for XmlElements<'a> {
    type Item = &'a craft_xml::El;

    fn next(&mut self) -> Option<Self::Item> {
        self.nodes.find_map(|node| match node {
            craft_xml::Node::El(element) => Some(element),
            _ => None,
        })
    }
}

impl XflElement for craft_xml::El {
    type Elements<'a> = XmlElements<'a>;

    fn local_name(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().rev().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    fn elements(&self) -> Self::Elements<'_> {
        XmlElements { nodes: self.children.iter() }
    }

    fn text_content(&self) -> Cow<'_, str> {
        self.text()
    }
}

/// XFL matrix coefficients, in the native units stored in XML.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

/// A color encoded by an XFL color attribute and its optional alpha attribute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub rgb: u32,
    pub alpha: f32,
}

/// A native XFL instance color record (`<color><Color .../></color>`).
///
/// The view parses the authored tint, brightness, channel multipliers and offsets but does not
/// choose a consumer's color-effect style or clamp values into an application's model. Offsets
/// remain in XFL's native 0–255 scale. Invalid or non-finite channel multipliers use 1, and
/// invalid or non-finite offsets use 0, matching the historical AniCraft importer.
#[derive(Clone, Copy, Debug)]
pub struct NativeColorEffect<'a, E: XflElement> {
    element: &'a E,
}

impl Matrix {
    /// The matrix used when an XFL element omits its transform.
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };
}

impl<'a, E: XflElement> NativeColorEffect<'a, E> {
    /// Views a native XFL `Color` element without converting it to an application's model.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    /// The native tint amount, when authored.
    pub fn tint_multiplier(&self) -> Option<f32> {
        number(self.element, "tintMultiplier")
    }

    /// The native tint color. Missing or malformed color values use black as other XFL colors do.
    pub fn tint_color(&self) -> Color {
        color(self.element, "tintColor", "__none")
    }

    /// The native brightness value, when authored.
    pub fn brightness(&self) -> Option<f32> {
        number(self.element, "brightness")
    }

    /// Red, green, blue, and alpha multipliers, with identity defaults for absent or invalid data.
    pub fn multipliers(&self) -> [f32; 4] {
        ["redMultiplier", "greenMultiplier", "blueMultiplier", "alphaMultiplier"].map(|name| number(self.element, name).unwrap_or(1.0))
    }

    /// Red, green, blue, and alpha offsets in XFL's native 0–255 scale.
    pub fn offsets(&self) -> [f32; 4] {
        ["redOffset", "greenOffset", "blueOffset", "alphaOffset"].map(|name| number(self.element, name).unwrap_or(0.0))
    }
}

pub(crate) fn number(element: &impl XflElement, name: &str) -> Option<f32> {
    element.attr(name)?.trim().parse::<f32>().ok().filter(|value| value.is_finite())
}

pub(crate) fn integer(element: &impl XflElement, name: &str) -> Option<u32> {
    element.attr(name)?.trim().parse::<u32>().ok()
}

pub(crate) fn boolean(element: &impl XflElement, name: &str) -> Option<bool> {
    element.attr(name).map(|value| value.trim() == "true")
}

/// Parses XFL's hexadecimal color and alpha attributes. Missing or malformed colors use black;
/// missing, malformed, or non-finite alpha uses full opacity, matching the legacy importer.
pub fn color(element: &impl XflElement, color_attribute: &str, alpha_attribute: &str) -> Color {
    color_from_attributes(element.attr(color_attribute), element.attr(alpha_attribute))
}

/// Parses native color and alpha strings after a consumer has read them from its own element
/// adapter. This keeps wire-value interpretation shared even when an application already has a
/// borrowed schema view for the containing element.
pub fn color_from_attributes(color: Option<&str>, alpha: Option<&str>) -> Color {
    let rgb = color.and_then(|value| u32::from_str_radix(value.trim().trim_start_matches('#'), 16).ok()).unwrap_or(0);
    let alpha = alpha.and_then(|value| value.trim().parse::<f32>().ok()).filter(|value| value.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
    Color { rgb, alpha }
}

/// Reads the native `<matrix><Matrix .../></matrix>` transform.
///
/// Each absent, malformed, or non-finite coefficient uses its identity default independently.
/// This preserves the prior AniCraft reader's `Element::f32` behavior, including retaining other
/// valid coefficients when one coefficient is invalid.
pub fn matrix(element: &impl XflElement) -> Matrix {
    let Some(matrix) = element.child("matrix").and_then(|wrapper| wrapper.child("Matrix")) else {
        return Matrix::IDENTITY;
    };
    Matrix {
        a: number(matrix, "a").unwrap_or(1.0),
        b: number(matrix, "b").unwrap_or(0.0),
        c: number(matrix, "c").unwrap_or(0.0),
        d: number(matrix, "d").unwrap_or(1.0),
        tx: number(matrix, "tx").unwrap_or(0.0),
        ty: number(matrix, "ty").unwrap_or(0.0),
    }
}

/// A native `DOMShape` or drawing shape and its XFL style tables.
#[derive(Clone, Copy, Debug)]
pub struct Shape<'a, E: XflElement> {
    element: &'a E,
}

/// Kind of native drawable primitive stored in XFL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveKind {
    Rectangle,
    Oval,
}

/// Native rectangle or oval object, including its native style definitions.
#[derive(Clone, Copy, Debug)]
pub struct Primitive<'a, E: XflElement> {
    element: &'a E,
    kind: PrimitiveKind,
}

impl<'a, E: XflElement> Primitive<'a, E> {
    /// Views a supported native primitive by its local element name.
    pub fn new(element: &'a E) -> Option<Self> {
        let kind = match element.local_name() {
            "DOMRectangleObject" => PrimitiveKind::Rectangle,
            "DOMOvalObject" => PrimitiveKind::Oval,
            _ => return None,
        };
        Some(Self { element, kind })
    }

    pub fn kind(&self) -> PrimitiveKind {
        self.kind
    }

    pub fn x(&self) -> Option<f32> {
        number(self.element, "x")
    }

    pub fn y(&self) -> Option<f32> {
        number(self.element, "y")
    }

    pub fn width(&self) -> Option<f32> {
        number(self.element, "objectWidth")
    }

    pub fn height(&self) -> Option<f32> {
        number(self.element, "objectHeight")
    }

    pub fn top_left_radius(&self) -> Option<f32> {
        number(self.element, "topLeftRadius")
    }

    pub fn matrix(&self) -> Matrix {
        matrix(self.element)
    }

    pub fn fill(&self) -> Option<Fill<'_, E>> {
        self.element.child("fill")?.elements().next().map(|element| Fill { element })
    }

    pub fn stroke(&self) -> Option<Stroke<'_, E>> {
        self.element.child("stroke")?.elements().next().map(|element| Stroke { element })
    }
}

impl<'a, E: XflElement> Shape<'a, E> {
    /// Views an element as an XFL shape when its local name is `DOMShape`.
    pub fn new(element: &'a E) -> Option<Self> {
        (element.local_name() == "DOMShape").then_some(Self { element })
    }

    /// Native shape transform.
    pub fn matrix(&self) -> Matrix {
        matrix(self.element)
    }

    /// Whether Animate stores the shape as a drawing object.
    pub fn is_drawing_object(&self) -> Option<bool> {
        boolean(self.element, "isDrawingObject")
    }

    /// Fill styles, preserving their native order and index values.
    pub fn fills(&self) -> impl Iterator<Item = FillStyle<'_, E>> + '_ {
        self.element.child("fills").into_iter().flat_map(|table| table.elements()).map(|element| FillStyle { element })
    }

    /// Stroke styles, preserving their native order and index values.
    pub fn strokes(&self) -> impl Iterator<Item = StrokeStyle<'_, E>> + '_ {
        self.element.child("strokes").into_iter().flat_map(|table| table.elements()).map(|element| StrokeStyle { element })
    }

    /// Native edge records, preserving malformed and unknown data for consumer policy.
    pub fn edges(&self) -> impl Iterator<Item = Edge<'_, E>> + '_ {
        self.element.child("edges").into_iter().flat_map(|table| table.elements_named("Edge")).map(|element| Edge { element })
    }
}

/// A native `FillStyle` entry.
#[derive(Clone, Copy, Debug)]
pub struct FillStyle<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> FillStyle<'a, E> {
    pub fn index(&self) -> Option<u32> {
        integer(self.element, "index")
    }

    /// The first native definition element inside this table entry.
    pub fn definition(&self) -> Option<Fill<'_, E>> {
        self.element.elements().next().map(|element| Fill { element })
    }
}

/// Native fill definitions understood by XFL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillKind {
    SolidColor,
    LinearGradient,
    RadialGradient,
    Bitmap,
    Other,
}

/// A native XFL fill definition.
#[derive(Clone, Copy, Debug)]
pub struct Fill<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> Fill<'a, E> {
    /// Views any native fill-definition element, including fills nested in primitives or strokes.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    /// Underlying native definition element for consumers that map it into application types.
    pub fn element(&self) -> &'a E {
        self.element
    }

    pub fn kind(&self) -> FillKind {
        match self.element.local_name() {
            "SolidColor" => FillKind::SolidColor,
            "LinearGradient" => FillKind::LinearGradient,
            "RadialGradient" => FillKind::RadialGradient,
            "BitmapFill" => FillKind::Bitmap,
            _ => FillKind::Other,
        }
    }

    pub fn color(&self) -> Option<&str> {
        self.element.attr("color")
    }

    pub fn color_value(&self) -> Color {
        color(self.element, "color", "alpha")
    }

    pub fn alpha(&self) -> Option<f32> {
        number(self.element, "alpha")
    }

    pub fn matrix(&self) -> Matrix {
        matrix(self.element)
    }

    pub fn spread_method(&self) -> Option<&str> {
        self.element.attr("spreadMethod")
    }

    pub fn interpolation_method(&self) -> Option<&str> {
        self.element.attr("interpolationMethod")
    }

    pub fn focal_point_ratio(&self) -> Option<f32> {
        number(self.element, "focalPointRatio")
    }

    pub fn bitmap_path(&self) -> Option<&str> {
        self.element.attr("bitmapPath")
    }

    pub fn bitmap_is_clipped(&self) -> Option<bool> {
        boolean(self.element, "bitmapIsClipped")
    }

    /// Gradient entries, preserving source order and raw native ratios.
    pub fn gradient_entries(&self) -> impl Iterator<Item = GradientEntry<'_, E>> + '_ {
        self.element.elements_named("GradientEntry").map(|element| GradientEntry { element })
    }
}

/// A native gradient ramp entry.
#[derive(Clone, Copy, Debug)]
pub struct GradientEntry<'a, E: XflElement> {
    element: &'a E,
}

impl<E: XflElement> GradientEntry<'_, E> {
    pub fn ratio(&self) -> Option<f32> {
        number(self.element, "ratio")
    }

    pub fn color(&self) -> Option<&str> {
        self.element.attr("color")
    }

    pub fn color_value(&self) -> Color {
        color(self.element, "color", "alpha")
    }

    pub fn alpha(&self) -> Option<f32> {
        number(self.element, "alpha")
    }
}

/// Native stroke definitions understood by XFL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Solid,
    Dashed,
    Dotted,
    Ragged,
    Stipple,
    Hatched,
    Other,
}

/// A native `StrokeStyle` table entry.
#[derive(Clone, Copy, Debug)]
pub struct StrokeStyle<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> StrokeStyle<'a, E> {
    pub fn index(&self) -> Option<u32> {
        integer(self.element, "index")
    }

    /// The first native definition element inside this table entry.
    pub fn definition(&self) -> Option<Stroke<'_, E>> {
        self.element.elements().next().map(|element| Stroke { element })
    }
}

/// A native XFL stroke definition.
#[derive(Clone, Copy, Debug)]
pub struct Stroke<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> Stroke<'a, E> {
    /// Views a native stroke-definition element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    /// Underlying native definition element for consumers that map it into application types.
    pub fn element(&self) -> &'a E {
        self.element
    }

    pub fn kind(&self) -> StrokeKind {
        match self.element.local_name() {
            "SolidStroke" => StrokeKind::Solid,
            "DashedStroke" => StrokeKind::Dashed,
            "DottedStroke" => StrokeKind::Dotted,
            "RaggedStroke" => StrokeKind::Ragged,
            "StippleStroke" => StrokeKind::Stipple,
            "HatchedStroke" => StrokeKind::Hatched,
            _ => StrokeKind::Other,
        }
    }

    pub fn weight(&self) -> Option<f32> {
        number(self.element, "weight")
    }

    pub fn caps(&self) -> Option<&str> {
        self.element.attr("caps")
    }

    pub fn joints(&self) -> Option<&str> {
        self.element.attr("joints")
    }

    pub fn miter_limit(&self) -> Option<f32> {
        number(self.element, "miterLimit")
    }

    pub fn dash1(&self) -> Option<f32> {
        number(self.element, "dash1")
    }

    pub fn dash2(&self) -> Option<f32> {
        number(self.element, "dash2")
    }

    pub fn dot_space(&self) -> Option<f32> {
        number(self.element, "dotSpace")
    }

    /// The first fill definition nested in the stroke.
    pub fn fill(&self) -> Option<Fill<'_, E>> {
        self.element.child("fill")?.elements().next().map(|element| Fill { element })
    }
}

/// A native `<Edge>` record and its style indices.
#[derive(Clone, Copy, Debug)]
pub struct Edge<'a, E: XflElement> {
    element: &'a E,
}

impl<E: XflElement> Edge<'_, E> {
    pub fn fill_style0(&self) -> Option<u32> {
        integer(self.element, "fillStyle0")
    }

    pub fn fill_style1(&self) -> Option<u32> {
        integer(self.element, "fillStyle1")
    }

    pub fn stroke_style(&self) -> Option<u32> {
        integer(self.element, "strokeStyle")
    }

    pub fn encoded_segments(&self) -> Option<&str> {
        self.element.attr("edges")
    }

    /// Parsed XFL path segments, using the shared bounded edge grammar.
    pub fn segments(&self) -> Vec<Segment> {
        self.encoded_segments().map(edges::parse_edge_segments).unwrap_or_default()
    }

    /// Parsed XFL path segments, with a count of the runs that were cut short.
    pub fn parsed_edges(&self) -> ParsedEdges {
        self.encoded_segments().map(edges::parse_edges).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::Options::default().parse(source).unwrap()
    }

    #[test]
    fn reads_prefixed_shape_fill_stroke_and_edge_schema() {
        let root = parse(br##"<x:DOMShape xmlns:x="urn:xfl" isDrawingObject="false">
            <x:matrix><x:Matrix a="2" tx="12"/></x:matrix>
            <x:fills><x:FillStyle index="1"><x:LinearGradient spreadMethod="reflect" interpolationMethod="linearRGB">
              <x:GradientEntry ratio="0" color="#102030"/><x:GradientEntry ratio="255" color="#ffffff" alpha=".5"/>
            </x:LinearGradient></x:FillStyle><x:FillStyle index="2"><x:BitmapFill bitmapPath="Library/a.png" bitmapIsClipped="true"/></x:FillStyle></x:fills>
            <x:strokes><x:StrokeStyle index="3"><x:DashedStroke weight="2" dash1="8" dash2="3" caps="square" joints="miter" miterLimit="7"><x:fill><x:SolidColor color="#abcdef"/></x:fill></x:DashedStroke></x:StrokeStyle></x:strokes>
            <x:edges><x:Edge fillStyle0="2" fillStyle1="1" strokeStyle="3" edges="!0 0|200 0"/></x:edges>
        </x:DOMShape>"##);

        let shape = Shape::new(&root).unwrap();
        assert_eq!(shape.matrix(), Matrix { a: 2.0, b: 0.0, c: 0.0, d: 1.0, tx: 12.0, ty: 0.0 });
        assert_eq!(shape.is_drawing_object(), Some(false));
        let fills: Vec<_> = shape.fills().collect();
        assert_eq!(fills.iter().map(FillStyle::index).collect::<Vec<_>>(), [Some(1), Some(2)]);
        let gradient = fills[0].definition().unwrap();
        assert_eq!(gradient.kind(), FillKind::LinearGradient);
        assert_eq!(gradient.spread_method(), Some("reflect"));
        assert_eq!(gradient.interpolation_method(), Some("linearRGB"));
        assert_eq!(gradient.gradient_entries().map(|entry| entry.ratio()).collect::<Vec<_>>(), [Some(0.0), Some(255.0)]);
        let bitmap = fills[1].definition().unwrap();
        assert_eq!(bitmap.kind(), FillKind::Bitmap);
        assert_eq!(bitmap.bitmap_path(), Some("Library/a.png"));
        assert_eq!(bitmap.bitmap_is_clipped(), Some(true));

        let stroke_style = shape.strokes().next().unwrap();
        let stroke = stroke_style.definition().unwrap();
        assert_eq!(stroke.kind(), StrokeKind::Dashed);
        assert_eq!(stroke.weight(), Some(2.0));
        assert_eq!((stroke.dash1(), stroke.dash2()), (Some(8.0), Some(3.0)));
        assert_eq!(stroke.fill().unwrap().color(), Some("#abcdef"));
        let color = stroke.fill().unwrap().color_value();
        assert_eq!(color, Color { rgb: 0xabcdef, alpha: 1.0 });

        let edge = shape.edges().next().unwrap();
        assert_eq!((edge.fill_style0(), edge.fill_style1(), edge.stroke_style()), (Some(2), Some(1), Some(3)));
        assert_eq!(edge.segments().len(), 1);
    }

    #[test]
    fn duplicate_attributes_use_last_value_and_invalid_numbers_are_absent() {
        let root = parse(b"<DOMShape isDrawingObject='false' isDrawingObject='true'><matrix><Matrix a='NaN' tx='12'/></matrix><fills><FillStyle index='1' index='4'><SolidColor alpha='Infinity'/></FillStyle></fills></DOMShape>");
        let shape = Shape::new(&root).unwrap();
        assert_eq!(shape.is_drawing_object(), Some(true));
        assert_eq!(shape.matrix(), Matrix { tx: 12.0, ..Matrix::IDENTITY });
        let fill = shape.fills().next().unwrap();
        assert_eq!(fill.index(), Some(4));
        let fill = fill.definition().unwrap();
        assert_eq!(fill.alpha(), None);
        assert_eq!(fill.color_value(), Color { rgb: 0, alpha: 1.0 });
    }

    #[test]
    fn color_attribute_values_keep_native_defaults_and_clamping() {
        assert_eq!(color_from_attributes(Some(" #123456 "), Some("0.4")), Color { rgb: 0x123456, alpha: 0.4 });
        assert_eq!(color_from_attributes(Some("invalid"), Some("NaN")), Color { rgb: 0, alpha: 1.0 });
        assert_eq!(color_from_attributes(Some("#abcdef"), Some("2")), Color { rgb: 0xabcdef, alpha: 1.0 });
        assert_eq!(color_from_attributes(None, Some("-1")), Color { rgb: 0, alpha: 0.0 });
    }

    #[test]
    fn style_table_views_keep_the_legacy_first_child_and_index_behavior() {
        let root = parse(b"<DOMShape><fills><LegacyFill index='5'><SolidColor color='#123456'/></LegacyFill></fills><strokes><LegacyStroke index='6'><SolidStroke weight='2'/></LegacyStroke></strokes></DOMShape>");
        let shape = Shape::new(&root).unwrap();
        let fill_style = shape.fills().next().unwrap();
        assert_eq!(fill_style.index(), Some(5));
        assert_eq!(fill_style.definition().map(|fill| fill.kind()), Some(FillKind::SolidColor));
        let stroke_style = shape.strokes().next().unwrap();
        assert_eq!(stroke_style.index(), Some(6));
        assert_eq!(stroke_style.definition().map(|stroke| stroke.kind()), Some(StrokeKind::Solid));
    }

    #[test]
    fn an_application_element_adapter_can_use_the_same_views() {
        #[derive(Debug)]
        struct AppElement {
            name: String,
            attrs: Vec<(String, String)>,
            children: Vec<AppElement>,
        }
        impl XflElement for AppElement {
            type Elements<'a> = std::slice::Iter<'a, AppElement>;
            fn local_name(&self) -> &str {
                self.name.rsplit(':').next().unwrap_or(&self.name)
            }
            fn attr(&self, name: &str) -> Option<&str> {
                self.attrs.iter().rev().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
            }
            fn elements(&self) -> Self::Elements<'_> {
                self.children.iter()
            }
            fn text_content(&self) -> Cow<'_, str> {
                Cow::Borrowed("")
            }
        }
        let app = AppElement { name: "x:DOMShape".into(), attrs: vec![("isDrawingObject".into(), "true".into())], children: Vec::new() };
        assert_eq!(Shape::new(&app).unwrap().is_drawing_object(), Some(true));
    }

    #[test]
    fn reads_native_rectangle_and_oval_geometry_and_styles() {
        let root = parse(br##"<DOMRectangleObject x="-2" y="3" objectWidth="20" objectHeight="10" topLeftRadius="4"><matrix><Matrix tx="7"/></matrix><fill><SolidColor color="#123456"/></fill><stroke><DashedStroke weight="2" dash1="5" dash2="3"/></stroke></DOMRectangleObject>"##);
        let primitive = Primitive::new(&root).unwrap();
        assert_eq!(primitive.kind(), PrimitiveKind::Rectangle);
        assert_eq!(
            (primitive.x(), primitive.y(), primitive.width(), primitive.height(), primitive.top_left_radius()),
            (Some(-2.0), Some(3.0), Some(20.0), Some(10.0), Some(4.0))
        );
        assert_eq!(primitive.matrix().tx, 7.0);
        assert_eq!(primitive.fill().unwrap().kind(), FillKind::SolidColor);
        assert_eq!(primitive.stroke().unwrap().kind(), StrokeKind::Dashed);

        let oval = parse(b"<DOMOvalObject objectWidth='10' objectHeight='8'/>");
        assert_eq!(Primitive::new(&oval).unwrap().kind(), PrimitiveKind::Oval);
        assert!(Primitive::new(&parse(b"<DOMShape/>")).is_none());
    }

    #[test]
    fn native_color_effect_keeps_xfl_values_and_safe_channel_defaults() {
        let element = parse(
            br##"<x:Color xmlns:x="urn:xfl" tintMultiplier=".25" tintColor="#123456"
            brightness="-.4" alphaMultiplier=".5" redMultiplier="Infinity" greenMultiplier="-2"
            blueMultiplier=".5" redOffset="255" greenOffset="-255" blueOffset="NaN" alphaOffset=".25"/>"##,
        );
        let effect = NativeColorEffect::new(&element);

        assert_eq!(effect.tint_multiplier(), Some(0.25));
        assert_eq!(effect.tint_color(), Color { rgb: 0x123456, alpha: 1.0 });
        assert_eq!(effect.brightness(), Some(-0.4));
        assert_eq!(effect.multipliers(), [1.0, -2.0, 0.5, 0.5]);
        assert_eq!(effect.offsets(), [255.0, -255.0, 0.0, 0.25]);
    }
}

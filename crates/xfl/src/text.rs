//! Borrowed views of native XFL text-field, run and style records.
//!
//! The views expose format attributes and direct text only. Text layout, font selection,
//! unsupported-style reporting and conversion into an application's text model stay with the
//! consumer.

use crate::shape::{self, Color, XflElement};
use std::borrow::Cow;

/// A native static, dynamic or input text field.
#[derive(Clone, Copy, Debug)]
pub struct TextField<'a, E: XflElement> {
    element: &'a E,
    kind: TextFieldKind,
}

/// Kind of native XFL text field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldKind {
    Static,
    Dynamic,
    Input,
}

impl<'a, E: XflElement> TextField<'a, E> {
    /// Views a recognized XFL text field.
    pub fn new(element: &'a E) -> Option<Self> {
        let kind = match element.local_name() {
            "DOMStaticText" => TextFieldKind::Static,
            "DOMDynamicText" => TextFieldKind::Dynamic,
            "DOMInputText" => TextFieldKind::Input,
            _ => return None,
        };
        Some(Self { element, kind })
    }

    pub fn kind(&self) -> TextFieldKind {
        self.kind
    }

    /// Native field orientation (`horizontal`, `vertical`, etc.).
    pub fn orientation(&self) -> Option<&'a str> {
        self.element.attr("orientation")
    }

    /// Native `autoExpand` flag, if present and parseable.
    pub fn auto_expand(&self) -> Option<bool> {
        boolean(self.element, "autoExpand")
    }

    pub fn width(&self) -> Option<f32> {
        number(self.element, "width")
    }

    pub fn height(&self) -> Option<f32> {
        number(self.element, "height")
    }

    pub fn left(&self) -> Option<f32> {
        number(self.element, "left")
    }

    pub fn top(&self) -> Option<f32> {
        number(self.element, "top")
    }

    pub fn line_type(&self) -> Option<&'a str> {
        self.element.attr("lineType")
    }

    pub fn border(&self) -> Option<bool> {
        boolean(self.element, "border")
    }

    pub fn is_selectable(&self) -> Option<bool> {
        boolean(self.element, "isSelectable")
    }

    pub fn render_as_html(&self) -> Option<bool> {
        boolean(self.element, "renderAsHTML")
    }

    pub fn max_characters(&self) -> Option<f32> {
        number(self.element, "maxCharacters")
    }

    pub fn variable_name(&self) -> Option<&'a str> {
        self.element.attr("variableName")
    }

    /// Whether the native field has any filter records, which consumers may need to report as
    /// unsupported while keeping the field.
    pub fn has_filters(&self) -> bool {
        self.element.child("filters").is_some_and(|wrapper| wrapper.elements().next().is_some())
    }

    /// Text runs in document order. Non-`DOMTextRun` children are ignored.
    pub fn runs(&self) -> impl Iterator<Item = TextRun<'a, E>> + 'a {
        self.element.child("textRuns").into_iter().flat_map(|wrapper| wrapper.elements_named("DOMTextRun")).map(|element| TextRun { element })
    }

    /// Number of element records inside the native `<textRuns>` wrapper, including unknown
    /// records. This lets consumers retain their prior malformed-record and resource-limit
    /// policy while parsing recognized runs through [`Self::runs`].
    pub fn run_entries_count(&self) -> usize {
        self.element.child("textRuns").map_or(0, |wrapper| wrapper.elements().count())
    }
}

/// One native text run and its optional style record.
#[derive(Clone, Copy, Debug)]
pub struct TextRun<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> TextRun<'a, E> {
    /// Direct character data of this run's `<characters>` element.
    pub fn characters(&self) -> Cow<'a, str> {
        self.element.child("characters").map_or(Cow::Borrowed(""), XflElement::text_content)
    }

    /// The native text attributes nested under `<textAttrs><DOMTextAttrs .../>`.
    pub fn style(&self) -> Option<TextStyle<'a, E>> {
        self.element.child("textAttrs").and_then(|wrapper| wrapper.child("DOMTextAttrs")).map(|element| TextStyle { element })
    }
}

/// Native `DOMTextAttrs` fields. Values are parsed from the wire schema but not laid out or
/// normalized for any application.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> TextStyle<'a, E> {
    pub fn face(&self) -> Option<&'a str> {
        self.element.attr("face")
    }

    pub fn size(&self) -> Option<f32> {
        number(self.element, "size")
    }

    pub fn bold(&self) -> Option<bool> {
        boolean(self.element, "bold")
    }

    pub fn italic(&self) -> Option<bool> {
        boolean(self.element, "italic")
    }

    pub fn fill_color(&self) -> Option<&'a str> {
        self.element.attr("fillColor")
    }

    /// Parsed native fill color and alpha, with XFL's black/full-opacity defaults.
    pub fn color(&self) -> Color {
        shape::color(self.element, "fillColor", "alpha")
    }

    pub fn alpha(&self) -> Option<f32> {
        number(self.element, "alpha")
    }

    pub fn alignment(&self) -> Option<&'a str> {
        self.element.attr("alignment")
    }

    pub fn indent(&self) -> Option<f32> {
        number(self.element, "indent")
    }

    pub fn character_position(&self) -> Option<&'a str> {
        self.element.attr("characterPosition")
    }

    pub fn letter_spacing(&self) -> Option<f32> {
        number(self.element, "letterSpacing")
    }

    pub fn line_spacing(&self) -> Option<f32> {
        number(self.element, "lineSpacing")
    }

    pub fn left_margin(&self) -> Option<f32> {
        number(self.element, "leftMargin")
    }

    pub fn right_margin(&self) -> Option<f32> {
        number(self.element, "rightMargin")
    }
}

fn number(element: &impl XflElement, name: &str) -> Option<f32> {
    element.attr(name)?.trim().parse::<f32>().ok().filter(|value| value.is_finite())
}

fn boolean(element: &impl XflElement, name: &str) -> Option<bool> {
    element.attr(name).map(|value| value.trim() == "true")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::parse(source).expect("well-formed test XML")
    }

    #[test]
    fn reads_prefixed_text_fields_runs_and_native_style_values() {
        let root = parse(
            br##"<x:DOMDynamicText xmlns:x="urn:xfl" orientation="horizontal" autoExpand="true" width="320" height="40" left="12" top="3" lineType="multiline" border="true" isSelectable="false" renderAsHTML="false" maxCharacters="45" variableName="message"><x:filters><x:BlurFilter/></x:filters><x:textRuns><x:DOMTextRun><x:characters>one &amp; two</x:characters><x:textAttrs><x:DOMTextAttrs face="Arial" size="24" bold="true" italic="false" fillColor="#123456" alpha="0.5" alignment="center" indent="3" characterPosition="superscript" letterSpacing="1.25" lineSpacing="2" leftMargin="4" rightMargin="5"/></x:textAttrs></x:DOMTextRun><x:Other/><x:DOMTextRun><x:characters><![CDATA[third]]></x:characters></x:DOMTextRun></x:textRuns></x:DOMDynamicText>"##,
        );
        let field = TextField::new(&root).expect("dynamic text field");
        assert_eq!(field.kind(), TextFieldKind::Dynamic);
        assert_eq!(field.orientation(), Some("horizontal"));
        assert_eq!(field.auto_expand(), Some(true));
        assert_eq!((field.width(), field.height(), field.left(), field.top()), (Some(320.0), Some(40.0), Some(12.0), Some(3.0)));
        assert_eq!(field.line_type(), Some("multiline"));
        assert_eq!(field.border(), Some(true));
        assert_eq!(field.is_selectable(), Some(false));
        assert_eq!(field.render_as_html(), Some(false));
        assert_eq!(field.max_characters(), Some(45.0));
        assert_eq!(field.variable_name(), Some("message"));
        assert!(field.has_filters());

        let mut runs = field.runs();
        let first = runs.next().expect("first run");
        assert_eq!(first.characters(), "one & two");
        let style = first.style().expect("text style");
        assert_eq!(style.face(), Some("Arial"));
        assert_eq!(style.size(), Some(24.0));
        assert_eq!(style.bold(), Some(true));
        assert_eq!(style.italic(), Some(false));
        assert_eq!(style.fill_color(), Some("#123456"));
        assert_eq!(style.alpha(), Some(0.5));
        assert_eq!(style.alignment(), Some("center"));
        assert_eq!(style.indent(), Some(3.0));
        assert_eq!(style.character_position(), Some("superscript"));
        assert_eq!(style.letter_spacing(), Some(1.25));
        assert_eq!(style.line_spacing(), Some(2.0));
        assert_eq!(style.left_margin(), Some(4.0));
        assert_eq!(style.right_margin(), Some(5.0));

        let second = runs.next().expect("second run");
        assert_eq!(second.characters(), "third");
        assert!(second.style().is_none());
        assert!(runs.next().is_none());
    }

    #[test]
    fn recognizes_all_native_field_kinds_and_ignores_non_text_elements() {
        for (name, expected) in
            [("DOMStaticText", TextFieldKind::Static), ("DOMDynamicText", TextFieldKind::Dynamic), ("DOMInputText", TextFieldKind::Input)]
        {
            let xml = format!("<{name}/>");
            let root = craft_xml::parse(xml.as_bytes()).expect("test XML");
            assert_eq!(TextField::new(&root).map(|field| field.kind()), Some(expected));
        }
        let root = parse(b"<DOMShape/>");
        assert!(TextField::new(&root).is_none());
    }

    #[test]
    fn malformed_and_non_finite_numbers_remain_absent() {
        let root = parse(b"<DOMStaticText><textRuns><DOMTextRun><textAttrs><DOMTextAttrs size='NaN' letterSpacing='bad'/></textAttrs></DOMTextRun></textRuns></DOMStaticText>");
        let field = TextField::new(&root).expect("text field");
        let style = field.runs().next().and_then(|run| run.style()).expect("style");
        assert_eq!(style.size(), None);
        assert_eq!(style.letter_spacing(), None);
    }
}

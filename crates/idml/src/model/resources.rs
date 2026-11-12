//! Resources: style groups and styles, swatches, inks, stroke styles, colour groups, and the
//! preference elements.

use craft_xml::{El, Local, Node};

use super::{Values, tidy};
use crate::value::nums;

/// The children of `el` other than `<Properties>`.
fn others(el: &El) -> impl Iterator<Item = &El> {
    el.elements().filter(|e| !e.is(Local("Properties")))
}

/// What a style styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    Paragraph,
    Character,
    Object,
    Cell,
    Table,
}

impl StyleKind {
    pub const ALL: [StyleKind; 5] = [StyleKind::Paragraph, StyleKind::Character, StyleKind::Object, StyleKind::Cell, StyleKind::Table];

    /// The element name of a style: `ParagraphStyle`, ….
    pub fn style(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "ParagraphStyle",
            StyleKind::Character => "CharacterStyle",
            StyleKind::Object => "ObjectStyle",
            StyleKind::Cell => "CellStyle",
            StyleKind::Table => "TableStyle",
        }
    }

    /// The element name of a group: `ParagraphStyleGroup`, ….
    pub fn group(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "ParagraphStyleGroup",
            StyleKind::Character => "CharacterStyleGroup",
            StyleKind::Object => "ObjectStyleGroup",
            StyleKind::Cell => "CellStyleGroup",
            StyleKind::Table => "TableStyleGroup",
        }
    }

    /// The element name of the root group: `RootParagraphStyleGroup`, ….
    pub fn root(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "RootParagraphStyleGroup",
            StyleKind::Character => "RootCharacterStyleGroup",
            StyleKind::Object => "RootObjectStyleGroup",
            StyleKind::Cell => "RootCellStyleGroup",
            StyleKind::Table => "RootTableStyleGroup",
        }
    }

    /// The kind whose root group is called `name`.
    pub fn of_root(name: &str) -> Option<StyleKind> {
        StyleKind::ALL.into_iter().find(|k| k.root() == name)
    }
}

/// A style group: a root group (`RootParagraphStyleGroup`, …) or a group in one.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleGroup {
    pub kind: StyleKind,
    /// The root group of its kind.
    pub root: bool,
    pub values: Values,
    pub children: Vec<StyleNode>,
}

/// A child of a style group, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleNode {
    Group(StyleGroup),
    Style(Style),
    Other(El),
}

/// A style, its values as written: `Name`, `Self`, `Properties/BasedOn`, its attributes; and its
/// other children (an object style's `TextFramePreference`, effect settings, …).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub kind: StyleKind,
    pub values: Values,
    pub rest: Vec<El>,
}

impl StyleGroup {
    /// The root group `el`; `None` when `el` is no root style group.
    pub fn from_el(el: &El) -> Option<StyleGroup> {
        let kind = StyleKind::of_root(el.local())?;
        Some(Self::read(kind, true, el))
    }

    fn read(kind: StyleKind, root: bool, el: &El) -> StyleGroup {
        let children = others(el)
            .map(|c| match c.local() {
                n if n == kind.group() => StyleNode::Group(Self::read(kind, false, c)),
                n if n == kind.style() => StyleNode::Style(Style { kind, values: Values::of(c), rest: others(c).map(tidy).collect() }),
                _ => StyleNode::Other(tidy(c)),
            })
            .collect();
        StyleGroup { kind, root, values: Values::of(el), children }
    }

    pub fn to_el(&self) -> El {
        let mut el = self.values.element(if self.root { self.kind.root() } else { self.kind.group() });
        el.children.extend(self.children.iter().map(|c| {
            Node::El(match c {
                StyleNode::Group(g) => g.to_el(),
                StyleNode::Style(s) => s.to_el(),
                StyleNode::Other(e) => e.clone(),
            })
        }));
        el
    }

    /// Every style in this group and the groups in it, in document order, with the names of
    /// the groups it is in (outermost first, this group's own name left out for a root group).
    pub fn styles(&self) -> Vec<(Vec<&str>, &Style)> {
        let mut out = Vec::new();
        let start: Vec<&str> = if self.root { Vec::new() } else { self.values.name().into_iter().collect() };
        // Iterative: nesting is bounded by the XML reader, but there is no need to recurse.
        let mut stack: Vec<(Vec<&str>, &StyleNode)> = self.children.iter().rev().map(|c| (start.clone(), c)).collect();
        while let Some((path, node)) = stack.pop() {
            match node {
                StyleNode::Style(s) => out.push((path, s)),
                StyleNode::Group(g) => {
                    let mut inner = path.clone();
                    inner.extend(g.values.name());
                    stack.extend(g.children.iter().rev().map(|c| (inner.clone(), c)));
                }
                StyleNode::Other(_) => {}
            }
        }
        out
    }
}

impl Style {
    pub fn to_el(&self) -> El {
        let mut el = self.values.element(self.kind.style());
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `Properties/BasedOn`: the `Self` of the style this one is based on (`$ID/[No paragraph
    /// style]` and the like for none).
    pub fn based_on(&self) -> Option<String> {
        self.values.value("BasedOn")
    }

    /// `NextStyle` of a paragraph style.
    pub fn next_style(&self) -> Option<String> {
        self.values.value("NextStyle")
    }

    /// The child element `name` (`TextFramePreference`, `TransparencySetting`, …).
    pub fn other(&self, name: &str) -> Option<&El> {
        self.rest.iter().find(|e| e.local() == name)
    }
}

/// The root style groups among `top`, in order.
pub fn style_groups(top: &[El]) -> Vec<StyleGroup> {
    top.iter().filter_map(StyleGroup::from_el).collect()
}

/// What a swatch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwatchKind {
    Color,
    Tint,
    Gradient,
    MixedInk,
    MixedInkGroup,
    /// `Swatch`: the None swatch.
    Swatch,
}

impl SwatchKind {
    pub fn of(name: &str) -> Option<SwatchKind> {
        Some(match name {
            "Color" => SwatchKind::Color,
            "Tint" => SwatchKind::Tint,
            "Gradient" => SwatchKind::Gradient,
            "MixedInk" => SwatchKind::MixedInk,
            "MixedInkGroup" => SwatchKind::MixedInkGroup,
            "Swatch" => SwatchKind::Swatch,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            SwatchKind::Color => "Color",
            SwatchKind::Tint => "Tint",
            SwatchKind::Gradient => "Gradient",
            SwatchKind::MixedInk => "MixedInk",
            SwatchKind::MixedInkGroup => "MixedInkGroup",
            SwatchKind::Swatch => "Swatch",
        }
    }
}

/// A swatch (`Color`, `Tint`, `Gradient`, …), its values as written and its other children
/// (a gradient's `GradientStop`s).
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub kind: SwatchKind,
    pub values: Values,
    pub rest: Vec<El>,
}

/// A colour's space, as `Space` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Cmyk,
    Rgb,
    Lab,
    /// `MixedInk`.
    MixedInk,
}

/// A stop of a gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStop {
    /// `StopColor`: the `Self` of a colour.
    pub color: String,
    /// `Location`, 0 to 100.
    pub location: f64,
    /// `Midpoint`, 0 to 100 (50 when absent).
    pub midpoint: f64,
}

impl Swatch {
    pub fn to_el(&self) -> El {
        let mut el = self.values.element(self.kind.name());
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `Model` is `Spot`.
    pub fn spot(&self) -> bool {
        self.values.get("Model") == Some("Spot")
    }

    /// `Space`; CMYK when absent or unknown.
    pub fn space(&self) -> ColorSpace {
        match self.values.get("Space") {
            Some("RGB") => ColorSpace::Rgb,
            Some("LAB") => ColorSpace::Lab,
            Some("MixedInk") => ColorSpace::MixedInk,
            _ => ColorSpace::Cmyk,
        }
    }

    /// `ColorValue`: CMYK in percent, RGB 0 to 255, Lab as L a b. Empty when absent or when a
    /// number is not finite.
    pub fn color_value(&self) -> Vec<f64> {
        self.values.get("ColorValue").map(nums).unwrap_or_default()
    }

    /// `BaseColor` of a tint: the `Self` of its colour.
    pub fn base_color(&self) -> Option<&str> {
        self.values.get("BaseColor")
    }

    /// `TintValue` of a tint, in percent.
    pub fn tint_value(&self) -> Option<f64> {
        self.values.num("TintValue")
    }

    /// `Type` of a gradient is `Radial`; linear otherwise.
    pub fn radial(&self) -> bool {
        self.values.get("Type") == Some("Radial")
    }

    /// A gradient's stops in order; a stop without a colour or a finite location is left out.
    pub fn stops(&self) -> Vec<GradientStop> {
        self.rest
            .iter()
            .filter(|e| e.local() == "GradientStop")
            .filter_map(|s| {
                let v = Values::of(s);
                Some(GradientStop {
                    color: v.get("StopColor")?.to_string(),
                    location: v.num("Location")?,
                    midpoint: v.num("Midpoint").unwrap_or(50.0),
                })
            })
            .collect()
    }
}

/// The swatches among `top`, in order.
pub fn swatches(top: &[El]) -> Vec<Swatch> {
    top.iter().filter_map(|e| Some(Swatch { kind: SwatchKind::of(e.local())?, values: Values::of(e), rest: others(e).map(tidy).collect() })).collect()
}

/// What a stroke style is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeStyleKind {
    Dashed,
    Dotted,
    Striped,
}

/// A custom stroke style.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub kind: StrokeStyleKind,
    pub values: Values,
    pub rest: Vec<El>,
}

impl StrokeStyle {
    pub fn to_el(&self) -> El {
        let name = match self.kind {
            StrokeStyleKind::Dashed => "DashedStrokeStyle",
            StrokeStyleKind::Dotted => "DottedStrokeStyle",
            StrokeStyleKind::Striped => "StripedStrokeStyle",
        };
        let mut el = self.values.element(name);
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `DashArray` of a dashed style: dash, gap, dash, gap, … in points.
    pub fn dash_array(&self) -> Vec<f64> {
        self.values.get("DashArray").map(nums).unwrap_or_default()
    }

    /// `StripeArray` of a striped style: each stripe's start and end, in percent of the weight.
    pub fn stripe_array(&self) -> Vec<f64> {
        self.values.get("StripeArray").map(nums).unwrap_or_default()
    }
}

/// The custom stroke styles among `top`, in order.
pub fn stroke_styles(top: &[El]) -> Vec<StrokeStyle> {
    top.iter()
        .filter_map(|e| {
            let kind = match e.local() {
                "DashedStrokeStyle" => StrokeStyleKind::Dashed,
                "DottedStrokeStyle" => StrokeStyleKind::Dotted,
                "StripedStrokeStyle" => StrokeStyleKind::Striped,
                _ => return None,
            };
            Some(StrokeStyle { kind, values: Values::of(e), rest: others(e).map(tidy).collect() })
        })
        .collect()
}

/// A colour group (`ColorGroup`): its values and the swatches it lists.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGroup {
    pub values: Values,
    pub rest: Vec<El>,
}

impl ColorGroup {
    pub fn to_el(&self) -> El {
        let mut el = self.values.element("ColorGroup");
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `IsRootColorGroup`.
    pub fn is_root(&self) -> bool {
        self.values.boolean("IsRootColorGroup") == Some(true)
    }

    /// The `Self`s of the swatches the group lists (`ColorGroupSwatch`'s `SwatchItemRef`), in
    /// order.
    pub fn members(&self) -> Vec<&str> {
        self.rest.iter().filter(|e| e.local() == "ColorGroupSwatch").filter_map(|e| e.attr("SwatchItemRef")).collect()
    }
}

/// The colour groups among `top`, in order.
pub fn color_groups(top: &[El]) -> Vec<ColorGroup> {
    top.iter().filter(|e| e.local() == "ColorGroup").map(|e| ColorGroup { values: Values::of(e), rest: others(e).map(tidy).collect() }).collect()
}

/// The values of every element named `name` among `top`, in order: `Ink`, `Layer`, `Section`,
/// `Condition`, `NumberingList`, ….
pub fn named(top: &[El], name: &str) -> Vec<Values> {
    top.iter().filter(|e| e.local() == name).map(Values::of).collect()
}

/// The first element named `name` among `top` (a preference: `DocumentPreference`,
/// `MarginPreference`, `TextFramePreference`, `GridPreference`, …), its values.
pub fn preference(top: &[El], name: &str) -> Option<Values> {
    top.iter().find(|e| e.local() == name).map(Values::of)
}

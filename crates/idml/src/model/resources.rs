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

/// A `GradientStop` in source order, including optional values as written.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStopValue {
    pub color: Option<String>,
    pub location: Option<f64>,
    pub midpoint: Option<f64>,
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

    /// Every gradient stop in source order, retaining absent or malformed optional values.
    pub fn stops_in_order(&self) -> Vec<GradientStopValue> {
        self.rest
            .iter()
            .filter(|e| e.local() == "GradientStop")
            .map(|e| {
                let values = Values::of(e);
                GradientStopValue {
                    color: values.get("StopColor").map(str::to_string),
                    location: values.num("Location"),
                    midpoint: values.num("Midpoint"),
                }
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

/// An Ink Manager entry, preserving its source values.
#[derive(Clone, Debug, PartialEq)]
pub struct Ink {
    pub values: Values,
}

/// A neutral colour definition for writing `Graphic.xml`.
///
/// The caller supplies colour-space values as IDML's serialized value list; this type owns the
/// element and attribute grammar so applications do not need to emit `Color` XML themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorDefinition {
    pub self_id: String,
    pub name: String,
    pub model: String,
    pub space: String,
    pub value: String,
    pub override_kind: String,
    pub editable: bool,
    pub removable: bool,
    pub visible: bool,
}

impl ColorDefinition {
    pub fn to_el(&self) -> El {
        El::new("Color")
            .with_attr("Self", &self.self_id)
            .with_attr("Model", &self.model)
            .with_attr("Space", &self.space)
            .with_attr("ColorValue", &self.value)
            .with_attr("ColorOverride", &self.override_kind)
            .with_attr("AlternateSpace", "NoAlternateColor")
            .with_attr("AlternateColorValue", "")
            .with_attr("Name", &self.name)
            .with_attr("ColorEditable", self.editable)
            .with_attr("ColorRemovable", self.removable)
            .with_attr("Visible", self.visible)
            .with_attr("SwatchCreatorID", "7937")
    }
}

/// A neutral `Ink` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct InkDefinition {
    pub self_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub angle: String,
    pub convert_to_process: bool,
    pub frequency: String,
    pub neutral_density: String,
    pub trap_order: usize,
    pub ink_type: String,
}

impl InkDefinition {
    fn to_el(&self) -> El {
        let mut el = El::new("Ink")
            .with_attr("Self", &self.self_id)
            .with_attr("Name", &self.name)
            .with_attr("Angle", &self.angle)
            .with_attr("ConvertToProcess", self.convert_to_process)
            .with_attr("Frequency", &self.frequency)
            .with_attr("NeutralDensity", &self.neutral_density)
            .with_attr("PrintInk", "true")
            .with_attr("TrapOrder", self.trap_order)
            .with_attr("InkType", &self.ink_type);
        if let Some(alias) = &self.alias {
            el.set_attr("AliasInkName", alias);
        }
        el
    }
}

/// A neutral `Tint` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct TintDefinition {
    pub self_id: String,
    pub value: String,
    pub base_color: String,
    pub name: String,
}

impl TintDefinition {
    fn to_el(&self) -> El {
        El::new("Tint")
            .with_attr("Self", &self.self_id)
            .with_attr("TintValue", &self.value)
            .with_attr("BaseColor", &self.base_color)
            .with_attr("Name", &self.name)
            .with_attr("ColorOverride", "Normal")
            .with_attr("AlternateSpace", "NoAlternateColor")
            .with_attr("AlternateColorValue", "")
            .with_attr("ColorEditable", "true")
            .with_attr("ColorRemovable", "true")
            .with_attr("Visible", "true")
            .with_attr("SwatchCreatorID", "7937")
    }
}

/// A stop in a neutral `Gradient` definition.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStopDefinition {
    pub self_id: String,
    pub color_id: String,
    pub location: String,
    pub midpoint: Option<String>,
}

/// A neutral `Gradient` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientDefinition {
    pub self_id: String,
    pub name: String,
    pub radial: bool,
    pub stops: Vec<GradientStopDefinition>,
}

impl GradientDefinition {
    fn to_el(&self) -> El {
        let mut el = El::new("Gradient")
            .with_attr("Self", &self.self_id)
            .with_attr("Type", if self.radial { "Radial" } else { "Linear" })
            .with_attr("Name", &self.name)
            .with_attr("ColorEditable", "true")
            .with_attr("ColorRemovable", "true")
            .with_attr("Visible", "true")
            .with_attr("SwatchCreatorID", "7937");
        for stop in &self.stops {
            let mut child =
                El::new("GradientStop").with_attr("Self", &stop.self_id).with_attr("StopColor", &stop.color_id).with_attr("Location", &stop.location);
            if let Some(midpoint) = &stop.midpoint {
                child.set_attr("Midpoint", midpoint);
            }
            el.push(child);
        }
        el
    }
}

/// A member reference in a neutral `ColorGroup` definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorGroupMember {
    pub self_id: String,
    pub swatch_id: String,
}

/// A neutral `ColorGroup` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorGroupDefinition {
    pub self_id: String,
    pub name: String,
    pub members: Vec<ColorGroupMember>,
}

impl ColorGroupDefinition {
    fn to_el(&self) -> El {
        let mut el = El::new("ColorGroup").with_attr("Self", &self.self_id).with_attr("Name", &self.name).with_attr("IsRootColorGroup", "false");
        for member in &self.members {
            el.push(El::new("ColorGroupSwatch").with_attr("Self", &member.self_id).with_attr("SwatchItemRef", &member.swatch_id));
        }
        el
    }
}

/// A custom stroke style in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub enum StrokeDefinition {
    Striped { self_id: String, name: String, array: String },
    Dashed { self_id: String, name: String, array: String },
    Dotted { self_id: String, name: String, array: String },
    Named { self_id: String, name: String },
}

impl StrokeDefinition {
    fn to_el(&self) -> El {
        match self {
            Self::Striped { self_id, name, array } => {
                El::new("StripedStrokeStyle").with_attr("StripeArray", array).with_attr("Self", self_id).with_attr("Name", name)
            }
            Self::Dashed { self_id, name, array } => {
                El::new("DashedStrokeStyle").with_attr("DashArray", array).with_attr("Self", self_id).with_attr("Name", name)
            }
            Self::Dotted { self_id, name, array } => {
                El::new("DottedStrokeStyle").with_attr("DotArray", array).with_attr("Self", self_id).with_attr("Name", name)
            }
            Self::Named { self_id, name } => El::new("StrokeStyle").with_attr("Self", self_id).with_attr("Name", name),
        }
    }
}

/// Semantic values needed to write the `Graphic.xml` part. This type owns element names,
/// attributes, child ordering and the required None swatch; callers supply their mapped values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphicPart {
    pub colors: Vec<ColorDefinition>,
    pub inks: Vec<InkDefinition>,
    pub tints: Vec<TintDefinition>,
    pub gradients: Vec<GradientDefinition>,
    pub color_groups: Vec<ColorGroupDefinition>,
    pub stroke_styles: Vec<StrokeDefinition>,
}

impl GraphicPart {
    pub fn to_el(&self) -> El {
        let mut root = crate::part_root("Graphic");
        for color in &self.colors {
            root.push(color.to_el());
        }
        for ink in &self.inks {
            root.push(ink.to_el());
        }
        for tint in &self.tints {
            root.push(tint.to_el());
        }
        root.push(
            El::new("Swatch")
                .with_attr("Self", "Swatch/None")
                .with_attr("Name", "None")
                .with_attr("ColorEditable", "false")
                .with_attr("ColorRemovable", "false")
                .with_attr("Visible", "true")
                .with_attr("SwatchCreatorID", "7937"),
        );
        for gradient in &self.gradients {
            root.push(gradient.to_el());
        }
        for group in &self.color_groups {
            root.push(group.to_el());
        }
        for style in &self.stroke_styles {
            root.push(style.to_el());
        }
        root
    }
}

/// A composite-font entry, including the applied font family stored in `Properties`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositeFontEntryDefinition {
    pub self_id: String,
    pub name: String,
    pub characters: String,
    pub font_style: String,
    pub relative_size: String,
    pub horizontal_scale: String,
    pub vertical_scale: String,
    pub baseline_shift: String,
    pub scale_option: bool,
    pub applied_font: String,
}

/// A composite font in `Fonts.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositeFontDefinition {
    pub self_id: String,
    pub name: String,
    pub entries: Vec<CompositeFontEntryDefinition>,
}

/// A font face in a family in `Fonts.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct FontDefinition {
    pub self_id: String,
    pub family: String,
    pub name: String,
    pub postscript_name: String,
    pub style: String,
}

/// A family of font faces in `Fonts.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFamilyDefinition {
    pub self_id: String,
    pub name: String,
    pub fonts: Vec<FontDefinition>,
}

/// Neutral data for writing the complete `Fonts.xml` part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontsPart {
    pub composites: Vec<CompositeFontDefinition>,
    pub families: Vec<FontFamilyDefinition>,
}

impl FontsPart {
    pub fn to_el(&self) -> El {
        let mut root = crate::part_root("Fonts");
        for composite in &self.composites {
            let mut element = El::new("CompositeFont").with_attr("Self", &composite.self_id).with_attr("Name", &composite.name);
            for entry in &composite.entries {
                let mut node = El::new("CompositeFontEntry")
                    .with_attr("Self", &entry.self_id)
                    .with_attr("Name", &entry.name)
                    .with_attr("CustomCharacters", &entry.characters)
                    .with_attr("FontStyle", &entry.font_style)
                    .with_attr("RelativeSize", &entry.relative_size)
                    .with_attr("HorizontalScale", &entry.horizontal_scale)
                    .with_attr("VerticalScale", &entry.vertical_scale)
                    .with_attr("BaselineShift", &entry.baseline_shift)
                    .with_attr("ScaleOption", entry.scale_option);
                let mut properties = El::new("Properties");
                properties.push(crate::prop("AppliedFont", "string", &entry.applied_font));
                node.push(properties);
                element.push(node);
            }
            root.push(element);
        }
        for family in &self.families {
            let mut element = El::new("FontFamily").with_attr("Self", &family.self_id).with_attr("Name", &family.name);
            for font in &family.fonts {
                element.push(
                    El::new("Font")
                        .with_attr("Self", &font.self_id)
                        .with_attr("FontFamily", &font.family)
                        .with_attr("Name", &font.name)
                        .with_attr("PostScriptName", &font.postscript_name)
                        .with_attr("Status", "NotAvailable")
                        .with_attr("FontStyleName", &font.style)
                        .with_attr("FontType", "OpenTypeTT")
                        .with_attr("WritingScript", "0")
                        .with_attr("FullName", &font.name)
                        .with_attr("FullNameNative", &font.name)
                        .with_attr("FontStyleNameNative", &font.style)
                        .with_attr("PlatformName", "$ID/")
                        .with_attr("Version", "$ID/")
                        .with_attr("TypekitID", "$ID/"),
                );
            }
            root.push(element);
        }
        root
    }
}

impl Ink {
    pub fn name(&self) -> Option<&str> {
        self.values.name()
    }

    pub fn convert_to_process(&self) -> bool {
        self.values.get("ConvertToProcess") == Some("true")
    }

    pub fn alias(&self) -> Option<&str> {
        self.values.get("AliasInkName")
    }
}

/// Ink Manager entries among the document's top-level elements.
pub fn inks(top: &[El]) -> Vec<Ink> {
    top.iter().filter(|el| el.local() == "Ink").map(|el| Ink { values: Values::of(el) }).collect()
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

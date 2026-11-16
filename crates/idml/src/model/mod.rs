//! A typed model of IDML: spreads and parent (master) spreads, pages, guides, page items and the
//! graphics placed in them; stories with their paragraph and character ranges, text, tables,
//! notes and anchored items; style groups and styles, swatches, inks, stroke styles, colour
//! groups and preferences.
//!
//! The model types what contains what. Each node keeps its attributes and `<Properties>`
//! children as written ([`Values`]) and reads them through typed accessors, path geometry among
//! them; anything the model does not type is kept as XML where it stood ([`tidy`]: without the
//! part's indentation). Parsing and writing a node therefore loses nothing, and a caller reads
//! the properties it maps and leaves the rest.
//!
//! Nodes are built from the elements [`crate::Package::document`] gives ([`Spread::from_el`],
//! [`spreads`]) and written back with `to_el`, then [`crate::part_xml`].

mod document;
mod effects;
mod geometry;
mod preferences;
mod resources;
mod spread;
mod story;

use craft_xml::{El, Local, Node};
pub use document::{
    CellStyleValues, ConditionValues, CrossReferenceBuildingBlock, CrossReferenceFormatValues, EpubSplitValues, ModelDocument, NumberingListValues,
    TableStyleValues, TextDateKind, TextVariableKind, TextVariableValues, TocStyleEntryValues, TocStyleValues, TopLevelKind, TopLevelNode,
};
pub use effects::{
    DirectionalFeatherValues, DropShadowValues, FeatherValues, GradientFeatherValues, ObjectStyleStack, OpacityStopValues, TransparencySetting,
};
pub use geometry::{MAX_PATH_POINTS, PathGeometry, PathGeometryError, PathPoint, SubPath};
pub use preferences::{DocumentPreferences, FootnotePreferences, GridPreferences, TextFramePreferences};
pub use resources::{
    CharacterAppearanceValues, CharacterSpecialValues, CharacterTypographyValues, CjkCharacterValues, CjkParagraphValues, ColorDefinition,
    ColorGroup, ColorGroupDefinition, ColorGroupMember, ColorSpace, CompositeFontDefinition, CompositeFontEntryDefinition, EpubExportTagMap,
    FontDefinition, FontFamilyDefinition, FontsPart, GradientDefinition, GradientStop, GradientStopDefinition, GradientStopValue, GraphicPart,
    GrepStyleValue, Ink, InkDefinition, MojikumiRow, NestedLineStyleValue, NestedStyleValue, NestedStyleValues, ObjectStyleValues,
    OpenTypeFeatureValues, ParagraphAlignmentValues, ParagraphBoxValues, ParagraphFlowValues, ParagraphListValues, ParagraphSpanValues, RubyValues,
    StrokeDefinition, StrokeStyle, StrokeStyleKind, Style, StyleGroup, StyleGroupWriter, StyleKind, StyleNode, Swatch, SwatchKind, TabStopValues,
    TextDecorationValues, TintDefinition, color_groups, inks, named, preference, stroke_styles, style_groups, swatches,
};
pub use spread::{
    BaselineFrameGridOption, BaselineFrameGridRelative, ClippingPathValues, FrameFittingOption, Graphic, GraphicChild, GraphicKind,
    GraphicLayerOptions, Guide, ItemChild, ItemKind, Link, MarginPreference, Page, PageChild, PageItem, PdfAttribute, Spread, SpreadChild, TextPath,
    TextWrapPreference, spreads,
};
pub use story::{
    Ace, AnchoredObjectSetting, AnchoredObjectValues, CONTAINERS, Cell, CellOptions, Container, ContainerKind, ContentPart, CrossReferenceSource,
    IndexCrossReferenceValues, IndexDocument, IndexReference, IndexTopicPath, IndexValues, PageReference, Range, SavedCrossReference,
    SavedCrossReferencePart, Story, StoryAnchor, StoryPreference, Table, TableChild, TableColumn, TableLayout, TableRow, TextNode, TrackedChangeKind,
    stories,
};

use crate::value::{matrix, nums};

/// An IDML transform: `a b c d tx ty`, inner coordinates to parent coordinates.
pub type Matrix = [f64; 6];

/// The identity transform.
pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A node's own values: its attributes and its `<Properties>` children, as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    pub attrs: Vec<(String, String)>,
    /// The children of `<Properties>`, in order.
    pub properties: Vec<El>,
}

/// Raw IDML stroke values before the caller maps swatches and stroke styles into its model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StrokeValues {
    pub weight: Option<f64>,
    pub color: Option<String>,
    pub tint: Option<f64>,
    pub kind: Option<String>,
    pub alignment: Option<String>,
    pub cap: Option<String>,
    pub join: Option<String>,
    pub miter_limit: Option<f64>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub gap_color: Option<String>,
    pub gap_tint: Option<f64>,
    pub overprint: Option<bool>,
    pub gap_overprint: Option<bool>,
}

impl StrokeValues {
    /// Reads stroke properties from elements in precedence order (item first, then nearer styles).
    pub fn from_elements(elements: &[&El]) -> StrokeValues {
        let mut result = StrokeValues::default();
        for el in elements {
            let values = Values::of(el);
            let num_attr = |key: &str| values.get(key).and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite());
            let bool_attr = |key: &str| match values.get(key) {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            };
            if result.weight.is_none() {
                result.weight = num_attr("StrokeWeight");
            }
            if result.color.is_none() {
                result.color = values.get("StrokeColor").map(str::to_owned);
            }
            if result.tint.is_none() {
                result.tint = num_attr("StrokeTint");
            }
            if result.kind.is_none() {
                result.kind = values.get("StrokeType").map(str::to_owned);
            }
            if result.alignment.is_none() {
                result.alignment = values.get("StrokeAlignment").map(str::to_owned);
            }
            if result.cap.is_none() {
                result.cap = values.get("EndCap").map(str::to_owned);
            }
            if result.join.is_none() {
                result.join = values.get("EndJoin").map(str::to_owned);
            }
            if result.miter_limit.is_none() {
                result.miter_limit = num_attr("MiterLimit");
            }
            if result.start.is_none() {
                result.start = values.get("LeftLineEnd").map(str::to_owned);
            }
            if result.end.is_none() {
                result.end = values.get("RightLineEnd").map(str::to_owned);
            }
            if result.gap_color.is_none() {
                result.gap_color = values.get("GapColor").map(str::to_owned);
            }
            if result.gap_tint.is_none() {
                result.gap_tint = num_attr("GapTint");
            }
            if result.overprint.is_none() {
                result.overprint = bool_attr("OverprintStroke");
            }
            if result.gap_overprint.is_none() {
                result.gap_overprint = bool_attr("OverprintGap");
            }
        }
        result
    }
}

/// Raw fill properties before a caller resolves swatch names to its own colour model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FillValues {
    pub color: Option<String>,
    pub tint: Option<f64>,
    pub overprint: Option<bool>,
}

impl FillValues {
    /// Reads fill attributes in precedence order (item first, then nearer styles).
    pub fn from_elements(elements: &[&El]) -> FillValues {
        let mut result = FillValues::default();
        for el in elements {
            let values = Values::of(el);
            if result.color.is_none() {
                result.color = values.get("FillColor").map(str::to_owned);
            }
            if result.tint.is_none() {
                result.tint = values.get("FillTint").and_then(|value| value.parse::<f64>().ok());
            }
            if result.overprint.is_none() {
                result.overprint = match values.get("OverprintFill") {
                    Some("true") => Some(true),
                    Some("false") => Some(false),
                    _ => None,
                };
            }
        }
        result
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlternatingFillValues {
    pub first_count: Option<f64>,
    pub first_color: Option<String>,
    pub first_tint: Option<f64>,
    pub next_count: Option<f64>,
    pub next_color: Option<String>,
    pub next_tint: Option<f64>,
    pub skip_first: Option<f64>,
    pub skip_last: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlternatingStrokeValues {
    pub first_count: Option<f64>,
    pub next_count: Option<f64>,
    pub first: StrokeValues,
    pub next: StrokeValues,
    pub skip_first: Option<f64>,
    pub skip_last: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableOptions {
    pub direction: Option<String>,
    pub header_rows: Option<f64>,
    pub footer_rows: Option<f64>,
    pub applied_style: Option<String>,
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
    pub repeat_header: bool,
    pub repeat_footer: bool,
    pub borders: [StrokeValues; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CornerValues {
    pub option: Option<String>,
    pub radius: Option<f64>,
}

impl Values {
    /// The attributes and `<Properties>` of `el`.
    pub fn of(el: &El) -> Values {
        let properties = el.child(Local("Properties")).map(|p| p.elements().map(tidy).collect()).unwrap_or_default();
        Values { attrs: el.attrs.clone(), properties }
    }

    /// An attribute as written.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Sets an attribute, replacing one of that name.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((key.to_string(), value)),
        }
    }

    /// The `<Properties>` child `key`.
    pub fn prop_el(&self, key: &str) -> Option<&El> {
        self.properties.iter().find(|e| e.local() == key)
    }

    /// Replaces the `<Properties>` child of `el`'s name, or adds it; `None` removes it.
    pub fn set_prop_el(&mut self, key: &str, el: Option<El>) {
        let at = self.properties.iter().position(|e| e.local() == key);
        match (at, el) {
            (Some(i), Some(el)) => {
                if let Some(slot) = self.properties.get_mut(i) {
                    *slot = el;
                }
            }
            (Some(i), None) => {
                self.properties.remove(i);
            }
            (None, Some(el)) => self.properties.push(el),
            (None, None) => {}
        }
    }

    /// A property in either spelling: the attribute, else the text of the `<Properties>` child.
    pub fn prop(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(v) => Some(v.to_string()),
            None => self.prop_el(key).map(El::deep_text),
        }
    }

    /// [`Values::prop`] trimmed; `None` when blank.
    pub fn value(&self, key: &str) -> Option<String> {
        self.prop(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// A finite number; `NaN` and infinities count as absent.
    pub fn num(&self, key: &str) -> Option<f64> {
        self.prop(key).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
    }

    /// A two-number value (for properties such as `GradientFillStart`), bounded to three tokens.
    pub fn pair(&self, key: &str) -> Option<[f64; 2]> {
        let source = self.prop(key)?;
        let values: Vec<f64> = source.split_whitespace().take(3).filter_map(|v| v.parse::<f64>().ok()).collect();
        <[f64; 2]>::try_from(values).ok().filter(|v| v.iter().all(|n| n.is_finite()))
    }

    /// `true` or `false`; anything else counts as absent.
    pub fn boolean(&self, key: &str) -> Option<bool> {
        match self.prop(key)?.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    /// `Self`, the node's id.
    pub fn self_id(&self) -> Option<&str> {
        self.get("Self")
    }

    pub fn name(&self) -> Option<&str> {
        self.get("Name")
    }

    /// `ItemTransform`; the identity when absent or not six finite numbers.
    pub fn item_transform(&self) -> Matrix {
        self.get("ItemTransform").and_then(matrix).unwrap_or(IDENTITY)
    }

    /// `ItemLayer`: the `Self` of the layer the node is on.
    pub fn item_layer(&self) -> Option<&str> {
        self.get("ItemLayer")
    }

    /// `Visible`; visible when absent.
    pub fn visible(&self) -> bool {
        self.boolean("Visible") != Some(false)
    }

    /// `Locked`; unlocked when absent.
    pub fn locked(&self) -> bool {
        self.boolean("Locked") == Some(true)
    }

    /// The element `name` with these values: attributes, then `<Properties>` (when there are
    /// any) as its first child.
    pub fn element(&self, name: &str) -> El {
        let mut el = El::new(name);
        el.attrs = self.attrs.clone();
        if !self.properties.is_empty() {
            let mut p = El::new("Properties");
            p.children = self.properties.iter().cloned().map(Node::El).collect();
            el.push(p);
        }
        el
    }

    /// Read an IDML stroke attribute family such as `TopEdgeStrokeWeight` or
    /// `StartRowStrokeColor`.
    pub fn stroke_values(&self, prefix: &str) -> StrokeValues {
        StrokeValues {
            weight: self.num(&format!("{prefix}StrokeWeight")).map(|v| v.max(0.0)),
            color: self.value(&format!("{prefix}StrokeColor")),
            tint: self.num(&format!("{prefix}StrokeTint")),
            kind: self.value(&format!("{prefix}StrokeType")),
            gap_color: self.value(&format!("{prefix}StrokeGapColor")),
            gap_tint: self.num(&format!("{prefix}StrokeGapTint")),
            ..StrokeValues::default()
        }
    }

    pub fn alternating_fill_values(&self, kind: &str) -> AlternatingFillValues {
        let num = |key: &str| self.num(key).map(|v| v.max(0.0));
        AlternatingFillValues {
            first_count: num(&format!("Start{kind}FillCount")),
            first_color: self.value(&format!("Start{kind}FillColor")),
            first_tint: self.num(&format!("Start{kind}FillTint")),
            next_count: num(&format!("End{kind}FillCount")),
            next_color: self.value(&format!("End{kind}FillColor")),
            next_tint: self.num(&format!("End{kind}FillTint")),
            skip_first: num(&format!("SkipFirstAlternatingFill{kind}s")),
            skip_last: num(&format!("SkipLastAlternatingFill{kind}s")),
        }
    }

    pub fn alternating_stroke_values(&self, kind: &str) -> AlternatingStrokeValues {
        let num = |key: &str| self.num(key).map(|v| v.max(0.0));
        let mut next = self.stroke_values(&format!("End{kind}"));
        // IDML writes this one table-column value as LineStyle rather than StrokeType.
        if kind == "Column" {
            next.kind = self.value("EndColumnLineStyle").or(next.kind);
        }
        AlternatingStrokeValues {
            first_count: num(&format!("Start{kind}StrokeCount")),
            next_count: num(&format!("End{kind}StrokeCount")),
            first: self.stroke_values(&format!("Start{kind}")),
            next,
            skip_first: num(&format!("SkipFirstAlternatingStroke{kind}s")),
            skip_last: num(&format!("SkipLastAlternatingStroke{kind}s")),
        }
    }

    pub fn table_options(&self) -> TableOptions {
        TableOptions {
            direction: self.value("TableDirection"),
            header_rows: self.num("HeaderRowCount"),
            footer_rows: self.num("FooterRowCount"),
            applied_style: self.value("AppliedTableStyle"),
            space_before: self.num("SpaceBefore"),
            space_after: self.num("SpaceAfter"),
            repeat_header: self.get("HeaderBehavior") != Some("RepeatOnce"),
            repeat_footer: self.get("FooterBehavior") != Some("RepeatOnce"),
            borders: [
                self.stroke_values("TopBorder"),
                self.stroke_values("LeftBorder"),
                self.stroke_values("BottomBorder"),
                self.stroke_values("RightBorder"),
            ],
        }
    }

    pub fn corner_values(&self, corner: &str) -> CornerValues {
        CornerValues {
            option: self.value(&format!("{corner}CornerOption")).or_else(|| self.value("CornerOption")),
            radius: self.num(&format!("{corner}CornerRadius")).or_else(|| self.num("CornerRadius")),
        }
    }
}

/// `el` without its indentation: the whitespace-only text between the children of an element
/// that has child elements. Text that is not only whitespace, and an element's only text, is kept.
/// A part's indentation means nothing, and the writer makes its own.
pub fn tidy(el: &El) -> El {
    let blank = |n: &Node| matches!(n, Node::Text(t) if t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')));
    let element_only = el.children.iter().any(|n| matches!(n, Node::El(_))) && el.children.iter().all(|n| matches!(n, Node::El(_)) || blank(n));
    let mut out = El::new(el.name.clone());
    out.attrs = el.attrs.clone();
    for n in &el.children {
        match n {
            Node::El(c) => out.push(tidy(c)),
            n if element_only && blank(n) => {}
            n => out.children.push(n.clone()),
        }
    }
    out
}

/// Four numbers `top left bottom right` (`GeometricBounds`, `VisibleBounds`), finite.
pub fn bounds(s: &str) -> Option<[f64; 4]> {
    <[f64; 4]>::try_from(nums(s)).ok()
}

/// `p` through `m`.
pub fn apply(m: &Matrix, p: [f64; 2]) -> [f64; 2] {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

/// `outer · inner`: `inner` first, then `outer` (a child's transform into its parent's space,
/// then the parent's into its own parent's).
pub fn concat(outer: &Matrix, inner: &Matrix) -> Matrix {
    let [a, b, c, d, e, f] = *outer;
    let [p, q, r, s, t, u] = *inner;
    [a * p + c * q, b * p + d * q, a * r + c * s, b * r + d * s, a * t + c * u + e, b * t + d * u + f]
}

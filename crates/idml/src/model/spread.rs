//! Spreads, pages, guides, page items and graphics.

use craft_xml::{El, Local, Node};

use super::{Matrix, PathGeometry, Values, bounds, concat, tidy};
use crate::Document;
use crate::value::base64_decode;

/// The children of `el` other than `<Properties>`.
fn others(el: &El) -> impl Iterator<Item = &El> {
    el.elements().filter(|e| !e.is(Local("Properties")))
}

/// `el` (built by [`Values::element`]) with `children` after its `<Properties>`.
fn with_children(mut el: El, children: impl IntoIterator<Item = El>) -> El {
    el.children.extend(children.into_iter().map(Node::El));
    el
}

/// Raw clip-path settings attached to a placed graphic, before the caller maps type and
/// coordinates into its scene model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClippingPathValues {
    pub clipping_type: Option<String>,
    pub geometry: PathGeometry,
    pub invert: Option<bool>,
    pub include_inside_edges: Option<bool>,
    pub restrict_to_frame: Option<bool>,
    pub use_high_resolution: Option<bool>,
    pub threshold: Option<f64>,
    pub tolerance: Option<f64>,
    pub inset: Option<f64>,
    pub source_name: Option<String>,
    pub index: Option<f64>,
}

impl ClippingPathValues {
    /// Reads a graphic's `ClippingPathSettings` and saved geometry, with the caller's point cap.
    pub fn from_graphic(graphic: &El, max_points: usize) -> Option<Self> {
        let settings = graphic.find("ClippingPathSettings")?;
        let settings_values = Values::of(settings);
        let graphic_values = Values::of(graphic);
        let clipping_type = settings_values.get("ClippingType").map(str::to_owned);
        let invert = settings_values.boolean("InvertPath");
        let include_inside_edges = settings_values.boolean("IncludeInsideEdges");
        let restrict_to_frame = settings_values.boolean("RestrictToFrame");
        let use_high_resolution = settings_values.boolean("UseHighResolutionImage");
        let threshold = settings_values.num("Threshold");
        let tolerance = settings_values.num("Tolerance");
        let inset = settings_values.num("InsetFrame");
        let source_name = settings_values.get("AppliedPathName").map(str::to_owned);
        let index = settings_values.num("Index");
        let geometry = settings_values
            .prop_el("ClippingPathGeometry")
            .or_else(|| settings.find("ClippingPathGeometry"))
            .or_else(|| graphic_values.prop_el("ClippingPathGeometry"))?;
        let geometry = PathGeometry::from_el_capped(geometry, max_points);
        Some(Self {
            clipping_type,
            geometry,
            invert,
            include_inside_edges,
            restrict_to_frame,
            use_high_resolution,
            threshold,
            tolerance,
            inset,
            source_name,
            index,
        })
    }
}

/// A spread (`Spread`) or a parent spread (`MasterSpread`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spread {
    /// A parent ("master") spread.
    pub master: bool,
    pub values: Values,
    pub children: Vec<SpreadChild>,
}

/// A child of a spread, in document order: the order of its page items is their stacking order,
/// back to front.
#[derive(Clone, Debug, PartialEq)]
pub enum SpreadChild {
    Page(Page),
    Item(PageItem),
    /// Anything else (`FlattenerPreference`, …), as written.
    Other(El),
}

impl Spread {
    pub fn from_el(el: &El) -> Spread {
        let children = others(el)
            .map(|c| match c.local() {
                "Page" => SpreadChild::Page(Page::from_el(c)),
                n if ItemKind::of(n).is_some() => PageItem::from_el(c).map_or_else(|| SpreadChild::Other(tidy(c)), SpreadChild::Item),
                _ => SpreadChild::Other(tidy(c)),
            })
            .collect();
        Spread { master: el.local() == "MasterSpread", values: Values::of(el), children }
    }

    pub fn to_el(&self) -> El {
        let name = if self.master { "MasterSpread" } else { "Spread" };
        with_children(
            self.values.element(name),
            self.children.iter().map(|c| match c {
                SpreadChild::Page(p) => p.to_el(),
                SpreadChild::Item(i) => i.to_el(),
                SpreadChild::Other(e) => e.clone(),
            }),
        )
    }

    pub fn pages(&self) -> impl DoubleEndedIterator<Item = &Page> {
        self.children.iter().filter_map(|c| if let SpreadChild::Page(p) = c { Some(p) } else { None })
    }

    /// The top-level page items, back to front.
    pub fn items(&self) -> impl DoubleEndedIterator<Item = &PageItem> {
        self.children.iter().filter_map(|c| if let SpreadChild::Item(i) = c { Some(i) } else { None })
    }

    /// Every page item, nested ones included, in document order, each with its transform into
    /// spread coordinates.
    pub fn all_items(&self) -> Vec<(&PageItem, Matrix)> {
        let mut out = Vec::new();
        // Iterative: nesting is bounded by the XML reader, but there is no need to recurse.
        let mut stack: Vec<(&PageItem, Matrix)> = self.items().rev().map(|i| (i, i.values.item_transform())).collect();
        while let Some((item, m)) = stack.pop() {
            out.push((item, m));
            stack.extend(item.items().rev().map(|c| (c, concat(&m, &c.values.item_transform()))));
        }
        out
    }

    /// `BaseName` and `NamePrefix` of a parent spread (`A` and `Parent`).
    pub fn parent_name(&self) -> Option<(&str, &str)> {
        Some((self.values.get("NamePrefix")?, self.values.get("BaseName")?))
    }

    /// Parent spread's prefix, using the prefix of `Name` or `A` when absent.
    pub fn parent_prefix(&self) -> String {
        self.values.get("NamePrefix").map(str::to_string).unwrap_or_else(|| self.name().split('-').next().unwrap_or("A").to_string())
    }

    /// Parent spread's base name, using the suffix of `Name` or `Parent` when absent.
    pub fn base_name(&self) -> String {
        self.values
            .get("BaseName")
            .map(str::to_string)
            .unwrap_or_else(|| self.name().split_once('-').map(|(_, b)| b.to_string()).unwrap_or_else(|| self.name().to_string()))
    }

    pub fn name(&self) -> &str {
        self.values.name().unwrap_or("Parent")
    }

    /// Whether master page items are shown.
    pub fn show_master_items(&self) -> bool {
        self.values.boolean("ShowMasterItems") != Some(false)
    }

    /// Whether page shuffle is allowed.
    pub fn allow_shuffle(&self) -> bool {
        self.values.boolean("AllowPageShuffle") != Some(false)
    }

    /// Exact quarter-turn view rotation encoded by the spread transform, clockwise on screen.
    pub fn view_rotation(&self) -> u8 {
        if self.master {
            return 0;
        }
        let [a, b, c, d, _, _] = self.values.item_transform();
        let is = |x: f64, y: f64| (x - y).abs() < 1e-6;
        let quarter = |m: [f64; 4]| is(a, m[0]) && is(b, m[1]) && is(c, m[2]) && is(d, m[3]);
        if quarter([0.0, 1.0, -1.0, 0.0]) {
            1
        } else if quarter([-1.0, 0.0, 0.0, -1.0]) {
            2
        } else if quarter([0.0, -1.0, 1.0, 0.0]) {
            3
        } else {
            0
        }
    }
}

/// The spreads and parent spreads of a document, in its order.
pub fn spreads(doc: &Document) -> Vec<Spread> {
    doc.top.iter().filter(|e| matches!(e.local(), "Spread" | "MasterSpread")).map(Spread::from_el).collect()
}

/// A page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub values: Values,
    pub children: Vec<PageChild>,
}

/// Parsed layout values of an IDML `MarginPreference`.
#[derive(Clone, Debug, PartialEq)]
pub struct MarginPreference {
    pub values: Values,
}

impl MarginPreference {
    pub fn from_el(el: &El) -> Self {
        Self { values: Values::of(el) }
    }
    pub fn top(&self) -> Option<f64> {
        self.values.num("Top")
    }
    pub fn left(&self) -> Option<f64> {
        self.values.num("Left")
    }
    pub fn bottom(&self) -> Option<f64> {
        self.values.num("Bottom")
    }
    pub fn right(&self) -> Option<f64> {
        self.values.num("Right")
    }
    pub fn column_count(&self) -> Option<f64> {
        self.values.num("ColumnCount")
    }
    pub fn column_gutter(&self) -> Option<f64> {
        self.values.num("ColumnGutter")
    }

    /// Parse at most `limit` whitespace-separated `ColumnsPositions` values; malformed or
    /// non-finite values invalidate the list.
    pub fn column_positions(&self, limit: usize) -> Option<Vec<f64>> {
        let source = self.values.get("ColumnsPositions")?;
        let values: Option<Vec<f64>> = source.split_whitespace().take(limit).map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite())).collect();
        values.filter(|values| !values.is_empty())
    }
}

/// A child of a page, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum PageChild {
    Guide(Guide),
    /// `MarginPreference`, `GridDataInformation`, … as written.
    Other(El),
}

impl Page {
    pub fn from_el(el: &El) -> Page {
        let children = others(el)
            .map(|c| {
                if c.local() == "Guide" {
                    PageChild::Guide(Guide { values: Values::of(c), rest: others(c).map(tidy).collect() })
                } else {
                    PageChild::Other(tidy(c))
                }
            })
            .collect();
        Page { values: Values::of(el), children }
    }

    pub fn to_el(&self) -> El {
        with_children(
            self.values.element("Page"),
            self.children.iter().map(|c| match c {
                PageChild::Guide(g) => with_children(g.values.element("Guide"), g.rest.iter().cloned()),
                PageChild::Other(e) => e.clone(),
            }),
        )
    }

    /// `GeometricBounds`: `top left bottom right` in the page's inner coordinates.
    pub fn geometric_bounds(&self) -> Option<[f64; 4]> {
        self.values.get("GeometricBounds").and_then(bounds)
    }

    /// `AppliedMaster`: the `Self` of the parent spread, or `None` for none (`n`).
    pub fn applied_master(&self) -> Option<&str> {
        self.values.get("AppliedMaster").filter(|m| *m != "n")
    }

    pub fn self_id(&self) -> Option<&str> {
        self.values.self_id()
    }

    /// `ItemTransform`; the identity when absent or malformed.
    pub fn item_transform(&self) -> Matrix {
        self.values.item_transform()
    }

    /// `LayoutRule`, defaulting to `Off`.
    pub fn layout_rule(&self) -> &str {
        self.values.get("LayoutRule").unwrap_or("Off")
    }

    /// `OverrideList`: the `Self`s of the parent items overridden on this page.
    pub fn override_list(&self) -> Vec<&str> {
        self.values.get("OverrideList").map(|l| l.split_whitespace().collect()).unwrap_or_default()
    }

    /// `MarginPreference`: margins, columns and their positions.
    pub fn margin_preference(&self) -> Option<Values> {
        self.children.iter().find_map(|c| match c {
            PageChild::Other(e) if e.local() == "MarginPreference" => Some(Values::of(e)),
            _ => None,
        })
    }

    pub fn margin_settings(&self) -> Option<MarginPreference> {
        self.margin_preference().map(|values| MarginPreference { values })
    }

    pub fn guides(&self) -> impl DoubleEndedIterator<Item = &Guide> {
        self.children.iter().filter_map(|c| if let PageChild::Guide(g) = c { Some(g) } else { None })
    }
}

/// A ruler guide.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guide {
    pub values: Values,
    /// Its children other than `<Properties>`, as written.
    pub rest: Vec<El>,
}

impl Guide {
    /// `Orientation` is `Horizontal`; a guide is vertical otherwise.
    pub fn horizontal(&self) -> bool {
        self.values.get("Orientation") == Some("Horizontal")
    }

    /// `Location`: from the ruler origin, along the axis across the guide.
    pub fn location(&self) -> Option<f64> {
        self.values.num("Location")
    }

    /// `FitToPage`: across the page only (else across the spread). True when absent.
    pub fn fit_to_page(&self) -> bool {
        self.values.boolean("FitToPage") != Some(false)
    }

    /// `GuideType` is `Liquid`.
    pub fn liquid(&self) -> bool {
        self.values.get("GuideType") == Some("Liquid")
    }

    /// `Locked` is true.
    pub fn locked(&self) -> bool {
        self.values.boolean("Locked") == Some(true)
    }

    /// `ItemLayer`: the identifier of the layer the guide belongs to.
    pub fn item_layer(&self) -> Option<&str> {
        self.values.item_layer()
    }
}

/// What a page item is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    TextFrame,
    Rectangle,
    Oval,
    Polygon,
    GraphicLine,
    Group,
    Button,
    EPSText,
    /// Another page item element (`MultiStateObject`, form fields, `HtmlItem`), by name; its
    /// children are kept as written.
    Other(String),
}

impl ItemKind {
    /// The kind of a page item element, or `None` for an element that is no page item.
    pub fn of(name: &str) -> Option<ItemKind> {
        Some(match name {
            "TextFrame" => ItemKind::TextFrame,
            "Rectangle" => ItemKind::Rectangle,
            "Oval" => ItemKind::Oval,
            "Polygon" => ItemKind::Polygon,
            "GraphicLine" => ItemKind::GraphicLine,
            "Group" => ItemKind::Group,
            "Button" => ItemKind::Button,
            "EPSText" => ItemKind::EPSText,
            "MultiStateObject" | "HtmlItem" | "CheckBox" | "ComboBox" | "ListBox" | "RadioButton" | "TextBox" | "SignatureField" => {
                ItemKind::Other(name.to_string())
            }
            _ => return None,
        })
    }

    pub fn name(&self) -> &str {
        match self {
            ItemKind::TextFrame => "TextFrame",
            ItemKind::Rectangle => "Rectangle",
            ItemKind::Oval => "Oval",
            ItemKind::Polygon => "Polygon",
            ItemKind::GraphicLine => "GraphicLine",
            ItemKind::Group => "Group",
            ItemKind::Button => "Button",
            ItemKind::EPSText => "EPSText",
            ItemKind::Other(n) => n,
        }
    }
}

/// A page item.
#[derive(Clone, Debug, PartialEq)]
pub struct PageItem {
    pub kind: ItemKind,
    pub values: Values,
    pub children: Vec<ItemChild>,
}

/// A child of a page item, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemChild {
    /// A nested page item: a group's members, a frame's content.
    Item(PageItem),
    /// A placed graphic.
    Graphic(Graphic),
    /// Anything else (`TextFramePreference`, `TextWrapPreference`, `TextPath`, …), as written.
    Other(El),
}

impl PageItem {
    /// The page item `el`; `None` when it is no page item element.
    pub fn from_el(el: &El) -> Option<PageItem> {
        let kind = ItemKind::of(el.local())?;
        let children = if let ItemKind::Other(_) = kind {
            others(el).map(tidy).map(ItemChild::Other).collect()
        } else {
            others(el)
                .map(|c| {
                    if let Some(g) = Graphic::from_el(c) {
                        ItemChild::Graphic(g)
                    } else if let Some(i) = PageItem::from_el(c) {
                        ItemChild::Item(i)
                    } else {
                        ItemChild::Other(tidy(c))
                    }
                })
                .collect()
        };
        Some(PageItem { kind, values: Values::of(el), children })
    }

    pub fn to_el(&self) -> El {
        with_children(
            self.values.element(self.kind.name()),
            self.children.iter().map(|c| match c {
                ItemChild::Item(i) => i.to_el(),
                ItemChild::Graphic(g) => g.to_el(),
                ItemChild::Other(e) => e.clone(),
            }),
        )
    }

    /// The outline (`Properties/PathGeometry`) in inner coordinates; empty when it has none.
    pub fn geometry(&self) -> PathGeometry {
        self.values.prop_el("PathGeometry").map(PathGeometry::from_el).unwrap_or_default()
    }

    pub fn self_id(&self) -> Option<&str> {
        self.values.self_id()
    }

    pub fn item_layer(&self) -> Option<&str> {
        self.values.item_layer()
    }

    pub fn item_transform(&self) -> Matrix {
        self.values.item_transform()
    }

    pub fn name(&self) -> Option<&str> {
        self.values.name()
    }

    pub fn locked(&self) -> bool {
        self.values.locked()
    }

    pub fn hidden(&self) -> bool {
        !self.values.visible()
    }

    pub fn nonprinting(&self) -> bool {
        self.values.boolean("Nonprinting") == Some(true)
    }

    pub fn gradient_fill_angle(&self) -> Option<f64> {
        self.values.num("GradientFillAngle")
    }
    pub fn gradient_fill_start(&self) -> Option<[f64; 2]> {
        self.values.pair("GradientFillStart")
    }
    pub fn gradient_fill_length(&self) -> Option<f64> {
        self.values.num("GradientFillLength")
    }
    pub fn gradient_stroke_angle(&self) -> Option<f64> {
        self.values.num("GradientStrokeAngle")
    }
    pub fn gradient_stroke_start(&self) -> Option<[f64; 2]> {
        self.values.pair("GradientStrokeStart")
    }
    pub fn gradient_stroke_length(&self) -> Option<f64> {
        self.values.num("GradientStrokeLength")
    }

    pub fn text_path(&self) -> Option<TextPath> {
        self.other("TextPath").map(|el| TextPath { values: Values::of(el) })
    }

    pub fn text_wrap(&self) -> Option<TextWrapPreference> {
        self.other("TextWrapPreference").map(TextWrapPreference::from_el)
    }

    /// Replaces the outline.
    pub fn set_geometry(&mut self, g: &PathGeometry) {
        self.values.set_prop_el("PathGeometry", Some(g.to_el()));
    }

    /// The nested page items, back to front.
    pub fn items(&self) -> impl DoubleEndedIterator<Item = &PageItem> {
        self.children.iter().filter_map(|c| if let ItemChild::Item(i) = c { Some(i) } else { None })
    }

    pub fn graphics(&self) -> impl DoubleEndedIterator<Item = &Graphic> {
        self.children.iter().filter_map(|c| if let ItemChild::Graphic(g) = c { Some(g) } else { None })
    }

    /// The child element `name` other than a page item or graphic (`TextFramePreference`,
    /// `TextWrapPreference`, `FrameFittingOption`, …).
    pub fn other(&self, name: &str) -> Option<&El> {
        self.children.iter().find_map(|c| match c {
            ItemChild::Other(e) if e.local() == name => Some(e),
            _ => None,
        })
    }

    /// `ParentStory` of a text frame: the `Self` of its story.
    pub fn parent_story(&self) -> Option<&str> {
        self.values.get("ParentStory").filter(|s| *s != "n")
    }

    /// `PreviousTextFrame` of a threaded text frame.
    pub fn previous_text_frame(&self) -> Option<&str> {
        self.values.get("PreviousTextFrame").filter(|s| *s != "n")
    }

    /// `NextTextFrame` of a threaded text frame.
    pub fn next_text_frame(&self) -> Option<&str> {
        self.values.get("NextTextFrame").filter(|s| *s != "n")
    }

    /// `AppliedObjectStyle`.
    pub fn applied_object_style(&self) -> Option<&str> {
        self.values.get("AppliedObjectStyle")
    }

    /// `OverriddenPageItem`: the `Self` of the parent item this item overrides.
    pub fn overridden_page_item(&self) -> Option<&str> {
        self.values.get("OverriddenPageItem").filter(|s| *s != "n")
    }
}

/// Parsed IDML `TextWrapPreference` values, including the nested offset and contour elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextWrapPreference {
    values: Values,
    contour_type: Option<String>,
    offsets: Option<[f64; 4]>,
}

impl TextWrapPreference {
    pub fn from_el(el: &El) -> Self {
        let contour_type = el.child(Local("ContourOption")).and_then(|c| Values::of(c).get("ContourType").map(str::to_string));
        let offsets = el.child(Local("Properties")).and_then(|p| p.child(Local("TextWrapOffset"))).map(|o| {
            let values = Values::of(o);
            [
                values.num("Top").unwrap_or(0.0),
                values.num("Left").unwrap_or(0.0),
                values.num("Bottom").unwrap_or(0.0),
                values.num("Right").unwrap_or(0.0),
            ]
        });
        Self { values: Values::of(el), contour_type, offsets }
    }

    pub fn mode(&self) -> Option<&str> {
        self.values.get("TextWrapMode")
    }
    pub fn inverse(&self) -> bool {
        self.values.boolean("Inverse") == Some(true)
    }
    pub fn side(&self) -> Option<&str> {
        self.values.get("TextWrapSide")
    }
    pub fn contour_type(&self) -> Option<&str> {
        self.contour_type.as_deref()
    }
    pub fn offsets(&self) -> Option<[f64; 4]> {
        self.offsets
    }
}

/// Type-on-a-path settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPath {
    pub values: Values,
}

impl TextPath {
    pub fn parent_story(&self) -> Option<&str> {
        self.values.get("ParentStory").filter(|s| *s != "n")
    }
    pub fn start_bracket(&self) -> Option<f64> {
        self.values.num("StartBracket")
    }
    pub fn end_bracket(&self) -> Option<f64> {
        self.values.num("EndBracket")
    }
    pub fn flipped(&self) -> bool {
        self.values.get("FlipPathEffect") == Some("Flipped")
    }
    pub fn alignment(&self) -> Option<&str> {
        self.values.get("TextAlignment")
    }
}

/// What a placed graphic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicKind {
    Image,
    EPS,
    PDF,
    ImportedPage,
    WMF,
    PICT,
    SVG,
}

impl GraphicKind {
    pub fn of(name: &str) -> Option<GraphicKind> {
        Some(match name {
            "Image" => GraphicKind::Image,
            "EPS" => GraphicKind::EPS,
            "PDF" => GraphicKind::PDF,
            "ImportedPage" => GraphicKind::ImportedPage,
            "WMF" => GraphicKind::WMF,
            "PICT" => GraphicKind::PICT,
            "SVG" => GraphicKind::SVG,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            GraphicKind::Image => "Image",
            GraphicKind::EPS => "EPS",
            GraphicKind::PDF => "PDF",
            GraphicKind::ImportedPage => "ImportedPage",
            GraphicKind::WMF => "WMF",
            GraphicKind::PICT => "PICT",
            GraphicKind::SVG => "SVG",
        }
    }
}

/// A graphic placed in a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Graphic {
    pub kind: GraphicKind,
    pub values: Values,
    pub children: Vec<GraphicChild>,
}

/// A child of a graphic, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicChild {
    Link(Link),
    /// `ClippingPathSettings`, `GraphicLayerOption`, `PDFAttribute`, … as written.
    Other(El),
}

/// Where a graphic's file is (`Link`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub values: Values,
    pub rest: Vec<El>,
}

impl Link {
    /// `LinkResourceURI` (`file:…`; see [`crate::value::uri_to_path`]).
    pub fn uri(&self) -> Option<&str> {
        self.values.get("LinkResourceURI")
    }

    /// `LinkResourceFormat` (`$ID/JPEG`, `$ID/Portable Document Format (PDF)`, …).
    pub fn format(&self) -> Option<&str> {
        self.values.get("LinkResourceFormat")
    }
}

impl Graphic {
    pub fn from_el(el: &El) -> Option<Graphic> {
        let kind = GraphicKind::of(el.local())?;
        let children = others(el)
            .map(|c| {
                if c.local() == "Link" {
                    GraphicChild::Link(Link { values: Values::of(c), rest: others(c).map(tidy).collect() })
                } else {
                    GraphicChild::Other(tidy(c))
                }
            })
            .collect();
        Some(Graphic { kind, values: Values::of(el), children })
    }

    pub fn to_el(&self) -> El {
        with_children(
            self.values.element(self.kind.name()),
            self.children.iter().map(|c| match c {
                GraphicChild::Link(l) => with_children(l.values.element("Link"), l.rest.iter().cloned()),
                GraphicChild::Other(e) => e.clone(),
            }),
        )
    }

    pub fn link(&self) -> Option<&Link> {
        self.children.iter().find_map(|c| if let GraphicChild::Link(l) = c { Some(l) } else { None })
    }

    /// `Properties/GraphicBounds`: `top left bottom right` in the graphic's inner coordinates.
    pub fn graphic_bounds(&self) -> Option<[f64; 4]> {
        let b = self.values.prop_el("GraphicBounds")?;
        let n = |k: &str| b.attr(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
        Some([n("Top")?, n("Left")?, n("Bottom")?, n("Right")?])
    }

    /// The embedded file (`Properties/Contents`, base64), decoded; `None` for a graphic that
    /// is only linked.
    pub fn contents(&self) -> Option<Vec<u8>> {
        self.values.prop_el("Contents").map(|c| base64_decode(&c.deep_text())).filter(|d| !d.is_empty())
    }
}

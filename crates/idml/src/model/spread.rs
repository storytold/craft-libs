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

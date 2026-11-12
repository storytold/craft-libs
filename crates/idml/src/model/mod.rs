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

mod geometry;
mod resources;
mod spread;
mod story;

pub use geometry::{MAX_PATH_POINTS, PathGeometry, PathPoint, SubPath};
pub use resources::{
    ColorGroup, ColorSpace, GradientStop, StrokeStyle, StrokeStyleKind, Style, StyleGroup, StyleKind, StyleNode, Swatch, SwatchKind, color_groups,
    named, preference, stroke_styles, style_groups, swatches,
};
pub use spread::{Graphic, GraphicChild, GraphicKind, Guide, ItemChild, ItemKind, Link, Page, PageChild, PageItem, Spread, SpreadChild, spreads};
pub use story::{Ace, CONTAINERS, Cell, Container, ContentPart, Range, Story, Table, TableChild, TextNode, stories};

use craft_xml::{El, Local, Node};

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

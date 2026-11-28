//! Styles (ODF 1.3 Part 3, section 16): font declarations, default styles, common and automatic
//! styles with their parents, page layouts and master pages, read from the element trees of
//! `styles.xml` and `content.xml`. What every OpenDocument type shares; what a property means
//! for a cell, a paragraph or a shape is the caller's.
//!
//! The trees are named with ODF's usual prefixes (`style:style`, `fo:color`), whatever the file
//! declared: read them with [`NAMESPACES`](crate::NAMESPACES) as canonical prefixes.
//!
//! A style's [`Properties`] are its property elements (`style:text-properties`,
//! `style:table-cell-properties`, …) by name. [`Styles::resolve`] merges the family's default
//! style, the style's ancestors and the style, nearest last, attribute by attribute, as §16.2
//! inherits them; a shorthand (`fo:border`, `fo:padding`, `fo:margin`, `style:border-line-width`)
//! replaces the inherited values of the sides it stands for.

use std::collections::HashMap;

use craft_xml::{El, Node};

use crate::number::DataStyles;

/// Deepest chain of parents followed; a deeper or circular chain stops there.
const MAX_DEPTH: usize = 64;

/// A shorthand and the attributes it sets in one go.
const SHORTHANDS: &[(&str, &[&str])] = &[
    ("fo:border", &["fo:border-top", "fo:border-bottom", "fo:border-left", "fo:border-right"]),
    ("fo:padding", &["fo:padding-top", "fo:padding-bottom", "fo:padding-left", "fo:padding-right"]),
    ("fo:margin", &["fo:margin-top", "fo:margin-bottom", "fo:margin-left", "fo:margin-right"]),
    (
        "style:border-line-width",
        &["style:border-line-width-top", "style:border-line-width-bottom", "style:border-line-width-left", "style:border-line-width-right"],
    ),
];

/// A `style:font-face` (§16.23).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFace {
    /// `style:name`, what `style:font-name` refers to.
    pub name: String,
    /// `svg:font-family`, quotes removed; the name when it states none.
    pub family: String,
    /// `style:font-family-generic`: `roman`, `swiss`, `modern`, `decorative`, `script`, `system`.
    pub generic: Option<String>,
    /// `style:font-pitch`: `fixed` or `variable`.
    pub pitch: Option<String>,
    /// `style:font-charset`.
    pub charset: Option<String>,
}

/// A style's property elements by name, each with its attributes and children as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties(Vec<El>);

impl Properties {
    fn of(style: &El) -> Properties {
        Properties(style.elements().filter(|e| e.name.ends_with("-properties")).cloned().collect())
    }

    /// The property element `group` (`style:text-properties`, …).
    pub fn group(&self, group: &str) -> Option<&El> {
        self.0.iter().find(|e| e.name == group)
    }

    /// An attribute of the property element `group`.
    pub fn get(&self, group: &str, attr: &str) -> Option<&str> {
        self.group(group).and_then(|g| g.attr(attr))
    }

    /// The property elements, in the order they were first stated.
    pub fn groups(&self) -> &[El] {
        &self.0
    }

    /// `over` on top of these: attribute by attribute, a shorthand replacing the sides it sets,
    /// and the children of `over`'s element replacing these when it has any.
    fn merge(&mut self, over: &Properties) {
        for o in &over.0 {
            let Some(g) = self.0.iter_mut().find(|g| g.name == o.name) else {
                self.0.push(o.clone());
                continue;
            };
            for (k, v) in &o.attrs {
                if let Some((_, sides)) = SHORTHANDS.iter().find(|(s, _)| s == k) {
                    g.attrs.retain(|(a, _)| !sides.contains(&a.as_str()));
                }
                g.set_attr(k, v);
            }
            if o.children.iter().any(|n| matches!(n, Node::El(_))) {
                g.children.clone_from(&o.children);
            }
        }
    }
}

/// A `style:map` (§16.3): the style to apply when a condition holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Map {
    /// `style:condition` as written (`cell-content()>0`, `value()>=0`).
    pub condition: String,
    /// `style:apply-style-name`.
    pub apply: String,
    /// `style:base-cell-address`.
    pub base_cell: Option<String>,
}

/// A `style:style` or `style:default-style` (§16.2, §16.4), or a `style:page-layout`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// `style:name`; empty for a default style.
    pub name: String,
    /// `style:display-name`.
    pub display_name: Option<String>,
    /// `style:family`: `table-cell`, `table-row`, `paragraph`, `text`, `graphic`, …
    pub family: String,
    /// `style:parent-style-name`.
    pub parent: Option<String>,
    /// `style:data-style-name`.
    pub data_style: Option<String>,
    /// `style:master-page-name`.
    pub master_page: Option<String>,
    /// `style:map` conditions, in order.
    pub maps: Vec<Map>,
    /// The property elements this style states itself.
    pub properties: Properties,
    /// The element's attributes as written.
    pub attrs: Vec<(String, String)>,
}

impl Style {
    fn of(e: &El) -> Style {
        let attr = |n: &str| e.attr(n).map(str::to_string);
        Style {
            name: attr("style:name").unwrap_or_default(),
            display_name: attr("style:display-name"),
            family: attr("style:family").unwrap_or_default(),
            parent: attr("style:parent-style-name"),
            data_style: attr("style:data-style-name"),
            master_page: attr("style:master-page-name"),
            maps: e
                .children("style:map")
                .filter_map(|m| {
                    Some(Map {
                        condition: m.attr("style:condition")?.to_string(),
                        apply: m.attr("style:apply-style-name")?.to_string(),
                        base_cell: m.attr("style:base-cell-address").map(str::to_string),
                    })
                })
                .collect(),
            properties: Properties::of(e),
            attrs: e.attrs.clone(),
        }
    }
}

/// A `style:page-layout` (§16.5): the page's properties and its header's and footer's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLayout {
    pub name: String,
    /// `style:page-layout-properties`.
    pub page: Option<El>,
    /// `style:header-style`'s `style:header-footer-properties`.
    pub header: Option<El>,
    /// `style:footer-style`'s `style:header-footer-properties`.
    pub footer: Option<El>,
}

/// The styles of a document, or of one of its parts. See [`Styles::read`].
#[derive(Clone, Debug, Default)]
pub struct Styles {
    fonts: HashMap<String, FontFace>,
    defaults: HashMap<String, Style>,
    common: HashMap<(String, String), Style>,
    /// Common styles by display name, for files that refer to them so.
    by_display: HashMap<(String, String), String>,
    automatic: HashMap<(String, String), Style>,
    page_layouts: HashMap<String, PageLayout>,
    master_pages: HashMap<String, El>,
    data: DataStyles,
}

impl Styles {
    /// The styles `content.xml` refers to: the common styles, default styles, page layouts,
    /// master pages and fonts of `styles` (`office:document-styles`), and the fonts and
    /// automatic styles of `content` (`office:document-content`, or only those two elements
    /// under any root). For what `styles.xml` itself refers to (a master page's header), pass
    /// it as both.
    pub fn read(styles: Option<&El>, content: Option<&El>) -> Styles {
        let mut s = Styles::default();
        for part in [styles, content].into_iter().flatten() {
            if let Some(f) = part.child("office:font-face-decls") {
                s.add_fonts(f);
            }
        }
        if let Some(part) = styles {
            if let Some(common) = part.child("office:styles") {
                s.add_common(common);
            }
            if let Some(m) = part.child("office:master-styles") {
                s.add_master_pages(m);
            }
            // Page layouts are automatic styles of styles.xml whichever part refers to them.
            if let Some(a) = part.child("office:automatic-styles") {
                s.add_page_layouts(a);
                s.data.add(a);
            }
        }
        if let Some(a) = content.and_then(|c| c.child("office:automatic-styles")) {
            s.add_automatic(a);
        }
        s
    }

    /// Adds the `style:font-face`s of an `office:font-face-decls`.
    pub fn add_fonts(&mut self, decls: &El) {
        for f in decls.children("style:font-face") {
            let Some(name) = f.attr("style:name") else { continue };
            let family = f.attr("svg:font-family").unwrap_or(name).trim().trim_matches(|c| c == '\'' || c == '"').to_string();
            let attr = |n: &str| f.attr(n).map(str::to_string);
            let face = FontFace {
                name: name.to_string(),
                family,
                generic: attr("style:font-family-generic"),
                pitch: attr("style:font-pitch"),
                charset: attr("style:font-charset"),
            };
            self.fonts.insert(name.to_string(), face);
        }
    }

    /// Adds the default and common styles, and the data styles, of an `office:styles`.
    pub fn add_common(&mut self, list: &El) {
        self.data.add(list);
        for e in list.elements() {
            if e.name == "style:default-style" {
                let s = Style::of(e);
                self.defaults.insert(s.family.clone(), s);
            } else if e.name == "style:style" {
                let s = Style::of(e);
                if let Some(d) = &s.display_name {
                    self.by_display.insert((s.family.clone(), d.clone()), s.name.clone());
                }
                self.common.insert((s.family.clone(), s.name.clone()), s);
            }
        }
    }

    /// Adds the automatic styles, and the data styles, of an `office:automatic-styles`.
    pub fn add_automatic(&mut self, list: &El) {
        self.data.add(list);
        for e in list.children("style:style") {
            let s = Style::of(e);
            self.automatic.insert((s.family.clone(), s.name.clone()), s);
        }
        self.add_page_layouts(list);
    }

    fn add_page_layouts(&mut self, list: &El) {
        for e in list.children("style:page-layout") {
            let Some(name) = e.attr("style:name") else { continue };
            let hf = |n: &str| e.child(n).and_then(|h| h.child("style:header-footer-properties")).cloned();
            let layout = PageLayout {
                name: name.to_string(),
                page: e.child("style:page-layout-properties").cloned(),
                header: hf("style:header-style"),
                footer: hf("style:footer-style"),
            };
            self.page_layouts.insert(name.to_string(), layout);
        }
    }

    /// Adds the `style:master-page`s of an `office:master-styles`.
    pub fn add_master_pages(&mut self, list: &El) {
        for e in list.children("style:master-page") {
            if let Some(name) = e.attr("style:name") {
                self.master_pages.insert(name.to_string(), e.clone());
            }
        }
    }

    /// The font declared as `name`.
    pub fn font(&self, name: &str) -> Option<&FontFace> {
        self.fonts.get(name)
    }

    /// The style `name` of `family`: an automatic one, else a common one by name, else a common
    /// one by display name.
    pub fn style(&self, family: &str, name: &str) -> Option<&Style> {
        let key = (family.to_string(), name.to_string());
        self.automatic
            .get(&key)
            .or_else(|| self.common.get(&key))
            .or_else(|| self.by_display.get(&key).and_then(|n| self.common.get(&(key.0.clone(), n.clone()))))
    }

    /// The common style `name` of `family` (a parent is always one).
    fn common(&self, family: &str, name: &str) -> Option<&Style> {
        let key = (family.to_string(), name.to_string());
        self.common.get(&key).or_else(|| self.by_display.get(&key).and_then(|n| self.common.get(&(key.0.clone(), n.clone()))))
    }

    /// The default style of `family`.
    pub fn default_style(&self, family: &str) -> Option<&Style> {
        self.defaults.get(family)
    }

    /// The style `name` of `family` and its ancestors, nearest first. A parent that is not there,
    /// a cycle, or a chain past 64 styles ends it.
    pub fn chain(&self, family: &str, name: &str) -> Vec<&Style> {
        let mut out: Vec<&Style> = Vec::new();
        let mut next = self.style(family, name);
        while let Some(s) = next {
            if out.len() >= MAX_DEPTH || out.iter().any(|o| std::ptr::eq(*o, s)) {
                break;
            }
            out.push(s);
            next = s.parent.as_deref().and_then(|p| self.common(family, p));
        }
        out
    }

    /// Everything the style `name` of `family` shows: the family's default style, then its
    /// ancestors from the farthest, then the style. `None` gives the default style's alone.
    pub fn resolve(&self, family: &str, name: Option<&str>) -> Properties {
        let mut props = self.default_style(family).map(|d| d.properties.clone()).unwrap_or_default();
        if let Some(name) = name {
            for s in self.chain(family, name).into_iter().rev() {
                props.merge(&s.properties);
            }
        }
        props
    }

    /// The page layout `name`.
    pub fn page_layout(&self, name: &str) -> Option<&PageLayout> {
        self.page_layouts.get(name)
    }

    /// The master page `name` (`style:master-page`), with its headers and footers as written.
    pub fn master_page(&self, name: &str) -> Option<&El> {
        self.master_pages.get(name)
    }

    /// The data styles (number formats) of the parts read.
    pub fn data_styles(&self) -> &DataStyles {
        &self.data
    }
}

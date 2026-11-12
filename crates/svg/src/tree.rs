//! The full SVG parser: usvg, behind one bounded entry point.
//!
//! usvg (re-exported as [`usvg`]) turns an SVG into a tree with styles resolved, `use` expanded,
//! text laid out and units converted. It is the parser every Craft app that reads SVG in full
//! shares, at the one version this crate pins; apps take it from here and not as a dependency of
//! their own, so they also share the checks it needs. usvg trusts its input more than a file
//! from anywhere deserves: it unpacks SVGZ with no limit, expands entities with no total limit,
//! and converts the tree by recursion, a few stack frames an element, so a small hostile file
//! can exhaust memory or overflow the stack (an abort, not an error).
//!
//! [`parse`] is the entry point: the bytes of an SVG or SVGZ file and [`Limits`] in, a
//! [`usvg::Tree`] out. It runs, in order: [`crate::svgz::text`] (bounded unpacking, encodings),
//! [`crate::guard::entities_expand_past`], [`crate::guard::too_wide`] (attributes an element),
//! [`crate::guard::prescan`] (nesting in the text),
//! [`height`] (nesting with references followed, as usvg walks them), then usvg.
//! [`image_resolver`] bounds the SVG documents embedded in `<image>` data URLs the same way.
//! [`draw`] bounds what drawing a tree with resvg allocates.
//!
//! Stack: usvg uses about 15 kB of stack for each level of nesting (measured natively by
//! VectorCraft). [`Limits::depth`] levels, times one more than [`Limits::embedded_depth`], must
//! fit the stack of the thread that calls [`parse`]: the default of 256 needs a thread with
//! 16 MB or so, and a 1 MB stack takes a depth of about 34.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use usvg;

use crate::paths::{MAX_CSS_SELECTOR_CHECKS, StyleMap, computed_style, parse_stylesheets};
use crate::{guard, svgz};

pub mod draw;

/// What one import may use. The defaults suit a desktop app that parses on a thread with a
/// large stack (see the module text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of the document, unpacked if it is SVGZ.
    pub bytes: usize,
    /// How deep elements may nest, references followed (the root alone is 1).
    pub depth: usize,
    /// Bytes the document type's entity references may expand to in all.
    pub entity_bytes: usize,
    /// Refuse a document type that declares an entity holding markup, instead of counting what
    /// it expands to ([`guard::entity_holds_markup`]).
    pub refuse_entity_markup: bool,
    /// Attributes one element may have ([`guard::too_wide`]).
    pub attributes: usize,
    /// The sum over all elements of their attribute count squared ([`guard::too_wide`]).
    pub attribute_work: usize,
    /// Nodes the XML may hold once entities are expanded.
    pub xml_nodes: u32,
    /// Bytes of all SVG documents embedded in `<image>` data URLs, unpacked, for
    /// [`image_resolver`].
    pub embedded_bytes: usize,
    /// SVG documents embedded inside one another, for [`image_resolver`].
    pub embedded_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            bytes: 64 << 20,
            depth: 256,
            entity_bytes: guard::MAX_ENTITY_EXPANSION,
            refuse_entity_markup: false,
            attributes: guard::MAX_ATTRIBUTES,
            attribute_work: guard::MAX_ATTRIBUTE_WORK,
            xml_nodes: 4_000_000,
            embedded_bytes: 32 << 20,
            embedded_depth: 4,
        }
    }
}

/// Why a file was not parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Too large, a damaged or nested SVGZ, or text in no encoding SVG files use.
    #[error(transparent)]
    Text(#[from] svgz::Error),
    #[error("the SVG's entities expand to too much text")]
    Entities,
    #[error("the SVG's document type declares an entity holding markup, which isn't supported")]
    EntityMarkup,
    /// Elements nest deeper than this many levels, references followed.
    #[error("the SVG's elements are nested more than {0} levels deep")]
    TooDeep(usize),
    /// An element has more than this many attributes, or all elements together too many.
    #[error("an element of the SVG has more than {0} attributes, or its elements have too many in all")]
    TooWide(usize),
    /// A style sheet that names other elements (`url(#…)`) and that this crate cannot read as
    /// usvg will, so the nesting it makes cannot be measured.
    #[error("the SVG's style sheet can't be checked: {0}")]
    Stylesheet(String),
    #[error("invalid SVG: {0}")]
    Xml(String),
    /// usvg's own error.
    #[error("invalid SVG: {0}")]
    Parse(String),
}

/// The SVG text of the bytes of an SVG or SVGZ file, checked against `limits`: what [`parse`]
/// hands usvg. For a caller that edits the text before it parses ([`parse_text`]).
pub fn text<'a>(bytes: &'a [u8], limits: &Limits) -> Result<Cow<'a, str>, Error> {
    let text = svgz::text(bytes, limits.bytes)?;
    checked(&text, limits)?;
    Ok(text)
}

/// The tree of an SVG or SVGZ file, within `limits`. `options` are usvg's: fonts, the default
/// size, and the image resolver ([`image_resolver`] unless the caller reads linked files).
pub fn parse(bytes: &[u8], limits: &Limits, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
    parse_text(&svgz::text(bytes, limits.bytes)?, limits, options)
}

/// [`parse`] for SVG text the caller already holds.
pub fn parse_text(svg: &str, limits: &Limits, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
    let xml = checked(svg, limits)?;
    usvg::Tree::from_xmltree(&xml, options).map_err(|e| Error::Parse(e.to_string()))
}

/// Every check on SVG text, cheapest first; the XML it parses to on the way.
fn checked<'s>(svg: &'s str, limits: &Limits) -> Result<roxmltree::Document<'s>, Error> {
    if svg.len() > limits.bytes {
        return Err(Error::Text(svgz::Error::TooLarge(limits.bytes)));
    }
    if limits.refuse_entity_markup && guard::entity_holds_markup(svg) {
        return Err(Error::EntityMarkup);
    }
    if guard::entities_expand_past(svg, limits.entity_bytes) {
        return Err(Error::Entities);
    }
    if guard::too_wide(svg, limits.attributes, limits.attribute_work) {
        return Err(Error::TooWide(limits.attributes));
    }
    if guard::deeper_than(svg, limits.depth) {
        return Err(Error::TooDeep(limits.depth));
    }
    let options = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: limits.xml_nodes, ..Default::default() };
    let xml = roxmltree::Document::parse_with_options(svg, options).map_err(|e| Error::Xml(e.to_string()))?;
    if height(&xml, limits.depth)?.is_none() {
        return Err(Error::TooDeep(limits.depth));
    }
    Ok(xml)
}

/// Properties whose value an element's descendants use too: the paint server they name is
/// converted at each descendant, under everything between.
const INHERITED: [&str; 6] = ["fill", "stroke", "marker", "marker-start", "marker-mid", "marker-end"];

/// Properties that name other elements and that the cascade does not track by itself.
const MARKERS: [&str; 4] = ["marker", "marker-start", "marker-mid", "marker-end"];

/// The ids of the `url(#id)` references in a property value.
fn urls(value: &str) -> impl Iterator<Item = &str> {
    value.split("url(").skip(1).filter_map(|rest| {
        let id = rest.trim_start().trim_start_matches(['"', '\'']).strip_prefix('#')?;
        id.split([')', '"', '\'']).next().map(str::trim).filter(|id| !id.is_empty())
    })
}

type XNode<'a, 'i> = roxmltree::Node<'a, 'i>;

/// How deep the elements of `xml` nest as usvg walks them (the root alone is 1); `None` past
/// `most`. Without recursion.
///
/// What counts is the tree usvg converts, not the XML alone: a `<use>` holds a copy of what it
/// refers to, and a clip path, mask, pattern, marker or filter is converted where it is used
/// (`clip-path: url(#a)` on an element of a clip path makes a chain as deep as there are clip
/// paths). So each element's depth is the deepest of what it holds and what it refers to, by
/// attribute, `style` and style sheet. A loop of references is not deep: usvg leaves it out.
///
/// An error when a style sheet names elements and cannot be read here as usvg reads it (a
/// selector or at-rule this crate's cascade does not apply, a sheet for another medium, a sheet
/// past its size or work limits): the nesting such a sheet makes cannot be measured.
pub fn height(xml: &roxmltree::Document<'_>, most: usize) -> Result<Option<usize>, Error> {
    let elements = || xml.descendants().filter(|n| n.is_element());
    // The first element of an id, as usvg resolves references: a later one of the same id would
    // hide a deep target behind a shallow one.
    let mut ids: HashMap<&str, XNode> = HashMap::new();
    for n in elements() {
        if let Some(id) = n.attribute("id") {
            ids.entry(id).or_insert(n);
        }
    }
    let mut warnings = Vec::new();
    let rules =
        parse_stylesheets(xml.root_element(), &MARKERS, "", &mut warnings).map_err(|e| Error::Stylesheet(e.0.trim_start_matches(": ").to_string()));
    let sheets = || elements().filter(|n| n.tag_name().name() == "style");
    let names_elements = sheets().any(|n| n.text().is_some_and(|t| t.contains("url(")));
    let rules = match rules {
        Ok(rules) => rules,
        // A sheet that names no element adds no nesting, whatever else is in it.
        Err(_) if !names_elements => Vec::new(),
        Err(e) => return Err(e),
    };
    if names_elements {
        if let Some(w) = warnings.first() {
            return Err(Error::Stylesheet(w.clone()));
        }
        if sheets().any(|n| n.attribute("media").is_some_and(|m| !matches!(m.trim(), "" | "all" | "screen"))) {
            return Err(Error::Stylesheet("a style sheet for another medium names elements".into()));
        }
    }
    let nothing = StyleMap::new();
    let mut checks = 0usize;
    // The elements `n` refers to, each with whether `n`'s descendants use it too.
    let mut references = |n: XNode, out: &mut Vec<(roxmltree::NodeId, bool)>| -> Result<(), Error> {
        for a in n.attributes().filter(|a| a.name() == "href") {
            out.extend(a.value().trim().strip_prefix('#').and_then(|id| ids.get(id)).map(|t| (t.id(), false)));
        }
        for (name, value) in computed_style(n, &nothing, &rules, &MARKERS, &mut checks) {
            let inherited = INHERITED.contains(&name.as_str());
            out.extend(urls(&value).filter_map(|id| ids.get(id)).map(|t| (t.id(), inherited)));
        }
        if checks > MAX_CSS_SELECTOR_CHECKS {
            return Err(Error::Stylesheet("the style sheet is too large for this many elements".into()));
        }
        Ok(())
    };
    // How deep each element's own tree goes, itself included; 0 while it is being worked out: a
    // reference back to it then is a loop, which usvg leaves out.
    let mut heights: HashMap<roxmltree::NodeId, usize> = HashMap::new();
    enum Step {
        Enter(roxmltree::NodeId),
        Leave(roxmltree::NodeId, Vec<(roxmltree::NodeId, bool)>),
    }
    let mut stack = vec![Step::Enter(xml.root_element().id())];
    while let Some(step) = stack.pop() {
        match step {
            Step::Enter(id) => {
                let Some(n) = xml.get_node(id) else { continue };
                match heights.entry(id) {
                    Entry::Occupied(_) => continue,
                    Entry::Vacant(v) => v.insert(0),
                };
                let mut refs = vec![];
                references(n, &mut refs)?;
                let targets: Vec<roxmltree::NodeId> = refs.iter().map(|(t, _)| *t).collect();
                stack.push(Step::Leave(id, refs));
                stack.extend(n.children().filter(|c| c.is_element()).map(|c| c.id()).chain(targets).map(Step::Enter));
            }
            Step::Leave(id, refs) => {
                let Some(n) = xml.get_node(id) else { continue };
                let of = |id: roxmltree::NodeId| heights.get(&id).copied().unwrap_or(0);
                let held = n.children().filter(|c| c.is_element()).map(|c| of(c.id())).max().unwrap_or(0);
                let used = |inherited: bool| refs.iter().filter(|(_, i)| *i == inherited).map(|(t, _)| of(*t)).max().unwrap_or(0);
                let height = 1 + held.max(used(false)) + used(true);
                if height > most {
                    return Ok(None);
                }
                heights.insert(id, height);
            }
        }
    }
    Ok(Some(heights.get(&xml.root_element().id()).copied().unwrap_or(0)))
}

/// What the embedded SVGs of one import have used so far.
struct Used {
    bytes: AtomicUsize,
    depth: AtomicUsize,
}

fn raster(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG")
        || data.starts_with(&[0xff, 0xd8, 0xff])
        || data.starts_with(b"GIF8")
        || (data.len() > 12 && data.get(8..12) == Some(b"WEBP"))
}

/// An embedded SVG document, if the budgets allow it.
fn embedded(used: &Used, limits: &Limits, data: &[u8], options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
    if used.depth.load(Ordering::Relaxed) >= limits.embedded_depth {
        return None;
    }
    let left = limits.embedded_bytes.saturating_sub(used.bytes.load(Ordering::Relaxed));
    let text = match svgz::text(data, left.min(limits.bytes)) {
        Ok(text) => text,
        // What a refused document unpacked before it was refused was work too: it uses up what
        // is left, so the next image can't repeat it.
        Err(_) => {
            used.bytes.fetch_add(left, Ordering::Relaxed);
            return None;
        }
    };
    used.bytes.fetch_add(text.len(), Ordering::Relaxed);
    let xml = checked(&text, limits).ok()?;
    used.depth.fetch_add(1, Ordering::Relaxed);
    let tree = usvg::Tree::from_xmltree(&xml, options);
    used.depth.fetch_sub(1, Ordering::Relaxed);
    tree.ok().map(usvg::ImageKind::SVG)
}

/// An image resolver for `usvg::Options::image_href_resolver` that reads `<image>` data URLs
/// only: raster images as they are (PNG, JPEG, GIF, WebP), SVG documents (SVGZ too) through the
/// same checks as [`parse`] and within [`Limits::embedded_bytes`] in all and
/// [`Limits::embedded_depth`] inside one another. An image past a limit is left out. Linked
/// files are never read. One resolver is one import's budget: make a new one for each [`parse`].
pub fn image_resolver(limits: Limits) -> usvg::ImageHrefResolver<'static> {
    let used = Arc::new(Used { bytes: AtomicUsize::new(0), depth: AtomicUsize::new(0) });
    usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, options| match mime {
            "image/jpg" | "image/jpeg" => Some(usvg::ImageKind::JPEG(data)),
            "image/png" => Some(usvg::ImageKind::PNG(data)),
            "image/gif" => Some(usvg::ImageKind::GIF(data)),
            "image/webp" => Some(usvg::ImageKind::WEBP(data)),
            "image/svg+xml" => embedded(&used, &limits, &data, options),
            "text/plain" if raster(&data) => match data.first() {
                Some(0x89) => Some(usvg::ImageKind::PNG(data)),
                Some(0xff) => Some(usvg::ImageKind::JPEG(data)),
                Some(b'G') => Some(usvg::ImageKind::GIF(data)),
                _ => Some(usvg::ImageKind::WEBP(data)),
            },
            "text/plain" => embedded(&used, &limits, &data, options),
            _ => None,
        }),
        resolve_string: Box::new(|_, _| None),
    }
}

#[cfg(test)]
mod tests;

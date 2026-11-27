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
//! `image_resolver` (with the `tree` feature) bounds the SVG documents embedded in `<image>` data URLs the same way.
//! [`draw`] bounds what drawing a tree with resvg allocates.
//!
//! Stack: usvg uses about 15 kB of stack for each level of nesting (measured natively by
//! VectorCraft). [`Limits::tree_depth`] levels, times one more than [`Limits::embedded_depth`], must
//! fit the stack of the thread that calls [`parse`]: the default of 256 needs a thread with
//! 16 MB or so, and a 1 MB stack takes a depth of about 34.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
#[cfg(feature = "tree")]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(feature = "tree")]
use std::sync::{Arc, Mutex, PoisonError, Weak};

pub use usvg;

use crate::paths::{MAX_CSS_SELECTOR_CHECKS, StyleMap, computed_style, parse_stylesheets};
use crate::{guard, svgz};

pub mod document;
pub mod draw;
#[cfg(feature = "tree-render")]
mod image_href;
pub mod primitive;
#[cfg(feature = "tree-render")]
mod validated;
#[cfg(feature = "tree-render")]
pub use validated::ValidatedImageResolver;

/// What one import may use. The defaults suit a desktop app that parses on a thread with a
/// large stack (see the module text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of the document, unpacked if it is SVGZ.
    pub bytes: usize,
    /// How deep the XML elements may nest (the root alone is 1).
    pub depth: usize,
    /// Maximum depth usvg may create after following references such as `<use>`, patterns,
    /// clip paths, masks and markers (the root alone is 1). Choose this for the stack available
    /// to the parsing thread; usvg conversion recurses after this preflight.
    pub tree_depth: usize,
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
    /// Accept a document type declaration; `false` refuses any `<!DOCTYPE`.
    pub allow_dtd: bool,
    /// Bytes of all SVG documents embedded in `<image>` data URLs, unpacked, for
    /// `image_resolver`.
    pub embedded_bytes: usize,
    /// SVG documents embedded inside one another, for `image_resolver`.
    pub embedded_depth: usize,
    /// Cumulative raster decode pixels for a drawing. usvg sends each data-URL
    /// resolution separately, so repeated uses normally spend a separate charge.
    /// The legacy resolver can share a charge for the same live input allocation.
    pub embedded_raster_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            bytes: 64 << 20,
            depth: 256,
            tree_depth: 256,
            entity_bytes: guard::MAX_ENTITY_EXPANSION,
            refuse_entity_markup: false,
            attributes: guard::MAX_ATTRIBUTES,
            attribute_work: guard::MAX_ATTRIBUTE_WORK,
            xml_nodes: 4_000_000,
            allow_dtd: true,
            embedded_bytes: 32 << 20,
            embedded_depth: 4,
            embedded_raster_pixels: 24_000_000,
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
    /// The root viewport is missing or has dimensions this SVG helper cannot preserve.
    #[error("invalid SVG root viewport: {0}")]
    Viewport(String),
    /// The rewritten SVG would exceed the caller's output byte budget.
    #[error("rewritten SVG exceeds {0} bytes")]
    OutputTooLarge(usize),
}

/// The SVG text of the bytes of an SVG or SVGZ file, checked against `limits`: what [`parse`]
/// hands usvg. For a caller that edits the text before it parses ([`parse_text`]).
pub fn text<'a>(bytes: &'a [u8], limits: &Limits) -> Result<Cow<'a, str>, Error> {
    let text = svgz::text(bytes, limits.bytes)?;
    preflight(&text, limits)?;
    Ok(text)
}

/// The tree of an SVG or SVGZ file, within `limits`. `options` are usvg's: fonts, the default
/// size, and the image resolver (`image_resolver` unless the caller reads linked files).
pub fn parse(bytes: &[u8], limits: &Limits, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
    parse_text(&svgz::text(bytes, limits.bytes)?, limits, options)
}

/// [`parse`] for SVG text the caller already holds.
pub fn parse_text(svg: &str, limits: &Limits, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
    preflight(svg, limits)?.parse(options)
}

/// Read the root SVG's unitless width and height without constructing a rendering tree.
///
/// Lengths with units, percentages, missing dimensions, and non-SVG roots return `Ok(None)`;
/// malformed or over-limit XML returns the same bounded parsing error as [`parse_text`].
pub fn root_unitless_size(svg: &str, limits: &Limits) -> Result<Option<(f64, f64)>, Error> {
    let checked = preflight(svg, limits)?;
    let root = checked.xml.root_element();
    if root.tag_name().name() != "svg" {
        return Ok(None);
    }
    let Some((width, height)) =
        root.attribute("width").and_then(|value| value.parse::<f64>().ok()).zip(root.attribute("height").and_then(|value| value.parse::<f64>().ok()))
    else {
        return Ok(None);
    };
    Ok(Some((width, height)))
}

/// SVG text after shared byte, entity, attribute, XML and expansion-depth checks.
/// Keep it opaque so consumers use the same checked tree and don't reparse XML locally.
pub struct Preflight<'s> {
    #[cfg(feature = "tree-render")]
    limits: Limits,
    xml: roxmltree::Document<'s>,
    depth: usize,
    /// [`height`]: the nesting usvg converts, references followed.
    #[cfg_attr(not(feature = "tree"), expect(dead_code, reason = "only the embedded-image resolver reads it"))]
    height: usize,
}

impl Preflight<'_> {
    /// The deepest stack of open source elements, before references are followed; a
    /// self-closing leaf opens no level (`<svg><g><path/></g></svg>` is 2).
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The SVG elements in the document that the parsed tree leaves out, by name and count,
    /// sorted by name: animation, scripts, embedded HTML and media, and text when this build
    /// converts no text (neither `tree-text` nor `tree-render`). For an import's "not imported" notes.
    pub fn unsupported_elements(&self) -> Vec<(&'static str, usize)> {
        const SVG_NS: &str = "http://www.w3.org/2000/svg";
        let mut counts: std::collections::BTreeMap<&'static str, usize> = std::collections::BTreeMap::new();
        for node in self.xml.descendants().filter(roxmltree::Node::is_element) {
            if !matches!(node.tag_name().namespace(), None | Some(SVG_NS)) {
                continue;
            }
            if let Some(name) = UNSUPPORTED.iter().copied().find(|name| *name == node.tag_name().name()) {
                *counts.entry(name).or_default() += 1;
            }
        }
        counts.into_iter().collect()
    }

    /// Converts this already-checked XML into the configured usvg tree.
    pub fn parse(self, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
        usvg::Tree::from_xmltree(&self.xml, options).map_err(|e| Error::Parse(e.to_string()))
    }
}

/// Elements the parsed tree never holds: usvg reads static content only, and text needs fonts.
const UNSUPPORTED: &[&str] = &[
    "animate",
    "animateColor",
    "animateMotion",
    "animateTransform",
    "audio",
    "canvas",
    "discard",
    "foreignObject",
    "iframe",
    "script",
    "set",
    // usvg converts text when built with its `text` feature: `tree-text`, or `tree-render`
    // through resvg.
    #[cfg(not(any(feature = "tree-text", feature = "tree-render")))]
    "text",
    "video",
];

/// Checks already-decoded SVG text once, retaining the parsed XML for both depth policy and tree conversion.
pub fn preflight<'s>(svg: &'s str, limits: &Limits) -> Result<Preflight<'s>, Error> {
    #[cfg(test)]
    PREFLIGHT_PASSES.with(|count| count.set(count.get().saturating_add(1)));
    let preflight_limits = crate::preflight::Limits {
        bytes: limits.bytes,
        depth: limits.depth,
        entity_bytes: limits.entity_bytes,
        refuse_entity_markup: limits.refuse_entity_markup,
        attributes: limits.attributes,
        attribute_work: limits.attribute_work,
        xml_nodes: limits.xml_nodes,
        allow_dtd: limits.allow_dtd,
    };
    let (xml, _, depth) = crate::preflight::parse_text_with_depth(svg, &preflight_limits).map_err(|e| match e {
        crate::preflight::Error::Text(e) => Error::Text(e),
        crate::preflight::Error::Entities => Error::Entities,
        crate::preflight::Error::EntityMarkup => Error::EntityMarkup,
        crate::preflight::Error::TooDeep(depth) => Error::TooDeep(depth),
        crate::preflight::Error::TooWide(attributes) => Error::TooWide(attributes),
        crate::preflight::Error::Xml(msg) => Error::Xml(msg),
    })?;
    let Some(height) = height(&xml, limits.tree_depth)? else {
        return Err(Error::TooDeep(limits.tree_depth));
    };
    Ok(Preflight {
        #[cfg(feature = "tree-render")]
        limits: *limits,
        xml,
        depth,
        height,
    })
}

/// Dimensions of an SVG viewport in SVG user units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
}

/// Wraps a validated SVG root in a new viewport while preserving the original root as content.
/// The original root must have finite, positive pixel or point dimensions, and the wrapped
/// output must still pass [`preflight`] under `limits`.
pub fn rewrite_root_viewport(svg: &str, target: Viewport, limits: &Limits, output_limit: usize) -> Result<String, Error> {
    if !target.width.is_finite() || !target.height.is_finite() || target.width <= 0.0 || target.height <= 0.0 {
        return Err(Error::Viewport("target dimensions must be finite and positive".into()));
    }
    let checked = preflight(svg, limits)?;
    let root = checked.xml.root_element();
    if root.tag_name().name() != "svg" {
        return Err(Error::Viewport("root element is not svg".into()));
    }
    let root_dimension = |key: &str| {
        let value = root.attribute(key)?;
        let value = value.trim();
        let (number, scale) = match value.strip_suffix("pt") {
            Some(points) => (points, 4.0 / 3.0),
            None => (value.strip_suffix("px").unwrap_or(value), 1.0),
        };
        number.parse::<f64>().ok().map(|n| n * scale).filter(|n| n.is_finite() && *n > 0.0)
    };
    let (width, height) = root_dimension("width")
        .zip(root_dimension("height"))
        .ok_or_else(|| Error::Viewport("source has no finite pixel or point dimensions".into()))?;
    let range = root.range();
    let preamble = svg.get(..range.start).ok_or_else(|| Error::Viewport("invalid XML preamble range".into()))?;
    let inner = svg.get(range).ok_or_else(|| Error::Viewport("invalid root element range".into()))?;
    let prefix = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" preserveAspectRatio="none">"#,
        target.width, target.height, width, height
    );
    let len = preamble
        .len()
        .checked_add(prefix.len())
        .and_then(|n| n.checked_add(inner.len()))
        .and_then(|n| n.checked_add(6))
        .ok_or(Error::OutputTooLarge(output_limit))?;
    if len > output_limit {
        return Err(Error::OutputTooLarge(output_limit));
    }
    let mut output = String::new();
    output.try_reserve_exact(len).map_err(|_| Error::OutputTooLarge(output_limit))?;
    output.push_str(preamble);
    output.push_str(&prefix);
    output.push_str(inner);
    output.push_str("</svg>");
    // The wrapper adds a level, so a source at the depth limit can come out over it: check the
    // output under the limits `parse_text` applies rather than return text it would refuse.
    preflight(&output, limits)?;
    Ok(output)
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

#[cfg(feature = "tree")]
/// What the embedded SVGs of one import have used so far.
struct Used {
    bytes: AtomicUsize,
    depth: AtomicUsize,
    xml_depth: AtomicUsize,
    svg_count: AtomicUsize,
    svg_depth: AtomicUsize,
    raster_pixels: AtomicU64,
    raster_data: Mutex<HashMap<usize, Weak<Vec<u8>>>>,
    invalid_raster: AtomicUsize,
    raster_limit: AtomicUsize,
    invalid_svg: AtomicUsize,
    svg_limit: AtomicUsize,
    svg_resource_limit: AtomicUsize,
}

#[cfg(feature = "tree")]
/// What resolving one drawing's embedded images used or refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageUsage {
    /// Decoded raster pixel work reserved before decode. The legacy resolver
    /// shares a charge across repeated data; the validated context charges each
    /// attempted decode, including failures.
    pub raster_pixels: u64,
    /// Decoded SVG text work charged to embedded data URLs, including the
    /// remaining allowance consumed by a failed inflation attempt.
    pub embedded_svg_bytes: usize,
    /// Embedded SVG images parsed, including nested images.
    pub embedded_svg_count: usize,
    /// Deepest embedded SVG chain.
    pub embedded_svg_depth: usize,
    /// Raster image data that could not be probed or decoded by the usvg resolver.
    pub invalid_raster_images: usize,
    /// Raster images refused by their per-image or aggregate pixel limit.
    pub raster_limit_images: usize,
    /// Embedded SVG data that was invalid.
    pub invalid_svg_images: usize,
    /// Embedded SVG data refused by byte or nesting limits.
    pub svg_limit_images: usize,
    /// Subset of `svg_limit_images` refused by unpacked byte or embedded-document-chain limits.
    /// XML/tree-depth omissions and invalid or corrupt data do not set this counter. Consumers
    /// can reject resource overruns while still allowing an optional, over-deep child to be omitted.
    pub svg_resource_limit_images: usize,
}

#[cfg(feature = "tree")]
/// A data-URL resolver and the accounting for the one drawing it serves.
///
/// Keep this value while parsing, then read [`usage`](Self::usage). Linked files are never read.
pub struct ImageResolver {
    limits: Limits,
    used: Arc<Used>,
}

#[cfg(feature = "tree")]
impl ImageResolver {
    /// Create one resolver budget for one drawing import.
    pub fn new(limits: Limits) -> Self {
        ImageResolver {
            limits,
            used: Arc::new(Used {
                bytes: AtomicUsize::new(0),
                depth: AtomicUsize::new(0),
                xml_depth: AtomicUsize::new(0),
                svg_count: AtomicUsize::new(0),
                svg_depth: AtomicUsize::new(0),
                raster_pixels: AtomicU64::new(0),
                raster_data: Mutex::new(HashMap::new()),
                invalid_raster: AtomicUsize::new(0),
                raster_limit: AtomicUsize::new(0),
                invalid_svg: AtomicUsize::new(0),
                svg_limit: AtomicUsize::new(0),
                svg_resource_limit: AtomicUsize::new(0),
            }),
        }
    }

    /// Resolver to put in `usvg::Options::image_href_resolver`.
    pub fn resolver(&self) -> usvg::ImageHrefResolver<'static> {
        image_resolver_for(self.limits, Arc::clone(&self.used))
    }

    /// Record the already-checked depth of the outer SVG before parsing it. Embedded SVGs then
    /// share the same total XML recursion budget instead of each receiving a fresh allowance.
    pub fn set_document_depth(&self, depth: usize) {
        self.used.xml_depth.store(depth, Ordering::Relaxed);
    }

    /// Account for malformed image data URLs before usvg discards them without calling
    /// its image resolver. Call once for the outer document, before `parse_text`.
    /// Nested SVGs are checked automatically. The scan shares the normal XML limits and
    /// streaming-validates URL encoding without allocating another decoded image buffer.
    pub fn check_document(&self, svg: &str) -> Result<(), Error> {
        let checked = preflight(svg, &self.limits)?;
        account_invalid_data_urls(&checked.xml, &self.used);
        Ok(())
    }

    /// Snapshot accounting after parsing has finished.
    pub fn usage(&self) -> ImageUsage {
        ImageUsage {
            raster_pixels: self.used.raster_pixels.load(Ordering::Relaxed),
            embedded_svg_bytes: self.used.bytes.load(Ordering::Relaxed),
            embedded_svg_count: self.used.svg_count.load(Ordering::Relaxed),
            embedded_svg_depth: self.used.svg_depth.load(Ordering::Relaxed),
            invalid_raster_images: self.used.invalid_raster.load(Ordering::Relaxed),
            raster_limit_images: self.used.raster_limit.load(Ordering::Relaxed),
            invalid_svg_images: self.used.invalid_svg.load(Ordering::Relaxed),
            svg_limit_images: self.used.svg_limit.load(Ordering::Relaxed),
            svg_resource_limit_images: self.used.svg_resource_limit.load(Ordering::Relaxed),
        }
    }
}

#[cfg(feature = "tree")]
// usvg's data-URL decoder returns before resolve_data for invalid base64. Count
// these omissions separately, including feImage, without reading linked resources.
fn account_invalid_data_urls(xml: &roxmltree::Document<'_>, used: &Used) {
    for node in xml.descendants().filter(|n| n.is_element() && matches!(n.tag_name().name(), "image" | "feImage")) {
        let href = node.attribute(("http://www.w3.org/1999/xlink", "href")).or_else(|| node.attribute("href"));
        let Some(href) = href else { continue };
        let url = match data_url::DataUrl::process(href) {
            Ok(url) => url,
            // Use the parser's normalized scheme recognition, including whitespace.
            Err(data_url::DataUrlError::NoComma) => {
                used.invalid_raster.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            Err(data_url::DataUrlError::NotADataUrl) => continue,
        };
        if url.decode(|_| Ok::<(), std::convert::Infallible>(())).is_err() {
            used.invalid_raster.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "tree")]
#[derive(Clone, Copy)]
enum SvgRaster {
    Png,
    Jpeg,
    Gif,
    WebP,
}

#[cfg(feature = "tree")]
impl SvgRaster {
    fn codec(self) -> craft_image_codecs::Format {
        match self {
            Self::Png => craft_image_codecs::Format::Png,
            Self::Jpeg => craft_image_codecs::Format::Jpeg,
            Self::Gif => craft_image_codecs::Format::Gif,
            Self::WebP => craft_image_codecs::Format::WebP,
        }
    }

    fn image_kind(self, data: Arc<Vec<u8>>) -> usvg::ImageKind {
        match self {
            Self::Png => usvg::ImageKind::PNG(data),
            Self::Jpeg => usvg::ImageKind::JPEG(data),
            Self::Gif => usvg::ImageKind::GIF(data),
            Self::WebP => usvg::ImageKind::WEBP(data),
        }
    }
}

/// Match supported raster signatures to the MIME types both image resolvers accept.
#[cfg(feature = "tree")]
fn declared_raster(mime: &str, data: &[u8]) -> Option<SvgRaster> {
    use craft_image_codecs::Format;
    match (mime, craft_image_codecs::detect(data)?) {
        ("image/png" | "text/plain", Format::Png) => Some(SvgRaster::Png),
        ("image/jpg" | "image/jpeg" | "text/plain", Format::Jpeg) => Some(SvgRaster::Jpeg),
        ("image/gif" | "text/plain", Format::Gif) => Some(SvgRaster::Gif),
        ("image/webp" | "text/plain", Format::WebP) => Some(SvgRaster::WebP),
        _ => None,
    }
}

#[cfg(feature = "tree")]
fn raster_image(used: &Used, limits: &Limits, data: Arc<Vec<u8>>, format: SvgRaster) -> Option<usvg::ImageKind> {
    let Ok((width, height)) = craft_image_codecs::probe_allocation_size(&data) else {
        used.invalid_raster.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if pixels == 0 || pixels > limits.embedded_raster_pixels {
        used.raster_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let pointer = Arc::as_ptr(&data) as usize;
    let mut seen = used.raster_data.lock().unwrap_or_else(PoisonError::into_inner);
    if let Entry::Vacant(entry) = seen.entry(pointer) {
        let mut current = used.raster_pixels.load(Ordering::Relaxed);
        loop {
            let next = current.checked_add(pixels)?;
            if next > limits.embedded_raster_pixels {
                used.raster_limit.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            match used.raster_pixels.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        // The weak reference pins the allocation identity, not its encoded bytes.
        // At most one entry is added for each nonrefundable pixel charge.
        entry.insert(Arc::downgrade(&data));
    }
    drop(seen);
    // A valid header is not proof of valid pixels. resvg can silently omit a damaged
    // image later; decode now under explicit limits so omission accounting is honest.
    let decode_limits = craft_image_codecs::Limits::new(width, height, pixels, limits.embedded_raster_pixels.saturating_mul(8));
    let options = craft_image_codecs::DecodeOptions { limits: decode_limits, keep_orientation: true };
    if let Err(error) = craft_image_codecs::decode_as_with(format.codec(), &data, &options) {
        // No tree retains this allocation on failure. Forget its address before it
        // can be reused, but keep the pixel charge for the decode work already spent.
        used.raster_data.lock().unwrap_or_else(PoisonError::into_inner).remove(&pointer);
        match error {
            craft_image_codecs::CodecError::LimitExceeded(_) => {
                used.raster_limit.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                used.invalid_raster.fetch_add(1, Ordering::Relaxed);
            }
        }
        return None;
    }
    Some(format.image_kind(data))
}

/// Decode one child within the original byte allowance, including encoding expansion.
#[cfg(feature = "tree")]
fn embedded_text<'a>(data: &'a [u8], room: usize, limits: &Limits) -> Result<Cow<'a, str>, svgz::Error> {
    let text = svgz::text(data, room.min(limits.bytes))?;
    if text.len() > room {
        return Err(svgz::Error::TooLarge(room));
    }
    Ok(text)
}

#[cfg(feature = "tree")]
enum ChildRefusal {
    Depth,
    Bytes,
    Invalid,
}

#[cfg(feature = "tree")]
fn preflight_child<'a>(text: &'a str, limits: &Limits) -> Result<Preflight<'a>, ChildRefusal> {
    preflight(text, limits).map_err(|error| match error {
        Error::TooDeep(_) => ChildRefusal::Depth,
        Error::Text(svgz::Error::TooLarge(_)) => ChildRefusal::Bytes,
        _ => ChildRefusal::Invalid,
    })
}

/// Reserve the complete checked height within the parent's remaining XML room.
#[cfg(feature = "tree")]
fn child_total_depth(parent: usize, checked: &Preflight<'_>, limits: &Limits) -> Option<usize> {
    parent.checked_add(checked.height).filter(|depth| *depth <= limits.tree_depth)
}

#[cfg(feature = "tree")]
/// An embedded SVG document, if the budgets allow it.
fn embedded(used: &Used, limits: &Limits, data: &[u8], options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
    if used.depth.load(Ordering::Relaxed) >= limits.embedded_depth {
        used.svg_limit.fetch_add(1, Ordering::Relaxed);
        used.svg_resource_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let left = limits.embedded_bytes.saturating_sub(used.bytes.load(Ordering::Relaxed));
    let text = match embedded_text(data, left, limits) {
        Ok(text) => text,
        // What a refused document unpacked before it was refused was work too: it uses up what
        // is left, so the next image can't repeat it.
        Err(error) => {
            used.bytes.fetch_add(left, Ordering::Relaxed);
            used.svg_limit.fetch_add(1, Ordering::Relaxed);
            if matches!(error, svgz::Error::TooLarge(_)) {
                used.svg_resource_limit.fetch_add(1, Ordering::Relaxed);
            }
            return None;
        }
    };
    used.bytes.fetch_add(text.len(), Ordering::Relaxed);
    let checked = match preflight_child(&text, limits) {
        Ok(checked) => checked,
        // Decoding UTF-16 or Latin-1 can exceed the text cap after the
        // raw/inflated bytes fitted. This is a resource refusal, not invalid XML.
        Err(ChildRefusal::Bytes) => {
            used.svg_limit.fetch_add(1, Ordering::Relaxed);
            used.svg_resource_limit.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Err(ChildRefusal::Depth) => {
            used.svg_limit.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Err(_) => {
            used.invalid_svg.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    // The nesting usvg converts, references followed, not the source depth alone.
    let Some(total_depth) = child_total_depth(used.xml_depth.load(Ordering::Relaxed), &checked, limits) else {
        used.svg_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let previous_depth = used.xml_depth.swap(total_depth, Ordering::Relaxed);
    let depth = used.depth.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    used.svg_depth.fetch_max(depth, Ordering::Relaxed);
    account_invalid_data_urls(&checked.xml, used);
    let tree = checked.parse(options);
    used.xml_depth.store(previous_depth, Ordering::Relaxed);
    used.depth.fetch_sub(1, Ordering::Relaxed);
    match tree {
        Ok(tree) => {
            used.svg_count.fetch_add(1, Ordering::Relaxed);
            Some(usvg::ImageKind::SVG(tree))
        }
        Err(_) => {
            used.invalid_svg.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

#[cfg(feature = "tree")]
/// An image resolver for `usvg::Options::image_href_resolver` that reads `<image>` data URLs
/// only: raster images as they are (PNG, JPEG, GIF, WebP), SVG documents (SVGZ too) through the
/// same checks as [`parse`] and within [`Limits::embedded_bytes`] in all and
/// [`Limits::embedded_depth`] inside one another. An image past a limit is left out. Linked
/// files are never read. One resolver is one import's budget: make a new one for each [`parse`].
pub fn image_resolver(limits: Limits) -> usvg::ImageHrefResolver<'static> {
    ImageResolver::new(limits).resolver()
}

#[cfg(feature = "tree")]
fn image_resolver_for(limits: Limits, used: Arc<Used>) -> usvg::ImageHrefResolver<'static> {
    usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, options| {
            if let Some(format) = declared_raster(mime, &data) {
                return raster_image(&used, &limits, data, format);
            }
            // usvg can decode only the raster variants above. Count every other declared
            // image as an invalid raster so strict consumers can reject it instead of
            // silently accepting a document after the image was omitted (for example BMP).
            if mime.starts_with("image/") && mime != "image/svg+xml" {
                used.invalid_raster.fetch_add(1, Ordering::Relaxed);
            }
            match mime {
                "image/svg+xml" => embedded(&used, &limits, &data, options),
                "text/plain" => embedded(&used, &limits, &data, options),
                _ => None,
            }
        }),
        resolve_string: Box::new(|_, _| None),
    }
}

#[cfg(all(test, feature = "tree"))]
mod tests;

#[cfg(test)]
thread_local! {
    static PREFLIGHT_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

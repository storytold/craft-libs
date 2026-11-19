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
//! VectorCraft). [`Limits::tree_depth`] levels, times one more than [`Limits::embedded_depth`], must
//! fit the stack of the thread that calls [`parse`]: the default of 256 needs a thread with
//! 16 MB or so, and a 1 MB stack takes a depth of about 34.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub use usvg;

use crate::paths::{MAX_CSS_SELECTOR_CHECKS, StyleMap, computed_style, parse_stylesheets};
use crate::{guard, svgz};

pub mod document;
pub mod draw;

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
    /// Bytes of all SVG documents embedded in `<image>` data URLs, unpacked, for
    /// [`image_resolver`].
    pub embedded_bytes: usize,
    /// SVG documents embedded inside one another, for [`image_resolver`].
    pub embedded_depth: usize,
    /// Decoded RGBA pixels retained across the raster images embedded in one drawing. Dimensions
    /// are probed before usvg hands the bytes to a decoder; repeated references to the same data
    /// URL share one charge.
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

/// Read the root SVG's unitless width and height without constructing a rendering tree.
///
/// Lengths with units, percentages, missing dimensions, and non-SVG roots return `Ok(None)`;
/// malformed or over-limit XML returns the same bounded parsing error as [`parse_text`].
pub fn root_unitless_size(svg: &str, limits: &Limits) -> Result<Option<(f64, f64)>, Error> {
    let xml = checked(svg, limits)?;
    let root = xml.root_element();
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

/// Every check on SVG text, cheapest first; the XML it parses to on the way.
fn checked<'s>(svg: &'s str, limits: &Limits) -> Result<roxmltree::Document<'s>, Error> {
    let preflight_limits = crate::preflight::Limits {
        bytes: limits.bytes,
        depth: limits.depth,
        entity_bytes: limits.entity_bytes,
        refuse_entity_markup: limits.refuse_entity_markup,
        attributes: limits.attributes,
        attribute_work: limits.attribute_work,
        xml_nodes: limits.xml_nodes,
    };
    let xml = crate::preflight::parse_text(svg, &preflight_limits).map_err(|e| match e {
        crate::preflight::Error::Text(e) => Error::Text(e),
        crate::preflight::Error::Entities => Error::Entities,
        crate::preflight::Error::EntityMarkup => Error::EntityMarkup,
        crate::preflight::Error::TooDeep(depth) => Error::TooDeep(depth),
        crate::preflight::Error::TooWide(attributes) => Error::TooWide(attributes),
        crate::preflight::Error::Xml(msg) => Error::Xml(msg),
    })?;
    if height(&xml, limits.tree_depth)?.is_none() {
        return Err(Error::TooDeep(limits.tree_depth));
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
    xml_depth: AtomicUsize,
    svg_count: AtomicUsize,
    svg_depth: AtomicUsize,
    raster_pixels: AtomicU64,
    raster_data: Mutex<HashSet<usize>>,
    invalid_raster: AtomicUsize,
    raster_limit: AtomicUsize,
    invalid_svg: AtomicUsize,
    svg_limit: AtomicUsize,
}

/// What resolving one drawing's embedded images used or refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageUsage {
    /// Unique decoded raster pixels reserved before decode.
    pub raster_pixels: u64,
    /// Unpacked SVG bytes consumed by embedded SVG data URLs.
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
}

/// A data-URL resolver and the accounting for the one drawing it serves.
///
/// Keep this value while parsing, then read [`usage`](Self::usage). Linked files are never read.
pub struct ImageResolver {
    limits: Limits,
    used: Arc<Used>,
}

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
                raster_data: Mutex::new(HashSet::new()),
                invalid_raster: AtomicUsize::new(0),
                raster_limit: AtomicUsize::new(0),
                invalid_svg: AtomicUsize::new(0),
                svg_limit: AtomicUsize::new(0),
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
        }
    }
}

fn raster_format(data: &[u8]) -> Option<craft_image_codecs::Format> {
    match craft_image_codecs::detect(data)? {
        format @ (craft_image_codecs::Format::Png
        | craft_image_codecs::Format::Jpeg
        | craft_image_codecs::Format::Gif
        | craft_image_codecs::Format::WebP) => Some(format),
        _ => None,
    }
}

fn raster_image(used: &Used, limits: &Limits, data: Arc<Vec<u8>>, format: craft_image_codecs::Format) -> Option<usvg::ImageKind> {
    let Ok((width, height)) = craft_image_codecs::probe_allocation_size(&data) else {
        used.invalid_raster.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if !matches!(
        format,
        craft_image_codecs::Format::Png | craft_image_codecs::Format::Jpeg | craft_image_codecs::Format::Gif | craft_image_codecs::Format::WebP
    ) {
        used.invalid_raster.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if pixels == 0 || pixels > limits.embedded_raster_pixels {
        used.raster_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let pointer = Arc::as_ptr(&data) as usize;
    let mut seen = used.raster_data.lock().unwrap_or_else(PoisonError::into_inner);
    if seen.insert(pointer) {
        let mut current = used.raster_pixels.load(Ordering::Relaxed);
        loop {
            let next = current.checked_add(pixels)?;
            if next > limits.embedded_raster_pixels {
                seen.remove(&pointer);
                used.raster_limit.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            match used.raster_pixels.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }
    drop(seen);
    Some(match format {
        craft_image_codecs::Format::Png => usvg::ImageKind::PNG(data),
        craft_image_codecs::Format::Jpeg => usvg::ImageKind::JPEG(data),
        craft_image_codecs::Format::Gif => usvg::ImageKind::GIF(data),
        craft_image_codecs::Format::WebP => usvg::ImageKind::WEBP(data),
        _ => return None,
    })
}

/// An embedded SVG document, if the budgets allow it.
fn embedded(used: &Used, limits: &Limits, data: &[u8], options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
    if used.depth.load(Ordering::Relaxed) >= limits.embedded_depth {
        used.svg_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let left = limits.embedded_bytes.saturating_sub(used.bytes.load(Ordering::Relaxed));
    let text = match svgz::text(data, left.min(limits.bytes)) {
        Ok(text) => text,
        // What a refused document unpacked before it was refused was work too: it uses up what
        // is left, so the next image can't repeat it.
        Err(_) => {
            used.bytes.fetch_add(left, Ordering::Relaxed);
            used.svg_limit.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    used.bytes.fetch_add(text.len(), Ordering::Relaxed);
    let xml = match checked(&text, limits) {
        Ok(xml) => xml,
        Err(_) => {
            used.invalid_svg.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let Some(xml_depth) = height(&xml, limits.tree_depth).ok().flatten() else {
        used.svg_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let total_depth = used.xml_depth.load(Ordering::Relaxed).checked_add(xml_depth);
    let Some(total_depth) = total_depth.filter(|depth| *depth <= limits.tree_depth) else {
        used.svg_limit.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let previous_depth = used.xml_depth.swap(total_depth, Ordering::Relaxed);
    let depth = used.depth.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    used.svg_depth.fetch_max(depth, Ordering::Relaxed);
    let tree = usvg::Tree::from_xmltree(&xml, options);
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

/// An image resolver for `usvg::Options::image_href_resolver` that reads `<image>` data URLs
/// only: raster images as they are (PNG, JPEG, GIF, WebP), SVG documents (SVGZ too) through the
/// same checks as [`parse`] and within [`Limits::embedded_bytes`] in all and
/// [`Limits::embedded_depth`] inside one another. An image past a limit is left out. Linked
/// files are never read. One resolver is one import's budget: make a new one for each [`parse`].
pub fn image_resolver(limits: Limits) -> usvg::ImageHrefResolver<'static> {
    ImageResolver::new(limits).resolver()
}

fn image_resolver_for(limits: Limits, used: Arc<Used>) -> usvg::ImageHrefResolver<'static> {
    usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, options| {
            let detected = raster_format(&data);
            let format = match mime {
                "image/jpg" | "image/jpeg" => (detected == Some(craft_image_codecs::Format::Jpeg)).then_some(craft_image_codecs::Format::Jpeg),
                "image/png" => (detected == Some(craft_image_codecs::Format::Png)).then_some(craft_image_codecs::Format::Png),
                "image/gif" => (detected == Some(craft_image_codecs::Format::Gif)).then_some(craft_image_codecs::Format::Gif),
                "image/webp" => (detected == Some(craft_image_codecs::Format::WebP)).then_some(craft_image_codecs::Format::WebP),
                "text/plain" => detected,
                _ => None,
            };
            if let Some(format) = format {
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

#[cfg(test)]
mod tests;

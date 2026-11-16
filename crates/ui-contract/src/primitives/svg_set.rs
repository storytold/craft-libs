//! The SVG icon catalog the build embeds, the app icon sets registered beside it, and their rasteriser.

use craft_svg::tree::draw::{self as svg_draw, tiny_skia};
use egui::{Color32, ColorImage};
use std::sync::{PoisonError, RwLock};

use super::svg_tree;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// An app's own icon set: SVG documents drawn beside the built-in catalog in every `IconSet`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct AppIcons {
    /// The set's name, for the log line that refuses a clashing registration.
    pub name: &'static str,
    /// The documents' own stroke width, `None` when they are drawn as drawn.
    pub stroke_width: Option<f32>,
    /// Each icon's name and SVG document.
    pub icons: &'static [(&'static str, &'static [u8])],
}

impl AppIcons {
    /// The set `name` of `icons`, each an icon name and its SVG document, drawn with `stroke_width` when it is `Some`.
    pub const fn new(name: &'static str, stroke_width: Option<f32>, icons: &'static [(&'static str, &'static [u8])]) -> Self {
        Self { name, stroke_width, icons }
    }
}

/// Registers `set` in the process-wide list beside the built-in catalog; runs before the first pass.
///
/// Registering the same set again is a no-op; a name the built-in catalog or an earlier app set has is refused with a
/// log line.
pub fn register_app_icons(set: &'static AppIcons) {
    let mut guard = APPS.write().unwrap_or_else(PoisonError::into_inner);
    let registry = &mut *guard;
    if registry.sets.iter().any(|known| std::ptr::eq(*known, set)) {
        return;
    }
    let set_at = registry.sets.len();
    registry.sets.push(set);
    for (icon, (name, _)) in set.icons.iter().enumerate() {
        if find(name).is_some() {
            log::warn!("app icon set {:?}: icon {name:?} is refused, the built-in catalog has it", set.name);
            continue;
        }
        match registry.index.binary_search_by(|(known, ..)| (*known).cmp(name)) {
            Ok(at) => {
                let earlier = registry.sets[registry.index[at].1].name;
                log::warn!("app icon set {:?}: icon {name:?} is refused, set {earlier:?} has it", set.name);
            }
            Err(at) => registry.index.insert(at, (name, set_at, icon)),
        }
    }
}

/// The registered sets and the sorted index of their icons: name, set, position in the set.
struct Registry {
    sets: Vec<&'static AppIcons>,
    index: Vec<(&'static str, usize, usize)>,
}

/// Process-wide like the generated catalog: it is the icon catalog, not pass state.
static APPS: RwLock<Registry> = RwLock::new(Registry { sets: Vec::new(), index: Vec::new() });

/// An icon the lookup found: a catalog position or an app set's icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Entry {
    /// The position in the built-in catalog.
    Catalog(usize),
    /// The icon `icon` of the registered set `set`.
    App { set: usize, icon: usize },
}

/// The icon called `name`: the built-in catalog first, then the registered app sets.
pub(crate) fn lookup(name: &str) -> Option<Entry> {
    if let Some(index) = find(name) {
        return Some(Entry::Catalog(index));
    }
    let registry = APPS.read().unwrap_or_else(PoisonError::into_inner);
    let at = registry.index.binary_search_by(|(known, ..)| (*known).cmp(name)).ok()?;
    let (_, set, icon) = registry.index[at];
    Some(Entry::App { set, icon })
}

/// The name, document and own stroke width of an app icon.
fn app_icon(set: usize, icon: usize) -> (&'static str, &'static [u8], Option<f32>) {
    let registry = APPS.read().unwrap_or_else(PoisonError::into_inner);
    let set = registry.sets[set];
    let (name, svg) = set.icons[icon];
    (name, svg, set.stroke_width)
}

/// The position of the icon called `name` in the catalog.
pub(crate) fn find(name: &str) -> Option<usize> {
    NAMES.binary_search(&name).ok()
}

/// Every icon name, sorted.
pub(crate) fn names() -> &'static [&'static str] {
    &NAMES
}

/// Whether `entry` flips under right-to-left; no app icon does.
pub(crate) fn directional(entry: Entry) -> bool {
    match entry {
        Entry::Catalog(index) => DIRECTIONAL[index],
        Entry::App { .. } => false,
    }
}

/// The document's own stroke width of `entry` in `set`, `None` for a document drawn with fills or as drawn.
pub(crate) fn entry_stroke_width(entry: Entry, set: IconSet) -> Option<f32> {
    match entry {
        Entry::Catalog(index) => stroke_width(index, set),
        Entry::App { set: s, icon } => app_icon(s, icon).2,
    }
}

/// `entry` in white at `px` x `px` physical pixels; a catalog entry as [`rasterise`], an app entry through the same
/// drawing with its set's stroke width.
pub(crate) fn rasterise_entry(entry: Entry, set: IconSet, px: u32, stroke_px: f32, mirrored: bool) -> ColorImage {
    match entry {
        Entry::Catalog(index) => rasterise(index, set, px, stroke_px, mirrored),
        Entry::App { set: s, icon } => {
            let (name, svg, own) = app_icon(s, icon);
            image(draw(svg, own, px, stroke_px, mirrored), px, || format!("app icon {name}"))
        }
    }
}

/// The white image of a drawn pixmap; a failed drawing gives a transparent image and a log line.
fn image(drawn: Result<tiny_skia::Pixmap, String>, px: u32, label: impl FnOnce() -> String) -> ColorImage {
    let size = [px as usize; 2];
    match drawn {
        Ok(pixmap) => ColorImage::new(size, pixmap.pixels().iter().map(|p| Color32::from_white_alpha(p.alpha())).collect()),
        Err(error) => {
            log::error!("{}: {error}", label());
            ColorImage::filled(size, Color32::TRANSPARENT)
        }
    }
}

/// The icon in white at `px` x `px` physical pixels, its alpha from the drawing; tinted when drawn.
///
/// A document drawn with strokes is drawn `stroke_px` physical pixels wide, one drawn with fills as drawn. `mirrored`
/// flips the drawing horizontally. A document that fails to parse gives a transparent image and a log line.
pub(crate) fn rasterise(index: usize, set: IconSet, px: u32, stroke_px: f32, mirrored: bool) -> ColorImage {
    image(draw(svg(index, set), stroke_width(index, set), px, stroke_px, mirrored), px, || format!("icon {} in {}", NAMES[index], set.id()))
}

/// `own` is the document's own stroke width in view-box units; a document without one is drawn as it is.
fn draw(svg: &[u8], own: Option<f32>, px: u32, stroke_px: f32, mirrored: bool) -> Result<tiny_skia::Pixmap, String> {
    let mut tree = svg_tree::parse(svg)?;
    let view = tree.size();
    if let Some(own) = own {
        // Every `stroke-width` scales by the same factor, so a set's thinner details stay thinner.
        let units = stroke_px * view.width() / px as f32;
        let text = std::str::from_utf8(svg).map_err(|e| e.to_string())?;
        tree = svg_tree::parse(scale_stroke_widths(text, units / own).as_bytes())?;
    }
    let (sx, sy) = (px as f32 / view.width(), px as f32 / view.height());
    let transform =
        if mirrored { tiny_skia::Transform::from_row(-sx, 0.0, 0.0, sy, px as f32, 0.0) } else { tiny_skia::Transform::from_scale(sx, sy) };
    svg_draw::render(&tree, transform, px, px, 0, ICON_DRAW_BUDGET).map_err(|refusal| refusal.message)
}

/// Bytes drawing one icon may allocate, its target included.
const ICON_DRAW_BUDGET: u64 = 64 << 20;

/// `svg` with each `stroke-width="..."` attribute multiplied by `factor`.
fn scale_stroke_widths(svg: &str, factor: f32) -> String {
    const ATTR: &str = "stroke-width=\"";
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(start) = rest.find(ATTR) {
        let value_start = start + ATTR.len();
        out.push_str(&rest[..value_start]);
        rest = &rest[value_start..];
        let end = rest.find('"').unwrap_or(rest.len());
        match rest[..end].parse::<f32>() {
            Ok(width) => out.push_str(&format!("{:.3}", width * factor)),
            Err(_) => out.push_str(&rest[..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

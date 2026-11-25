//! Prepare an application's embedded SVG icons once, then share their bytes with egui.
//!
//! The application owns its assets, tool-name mapping, theme, sizing and button behavior.
//! Install `egui_extras`' SVG image loader on the context before drawing an icon. This crate
//! does not implement an SVG parser or rasterizer, read files, or install global UI state.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Image, ImageSource, Rect, Ui, Vec2, load::Bytes};

use crate::direction::{self, HasDirection};

/// Literal substitutions for trusted, embedded, monochrome SVG assets.
///
/// These preserve the conventions of the existing Craft catalogs; they are not a general
/// CSS/SVG transformation. In particular, only the exact `stroke-width="2"` spelling is
/// replaced. Other widths, quoting and explicit colors are left alone.
#[derive(Clone, Copy, Debug)]
pub struct SvgStyle {
    /// Replacement for `currentColor`, usually `white` or `#ffffff` for draw-time tinting.
    pub current_color: &'static str,
    /// Replacement width for the catalog's default two-unit stroke; `None` preserves it.
    pub stroke_width: Option<&'static str>,
}

/// An immutable catalog of prepared SVG bytes.
///
/// Keep one set in an application-owned `OnceLock`. Repeated lookups clone an `Arc`, not SVG
/// data, and never grow this catalog. Each set's URI prefix must be unique within an egui
/// context (including sets with different styles): egui caches images by URI.
pub struct SvgIconSet {
    icons: HashMap<&'static str, Arc<[u8]>>,
    uri_prefix: &'static str,
    fallback: &'static str,
}

impl SvgIconSet {
    /// Prepare a finite, compile-time catalog. The last entry wins for duplicate names.
    ///
    /// `uri_prefix` includes the scheme and trailing separator, e.g. `bytes://my-app/icons/`.
    /// `fallback` is an icon name in the same catalog. If neither the requested icon nor the
    /// fallback exists, [`Self::source`] returns empty bytes for egui to report as a load error.
    /// Invalid UTF-8 is replaced lossily, matching the existing app loaders. This API is for
    /// trusted embedded assets, not user-supplied SVG documents.
    pub fn new(icons: &'static [(&'static str, &'static [u8])], style: SvgStyle, uri_prefix: &'static str, fallback: &'static str) -> Self {
        let stroke = style.stroke_width.map(|width| format!("stroke-width=\"{width}\""));
        let icons = icons
            .iter()
            .map(|(name, bytes)| {
                let mut svg = String::from_utf8_lossy(bytes).replace("currentColor", style.current_color);
                if let Some(stroke) = &stroke {
                    svg = svg.replace("stroke-width=\"2\"", stroke);
                }
                (*name, Arc::from(svg.into_bytes()))
            })
            .collect();
        Self { icons, uri_prefix, fallback }
    }

    /// Whether the catalog contains `name`, without applying the fallback.
    pub fn contains(&self, name: &str) -> bool {
        self.icons.contains_key(name)
    }

    /// Borrow the embedded name, for applications that map dynamic tool names to static names.
    pub fn canonical_name(&self, name: &str) -> Option<&'static str> {
        self.icons.get_key_value(name).map(|(key, _)| *key)
    }

    /// A source suitable for `egui::Image::new`, with shared bytes and the caller's requested
    /// name in the URI, even when the fallback is used. This preserves existing app cache keys.
    pub fn source(&self, name: &str) -> ImageSource<'static> {
        let bytes = self.icons.get(name).or_else(|| self.icons.get(self.fallback)).cloned().unwrap_or_default();
        ImageSource::Bytes { uri: format!("{}{name}.svg", self.uri_prefix).into(), bytes: Bytes::Shared(bytes) }
    }
}

/// An egui image for `name` in the interface direction: the mirror partner of a
/// [`direction::SWAPPED_ICONS`] glyph right to left, drawn flipped in place for a
/// [`direction::FLIPPED_ICONS`] one, and `name` as it is otherwise.
pub fn image(d: &impl HasDirection, set: &SvgIconSet, name: &str, size: f32, tint: Color32) -> Image<'static> {
    let icon = direction::icon(d, name);
    let mut img = image_raw(set, icon.name, size, tint);
    if icon.flip {
        img = img.uv(icon.uv());
    }
    img
}

/// The un-mirrored icon: canvas cursors and badges, and the opt-out for glyphs that point at
/// the canvas or a physical timeline ([`paint_ltr`]).
pub fn image_raw(set: &SvgIconSet, name: &str, size: f32, tint: Color32) -> Image<'static> {
    Image::new(set.source(name)).fit_to_exact_size(Vec2::splat(size)).tint(tint)
}

/// Paint `name` centred in `rect`, mirrored in a right-to-left interface ([`image`]).
pub fn paint(d: &impl HasDirection, ui: &Ui, set: &SvgIconSet, rect: Rect, name: &str, size: f32, tint: Color32) {
    let r = Rect::from_center_size(rect.center(), Vec2::splat(size));
    image(d, set, name, size, tint).paint_at(ui, r);
}

/// Paint an icon that must not follow the interface direction: timeline transport and tools
/// whose glyph points at the canvas.
pub fn paint_ltr(ui: &Ui, set: &SvgIconSet, rect: Rect, name: &str, size: f32, tint: Color32) {
    let r = Rect::from_center_size(rect.center(), Vec2::splat(size));
    image_raw(set, name, size, tint).paint_at(ui, r);
}

/// Whether the set contains `name` after direction mapping, so swapped-glyph lookups stay true.
pub fn exists(d: &impl HasDirection, set: &SvgIconSet, name: &str) -> bool {
    set.contains(direction::icon_name(d, name))
}

#[cfg(test)]
mod tests;

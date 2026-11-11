//! Filter Gallery's catalog: the app's filters, their categories and parameters, and the app's picture
//! sources.

use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Entry;
use craft_ui::patterns::form_dialogs::{DialogFields, Param, ParamKind};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::pending::Job;

use super::session::EffectLayer;
use crate::ui::wide_angle::PreviewDone;

/// One filter of the gallery: its key in the app's command (`"filter.artistic.cutout"`), its label, its
/// category key, its parameters in the step 2 notation (`Param::parse_list` of PhotoCraft's `params_doc`),
/// and whether it uses the foreground and background colours (Photoshop shows them in the settings).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GalleryFilter {
    /// The filter's key in the app's command.
    pub key: String,
    /// The filter's name: a word the app's catalogue holds, or a string it gives.
    pub label: Label,
    /// The key of the filter's category.
    pub category: String,
    /// The filter's parameters.
    pub params: Vec<Param>,
    /// Whether the filter uses the foreground and background colours.
    pub uses_colors: bool,
}

impl GalleryFilter {
    /// A filter `key` named `label` in the category `category`, with `params` and no use of the colours.
    pub fn new(key: impl Into<String>, label: impl Into<Label>, category: impl Into<String>, params: Vec<Param>) -> Self {
        Self { key: key.into(), label: label.into(), category: category.into(), params, uses_colors: false }
    }

    /// Whether the filter uses the foreground and background colours.
    #[must_use]
    pub fn uses_colors(mut self, on: bool) -> Self {
        self.uses_colors = on;
        self
    }

    /// The filter's defaults as fields: each Range's default, each Choice's default (or its first
    /// option), each Bool, and colour parameters (Neon Glow's) at their default; other kinds are
    /// left out.
    pub fn defaults(&self) -> DialogFields {
        let mut fields = DialogFields::new();
        for param in &self.params {
            if matches!(param.kind, ParamKind::Range { .. } | ParamKind::Choice { .. } | ParamKind::Bool { .. } | ParamKind::Color { .. })
                && let Some(value) = param.kind.default_value()
            {
                fields.set(param.key.clone(), value);
            }
        }
        fields
    }
}

/// A category folder: its key, its label and its filters' keys in menu order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FilterCategory {
    /// The category's key.
    pub key: String,
    /// The category's name: a word the app's catalogue holds, or a string it gives.
    pub label: Label,
    /// The keys of the category's filters, in menu order.
    pub filters: Vec<String>,
}

impl FilterCategory {
    /// The folder `key` named `label`, holding the filters `filters`.
    pub fn new(key: impl Into<String>, label: impl Into<Label>, filters: Vec<String>) -> Self {
        Self { key: key.into(), label: label.into(), filters }
    }
}

/// The app's filters. PhotoCraft implements it over `photocraft_algo::artistic`'s `GalleryFilter` (`key`,
/// `category`, `params_doc`).
pub trait FilterCatalog {
    /// The folders in Photoshop's order (Artistic, Brush Strokes, Distort, Sketch, Stylize,
    /// Texture).
    fn categories(&self) -> Vec<FilterCategory>;

    /// The filter of `key`, or `None` for a key the app no longer has.
    fn filter(&self, key: &str) -> Option<GalleryFilter>;
}

/// A filter's thumbnail key for the `Thumbnails` cache: the filter applied to a small crop of the
/// layer (PhotoCraft renders 6 a pass, 80 × 56). The app's `ThumbSource<GalleryThumb>` renders it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GalleryThumb {
    /// The key of the filtered effect.
    pub filter: String,
}

impl GalleryThumb {
    /// The thumbnail of the filter `filter`.
    pub fn new(filter: impl Into<String>) -> Self {
        Self { filter: filter.into() }
    }
}

/// The app's picture of the layer under the effects.
///
/// `preview` is the picture the canvas shows; its revision moves when a render lands. `render` starts the
/// job that renders `effects` over `visible` (document pixels, widened by the filters' reach) at `zoom`
/// (screen pixels per document pixel): the workspace asks again when the effects, the visible part or the
/// zoom change, and a newer request replaces the one running.
pub trait FilterGallerySource {
    /// The rendered preview.
    fn preview(&self) -> &dyn RasterSource;

    /// The job that renders `effects` over `visible` at `zoom` into the preview.
    fn render(&self, effects: &[EffectLayer], visible: PixelRect, zoom: f64) -> Job<PreviewDone, String>;
}

/// The catalog as one pass reads it: the folders and every filter in them, looked up by key.
///
/// The columns read names and parameters from here, so the catalog is asked once a pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterIndex {
    categories: Vec<FilterCategory>,
    filters: Vec<GalleryFilter>,
}

impl FilterIndex {
    /// The folders of `catalog` and the filters in them.
    pub fn of(catalog: &dyn FilterCatalog) -> Self {
        let categories = catalog.categories();
        let filters = categories.iter().flat_map(|category| category.filters.iter()).filter_map(|key| catalog.filter(key)).collect();
        Self { categories, filters }
    }

    /// The folders in the catalog's order.
    pub fn categories(&self) -> &[FilterCategory] {
        &self.categories
    }

    /// The filter of `key`.
    pub fn filter(&self, key: &str) -> Option<&GalleryFilter> {
        self.filters.iter().find(|filter| filter.key == key)
    }

    /// The name of the filter of `key`; the key itself for a filter the catalog no longer has.
    pub fn label(&self, key: &str) -> Label {
        self.filter(key).map_or_else(|| Label::from(key.to_owned()), |filter| filter.label.clone())
    }

    /// Every filter as a select entry, a separator between the folders.
    pub fn entries(&self) -> Vec<Entry<String>> {
        let mut entries = Vec::new();
        for category in &self.categories {
            if !entries.is_empty() {
                entries.push(Entry::separator());
            }
            entries.extend(category.filters.iter().map(|key| Entry::new(key.clone(), self.label(key))));
        }
        entries
    }
}

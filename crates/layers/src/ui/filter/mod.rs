//! `LayerFilter`, `FilterBy`, `FilterKind` and `LayerFilterBar`: the filter row and its matching
//! rule (F11.05).
//!
//! The matching rule ([`LayerFilter::shows`]) is a plain function, so a host's control channel can
//! read and set the criteria (PhotoCraft's `ui.set layerFilter {by, kinds, name, mode}`) without
//! the panel.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{EditEvent, IconId, Mixed};
use craft_theme::ControlSize;
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use craft_ui::text::TextField;
use egui::{Response, Ui};

use super::tree::{LayerInfo, LayerKind};

/// What the Layers panel filters by: the filter bar's type select (PhotoCraft's three types;
/// Photoshop's Effect, Attribute, Color, Selected and Artboard wait for the features that need
/// them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FilterBy {
    /// Layer kinds, toggled with five buttons.
    #[default]
    Kind,
    /// Part of the layer name, typed in a search field.
    Name,
    /// One blend mode, picked in a select.
    Mode,
}

/// A layer kind the Kind filter toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FilterKind {
    /// Pixel layers.
    Pixel,
    /// Adjustment and fill layers.
    Adjustment,
    /// Type layers.
    Type,
    /// Shape layers.
    Shape,
    /// Smart objects.
    SmartObject,
}

impl FilterKind {
    /// Every kind, in the filter bar's order.
    pub const ALL: [FilterKind; 5] = [FilterKind::Pixel, FilterKind::Adjustment, FilterKind::Type, FilterKind::Shape, FilterKind::SmartObject];

    /// PhotoCraft's control-channel name: `"pixel"`, `"adjustment"`, `"type"`, `"shape"`,
    /// `"smart"`.
    pub const fn name(self) -> &'static str {
        match self {
            FilterKind::Pixel => "pixel",
            FilterKind::Adjustment => "adjustment",
            FilterKind::Type => "type",
            FilterKind::Shape => "shape",
            FilterKind::SmartObject => "smart",
        }
    }
}

impl LayerKind {
    /// The kind the Kind filter matches this layer by: adjustment and fill layers are
    /// [`FilterKind::Adjustment`], text is [`FilterKind::Type`]; groups, artboards, paths, images
    /// and objects match no kind (`None`), as PhotoCraft's groups.
    pub const fn filter_kind(self) -> Option<FilterKind> {
        // pc: crates/ui-egui/src/panels/layer_filter_ui.rs:110-119 `kind` ("" for groups).
        match self {
            LayerKind::Pixel => Some(FilterKind::Pixel),
            LayerKind::Adjustment | LayerKind::Fill => Some(FilterKind::Adjustment),
            LayerKind::Text => Some(FilterKind::Type),
            LayerKind::Shape => Some(FilterKind::Shape),
            LayerKind::SmartObject => Some(FilterKind::SmartObject),
            LayerKind::Group | LayerKind::Artboard | LayerKind::Path | LayerKind::Image | LayerKind::Object => None,
        }
    }
}

/// The filter's criteria. The app stores it (PhotoCraft: in `UiState`, not saved with the
/// document) and passes it to [`LayersPanel::filter`](super::panel::LayersPanel::filter); every
/// field has a serde default.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct LayerFilter {
    /// The filter type.
    pub by: FilterBy,
    /// The kinds toggled on, in the order they were toggled.
    pub kinds: Vec<FilterKind>,
    /// The name text as typed; matched trimmed and ignoring case.
    pub name: String,
    /// The blend mode's app key the Mode filter matches; `"normal"` at first.
    pub mode: String,
}

impl Default for LayerFilter {
    /// Kind, no kinds toggled, no name, mode `"normal"`: shows every layer.
    fn default() -> Self {
        Self { by: FilterBy::Kind, kinds: Vec::new(), name: String::new(), mode: String::from("normal") }
    }
}

impl LayerFilter {
    /// Whether the panel is filtered. As Photoshop, Mode always filters (by Normal at first);
    /// Kind only once a kind is toggled, Name only once the trimmed text is not empty.
    pub fn active(&self) -> bool {
        // pc: crates/ui-egui/src/panels/layer_filter_ui.rs:40-46 `Filter::active`.
        match self.by {
            FilterBy::Kind => !self.kinds.is_empty(),
            FilterBy::Name => !self.name.trim().is_empty(),
            FilterBy::Mode => true,
        }
    }

    /// Whether a layer is listed under this filter: always when the filter isn't
    /// [`active`](Self::active); Kind: its [`LayerKind::filter_kind`] is one of `kinds` (a group
    /// matches none); Name: `info.name` lowercased contains the trimmed `name` lowercased; Mode:
    /// `info.blend_key == mode`.
    pub fn shows(&self, info: &LayerInfo) -> bool {
        // pc: crates/ui-egui/src/panels/layer_filter_ui.rs:49-59 `Filter::shows`.
        if !self.active() {
            return true;
        }
        match self.by {
            FilterBy::Kind => info.kind.filter_kind().is_some_and(|kind| self.kinds.contains(&kind)),
            FilterBy::Name => info.name.to_lowercase().contains(&self.name.trim().to_lowercase()),
            FilterBy::Mode => info.blend_key == self.mode,
        }
    }
}

craft_ui::words! {
    /// The words of the filter bar.
    pub struct LayerFilterWords {
        /// The type select's name.
        filter_type: "layerFilterBar.type" = "Filter type";
        /// The type select's entries.
        kind: "layerFilterBar.kind" = "Kind";
        name: "layerFilterBar.name" = "Name";
        mode: "layerFilterBar.mode" = "Mode";
        /// The kind toggles' tooltips and names.
        pixel: "layerFilterBar.pixel" = "Filter for pixel layers";
        adjustment: "layerFilterBar.adjustment" = "Filter for adjustment layers";
        type_layers: "layerFilterBar.typeLayers" = "Filter for type layers";
        shape: "layerFilterBar.shape" = "Filter for shape layers";
        smart_object: "layerFilterBar.smartObject" = "Filter for smart objects";
        /// The name field's placeholder.
        search: "layerFilterBar.search" = "Search";
        /// The name field's name.
        search_layers: "layerFilterBar.searchLayers" = "Search layers";
        /// The mode select's name.
        blend_mode: "layerFilterBar.blendMode" = "Filter blend mode";
    }
}

/// PhotoCraft's filter row: the type select (Kind, Name, Mode), then that type's controls.
///
/// ```ignore
/// let out = LayerFilterBar::new(Bind::bound(&mut ui_state.layer_filter)).blend_modes(&modes).show(ui);
/// if out.changed { /* the app stores the filter; the panel reads it next time it's drawn */ }
/// ```
///
/// - **Kind**: five icon toggle buttons in [`FilterKind::ALL`] order, pressed while on; a click or
///   Space adds or removes the kind.
/// - **Name**: a search field named "Search layers", filtering as the user types; Escape clears.
/// - **Mode**: a [`Select`] with `blend_modes` (the app gives Pass Through first, as PhotoCraft).
/// - **Keyboard**: each control is its own Tab stop; the select opens with Space or Alt+Down.
#[must_use = "show the filter bar with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct LayerFilterBar<'a> {
    filter: Bind<'a, LayerFilter>,
    /// The modes the Mode select offers, in order; usually [`LayerTree::blend_modes`](super::tree::LayerTree::blend_modes)
    /// with groups, so Pass Through is in it.
    #[prop]
    blend_modes: &'a [Entry<String>],
}

/// What [`LayerFilterBar::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host stores a changed filter"]
pub struct LayerFilterOutput {
    /// The bar's row.
    pub response: Response,
    /// The criteria changed this pass (a toggle, a typed letter, a mode pick, the type select).
    pub changed: bool,
    /// The change as an edit, for a binding that does not write through.
    pub edit: Option<EditEvent<LayerFilter>>,
}

impl Outcome for LayerFilterOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> LayerFilterBar<'a> {
    /// The bar editing `filter`.
    pub fn new(filter: Bind<'a, LayerFilter>) -> Self {
        Self { filter, blend_modes: &[] }
    }

    /// Draws the bar on one row of the available width.
    pub fn show(self, ui: &mut Ui) -> LayerFilterOutput {
        let Self { mut filter, blend_modes } = self;
        let words = Env::of(ui).words::<LayerFilterWords>();
        let size = ControlSize::Sm;
        let mut edit = None;
        let row = Row::new().show(ui, |ui, cx| {
            let Some(Mixed::Same(current)) = filter.reborrow().read(cx) else { return };
            let mut next = current.clone();
            let types = [
                Entry::new(FilterBy::Kind, words.kind.clone()),
                Entry::new(FilterBy::Name, words.name.clone()),
                Entry::new(FilterBy::Mode, words.mode.clone()),
            ];
            let picked = Select::new(words.filter_type.clone(), Bind::given(current.by), &types).size(size).show(ui);
            if let Some(by) = picked.value() {
                next.by = *by;
            }
            match current.by {
                FilterBy::Kind => {
                    let kinds = [
                        (FilterKind::Pixel, "image", &words.pixel),
                        (FilterKind::Adjustment, "adjustment_layer", &words.adjustment),
                        (FilterKind::Type, "text", &words.type_layers),
                        (FilterKind::Shape, "square", &words.shape),
                        (FilterKind::SmartObject, "app_window", &words.smart_object),
                    ];
                    for (kind, icon, tip) in kinds {
                        let on = current.kinds.contains(&kind);
                        let toggled = ToggleButton::new(tip.clone(), Bind::given(on))
                            .icon(IconId::new(icon))
                            .icon_only()
                            .size(size)
                            .id_salt(kind.name())
                            .show(ui);
                        match toggled.value() {
                            Some(true) => next.kinds.push(kind),
                            Some(false) => next.kinds.retain(|k| *k != kind),
                            None => {}
                        }
                    }
                }
                FilterBy::Name => {
                    let typed = TextField::search(words.search_layers.clone(), Bind::given(current.name.clone()))
                        .placeholder(words.search.clone())
                        .size(size)
                        .show(ui);
                    if let Some(text) = typed.value() {
                        next.name.clone_from(text);
                    }
                }
                FilterBy::Mode => {
                    let picked = Select::new(words.blend_mode.clone(), Bind::given(current.mode.clone()), blend_modes).size(size).show(ui);
                    if let Some(mode) = picked.value() {
                        next.mode.clone_from(mode);
                    }
                }
            }
            if next != current {
                let name: std::sync::Arc<str> = std::sync::Arc::from(words.filter_type.as_str());
                let event = EditEvent::set(Mixed::Same(current), next, name, None);
                filter.reborrow().write(cx, &event);
                edit = Some(event);
            }
        });
        LayerFilterOutput { response: row.output, changed: edit.is_some(), edit }
    }
}

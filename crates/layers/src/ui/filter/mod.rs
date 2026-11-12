//! `LayerFilter`, its filter types and criteria, and `LayerFilterBar`: the filter row and its
//! matching rule (F11.05).
//!
//! The matching rule ([`LayerFilter::shows`] for one layer, [`LayerFilter::lists`] over a tree) is
//! plain code, so a host's control channel can read and set the criteria (PhotoCraft's
//! `ui.set layerFilter {by, kinds, name, effect, mode, attribute, color, smart}`) without the panel.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{EditEvent, IconId, Mixed};
use craft_theme::{ControlSize, LabelColor};
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::list::{ListWords, label_color_label};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::text::TextField;
use egui::{Response, Ui};

use super::tree::{LayerInfo, LayerKind, LayerPart, LayerTree};

/// What the Layers panel filters by: the filter bar's type select, Photoshop's types in its order
/// (Selected waits for the feature that needs it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FilterBy {
    /// Layer kinds, toggled with five buttons.
    #[default]
    Kind,
    /// Part of the layer name, typed in a search field.
    Name,
    /// One effect kind, picked in a select.
    Effect,
    /// One blend mode, picked in a select.
    Mode,
    /// One layer attribute or its opposite, picked in a select.
    Attribute,
    /// One label colour (or none), picked in a select.
    Color,
    /// Smart objects by source, toggled with three buttons.
    SmartObject,
    /// The artboards that hold the selected layers; no controls.
    Artboard,
    /// Every group with its contents; no controls.
    Group,
}

impl FilterBy {
    /// Every type, in the type select's order.
    pub const ALL: [FilterBy; 9] = [
        FilterBy::Kind,
        FilterBy::Name,
        FilterBy::Effect,
        FilterBy::Mode,
        FilterBy::Attribute,
        FilterBy::Color,
        FilterBy::SmartObject,
        FilterBy::Artboard,
        FilterBy::Group,
    ];
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

/// A layer effect kind, as the Effect filter picks it and [`LayerInfo::effect_kinds`] reports it,
/// in Photoshop's order. `Overlay` stands for Color, Gradient and Pattern Overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum EffectKind {
    /// Bevel & Emboss.
    #[default]
    BevelEmboss,
    /// Stroke.
    Stroke,
    /// Inner Shadow.
    InnerShadow,
    /// Inner Glow.
    InnerGlow,
    /// Satin.
    Satin,
    /// Color, Gradient or Pattern Overlay.
    Overlay,
    /// Outer Glow.
    OuterGlow,
    /// Drop Shadow.
    DropShadow,
}

impl EffectKind {
    /// Every kind, in the Effect select's order.
    pub const ALL: [EffectKind; 8] = [
        EffectKind::BevelEmboss,
        EffectKind::Stroke,
        EffectKind::InnerShadow,
        EffectKind::InnerGlow,
        EffectKind::Satin,
        EffectKind::Overlay,
        EffectKind::OuterGlow,
        EffectKind::DropShadow,
    ];

    fn label(self, words: &LayerFilterWords) -> &Label {
        match self {
            EffectKind::BevelEmboss => &words.bevel_emboss,
            EffectKind::Stroke => &words.stroke,
            EffectKind::InnerShadow => &words.inner_shadow,
            EffectKind::InnerGlow => &words.inner_glow,
            EffectKind::Satin => &words.satin,
            EffectKind::Overlay => &words.overlay,
            EffectKind::OuterGlow => &words.outer_glow,
            EffectKind::DropShadow => &words.drop_shadow,
        }
    }
}

/// A layer attribute the Attribute filter picks, or its opposite: Photoshop's nine attributes,
/// then their opposites. Each opposite lists the layers (and groups) its attribute doesn't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum LayerAttribute {
    /// The layer's own eye is on.
    #[default]
    Visible,
    /// Any lock is on, the Background's too.
    Locked,
    /// A pixel layer with no visible pixels: only the app knows ([`LayerTree::listed`]); the
    /// generic rule counts no layer as empty.
    Empty,
    /// Linked to other layers.
    Linked,
    /// Clipped to the layer below (the clipped layer, not its base).
    Clipped,
    /// Has a pixel mask.
    LayerMask,
    /// Has a vector mask; a shape layer's outline counts as one.
    VectorMask,
    /// Has effects, also when every one is turned off.
    LayerEffects,
    /// An Advanced Blending option is away from its default.
    AdvancedBlending,
    /// Not [`Visible`](Self::Visible).
    NotVisible,
    /// Not [`Locked`](Self::Locked).
    NotLocked,
    /// Not [`Empty`](Self::Empty).
    NotEmpty,
    /// Not [`Linked`](Self::Linked).
    NotLinked,
    /// Not [`Clipped`](Self::Clipped).
    NotClipped,
    /// Not [`LayerMask`](Self::LayerMask).
    NoLayerMask,
    /// Not [`VectorMask`](Self::VectorMask).
    NoVectorMask,
    /// Not [`LayerEffects`](Self::LayerEffects).
    NoLayerEffects,
    /// Not [`AdvancedBlending`](Self::AdvancedBlending).
    NoAdvancedBlending,
}

impl LayerAttribute {
    /// Every attribute, in the Attribute select's order.
    pub const ALL: [LayerAttribute; 18] = [
        LayerAttribute::Visible,
        LayerAttribute::Locked,
        LayerAttribute::Empty,
        LayerAttribute::Linked,
        LayerAttribute::Clipped,
        LayerAttribute::LayerMask,
        LayerAttribute::VectorMask,
        LayerAttribute::LayerEffects,
        LayerAttribute::AdvancedBlending,
        LayerAttribute::NotVisible,
        LayerAttribute::NotLocked,
        LayerAttribute::NotEmpty,
        LayerAttribute::NotLinked,
        LayerAttribute::NotClipped,
        LayerAttribute::NoLayerMask,
        LayerAttribute::NoVectorMask,
        LayerAttribute::NoLayerEffects,
        LayerAttribute::NoAdvancedBlending,
    ];

    /// The attribute this one names and whether it is its opposite: `NotLocked` is `(Locked, true)`.
    pub const fn base(self) -> (LayerAttribute, bool) {
        match self {
            LayerAttribute::NotVisible => (LayerAttribute::Visible, true),
            LayerAttribute::NotLocked => (LayerAttribute::Locked, true),
            LayerAttribute::NotEmpty => (LayerAttribute::Empty, true),
            LayerAttribute::NotLinked => (LayerAttribute::Linked, true),
            LayerAttribute::NotClipped => (LayerAttribute::Clipped, true),
            LayerAttribute::NoLayerMask => (LayerAttribute::LayerMask, true),
            LayerAttribute::NoVectorMask => (LayerAttribute::VectorMask, true),
            LayerAttribute::NoLayerEffects => (LayerAttribute::LayerEffects, true),
            LayerAttribute::NoAdvancedBlending => (LayerAttribute::AdvancedBlending, true),
            other => (other, false),
        }
    }

    /// Whether `info` has this attribute (or, for an opposite, lacks its attribute).
    pub fn shows(self, info: &LayerInfo) -> bool {
        // pc: crates/ui-egui/src/panels/layer_filter_ui/attribute.rs `Attribute::shows`.
        let (base, opposite) = self.base();
        let has = match base {
            LayerAttribute::Visible => info.visible,
            LayerAttribute::Locked => info.locks.any(),
            LayerAttribute::Linked => info.linked,
            LayerAttribute::Clipped => info.clipped,
            LayerAttribute::LayerMask => info.masks.iter().any(|mask| mask.part == LayerPart::LayerMask),
            LayerAttribute::VectorMask => info.kind == LayerKind::Shape || info.masks.iter().any(|mask| mask.part == LayerPart::VectorMask),
            LayerAttribute::LayerEffects => info.has_effects,
            LayerAttribute::AdvancedBlending => info.advanced_blending,
            _ => false,
        };
        has != opposite
    }

    fn label(self, words: &LayerFilterWords) -> &Label {
        match self {
            LayerAttribute::Visible => &words.visible,
            LayerAttribute::Locked => &words.locked,
            LayerAttribute::Empty => &words.empty,
            LayerAttribute::Linked => &words.linked,
            LayerAttribute::Clipped => &words.clipped,
            LayerAttribute::LayerMask => &words.layer_mask,
            LayerAttribute::VectorMask => &words.vector_mask,
            LayerAttribute::LayerEffects => &words.layer_effects,
            LayerAttribute::AdvancedBlending => &words.advanced_blending,
            LayerAttribute::NotVisible => &words.not_visible,
            LayerAttribute::NotLocked => &words.not_locked,
            LayerAttribute::NotEmpty => &words.not_empty,
            LayerAttribute::NotLinked => &words.not_linked,
            LayerAttribute::NotClipped => &words.not_clipped,
            LayerAttribute::NoLayerMask => &words.no_layer_mask,
            LayerAttribute::NoVectorMask => &words.no_vector_mask,
            LayerAttribute::NoLayerEffects => &words.no_layer_effects,
            LayerAttribute::NoAdvancedBlending => &words.no_advanced_blending,
        }
    }
}

/// Where a smart object's contents come from, as [`LayerInfo::smart`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SmartSource {
    /// A file outside the document.
    Linked,
    /// Kept in the document.
    Embedded,
}

/// A kind of smart object the Smart Object filter toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SmartKind {
    /// Linked to a file.
    Linked,
    /// Linked to a file that is not there: only the app knows ([`LayerTree::listed`]); the generic
    /// rule lists none.
    Missing,
    /// Embedded in the document.
    Embedded,
}

impl SmartKind {
    /// Every kind, in the filter bar's order.
    pub const ALL: [SmartKind; 3] = [SmartKind::Linked, SmartKind::Missing, SmartKind::Embedded];
}

/// The filter's criteria. The app stores it (PhotoCraft: in `UiState`, not saved with the
/// document) and passes it to [`LayersPanel::filter`](super::panel::LayersPanel::filter); every
/// field has a serde default. Each type reads its own field; the others keep their values while
/// another type is picked.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct LayerFilter {
    /// The filter type.
    pub by: FilterBy,
    /// Kind: the kinds toggled on, in the order they were toggled.
    pub kinds: Vec<FilterKind>,
    /// Name: the text as typed; matched trimmed and ignoring case.
    pub name: String,
    /// Effect: the effect kind; Bevel & Emboss at first.
    pub effect: EffectKind,
    /// Mode: the blend mode's app key; `"normal"` at first.
    pub mode: String,
    /// Attribute: the attribute; Visible at first.
    pub attribute: LayerAttribute,
    /// Color: the label colour; `None` (no label) at first. Serialized by name (`"none"`, `"red"`,
    /// … `"gray"`).
    #[serde(with = "label_name")]
    pub color: Option<LabelColor>,
    /// Smart Object: the kinds toggled on, in the order they were toggled.
    pub smart: Vec<SmartKind>,
}

impl Default for LayerFilter {
    /// Kind, no kinds toggled, no name, mode `"normal"`: shows every layer.
    fn default() -> Self {
        Self {
            by: FilterBy::Kind,
            kinds: Vec::new(),
            name: String::new(),
            effect: EffectKind::default(),
            mode: String::from("normal"),
            attribute: LayerAttribute::default(),
            color: None,
            smart: Vec::new(),
        }
    }
}

impl LayerFilter {
    /// Whether the panel is filtered. As Photoshop, Effect, Mode, Attribute, Color, Artboard and
    /// Group always filter (by Bevel & Emboss, Normal, Visible and no colour at first); Kind and
    /// Smart Object only once a toggle is on, Name only once the trimmed text is not empty.
    /// Artboard lists every layer while no artboard holds the selection ([`LayerFilter::lists`]).
    pub fn active(&self) -> bool {
        // pc: crates/ui-egui/src/panels/layer_filter_ui.rs `Filter::active`.
        match self.by {
            FilterBy::Kind => !self.kinds.is_empty(),
            FilterBy::Name => !self.name.trim().is_empty(),
            FilterBy::SmartObject => !self.smart.is_empty(),
            FilterBy::Effect | FilterBy::Mode | FilterBy::Attribute | FilterBy::Color | FilterBy::Artboard | FilterBy::Group => true,
        }
    }

    /// Whether the filtered panel lists the contents of closed groups, so a match inside one shows:
    /// every type except Artboard and Group, which list whole groups and keep each one open or
    /// closed as it is.
    pub fn lists_closed_contents(&self) -> bool {
        !matches!(self.by, FilterBy::Artboard | FilterBy::Group)
    }

    /// Whether a layer is listed under this filter, from its row alone: always when the filter
    /// isn't [`active`](Self::active). Kind: its [`LayerKind::filter_kind`] is one of `kinds` (a
    /// group matches none). Name: `info.name` lowercased contains the trimmed `name` lowercased.
    /// Effect: `effect` is one of its [`effect_kinds`](LayerInfo::effect_kinds). Mode:
    /// `info.blend_key == mode`. Attribute: [`LayerAttribute::shows`]. Color: its label is
    /// `color`. Smart Object: its [`smart`](LayerInfo::smart) source is toggled on (Missing lists
    /// none here). Artboard and Group read the tree, so this lists every layer for them; use
    /// [`LayerFilter::lists`].
    pub fn shows(&self, info: &LayerInfo) -> bool {
        // pc: crates/ui-egui/src/panels/layer_filter_ui.rs `Filter::shows`.
        if !self.active() {
            return true;
        }
        match self.by {
            FilterBy::Kind => info.kind.filter_kind().is_some_and(|kind| self.kinds.contains(&kind)),
            FilterBy::Name => info.name.to_lowercase().contains(&self.name.trim().to_lowercase()),
            FilterBy::Effect => info.effect_kinds.contains(&self.effect),
            FilterBy::Mode => info.blend_key == self.mode,
            FilterBy::Attribute => self.attribute.shows(info),
            FilterBy::Color => info.label == self.color,
            FilterBy::SmartObject => match info.smart {
                Some(SmartSource::Linked) => self.smart.contains(&SmartKind::Linked),
                Some(SmartSource::Embedded) => self.smart.contains(&SmartKind::Embedded),
                None => false,
            },
            FilterBy::Artboard | FilterBy::Group => true,
        }
    }

    /// Whether `tree` lists `id` under this filter, from what the tree tells: the default of
    /// [`LayerTree::listed`]. Group: the layer is a group or inside one (an artboard is not a
    /// group here, but the groups inside it are). Artboard: the layer is in the outermost artboard
    /// of a selected layer; every layer while nothing is selected or a selected layer is in no
    /// artboard. It reads the selection for each layer, so an app with large trees computes the
    /// artboards once in its own `listed`. Every other type: [`LayerFilter::shows`].
    pub fn lists<T: LayerTree + ?Sized>(&self, tree: &T, id: &T::Id) -> bool {
        // pc: crates/ui-egui/src/panels/layer_filter_ui/group.rs `listed`, layer_filter_ui.rs `board_rows`.
        match self.by {
            FilterBy::Group => {
                let mut at = Some(id.clone());
                while let Some(layer) = at {
                    if tree.info(&layer).kind == LayerKind::Group {
                        return true;
                    }
                    at = tree.parent(&layer);
                }
                false
            }
            FilterBy::Artboard => {
                let held: Option<Vec<T::Id>> = tree.selection().layers.iter().map(|layer| outer_artboard(tree, layer)).collect();
                match held {
                    Some(boards) if !boards.is_empty() => outer_artboard(tree, id).is_some_and(|board| boards.contains(&board)),
                    _ => true,
                }
            }
            _ => self.shows(&tree.info(id)),
        }
    }
}

/// The outermost artboard that holds `id`, or `id` itself when it is one.
fn outer_artboard<T: LayerTree + ?Sized>(tree: &T, id: &T::Id) -> Option<T::Id> {
    let mut found = None;
    let mut at = Some(id.clone());
    while let Some(layer) = at {
        at = tree.parent(&layer);
        if tree.info(&layer).kind == LayerKind::Artboard {
            found = Some(layer);
        }
    }
    found
}

/// [`LayerFilter::color`] as a label colour's name, `"none"` for no label.
mod label_name {
    use craft_theme::LabelColor;
    use serde::{Deserialize as _, Deserializer, Serializer, de::Error as _};

    pub(super) fn serialize<S: Serializer>(color: &Option<LabelColor>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(color.map_or("none", LabelColor::name))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<LabelColor>, D::Error> {
        let name = String::deserialize(deserializer)?;
        if name == "none" {
            return Ok(None);
        }
        LabelColor::ALL
            .into_iter()
            .find(|color| color.name() == name)
            .map(Some)
            .ok_or_else(|| D::Error::custom(format!("unknown label colour `{name}`")))
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
        effect: "layerFilterBar.effect" = "Effect";
        mode: "layerFilterBar.mode" = "Mode";
        attribute: "layerFilterBar.attribute" = "Attribute";
        color: "layerFilterBar.color" = "Color";
        smart_objects: "layerFilterBar.smartObjects" = "Smart Object";
        artboard: "layerFilterBar.artboard" = "Artboard";
        group: "layerFilterBar.group" = "Group";
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
        /// The effect select's name.
        filter_effect: "layerFilterBar.filterEffect" = "Filter effect";
        /// The effect select's entries.
        bevel_emboss: "layerFilterBar.bevelEmboss" = "Bevel & Emboss";
        stroke: "layerFilterBar.stroke" = "Stroke";
        inner_shadow: "layerFilterBar.innerShadow" = "Inner Shadow";
        inner_glow: "layerFilterBar.innerGlow" = "Inner Glow";
        satin: "layerFilterBar.satin" = "Satin";
        overlay: "layerFilterBar.overlay" = "Overlay";
        outer_glow: "layerFilterBar.outerGlow" = "Outer Glow";
        drop_shadow: "layerFilterBar.dropShadow" = "Drop Shadow";
        /// The mode select's name.
        blend_mode: "layerFilterBar.blendMode" = "Filter blend mode";
        /// The attribute select's name.
        filter_attribute: "layerFilterBar.filterAttribute" = "Filter attribute";
        /// The attribute select's entries.
        visible: "layerFilterBar.visible" = "Visible";
        locked: "layerFilterBar.locked" = "Locked";
        empty: "layerFilterBar.empty" = "Empty";
        linked: "layerFilterBar.linked" = "Linked";
        clipped: "layerFilterBar.clipped" = "Clipped";
        layer_mask: "layerFilterBar.layerMask" = "Layer Mask";
        vector_mask: "layerFilterBar.vectorMask" = "Vector Mask";
        layer_effects: "layerFilterBar.layerEffects" = "Layer Effects";
        advanced_blending: "layerFilterBar.advancedBlending" = "Advanced Blending";
        not_visible: "layerFilterBar.notVisible" = "Not Visible";
        not_locked: "layerFilterBar.notLocked" = "Not Locked";
        not_empty: "layerFilterBar.notEmpty" = "Not Empty";
        not_linked: "layerFilterBar.notLinked" = "Not Linked";
        not_clipped: "layerFilterBar.notClipped" = "Not Clipped";
        no_layer_mask: "layerFilterBar.noLayerMask" = "No Layer Mask";
        no_vector_mask: "layerFilterBar.noVectorMask" = "No Vector Mask";
        no_layer_effects: "layerFilterBar.noLayerEffects" = "No Layer Effects";
        no_advanced_blending: "layerFilterBar.noAdvancedBlending" = "No Adv. Blending";
        /// The colour select's name.
        filter_color: "layerFilterBar.filterColor" = "Filter color";
        /// The smart object toggles' tooltips and names.
        smart_linked: "layerFilterBar.smartLinked" = "Filter for smart objects linked locally";
        smart_missing: "layerFilterBar.smartMissing" = "Filter for linked smart objects that are missing";
        smart_embedded: "layerFilterBar.smartEmbedded" = "Filter for embedded smart objects";
    }
}

/// PhotoCraft's filter row: the type select (Kind, Name, Effect, Mode, Attribute, Color, Smart
/// Object, Artboard, Group), then that type's controls.
///
/// ```ignore
/// let out = LayerFilterBar::new(Bind::bound(&mut ui_state.layer_filter)).blend_modes(&modes).show(ui);
/// if out.changed { /* the app stores the filter; the panel reads it next time it's drawn */ }
/// ```
///
/// - **Kind**: five icon toggle buttons in [`FilterKind::ALL`] order, pressed while on; a click or
///   Space adds or removes the kind.
/// - **Name**: a search field named "Search layers", filtering as the user types; Escape clears.
///   [`focus_name`](Self::focus_name) puts the caret in it with its text selected (Find Layers).
/// - **Effect**, **Attribute**, **Color**: a [`Select`] of [`EffectKind::ALL`],
///   [`LayerAttribute::ALL`], or None and the label colours.
/// - **Mode**: a [`Select`] with `blend_modes` (the layer modes, without Pass Through: a group set
///   to Pass Through is never listed by Mode).
/// - **Smart Object**: three icon toggle buttons in [`SmartKind::ALL`] order, as Kind's.
/// - **Artboard**, **Group**: no controls.
/// - **Keyboard**: each control is its own Tab stop; a select opens with Space or Alt+Down.
#[must_use = "show the filter bar with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct LayerFilterBar<'a> {
    filter: Bind<'a, LayerFilter>,
    /// The modes the Mode select offers, in order; usually [`LayerTree::blend_modes`] without
    /// groups, so Pass Through is not in it.
    #[prop]
    blend_modes: &'a [Entry<String>],
    /// Find Layers, for this pass: the bar switches to Name and its field takes the focus with its
    /// text selected, so typing replaces it.
    #[prop]
    focus_name: bool,
}

/// What [`LayerFilterBar::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host stores a changed filter"]
pub struct LayerFilterOutput {
    /// The bar's row.
    pub response: Response,
    /// The criteria changed this pass (a toggle, a typed letter, a pick, the type select).
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

/// Whether the Name field selects its text as it takes focus: set in the pass of a find, for the
/// pass after it, where the field first has the focus the find asked for.
#[derive(Clone, Default)]
struct FindName {
    select: bool,
}

impl<'a> LayerFilterBar<'a> {
    /// The bar editing `filter`.
    pub fn new(filter: Bind<'a, LayerFilter>) -> Self {
        Self { filter, blend_modes: &[], focus_name: false }
    }

    /// Draws the bar on one row of the available width.
    pub fn show(self, ui: &mut Ui) -> LayerFilterOutput {
        let Self { mut filter, blend_modes, focus_name } = self;
        let words = Env::of(ui).words::<LayerFilterWords>();
        let size = ControlSize::Sm;
        let mut edit = None;
        let row = Row::new().show(ui, |ui, cx| {
            let select_name = std::mem::replace(&mut cx.memory::<FindName>().select, focus_name);
            let Some(Mixed::Same(current)) = filter.reborrow().read(cx) else { return };
            let mut next = current.clone();
            if focus_name {
                next.by = FilterBy::Name;
            }
            let types: Vec<Entry<FilterBy>> = FilterBy::ALL.into_iter().map(|by| Entry::new(by, type_label(by, &words).clone())).collect();
            let picked = Select::new(words.filter_type.clone(), Bind::given(next.by), &types).size(size).show(ui);
            if let Some(by) = picked.value() {
                next.by = *by;
            }
            // Controls follow the type picked before this pass; a find shows the Name field at once.
            let shown = if focus_name { FilterBy::Name } else { current.by };
            match shown {
                FilterBy::Kind => {
                    let kinds = [
                        (FilterKind::Pixel, "image", &words.pixel),
                        (FilterKind::Adjustment, "adjustment_layer", &words.adjustment),
                        (FilterKind::Type, "text", &words.type_layers),
                        (FilterKind::Shape, "square", &words.shape),
                        (FilterKind::SmartObject, "app_window", &words.smart_object),
                    ];
                    toggles(ui, size, &kinds, FilterKind::name, &mut next.kinds);
                }
                FilterBy::Name => {
                    let mut field = TextField::search(words.search_layers.clone(), Bind::given(current.name.clone()))
                        .placeholder(words.search.clone())
                        .size(size);
                    if focus_name || select_name {
                        field = field.select_on_focus();
                    }
                    let typed = field.show(ui);
                    if let Some(text) = typed.value() {
                        next.name.clone_from(text);
                    }
                    if focus_name {
                        focus::request(ui.ctx(), FocusTarget::Id(typed.response.id));
                    }
                }
                FilterBy::Effect => {
                    let effects: Vec<Entry<EffectKind>> =
                        EffectKind::ALL.into_iter().map(|kind| Entry::new(kind, kind.label(&words).clone())).collect();
                    let picked = Select::new(words.filter_effect.clone(), Bind::given(current.effect), &effects).size(size).show(ui);
                    if let Some(effect) = picked.value() {
                        next.effect = *effect;
                    }
                }
                FilterBy::Mode => {
                    let picked = Select::new(words.blend_mode.clone(), Bind::given(current.mode.clone()), blend_modes).size(size).show(ui);
                    if let Some(mode) = picked.value() {
                        next.mode.clone_from(mode);
                    }
                }
                FilterBy::Attribute => {
                    let attributes: Vec<Entry<LayerAttribute>> =
                        LayerAttribute::ALL.into_iter().map(|attribute| Entry::new(attribute, attribute.label(&words).clone())).collect();
                    let picked = Select::new(words.filter_attribute.clone(), Bind::given(current.attribute), &attributes).size(size).show(ui);
                    if let Some(attribute) = picked.value() {
                        next.attribute = *attribute;
                    }
                }
                FilterBy::Color => {
                    let list_words = Env::of(ui).words::<ListWords>();
                    let colors: Vec<Entry<Option<LabelColor>>> = std::iter::once(None)
                        .chain(LabelColor::ALL.into_iter().map(Some))
                        .map(|color| Entry::new(color, label_color_label(color, &list_words)))
                        .collect();
                    let picked = Select::new(words.filter_color.clone(), Bind::given(current.color), &colors).size(size).show(ui);
                    if let Some(color) = picked.value() {
                        next.color = *color;
                    }
                }
                FilterBy::SmartObject => {
                    let kinds = [
                        (SmartKind::Linked, "link", &words.smart_linked),
                        (SmartKind::Missing, "unlink", &words.smart_missing),
                        (SmartKind::Embedded, "file", &words.smart_embedded),
                    ];
                    toggles(ui, size, &kinds, smart_salt, &mut next.smart);
                }
                // The selected layers pick the artboards; Group lists every group.
                FilterBy::Artboard | FilterBy::Group => {}
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

fn type_label(by: FilterBy, words: &LayerFilterWords) -> &Label {
    match by {
        FilterBy::Kind => &words.kind,
        FilterBy::Name => &words.name,
        FilterBy::Effect => &words.effect,
        FilterBy::Mode => &words.mode,
        FilterBy::Attribute => &words.attribute,
        FilterBy::Color => &words.color,
        FilterBy::SmartObject => &words.smart_objects,
        FilterBy::Artboard => &words.artboard,
        FilterBy::Group => &words.group,
    }
}

const fn smart_salt(kind: SmartKind) -> &'static str {
    match kind {
        SmartKind::Linked => "linked",
        SmartKind::Missing => "missing",
        SmartKind::Embedded => "embedded",
    }
}

/// One icon toggle button per `(value, icon, name)`, pressed while `on` holds the value; a toggle
/// adds the value to `on` or takes it out.
fn toggles<K: Copy + PartialEq>(ui: &mut Ui, size: ControlSize, buttons: &[(K, &'static str, &Label)], salt: fn(K) -> &'static str, on: &mut Vec<K>) {
    let shown = on.clone();
    for &(value, icon, name) in buttons {
        let toggled = ToggleButton::new(name.clone(), Bind::given(shown.contains(&value)))
            .icon(IconId::new(icon))
            .icon_only()
            .size(size)
            .id_salt(salt(value))
            .show(ui);
        match toggled.value() {
            Some(true) => on.push(value),
            Some(false) => on.retain(|held| *held != value),
            None => {}
        }
    }
}

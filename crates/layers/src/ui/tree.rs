//! `LayerTree`, the trait an app implements to show its layers, and its data (`LayerInfo`,
//! `LayerKind`, `LayerSelection`, `LayerRowKey`, `SubRows`, `LayerProperties`, `LayersFooterState`).
//!
//! The trait is a read-only view of the app's document: the panel never changes it. What the user
//! does comes back as [`LayersEvent`](super::panel::LayersEvent)s, which the app applies with its
//! own commands and undo.

use std::fmt::Debug;
use std::hash::Hash;

use craft_behavior::IconId;
use craft_keymap::Mods;
use craft_theme::LabelColor;
use craft_ui::contract::{Entry, ReorderMove};

/// The app's view of its layers, built per pass over its document (no copies of the layers).
///
/// ```ignore
/// struct PhotoLayers<'a> { doc: &'a Document, ui: &'a UiState }
/// impl LayerTree for PhotoLayers<'_> {
///     type Id = LayerId;
///     fn children(&self, parent: Option<&LayerId>) -> Vec<LayerId> { /* top of the stack first */ }
///     /* … */
/// }
/// ```
///
/// The panel calls [`info`](Self::info) only for the rows it draws (the rows in view and one
/// above and below), so a tree of 10,000 layers costs what its visible rows cost.
/// [`children`](Self::children) and [`is_open`](Self::is_open) are walked once per pass to build
/// the display order, unless [`revision`](Self::revision) says nothing changed.
pub trait LayerTree {
    /// The app's layer id: the key of a row, of its thumbnails and of every event.
    type Id: Clone + Eq + Hash + Debug + Send + Sync + 'static;

    /// Children of `parent` (`None`: the top level), topmost first. The app leaves out what the
    /// panel never shows: layers outside an Isolate Layers set, a Type Mask session's working text.
    fn children(&self, parent: Option<&Self::Id>) -> Vec<Self::Id>;

    /// The group `id` is in; `None` at the top level.
    fn parent(&self, id: &Self::Id) -> Option<Self::Id>;

    /// Whether a group shows its children: a view state the app stores (PhotoCraft keeps it in
    /// the document). Not called for layers without children.
    fn is_open(&self, id: &Self::Id) -> bool;

    /// Whether a layer shows its effects and Smart Filter rows (one flag for both, PhotoCraft).
    fn effects_open(&self, id: &Self::Id) -> bool;

    /// What the row of `id` shows. Cheap: called only for rows in view.
    fn info(&self, id: &Self::Id) -> LayerInfo;

    /// The selected layers, the active one and the part edits go to.
    fn selection(&self) -> LayerSelection<Self::Id>;

    /// The effects or Smart Filter rows of a layer; `None` when it has none of that group.
    fn sub_rows(&self, id: &Self::Id, group: SubRowGroup) -> Option<SubRows>;

    /// The active layer's properties for the blend, opacity, lock and fill rows; `None` with no
    /// active layer (the rows grey out at Normal, 100 % and unlocked).
    fn properties(&self) -> Option<LayerProperties>;

    /// Blend modes in menu order, with separators. `group` adds Pass Through; `current` is the
    /// layer's own mode, kept in the list when the app's standard list lacks it (PhotoCraft's
    /// Paint.NET modes).
    fn blend_modes(&self, group: bool, current: &str) -> Vec<Entry<String>>;

    /// What the footer shows: the mask and delete wording, what is enabled, the two menus.
    fn footer(&self) -> LayersFooterState;

    /// Whether a drag of rows may drop here. A drop the app refuses shows no marker and takes
    /// nothing (PhotoCraft: nothing goes below the Background; a multi-selection is not copied
    /// onto one of its own rows; DesignCraft: a locked or hidden layer takes drops only with
    /// Cmd/Ctrl, read from `mods`). Accepts every drop by default.
    fn accepts_drop(&self, mv: &ReorderMove<LayerRowKey<Self::Id>>, mods: Mods) -> bool {
        let _ = (mv, mods);
        true
    }

    /// The layer under the pointer on the canvas, drawn with the list's hover look.
    fn hovered(&self) -> Option<Self::Id> {
        None
    }

    /// A number that changes whenever rows are added, removed, moved, opened or closed, or an
    /// effects or Smart Filters row is added or removed; with it
    /// the panel keeps its display order between passes instead of walking the tree each pass.
    /// `None` (the default) walks every pass.
    fn revision(&self) -> Option<u64> {
        None
    }
}

/// What one layer row shows. Build it with [`LayerInfo::new`] and set the fields that differ.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayerInfo {
    /// The layer's name, shown on one line with an ellipsis.
    pub name: String,
    /// What the layer is: picks the kind tile of groups, adjustments and type layers, the filter
    /// kind and the row's description.
    pub kind: LayerKind,
    /// Whether the layer's own eye is on: off leaves the eye cell empty.
    pub visible: bool,
    /// Whether the row draws faint: hidden itself or inside a hidden group (the app decides).
    pub dimmed: bool,
    /// The label colour that fills the eye cell; `None` for none.
    pub label: Option<LabelColor>,
    /// The layer's locks: any of them shows the lock indicator.
    pub locks: LayerLocks,
    /// The Background: it can't move below another layer, isn't renamed in place (a double click
    /// on its name reports [`ActivateOn::Name`](super::panel::ActivateOn::Name)) and greys blend,
    /// opacity and fill.
    pub background: bool,
    /// Whether a double click or F2 renames the layer in place.
    pub renamable: bool,
    /// Clipped to the layer below: the row's clip arrow and indent.
    pub clipped: bool,
    /// Linked to other layers: the link indicator.
    pub linked: bool,
    /// The app's key of the layer's blend mode.
    pub blend_key: String,
    /// The blend label at the row's trailing end: `Some` when the mode is neither Normal nor Pass
    /// Through. Dropped first when the name would get narrower than `list.min_label`.
    pub blend_label: Option<String>,
    /// The layer's masks, pixel mask first, then the vector mask; at most one of each.
    pub masks: Vec<MaskInfo>,
    /// The layer has effects: the `fx` indicator and the effects triangle.
    pub has_effects: bool,
    /// The layer is a smart object with Smart Filters: their badge.
    pub has_smart_filters: bool,
    /// Whether the Smart Filters are on (their header row's eye).
    pub smart_filters_enabled: bool,
    /// A warning shown as an icon at the trailing end with this text as its tooltip (PhotoCraft:
    /// "Not rendered: this adjustment … is kept as imported.").
    pub warning: Option<String>,
    /// Whether Cmd/Ctrl-click on the layer thumbnail loads it as a selection; false for the
    /// Background, where Cmd-click is a plain click.
    pub thumb_loads_selection: bool,
    /// The app's own words, added to the row's description for a screen reader.
    pub description: Option<String>,
}

impl LayerInfo {
    /// A visible, renamable, unlocked layer named `name` of `kind`, Normal, with no masks,
    /// effects or warning.
    pub fn new(name: impl Into<String>, kind: LayerKind) -> Self {
        Self {
            name: name.into(),
            kind,
            visible: true,
            dimmed: false,
            label: None,
            locks: LayerLocks::default(),
            background: false,
            renamable: true,
            clipped: false,
            linked: false,
            blend_key: String::from("normal"),
            blend_label: None,
            masks: Vec::new(),
            has_effects: false,
            has_smart_filters: false,
            smart_filters_enabled: true,
            warning: None,
            thumb_loads_selection: true,
            description: None,
        }
    }
}

/// What a layer is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayerKind {
    /// Pixels (PhotoCraft's raster layer).
    #[default]
    Pixel,
    /// A group: a folder tile.
    Group,
    /// An adjustment layer: the adjustment tile.
    Adjustment,
    /// A fill layer (solid colour, gradient, pattern): the app renders its picture.
    Fill,
    /// A type layer: the "T" tile.
    Text,
    /// A shape layer.
    Shape,
    /// A smart object.
    SmartObject,
    /// An artboard (rows for artboards are step 6b's; the kind exists for apps that list them).
    Artboard,
    /// A path (VectorCraft).
    Path,
    /// A placed image (VectorCraft, DesignCraft).
    Image,
    /// Any other object row (DesignCraft's object rows under a layer).
    Object,
}

/// The part of a layer edits go to, and the thumbnail that stands for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayerPart {
    /// The layer's own pixels.
    #[default]
    Pixels,
    /// The pixel mask.
    LayerMask,
    /// The vector mask.
    VectorMask,
    /// A smart object's shared Smart Filter mask.
    FilterMask,
}

/// One mask of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct MaskInfo {
    /// [`LayerPart::LayerMask`] or [`LayerPart::VectorMask`].
    pub part: LayerPart,
    /// A disabled mask is crossed out in red.
    pub enabled: bool,
    /// Whether the chain before the mask thumbnail shows linked (an unlinked chain shows only on
    /// hover).
    pub linked: bool,
}

/// A layer's locks. Start from `LayerLocks::default()` (nothing locked) and set the fields that are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct LayerLocks {
    /// Lock transparent pixels.
    pub transparency: bool,
    /// Lock image pixels (which disables Lock transparent pixels).
    pub pixels: bool,
    /// Lock position.
    pub position: bool,
    /// Prevent auto-nesting in and out of artboards and frames.
    pub artboard: bool,
    /// Lock all.
    pub all: bool,
}

impl MaskInfo {
    /// A mask of `part` that is `enabled` and `linked` or not.
    pub fn new(part: LayerPart, enabled: bool, linked: bool) -> Self {
        Self { part, enabled, linked }
    }
}

impl LayerLocks {
    /// Whether any lock is on: the row shows its lock indicator.
    pub fn any(&self) -> bool {
        self.transparency || self.pixels || self.position || self.artboard || self.all
    }

    /// Whether `kind` is on.
    pub fn get(&self, kind: LockKind) -> bool {
        match kind {
            LockKind::Transparency => self.transparency,
            LockKind::Pixels => self.pixels,
            LockKind::Position => self.position,
            LockKind::Artboard => self.artboard,
            LockKind::All => self.all,
        }
    }
}

/// One of the five locks, in the lock row's order (Photoshop's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LockKind {
    /// Lock transparent pixels.
    Transparency,
    /// Lock image pixels.
    Pixels,
    /// Lock position.
    Position,
    /// Prevent auto-nesting in and out of artboards and frames.
    Artboard,
    /// Lock all.
    All,
}

impl LockKind {
    /// Every lock, in the lock row's order.
    pub const ALL: [LockKind; 5] = [LockKind::Transparency, LockKind::Pixels, LockKind::Position, LockKind::Artboard, LockKind::All];
}

/// The selection as the app keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerSelection<Id> {
    /// The selected layers, in the order they were selected.
    pub layers: Vec<Id>,
    /// The active layer: the one the property rows show and whose target is framed.
    pub active: Option<Id>,
    /// The active layer's part edits go to: its thumbnail gets the target brackets.
    pub target: LayerPart,
}

impl<Id> LayerSelection<Id> {
    /// The `layers` selected, `active` the one the property rows show, edits going to `target`.
    pub fn new(layers: Vec<Id>, active: Option<Id>, target: LayerPart) -> Self {
        Self { layers, active, target }
    }
}

impl<Id> Default for LayerSelection<Id> {
    fn default() -> Self {
        Self { layers: Vec::new(), active: None, target: LayerPart::Pixels }
    }
}

/// The key of one row of the panel, and of what a drag carries.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayerRowKey<Id> {
    /// A layer's row.
    Layer(Id),
    /// An effects row of a layer: `None` is the "Effects" header row, `Some(i)` the `i`-th
    /// effect, top first.
    Effects(Id, Option<usize>),
    /// A Smart Filters row: `None` is the header row, `Some(i)` the `i`-th filter, top first.
    Filters(Id, Option<usize>),
    /// A mask thumbnail being dragged: a drag payload only, never a row. Only the footer's
    /// Delete takes it.
    Mask(Id, LayerPart),
}

impl<Id> LayerRowKey<Id> {
    /// The layer the row belongs to.
    pub fn layer(&self) -> &Id {
        match self {
            LayerRowKey::Layer(id) | LayerRowKey::Effects(id, _) | LayerRowKey::Filters(id, _) | LayerRowKey::Mask(id, _) => id,
        }
    }

    /// Whether this is a layer's own row.
    pub fn is_layer(&self) -> bool {
        matches!(self, LayerRowKey::Layer(_))
    }
}

/// Which sub-rows of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubRowGroup {
    /// The "Effects" row and one row per effect.
    Effects,
    /// The "Smart Filters" row and one row per filter.
    SmartFilters,
}

/// The effects or Smart Filter rows of a layer: a header row, then the items, top first.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SubRows {
    /// The "Effects" or "Smart Filters" row; its eye shows or hides them all.
    pub header: SubRow,
    /// One row per effect or filter, top first.
    pub items: Vec<SubRow>,
}

impl SubRows {
    /// A `header` row and its `items`.
    pub fn new(header: SubRow, items: Vec<SubRow>) -> Self {
        Self { header, items }
    }
}

/// One effects or Smart Filter row.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SubRow {
    /// The row's text: "Effects", "Drop Shadow", "Gaussian Blur".
    pub label: String,
    /// Whether it is on: its eye.
    pub on: bool,
    /// Whether the row has an eye cell at all.
    pub eye_shown: bool,
    /// The icon before the label (Photoshop's sparkles on the "Effects" row).
    pub icon: Option<IconId>,
    /// A Smart Filters header shows the shared filter-mask thumbnail (hidden at thumbnail size
    /// None).
    pub mask: bool,
}

impl SubRow {
    /// An "on" row with an eye, no icon and no mask thumbnail, reading `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), on: true, eye_shown: true, icon: None, mask: false }
    }
}

/// The active layer's properties, for [`BlendRows`](super::rows::BlendRows).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayerProperties {
    /// The blend mode's key.
    pub blend_key: String,
    /// The layer is a group, so Pass Through is offered.
    pub group: bool,
    /// Opacity, 0 to 100.
    pub opacity: f64,
    /// Fill opacity, 0 to 100.
    pub fill: f64,
    /// The layer's locks.
    pub locks: LayerLocks,
    /// False for the Background: blend, Opacity and Fill grey out.
    pub editable: bool,
}

impl LayerProperties {
    /// A Normal, 100 %, unlocked, editable layer whose blend mode has the key `blend_key`.
    pub fn new(blend_key: impl Into<String>) -> Self {
        Self { blend_key: blend_key.into(), group: false, opacity: 100.0, fill: 100.0, locks: LayerLocks::default(), editable: true }
    }
}

/// What the footer shows, from [`LayerTree::footer`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayersFooterState {
    /// Link Layers is enabled (two or more layers selected, PhotoCraft).
    pub link_enabled: bool,
    /// The mask button's tooltip and name: "Add layer mask" or "Add vector mask"; `None` greys
    /// the button (a layer with both masks, or no layer).
    pub mask_label: Option<String>,
    /// The Delete button's tooltip and name, by target: "Delete layer", "Delete Layer Mask",
    /// "Delete Vector Mask".
    pub delete_label: String,
    /// The Delete button is enabled.
    pub delete_enabled: bool,
    /// The fill or adjustment menu: one item per adjustment, a separator, Solid Color…,
    /// Gradient…; a pick comes back as
    /// [`LayersFooterAction::Adjustment`](super::panel::LayersFooterAction::Adjustment).
    pub adjustment_menu: Vec<Entry<String>>,
    /// The layer style (`fx`) menu: Blending Options…, a separator, one item per style kind; a
    /// pick comes back as [`LayersFooterAction::Style`](super::panel::LayersFooterAction::Style).
    pub style_menu: Vec<Entry<String>>,
}

impl LayersFooterState {
    /// A footer whose Delete button reads `delete_label`: Link Layers disabled, the mask button
    /// greyed, Delete disabled, both menus empty.
    pub fn new(delete_label: impl Into<String>) -> Self {
        Self {
            link_enabled: false,
            mask_label: None,
            delete_label: delete_label.into(),
            delete_enabled: false,
            adjustment_menu: Vec::new(),
            style_menu: Vec::new(),
        }
    }
}

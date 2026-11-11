//! `LayersPanel`: the whole panel, its output and events (F11.01).
//!
//! The panel shows the app's [`LayerTree`] and reports what the user did as [`LayersEvent`]s in
//! the order they happened; the app applies them with its own commands and undo (owner
//! question 1). It holds no document and no selection of its own.

mod body;
mod context;
mod drag;
mod events;
mod footer;
mod menu;
mod row;
#[cfg(feature = "screen-testing")]
mod stories;

pub use events::{
    ActivateOn, LayerColumn, LayerDrop, LayerMove, LayerPosition, LayerSelect, LayerToggle, LayersEvent, MenuTarget, RowTransfer, SubRowMove,
};
pub use footer::{LayersFooterAction, LayersFooterButton};
pub use menu::{LayersMenuAction, LayersPanelCommands, panel_rows};

use std::cell::RefCell;

use craft_ui::contract::{Bind, NoValue, Outcome, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::list::ListWords;
use craft_ui::primitives::textures::{ThumbSource, Thumbnails};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::memory::Remember as _;
use egui::{Response, Ui};

use body::{LayerRows, ToggleSweeps, reveal_plan, show_body, start_rename};
use context::{PanelContext, ThumbHost};
use footer::show_footer;
use menu::{OpenMenu, RowMenu, show_row_menus};

use super::filter::{LayerFilter, LayerFilterBar};
use super::options::LayersSettings;
use super::rows::BlendRows;
use super::thumbs::LayerThumb;
use super::tree::{LayerRowKey, LayerTree};

craft_ui::words! {
    /// The words of the Layers panel's rows, footer and menus. Patterns with `{row}`, `{layer}` and
    /// `{mask}` are whole phrases, so a language puts the name where its grammar wants it.
    pub struct LayersPanelWords {
        list: "layersPanel.list" = "Layers";
        visibility: "layersPanel.visibility" = "Visibility {row}";
        lock_cell: "layersPanel.lockCell" = "Lock {row}";
        unlock: "layersPanel.unlock" = "Unlock {row}";
        unlock_layer: "layersPanel.unlockLayer" = "Unlock Layer";
        link_mask: "layersPanel.linkMask" = "Link {mask} of {layer}";
        unlink_mask: "layersPanel.unlinkMask" = "Unlink {mask} of {layer}";
        expand_effects: "layersPanel.expandEffects" = "Expand effects {layer}";
        collapse_effects: "layersPanel.collapseEffects" = "Collapse effects {layer}";
        smart_filters: "layersPanel.smartFilters" = "Smart Filters";
        effect_of: "layersPanel.effectOf" = "effect of {layer}";
        smart_filter_of: "layersPanel.smartFilterOf" = "Smart Filter of {layer}";
        kind_pixel: "layersPanel.kindPixel" = "pixel layer";
        kind_group: "layersPanel.kindGroup" = "group";
        kind_adjustment: "layersPanel.kindAdjustment" = "adjustment layer";
        kind_fill: "layersPanel.kindFill" = "fill layer";
        kind_text: "layersPanel.kindText" = "type layer";
        kind_shape: "layersPanel.kindShape" = "shape layer";
        kind_smart_object: "layersPanel.kindSmartObject" = "smart object";
        kind_artboard: "layersPanel.kindArtboard" = "artboard";
        kind_path: "layersPanel.kindPath" = "path";
        kind_image: "layersPanel.kindImage" = "image";
        kind_object: "layersPanel.kindObject" = "object";
        clipped: "layersPanel.clipped" = "clipped";
        locked: "layersPanel.locked" = "locked";
        linked: "layersPanel.linked" = "linked";
        layer_mask: "layersPanel.layerMask" = "layer mask";
        vector_mask: "layersPanel.vectorMask" = "vector mask";
        fx: "layersPanel.fx" = "fx";
        disabled_mask: "layersPanel.disabledMask" = "disabled mask";
        has_effects: "layersPanel.hasEffects" = "effects";
        has_smart_filters: "layersPanel.hasSmartFilters" = "Smart Filters";
        hidden: "layersPanel.hidden" = "hidden";
        pixels_targeted: "layersPanel.pixelsTargeted" = "pixels targeted";
        layer_mask_targeted: "layersPanel.layerMaskTargeted" = "layer mask targeted";
        vector_mask_targeted: "layersPanel.vectorMaskTargeted" = "vector mask targeted";
        filter_mask_targeted: "layersPanel.filterMaskTargeted" = "filter mask targeted";
        link_layers: "layersPanel.linkLayers" = "Link layers";
        add_mask: "layersPanel.addMask" = "Add a mask";
        add_style: "layersPanel.addStyle" = "Add a layer style";
        add_adjustment: "layersPanel.addAdjustment" = "Create new fill or adjustment layer";
        new_group: "layersPanel.newGroup" = "Create a new group";
        new_layer: "layersPanel.newLayer" = "Create a new layer";
        filter_options: "layersPanel.filterOptions" = "Filter Options";
        row_menu: "layersPanel.rowMenu" = "Layer";
        label_color_menu: "layersPanel.labelColorMenu" = "Label Color";
    }
}

/// What the panel keeps between passes, in its root slot.
#[derive(Clone)]
pub(super) struct Session<Id> {
    /// The selection anchor the list left, with the layers of the pick that left it.
    anchor: Option<LayerRowKey<Id>>,
    produced: Vec<Id>,
    /// The active layer of the last pass, to reveal it when it changes.
    seen_active: Option<Option<Id>>,
    /// The menu the last context request chose.
    menu: Option<OpenMenu<Id>>,
    /// The eye or lock sweep in progress.
    sweeps: ToggleSweeps,
    /// The filter the panel keeps when the app gives none.
    filter: LayerFilter,
}

impl<Id> Default for Session<Id> {
    fn default() -> Self {
        Session { anchor: None, produced: Vec::new(), seen_active: None, menu: None, sweeps: ToggleSweeps::default(), filter: LayerFilter::default() }
    }
}

/// The Layers panel over the app's [`LayerTree`].
///
/// ```ignore
/// let tree = DocLayers::new(&doc, &ui_state);
/// let out = LayersPanel::new(&tree)
///     .settings(&prefs.layers)
///     .thumbnails(&app.layer_thumbs, &DocLayerThumbs::new(&doc))
///     .filter(Bind::bound(&mut ui_state.layer_filter))
///     .row_menu(|target| layer_menu_entries(&doc, target))
///     .show(ui);
/// for event in out.events { app.apply(event) }
/// ```
///
/// Top to bottom: the filter bar ([`LayerFilterBar`], while Show Filters is on and
/// [`filter_bar`](Self::filter_bar) is not off), the blend, Opacity, Lock and Fill rows
/// ([`BlendRows`] over `tree.properties()`), the rows (a [`List`](craft_ui::list::List) in tree
/// mode filling the height left), the footer. Row heights and thumbnail sizes come from the control
/// size and the Panel Options' thumbnail size.
///
/// - **Keys**: the rows are one Tab stop (the list's keys: arrows, Home, End and Page with Shift;
///   Cmd/Ctrl moves the lead alone; Left and Right close and open a group, Alt for every group;
///   Alt+Up and Down move the layer; Enter activates; the per-OS rename key renames). The panel's
///   commands ([`LayersPanelCommands`]) are the app's: the stacking keys and the effects keys.
/// - **Pointer**: clicks select (Cmd/Ctrl toggles, Shift extends, the empty space clears), the eye
///   and lock columns sweep, Alt-click on an eye shows only that layer, the eye's right-click opens
///   its colour menu, a modified thumbnail click is a [`LayersEvent::ThumbClick`], double clicks
///   rename or activate, rows drag to reorder (Alt copies) and onto the footer.
/// - **Right to left**: the list and the footer mirror.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(LayersPanel<'static, crate::ui::mock::FakeLayers>))]
pub struct LayersPanel<'a, T: LayerTree> {
    tree: &'a T,
    settings: Option<&'a LayersSettings>,
    thumbnails: Option<ThumbHost<'a, T::Id>>,
    filter: Option<Bind<'a, LayerFilter>>,
    lock_column: bool,
    blend_rows: bool,
    filter_bar: bool,
    row_menu: Option<Box<RowMenu<'a, T::Id>>>,
    reveal: Option<T::Id>,
    rename: Option<T::Id>,
}

/// What [`LayersPanel::show`] returns.
#[derive(Debug)]
#[must_use = "the app applies the panel's events"]
#[non_exhaustive]
pub struct LayersOutput<Id> {
    /// What the user did this pass, in order.
    pub events: Vec<LayersEvent<Id>>,
    /// The panel.
    pub response: Response,
}

impl<Id> Outcome for LayersOutput<Id> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a, T: LayerTree> LayersPanel<'a, T> {
    /// The panel showing `tree`.
    pub fn new(tree: &'a T) -> Self {
        Self {
            tree,
            settings: None,
            thumbnails: None,
            filter: None,
            lock_column: false,
            blend_rows: true,
            filter_bar: true,
            row_menu: None,
            reveal: None,
            rename: None,
        }
    }

    /// The panel's persisted section: the Panel Options and Show Filters. Without it the panel uses
    /// [`LayersSettings::default`].
    pub fn settings(mut self, settings: &'a LayersSettings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// The thumbnail cache and the app's source of the rows' pictures (one [`LayerThumb`] key per
    /// thumbnail). Without it the rows draw no pictures.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<LayerThumb<T::Id>>, source: &'a dyn ThumbSource<LayerThumb<T::Id>>) -> Self {
        self.thumbnails = Some(ThumbHost { thumbnails, source });
        self
    }

    /// The filter bar's criteria, which the app stores; the bar edits them. Without it the panel
    /// keeps its own.
    pub fn filter(mut self, filter: Bind<'a, LayerFilter>) -> Self {
        self.filter = Some(filter);
        self
    }

    /// A lock toggle column after the eye (default off): its edits are [`LayersEvent::Toggle`] with
    /// [`LayerColumn::Lock`].
    pub fn lock_column(mut self, lock_column: bool) -> Self {
        self.lock_column = lock_column;
        self
    }

    /// Shows the blend, Opacity, Lock and Fill rows (default on).
    pub fn blend_rows(mut self, blend_rows: bool) -> Self {
        self.blend_rows = blend_rows;
        self
    }

    /// Allows the filter bar (default on; it shows while Show Filters is on). An app that filters
    /// `children()` itself turns it off.
    pub fn filter_bar(mut self, filter_bar: bool) -> Self {
        self.filter_bar = filter_bar;
        self
    }

    /// The host's row menu: called with where it was opened (a layer row, an effects row) each pass
    /// while a row menu is chosen; a pick is [`LayersEvent::Menu`]. A right-click outside the
    /// selection selects the row first.
    pub fn row_menu(mut self, menu: impl Fn(&MenuTarget<T::Id>) -> Vec<craft_ui::contract::Entry<String>> + 'a) -> Self {
        self.row_menu = Some(Box::new(menu));
        self
    }

    /// Reveals `layer` this pass: its closed ancestors open ([`LayersEvent::Open`]) and its row
    /// scrolls into view when hidden. The active layer is revealed by itself when it changes.
    pub fn reveal(mut self, layer: T::Id) -> Self {
        self.reveal = Some(layer);
        self
    }

    /// Opens the rename field of `layer`'s row this pass, where the layer is renamable: the app calls
    /// it in the pass it handles its Rename Layer command from the row menu.
    pub fn start_rename(mut self, layer: T::Id) -> Self {
        self.rename = Some(layer);
        self
    }

    /// Draws the panel over the available rect: the filter bar and [`BlendRows`] in the header, the
    /// rows, the footer. Events are reported in that order.
    pub fn show(self, ui: &mut Ui) -> LayersOutput<T::Id> {
        let LayersPanel { tree, settings, thumbnails, mut filter, lock_column, blend_rows, filter_bar, row_menu, reveal, rename } = self;
        let env = Env::of(ui);
        let words = env.words::<LayersPanelWords>();
        let list_words = env.words::<ListWords>();
        let fallback = LayersSettings::default();
        let settings = settings.unwrap_or(&fallback);
        let list = ui.scope_id().with("layers-rows");
        let cx = PanelContext {
            list,
            tree,
            selection: tree.selection(),
            options: &settings.options,
            thumbs: thumbnails,
            words: &words,
            list_words: &list_words,
            lock_column,
        };
        let events = RefCell::new(Vec::new());
        let current = RefCell::new(LayerFilter::default());
        let unlock_label = words.unlock_layer.clone();

        let shown = Stack::new()
            .fill()
            .header(|ui, stack| {
                let held = |filter: &mut Option<Bind<'_, LayerFilter>>, stack: &mut craft_ui::contract::BodyCx<'_>| -> LayerFilter {
                    match filter.as_mut() {
                        Some(bind) => bind.reborrow().read(stack).and_then(|value| value.same().cloned()).unwrap_or_default(),
                        None => stack.memory::<Session<T::Id>>().filter.clone(),
                    }
                };
                let before = held(&mut filter, stack);
                if filter_bar && settings.show_filters {
                    let modes = tree.blend_modes(true, &before.mode);
                    match filter.as_mut() {
                        Some(bind) => {
                            let out = LayerFilterBar::new(bind.reborrowed()).blend_modes(&modes).show(ui);
                            events.borrow_mut().extend(out.edit.map(LayersEvent::Filter));
                        }
                        None => {
                            let own = &mut stack.memory::<Session<T::Id>>().filter;
                            let _ = LayerFilterBar::new(Bind::bound(own)).blend_modes(&modes).show(ui);
                        }
                    }
                }
                *current.borrow_mut() = held(&mut filter, stack);
                if blend_rows {
                    let properties = tree.properties();
                    let modes =
                        tree.blend_modes(properties.as_ref().is_some_and(|p| p.group), properties.as_ref().map_or("normal", |p| &p.blend_key));
                    let out = BlendRows::new(properties.as_ref(), &modes).show(ui);
                    events.borrow_mut().extend(out.events.into_iter().map(LayersEvent::Properties));
                }
            })
            .footer(|ui, _| {
                let out = show_footer(ui, &cx, &words);
                events.borrow_mut().extend(out);
            })
            .show(ui, |ui, stack| {
                let filter = current.borrow().clone();
                let session = stack.memory::<Session<T::Id>>();
                let plan = reveal_plan(tree, cx.selection.active.as_ref(), reveal.clone(), session);
                let rows = LayerRows::new(tree, &filter, &plan.opened());
                if let Some(layer) = rename.clone() {
                    start_rename(ui, list, layer);
                }
                let body = show_body(ui, &cx, &rows, session, &plan, filter.active(), &unlock_label);
                let menus = show_row_menus(ui, &cx, &body.response, row_menu.as_deref(), session);
                let mut reported = body.events;
                reported.extend(menus);
                reported
            });
        // Only the real run's result comes back, so the reveal's opens and the list's events are reported once.
        let mut events = events.into_inner();
        events.extend(shown.inner.unwrap_or_default());
        LayersOutput { events, response: shown.output }
    }
}

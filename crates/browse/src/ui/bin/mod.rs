//! `MediaBin` over the app's `BinModel`: breadcrumb, search, count, the List, Icon and Freeform
//! views, the footer.
//!
//! ```ignore
//! let out = MediaBin::new("project", Bind::bound(&mut state.bin), &project)   // project: impl BinModel<ItemId>
//!     .selection(Bind::bound(&mut state.selection))
//!     .thumbnails(&thumbs, &project_thumbs)
//!     .columns(columns())
//!     .footer_actions([FooterAction::new(IconId::new("plus"), "New Item").accepts_drops::<ListDrag<ItemId>>(|drag| !drag.keys.is_empty())])
//!     .show(ui);
//! for event in out.events {
//!     match event {
//!         BinEvent::Activate(id) => open_source(id),
//!         BinEvent::Rename { key, text } => cmd("project.rename", key, text),
//!         BinEvent::FooterDrop { action: 0, keys } => cmd("project.newSequence", keys),
//!         _ => {}
//!     }
//! }
//! ```

mod footer;
mod free_view;
mod icon;
mod list;
mod search;
#[cfg(feature = "screen-testing")]
mod stories;

use std::cell::RefCell;
use std::hash::Hash;
use std::sync::Arc;

use craft_behavior::{ColumnKey, EditEvent, IconId, Mixed, Selection, SelectionChange};
use craft_theme::{Srgba, TypeRole};
use craft_ui::bars::{Bar, BarKind, BreadcrumbBar};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Bind, BodyCx, ColumnEvent, ColumnSort, ColumnSpec, CommandSet, Commands as _, ContextMenuRequest, Entry, ListDrag, NoValue, OpenChange, Outcome,
    Pattern, Props as _, ReorderMove, Show as _, ShowContainer as _, Sizing, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::list::{OpenSet, ThumbSize};
use craft_ui::primitives::textures::{TextureKey, ThumbSource, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::sections::{DropAction, FooterAction};
use craft_ui::text::TextField;
use craft_ui::tile_grid::{TileBadge, TileStatus, TileZoom, ZoomStep};
use egui::{Id, Response, Ui, Vec2};

use super::freebin::{FreeBinMove, FreeformCard};

/// The default zoom: the card thumbnail.
const ZOOM_DEFAULT: f64 = 1.0;

/// The zoom range the footer slider spans, as multiples of the card thumbnail side, and the step of a zoom key.
const ZOOM_MIN: f64 = 0.5;
const ZOOM_MAX: f64 = 4.0;
const ZOOM_STEP: f64 = 0.25;

/// Bins nested deeper than this are not followed, so a model that contains itself ends.
const MAX_DEPTH: usize = 64;

pub use search::name_matches;

/// One child of a bin: a sub-bin or an item.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BinEntry<K> {
    /// A bin.
    Bin(K),
    /// An item.
    Item(K),
}

/// What an item or bin shows.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct BinItem {
    /// Its name.
    pub name: String,
    /// The item's kind glyph (video, audio, sequence); bins show the folder.
    pub icon: Option<IconId>,
    /// The label chip's colour (the app's).
    pub label_color: Option<Srgba>,
    /// Corner badges in the Icon view; in the List view the text ones follow the name.
    pub badges: Vec<TileBadge>,
    /// Offline or unreadable: the name in the failure colour with the glyph and text (never colour
    /// alone).
    pub status: TileStatus,
    /// The duration text at the end of an Icon card's caption.
    pub duration: Option<String>,
    /// In and Out as fractions of the thumbnail's width.
    pub range: Option<(f32, f32)>,
}

/// The app's project, read only. Every call is cheap: the bin reads it for the rows it builds.
pub trait BinModel<K> {
    /// The bins and items in `parent` (`None`: the root), unsorted: the bin sorts by the sort in
    /// force, bins before items; "User Order" is this order.
    fn children(&self, parent: Option<&K>, out: &mut Vec<BinEntry<K>>);
    /// Fills `item` for `key`.
    fn item(&self, key: &K, item: &mut BinItem);
    /// The text of `key`'s List-view `column` (any column but the name).
    fn cell(&self, key: &K, column: &ColumnKey) -> String;
    /// Whether `key` is a bin.
    fn is_bin(&self, key: &K) -> bool;
    /// The bin holding `key`; `None` at the root.
    fn parent(&self, key: &K) -> Option<K>;
    /// Items in `bin`, sub-bins included (the count line and folded bins).
    fn count(&self, bin: &K) -> usize;
}

/// The bin's view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum BinView {
    /// A table.
    #[default]
    List,
    /// Cards in a grid, 16:9.
    Icon,
    /// Cards placed by the user.
    Freeform,
}

/// How the user asked to open a bin, from the double click's modifiers: plain in place, Shift or
/// Alt in a new tab, Cmd/Ctrl in a new window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpenHow {
    /// In this panel.
    InPlace,
    /// In a new tab (the host's).
    NewTab,
    /// In a floating window (the host's).
    NewWindow,
}

/// The bin's saved look: a `craft_host::Section` the host stores per bin panel.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(default)]
pub struct BinSettings {
    /// The view.
    pub view: BinView,
    /// The card size in the Icon and Freeform views as a multiple of the card thumbnail side.
    pub zoom: f64,
    /// List-view thumbnails.
    pub thumbnails: ThumbSize,
    /// The List view's sort; `None` is User Order.
    pub sort: Option<ColumnSort>,
    /// The Icon view's sort (Sort Icons); `None` is User Order.
    pub icon_sort: Option<ColumnSort>,
    /// The widths the user dragged the List view's columns to.
    pub widths: Vec<(ColumnKey, f32)>,
}

impl Default for BinSettings {
    fn default() -> Self {
        Self { view: BinView::List, zoom: ZOOM_DEFAULT, thumbnails: ThumbSize::None, sort: None, icon_sort: None, widths: Vec::new() }
    }
}

impl craft_host::Section for BinSettings {
    const KEY: &'static str = "media_bin";
}

/// The bin's state between passes, kept by the host (one per panel or tab): what is shown, the
/// search, the open bins and the settings.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BinState<K: Eq + Hash> {
    /// The bin shown; `None` is the root.
    pub shown: Option<K>,
    /// The search text.
    pub search: String,
    /// The List view's open bins.
    pub expanded: OpenSet<K>,
    /// The saved look.
    pub settings: BinSettings,
}

impl<K: Eq + Hash + Clone> Default for BinState<K> {
    fn default() -> Self {
        Self { shown: None, search: String::new(), expanded: OpenSet::new(), settings: BinSettings::default() }
    }
}

impl<K: Clone + Eq + Hash> BinState<K> {
    /// Applies the view-state events (`Navigate`, `Search`, `View`, `Zoom`, `Column`, `IconSort`, and
    /// the List view's opens and closes) to the state; document events are left to the host. The bin
    /// applies its own events to the state it is bound to, so a host calls this only for events it
    /// produces itself.
    pub fn apply(&mut self, event: &BinEvent<K>) {
        match event {
            BinEvent::Navigate(to) => {
                self.shown.clone_from(to);
                self.search.clear();
            }
            BinEvent::OpenBin { key, how: OpenHow::InPlace } => {
                self.shown = Some(key.clone());
                self.search.clear();
            }
            BinEvent::Search(text) => self.search.clone_from(text),
            BinEvent::View(view) => self.settings.view = *view,
            BinEvent::Zoom(zoom) => {
                zoom.apply(&mut self.settings.zoom);
            }
            BinEvent::Open(change) => self.expanded.set(&change.key, change.open),
            BinEvent::Column(ColumnEvent::Sort { key, descending, .. }) => self.settings.sort = Some(ColumnSort::new(key.clone(), *descending)),
            BinEvent::Column(ColumnEvent::Resized { key, width, .. }) => match self.settings.widths.iter_mut().find(|(held, _)| held == key) {
                Some((_, held)) => *held = *width,
                None => self.settings.widths.push((key.clone(), *width)),
            },
            BinEvent::IconSort(sort) => self.settings.icon_sort.clone_from(sort),
            _ => {}
        }
    }
}

/// What the bin reported, in order; each pass's events are handled in that pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BinEvent<K> {
    /// The selection changed; already applied.
    Selection(SelectionChange<K>),
    /// Enter or a double click on an item.
    Activate(K),
    /// Enter or a double click on a bin, with how.
    #[non_exhaustive]
    OpenBin {
        /// The bin that was opened.
        key: K,
        /// How it was opened.
        how: OpenHow,
    },
    /// The breadcrumb, Up or a search result moved the shown bin.
    Navigate(Option<K>),
    /// A List-view bin opened or closed. With Alt the bin reports one change per bin under the shown
    /// bin, each with `all` false, so [`BinState::apply`] needs no model.
    Open(OpenChange<K>),
    /// A rename committed with non-empty text other than the name; one undo step.
    #[non_exhaustive]
    Rename {
        /// The renamed item.
        key: K,
        /// The new name.
        text: String,
    },
    /// Items dropped into a bin (`None`: the shown bin).
    #[non_exhaustive]
    Drop {
        /// The dropped items.
        items: Vec<K>,
        /// The target bin; `None` is the shown bin.
        into: Option<K>,
    },
    /// A reorder in a bin shown in User Order. `to` counts the rows shown, open bins' children included.
    Reorder(EditEvent<ReorderMove<K>>),
    /// Delete or Backspace.
    Remove(Vec<K>),
    /// A drag of the selection is under way.
    Drag(ListDrag<K>),
    /// Hover scrub over an item's picture.
    #[non_exhaustive]
    Scrub {
        /// The scrubbed item.
        key: K,
        /// Pointer position across the picture from the reading start, 0.0 to 1.0.
        fraction: f32,
    },
    /// The search text changed.
    Search(String),
    /// A view button.
    View(BinView),
    /// The zoom slider or keys: the new zoom.
    Zoom(EditEvent<f64>),
    /// The List view's header: sort or column width.
    Column(ColumnEvent),
    /// The Sort Icons menu: the sort, `None` for User Order.
    IconSort(Option<ColumnSort>),
    /// A click on a host footer action, by its place in the actions given.
    #[non_exhaustive]
    Footer {
        /// The action's index.
        action: usize,
    },
    /// A drop on a host footer action that accepts drops.
    #[non_exhaustive]
    FooterDrop {
        /// The action's index.
        action: usize,
        /// The dragged items.
        keys: Vec<K>,
    },
    /// The footer's New Bin.
    NewBin,
    /// A Freeform drag committed: the cards' new positions.
    #[non_exhaustive]
    FreeformMoved {
        /// The cards that moved.
        keys: Vec<K>,
        /// The new position of each card, in the same order as `keys`.
        to: Vec<Vec2>,
    },
    /// A context menu on an item or a bin.
    ContextMenu(ContextMenuRequest<K>),
}

/// What [`MediaBin::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the bin's events"]
pub struct BinOutput<K> {
    /// The panel's body.
    pub response: Response,
    /// What happened this pass, in order.
    pub events: Vec<BinEvent<K>>,
}

impl<K> Outcome for BinOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

craft_ui::words! {
    /// The bin's words, English by default.
    pub struct BinWords {
        name: "mediaBin.name" = "Name";
        search: "mediaBin.search" = "Search";
        views: "mediaBin.views" = "View";
        up: "mediaBin.up" = "Up One Level";
        root: "mediaBin.root" = "Project";
        path: "mediaBin.path" = "Bin path";
        /// `{selected}` of `{total}`.
        selected_of: template "mediaBin.selectedOf" = "{selected} of {total} items selected";
        items: count "mediaBin.items" = "{n} item" | "{n} items";
        list_view: "mediaBin.listView" = "List View";
        icon_view: "mediaBin.iconView" = "Icon View";
        freeform_view: "mediaBin.freeformView" = "Freeform View";
        new_bin: "mediaBin.new" = "New Bin";
        sort_icons: "mediaBin.sortIcons" = "Sort Icons";
        user_order: "mediaBin.userOrder" = "User Order";
        no_matches: "mediaBin.noMatches" = "No items match the search";
        empty: "mediaBin.empty" = "This bin is empty";
        thumb_size: "mediaBin.thumbSize" = "Thumbnail Size";
    }
}

/// The zoom and stacking keys of the bin: the keyboard path of the footer slider and of the order in User
/// Order. A key reaches the bin that holds key focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "mediaBin", scope = "mediaBin")]
#[non_exhaustive]
pub enum MediaBinCommand {
    /// Bigger cards.
    #[command(label = "Zoom In", default = "Cmd+=")]
    ZoomIn,
    /// Smaller cards.
    #[command(label = "Zoom Out", default = "Cmd+-")]
    ZoomOut,
    /// Back to the default card size.
    #[command(label = "Reset Zoom", default = "Cmd+0")]
    ZoomReset,
    /// Move the selection one place later in User Order.
    #[command(label = "Bring Forward", default = "Cmd+]")]
    BringForward,
    /// Move the selection one place earlier in User Order.
    #[command(label = "Send Backward", default = "Cmd+[")]
    SendBackward,
}

/// A host slot drawn inside the widget.
type Slot<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// The thumbnails a widget draws with: the cache and the app's source, keyed by the widget's key.
type ThumbPair<'a, K> = (&'a Thumbnails<K>, &'a dyn ThumbSource<K>);

/// What a view reads and edits in one pass.
pub(super) struct ViewIn<'a, K: TextureKey, M: ?Sized> {
    pub id: Id,
    pub model: &'a M,
    pub state: &'a BinState<K>,
    pub selection: Bind<'a, Selection<K>>,
    /// The selection as it stood at the start of the pass.
    pub held: &'a Selection<K>,
    pub thumbnails: Option<ThumbPair<'a, K>>,
    pub columns: &'a [ColumnSpec],
    pub words: &'a BinWords,
    pub collator: &'a craft_i18n::Collator,
    pub rename: Option<K>,
    pub hover_scrub: bool,
    pub scrub: Option<(K, f32)>,
    /// The step of the stacking keys pressed this pass: `true` moves later.
    pub stack: Option<bool>,
    /// The Freeform view's drag in progress, as its last pass reported it.
    pub moving: Option<FreeBinMove<K>>,
}

/// What a view reported.
pub(super) struct ViewOut<K> {
    pub response: Response,
    pub events: Vec<BinEvent<K>>,
    /// The Freeform view's drag in progress, handed back next pass.
    pub moving: Option<FreeBinMove<K>>,
}

/// What the bin keeps in its container's memory between passes.
#[derive(Clone)]
struct BinMemory<K: Eq + Hash> {
    before: BinState<K>,
    local: BinState<K>,
    selection: Selection<K>,
    scrub: Option<(K, f32)>,
    moving: Option<FreeBinMove<K>>,
}

impl<K: Eq + Hash + Clone> Default for BinMemory<K> {
    fn default() -> Self {
        BinMemory { before: BinState::default(), local: BinState::default(), selection: Selection::default(), scrub: None, moving: None }
    }
}

/// The project or media panel.
///
/// - **Header.** An Up `IconButton`, a `BreadcrumbBar` of the shown bin's ancestors (`BinModel::parent`,
///   the root named [`BinWords::root`]), a search field and the item count ([`BinWords::items`], or
///   [`BinWords::selected_of`] with a selection). Then the host's `.header(..)` slot.
/// - **Search.** Typing filters live by [`name_matches`]; a non-empty search lists matches from every
///   sub-bin, flat; Esc clears, a second Esc leaves the field.
/// - **List view** (`list.rs`): a tree `Table`, bins before items, every bin a group so an empty bin has
///   its disclosure triangle, open bins' children under them; header sort is the table's.
/// - **Icon view** (`icon.rs`): a `TileGrid` of 16:9 tiles, bins as folder tiles before the items, hover
///   scrub, In and Out bars.
/// - **Freeform view** (`free_view.rs`): a `FreeBin` of the host's [`FreeformCard`] positions.
/// - **Drag and drop.** Rows and cards drag the selection (`Drag`); a bin takes a drag from another list
///   (`Drop { into: Some(bin) }`), the view background takes drops into the shown bin (`into: None`). A
///   drop reports nothing for items already in the target bin, and none into a bin that is, or lies
///   inside, one of the dragged items. In User Order a drag of the List view's own rows reorders them,
///   and a drag onto a bin row's middle moves them into the bin.
/// - **Keys.** [`MediaBinCommand`] holds the zoom and stacking keys; Enter or a double click opens a bin
///   or activates an item; F2 renames; Delete and Backspace remove.
/// - **Footer** (`footer.rs`): the view switch, the zoom slider, Sort Icons (Icon and Freeform), the
///   host's actions, New Bin.
#[must_use = "show the bin with `show`"]
#[derive(Pattern)]
#[pattern(probe(MediaBin<'static, u32, dyn BinModel<u32>>))]
pub struct MediaBin<'a, K: TextureKey, M: ?Sized> {
    label: Label,
    state: Bind<'a, BinState<K>>,
    model: &'a M,
    selection: Option<Bind<'a, Selection<K>>>,
    thumbnails: Option<ThumbPair<'a, K>>,
    columns: Vec<ColumnSpec>,
    hover_scrub: bool,
    header: Option<Slot<'a>>,
    actions: Vec<FooterAction>,
    freeform: &'a [FreeformCard<K>],
    snap: Option<f32>,
    zoom_range: (f64, f64),
    rename: Option<K>,
}

impl<'a, K, M> MediaBin<'a, K, M>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    /// A bin panel named `label` over `model`, its view state in `state`.
    pub fn new(label: impl Into<Label>, state: Bind<'a, BinState<K>>, model: &'a M) -> Self {
        Self {
            label: label.into(),
            state,
            model,
            selection: None,
            thumbnails: None,
            columns: Vec::new(),
            hover_scrub: false,
            header: None,
            actions: Vec::new(),
            freeform: &[],
            snap: None,
            zoom_range: (ZOOM_MIN, ZOOM_MAX),
            rename: None,
        }
    }

    /// The selection the bin applies changes to; without one the bin keeps its own.
    pub fn selection(mut self, selection: Bind<'a, Selection<K>>) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Thumbnails keyed by the item key.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<K>, source: &'a dyn ThumbSource<K>) -> Self {
        self.thumbnails = Some((thumbnails, source));
        self
    }

    /// The List view's columns; the first is the name column (key `name`).
    pub fn columns(mut self, columns: Vec<ColumnSpec>) -> Self {
        self.columns = columns;
        self
    }

    /// Hover scrub in the Icon view.
    pub fn hover_scrub(mut self, on: bool) -> Self {
        self.hover_scrub = on;
        self
    }

    /// The host's area above the content (a preview).
    pub fn header(mut self, header: impl FnMut(&mut Ui) + 'a) -> Self {
        self.header = Some(Box::new(header));
        self
    }

    /// The host's footer buttons, after the view buttons and slider. An action that takes drops reports
    /// `FooterDrop` with the dragged keys when it was built with `accepts_drops::<ListDrag<K>>`.
    pub fn footer_actions(mut self, actions: impl IntoIterator<Item = FooterAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// The Freeform view's card positions.
    pub fn freeform(mut self, cards: &'a [FreeformCard<K>]) -> Self {
        self.freeform = cards;
        self
    }

    /// Freeform snapping to a grid of `step` points.
    pub fn snap(mut self, step: Option<f32>) -> Self {
        self.snap = step;
        self
    }

    /// The zoom slider's range as multiples of the card thumbnail side.
    pub fn zoom_range(mut self, min: f64, max: f64) -> Self {
        self.zoom_range = (min, max);
        self
    }

    /// Opens the in-place rename of `key` in this pass, as F2 would. Call it in the pass that handles the
    /// host's Rename command; ignored when `key` is not shown.
    pub fn start_rename(mut self, key: K) -> Self {
        self.rename = Some(key);
        self
    }
}

/// The ancestors of the shown bin as crumbs, the root first.
fn crumbs_of<K, M>(model: &M, shown: Option<&K>, root: &Label) -> Vec<Entry<Option<K>>>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    let mut chain: Vec<Option<K>> = vec![shown.cloned()];
    while let Some(parent) = chain.last().cloned().flatten().map(|key| model.parent(&key)) {
        if chain.len() > MAX_DEPTH {
            break;
        }
        chain.push(parent);
    }
    chain
        .into_iter()
        .rev()
        .map(|key| {
            let label = match &key {
                Some(key) => {
                    let mut item = BinItem::default();
                    model.item(key, &mut item);
                    Label::from(item.name)
                }
                None => root.clone(),
            };
            Entry::new(key, label)
        })
        .collect()
}

/// The number the count line gives: the matches of a search, else the items in the shown bin.
fn total_of<K: Clone, M: BinModel<K> + ?Sized>(model: &M, shown: Option<&K>, matches: Option<&[K]>) -> usize {
    match (shown, matches) {
        (_, Some(found)) => found.len(),
        (Some(bin), None) => model.count(bin),
        (None, None) => {
            let mut children = Vec::new();
            model.children(None, &mut children);
            children
                .iter()
                .map(|entry| match entry {
                    BinEntry::Bin(key) => model.count(key),
                    BinEntry::Item(_) => 1,
                })
                .sum()
        }
    }
}

impl<K, M> MediaBin<'_, K, M>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    /// Shows the panel across the available room and reports what happened.
    pub fn show(self, ui: &mut Ui) -> BinOutput<K> {
        let MediaBin { label, state, model, selection, thumbnails, columns, hover_scrub, header, actions, freeform, snap, zoom_range, rename } = self;
        let env = Env::of(ui);
        let words = env.words::<BinWords>();
        let collator = env.collator();
        let name: Arc<str> = Arc::from(label.as_str());
        let ids = ViewIds::of(ui);
        let state = RefCell::new(state);
        let selection = RefCell::new(selection);
        let header = RefCell::new(header);
        let footer_events: RefCell<Vec<BinEvent<K>>> = RefCell::new(Vec::new());
        let zoom = TileZoom::bounded(zoom_range.0, zoom_range.1, ZOOM_STEP, ZOOM_DEFAULT.clamp(zoom_range.0, zoom_range.1));

        let body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let before = match state.borrow_mut().reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => BinState::default(),
            };
            let mut local = before.clone();
            let (mut own, scrub, moving) = {
                let memory = cx.keep::<BinMemory<K>>();
                memory.before = before;
                memory.local = local.clone();
                (memory.selection.clone(), memory.scrub.clone(), memory.moving.clone())
            };
            let mut events: Vec<BinEvent<K>> = Vec::new();
            let view = local.settings.view;
            let list_id = ids.of_view(view);
            let searching = !local.search.trim().is_empty();

            let held = match selection.borrow_mut().as_mut() {
                Some(bind) => match bind.reborrow().read(cx) {
                    Some(Mixed::Same(held)) => held,
                    _ => Selection::default(),
                },
                None => own.clone(),
            };

            let mut commanded: Vec<ZoomStep> = Vec::new();
            let mut restack: Option<bool> = None;
            while let Some((command, _)) = cx.take_command::<MediaBinCommand>() {
                match command {
                    MediaBinCommand::ZoomIn => commanded.push(ZoomStep::In),
                    MediaBinCommand::ZoomOut => commanded.push(ZoomStep::Out),
                    MediaBinCommand::ZoomReset => commanded.push(ZoomStep::Reset),
                    MediaBinCommand::BringForward => restack = Some(true),
                    MediaBinCommand::SendBackward => restack = Some(false),
                }
            }

            // The matches, the count and the header row.
            let mut found: Vec<K> = Vec::new();
            if searching {
                search::collect_matches(model, local.shown.as_ref(), &local.search, 0, &mut found);
            }
            let total = total_of(model, local.shown.as_ref(), searching.then_some(found.as_slice()));
            let selected =
                held.items.iter().filter(|key| if searching { found.contains(key) } else { list::within(model, key, local.shown.as_ref()) }).count();
            let count_text = if selected == 0 {
                words.items.text(total as u64, &env.format().integer(total as u64))
            } else {
                words.selected_of.fill(&[("selected", &env.format().integer(selected as u64)), ("total", &env.format().integer(total as u64))])
            };
            let crumbs = crumbs_of(model, local.shown.as_ref(), &words.root.clone());
            let shown = local.shown.clone();
            let _ = Bar::new(Label::Empty, BarKind::Tool).show(ui, |ui, _| {
                let up = IconButton::new(IconId::new("chevron_up")).label(words.up.clone()).enabled(shown.is_some()).show(ui);
                if clicked(&up) {
                    let to = shown.as_ref().and_then(|key| model.parent(key));
                    local.apply(&BinEvent::Navigate(to.clone()));
                    events.push(BinEvent::Navigate(to));
                }
                if let Some(target) = BreadcrumbBar::new(words.path.clone(), &crumbs).show(ui).picked {
                    local.apply(&BinEvent::Navigate(target.clone()));
                    events.push(BinEvent::Navigate(target));
                }
                let typed = local.search.clone();
                TextField::search(words.search.clone(), Bind::bound(&mut local.search)).placeholder(words.search.clone()).drives(list_id).show(ui);
                if local.search != typed {
                    events.push(BinEvent::Search(local.search.clone()));
                }
                Text::new(count_text.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
            });
            if let Some(header) = header.borrow_mut().as_mut() {
                header(ui);
            }

            // The view, inside a slot that takes the drops the view leaves.
            let (response, mut view_events, background, moving_now) = {
                let mut hosted = selection.borrow_mut();
                let shown_view = DropAction::<ListDrag<K>>::new(|_| true).sizing(Sizing::Fill).show(ui, |ui, _| {
                    // The body runs in the measuring run and again in the real one: it builds its input each time.
                    let bind = match hosted.as_mut() {
                        Some(bind) => bind.reborrowed(),
                        None => Bind::bound(&mut own),
                    };
                    let input = ViewIn {
                        id: list_id,
                        model,
                        state: &local,
                        selection: bind,
                        held: &held,
                        thumbnails,
                        columns: &columns,
                        words: &words,
                        collator: &collator,
                        rename: rename.clone(),
                        hover_scrub,
                        scrub: scrub.clone(),
                        stack: restack,
                        moving: moving.clone(),
                    };
                    match view {
                        BinView::List => list::list_view(ui, input),
                        BinView::Icon => icon::icon_view(ui, input),
                        BinView::Freeform => free_view::free_view(ui, input, freeform, snap),
                    }
                });
                let background = shown_view.output.dropped.clone();
                let out =
                    shown_view.inner.unwrap_or_else(|| ViewOut { response: shown_view.output.response.clone(), events: Vec::new(), moving: None });
                (out.response, out.events, background, out.moving)
            };
            let mut zoom_now = local.settings.zoom;
            for step in commanded {
                let next = zoom.step(zoom_now, step);
                if next != zoom_now {
                    let event = EditEvent::set(Mixed::Same(zoom_now), next, name.clone(), None);
                    local.apply(&BinEvent::Zoom(event.clone()));
                    events.push(BinEvent::Zoom(event));
                    zoom_now = next;
                }
            }
            if let Some(drag) = background
                && let Some(items) = list::drop_items(model, local.shown.as_ref(), &drag.keys)
            {
                view_events.push(BinEvent::Drop { items, into: None });
            }
            let mut last_scrub = scrub;
            for event in &view_events {
                match event {
                    BinEvent::Scrub { key, fraction } => last_scrub = Some((key.clone(), *fraction)),
                    BinEvent::Selection(SelectionChange::Clear) | BinEvent::Activate(_) | BinEvent::OpenBin { .. } => last_scrub = None,
                    _ => {}
                }
                local.apply(event);
            }
            events.extend(view_events);

            let memory = cx.keep::<BinMemory<K>>();
            memory.local = local;
            memory.selection = own;
            memory.scrub = last_scrub;
            memory.moving = moving_now;
            (response, events)
        };

        let footer_region = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let mut local = cx.keep::<BinMemory<K>>().local.clone();
            let events = footer::footer(ui, &local, &columns, &actions, &words, zoom);
            for event in &events {
                local.apply(event);
            }
            *footer_events.borrow_mut() = events;
            cx.keep::<BinMemory<K>>().local = local;
            commit(cx, &state, &name);
        };

        let stack = Stack::new().fill().command_scope(MediaBinCommand::SCOPE).footer(footer_region);
        let shown = stack.show(ui, body);
        let (response, mut events) = shown.inner.unwrap_or_else(|| (shown.output.clone(), Vec::new()));
        events.extend(footer_events.take());
        BinOutput { response, events }
    }
}

/// The ids of the three views' collections: one per view, so a switch never reads another collection's memory.
struct ViewIds {
    list: Id,
    icon: Id,
    free: Id,
}

impl ViewIds {
    fn of(ui: &Ui) -> Self {
        let scope = ui.scope_id();
        ViewIds { list: scope.with("media-bin-list"), icon: scope.with("media-bin-icon"), free: scope.with("media-bin-free") }
    }

    fn of_view(&self, view: BinView) -> Id {
        match view {
            BinView::List => self.list,
            BinView::Icon => self.icon,
            BinView::Freeform => self.free,
        }
    }
}

/// Writes the state the pass changed through the binding.
fn commit<K: TextureKey>(cx: &mut BodyCx<'_>, state: &RefCell<Bind<'_, BinState<K>>>, name: &Arc<str>) {
    let (before, local) = {
        let memory = cx.keep::<BinMemory<K>>();
        (memory.before.clone(), memory.local.clone())
    };
    if local != before {
        let event = EditEvent::set(Mixed::Same(before), local, name.clone(), None);
        let _ = state.borrow_mut().reborrow().write(cx, &event);
    }
}

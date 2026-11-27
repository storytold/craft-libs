//! `PresetBrowser`: a library of named presets shown as thumbnails, with groups, a grid or a list, a thumbnail size,
//! search, rename and drag to reorder.
//!
//! The host keeps the library and performs every command; the browser shows the rows it is given, changes only its
//! `PresetState` and reports `PresetEvent`s. The grid view is a `TileGrid` of `Tile`s under group headers, the list
//! view a tree `List` of `ListRow`s; both read the same rows and the same state. Action rows (`actions`) follow the
//! presets as rows of the same collection in both views: the grid's tail rows, the list's last rows.

mod reorder;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use reorder::PresetReorder;

use super::event::{PresetEvent, PresetOutput};
use super::footer::{PresetFooter, PresetFooterEvent, PresetFooterOptions};
use super::model::{CellShape, PresetRow, PresetRowMenu, PresetTiles, ThumbInfo, children_end, has_captions};
use super::source::PresetSource;
use super::state::{DefaultOpen, PresetState, PresetView};
use super::words::PresetWords;
use crate::bars::{Bar, BarKind};
use crate::choice::Segmented;
use crate::contract::{
    Bind, BodyCx, CollectionEvent, DragPayload, DropAt, DropKind, DropMark, DropPayload, DropPlace, DropTarget, DropZone, Entry, ListDrag, MemoProps,
    MemoView, OpenChange, Outbox, Pattern, Props, Rename, ReorderMove, ReorderRule, RowHeight, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, TooltipSpec, request_rename,
};
use crate::display::Divider;
use crate::layout_components::Stack;
use crate::list::{List, ListRow, RowInk, ThumbSize};
use crate::menu::{Menu, MenuItem};
use crate::notices::{Notice, NoticeLayout};
use crate::primitives::{ContentPainterCx, IconSpec};
use crate::runtime::drag;
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::Remember as _;
use crate::text::TextField;
use crate::tile_grid::{CaptionPlace, GridEvent, GridZoom, GroupHeads, TailRow, Tile, TileGrid, TileGroup, TileZoom};
use craft_behavior::{DisplayOrder, DropOperation, EditEvent, GesturePhase, IconId, ItemSource, Mixed, Selection, SelectionChange, SelectionModel};
use craft_i18n::matcher::Matcher;
use craft_theme::variant::Tone;
use egui::{Id, Pos2, Rect, Ui};
use std::any::Any;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

/// The commands of a preset's context menu. They are app commands: the browser reports a run row as the event of the
/// same name, and a host that binds a key to one handles it as it handles the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "presetItem")]
#[non_exhaustive]
pub enum PresetItemCommands {
    /// Delete the preset.
    #[command(label = "Delete")]
    Delete,
    /// Delete the group and the presets in it.
    #[command(label = "Delete Group")]
    DeleteGroup,
    /// Rename the preset.
    #[command(label = "Rename…")]
    Rename,
    /// Rename the group.
    #[command(label = "Rename Group…")]
    RenameGroup,
}

/// The zoom from which list rows show their large thumbnail, and the zoom up to which they show the small one.
const LARGE_FROM: f64 = 1.5;
const SMALL_UNTIL: f64 = 0.75;

/// The key of a row of the browser's collections: a preset or group of the app's rows, the action row at an index of
/// `PresetBrowser::actions`, or the line between the presets and the actions. The browser maps it back to the app's key
/// at every edge it reports through.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum BrowserKey<K> {
    Preset(K),
    Action(usize),
    Separator,
}

impl<K> BrowserKey<K> {
    fn preset(&self) -> Option<&K> {
        match self {
            BrowserKey::Preset(key) => Some(key),
            BrowserKey::Action(_) | BrowserKey::Separator => None,
        }
    }

    /// The app's key of a preset or group row, `None` for an action row or the separator.
    fn into_preset(self) -> Option<K> {
        match self {
            BrowserKey::Preset(key) => Some(key),
            BrowserKey::Action(_) | BrowserKey::Separator => None,
        }
    }
}

/// The app's change of a selection change the browser's collection reports: only the rows `selectable` accepts stay in
/// it. Action rows are never part of the host's selection. Cell changes do not occur under the modes the browser allows.
fn app_change<K>(change: SelectionChange<BrowserKey<K>>, selectable: impl Fn(&K) -> bool) -> Option<SelectionChange<K>> {
    match change {
        SelectionChange::Set(keys) => {
            Some(SelectionChange::Set(keys.into_iter().filter_map(BrowserKey::into_preset).filter(|key| selectable(key)).collect()))
        }
        SelectionChange::Toggle(key) => key.into_preset().filter(|key| selectable(key)).map(SelectionChange::Toggle),
        SelectionChange::Clear => Some(SelectionChange::Clear),
        _ => None,
    }
}

/// The app's drag of a drag the browser's collection carries: its presets and groups. `None` for a drag grabbed at an
/// action row, which never drags.
fn app_drag<K: Clone>(drag: &ListDrag<BrowserKey<K>>) -> Option<ListDrag<K>> {
    let grabbed = drag.grabbed.preset()?.clone();
    Some(ListDrag::new(drag.list, drag.keys.iter().filter_map(BrowserKey::preset).cloned().collect(), grabbed))
}

/// The drop zone's target: a preset row (not a group) dragged from this browser's collection, dropped at the pointer.
struct ZoneDrop<'r, K> {
    source: Id,
    rows: &'r [PresetRow<K>],
    zone: DropZone,
}

impl<K: Clone + PartialEq> DropTarget<ListDrag<BrowserKey<K>>> for ZoneDrop<'_, K> {
    type Drop = (K, Pos2);

    fn accept(&self, drag: &ListDrag<BrowserKey<K>>, at: Pos2, _: DropOperation) -> Option<(K, Pos2)> {
        let key = drag.grabbed.preset().filter(|_| drag.list == self.source)?;
        self.rows.iter().any(|row| row.key == *key && !row.is_group()).then(|| (key.clone(), at))
    }

    fn mark(&self, _: &(K, Pos2)) -> DropMark {
        DropMark::Into(self.zone.rect())
    }
}

/// The app's move of a move the browser's collection reports. Only presets and groups drag, and only they take a drop
/// (`can_drag`, `accepts`), so every key of a move is one of the app's.
fn app_move<K>(mv: ReorderMove<BrowserKey<K>>) -> Option<ReorderMove<K>> {
    let app = BrowserKey::into_preset;
    Some(ReorderMove {
        key: app(mv.key)?,
        keys: mv.keys.into_iter().filter_map(app).collect(),
        to: mv.to,
        copy: mv.copy,
        parent: mv.parent.and_then(app),
    })
}

type RowItemsFn<'a, K> = Box<dyn Fn(&K) -> Vec<Entry<String>> + 'a>;

/// The host's rows of the row menu for a target ([`PresetBrowser::row_items`]).
pub struct RowItems<'a, K>(RowItemsFn<'a, K>);

impl<'a, K> RowItems<'a, K> {
    /// The rows that `items` gives for a target.
    pub fn new(items: impl Fn(&K) -> Vec<Entry<String>> + 'a) -> Self {
        RowItems(Box::new(items))
    }

    /// The rows for `key`: how the browser builds its row menu.
    pub(super) fn items(&self, key: &K) -> Vec<Entry<String>> {
        (self.0)(key)
    }
}

/// Paints the tip of a tile or row on hover ([`PresetBrowser::tip`]).
pub(super) type TipFn<'a, K> = Box<dyn Fn(&mut Ui, &K) + 'a>;

/// The rows and the order the shown collection lists them in: what a drop needs to say where a payload landed in the
/// host's rows.
struct RowMap<'a, K> {
    rows: &'a [PresetRow<K>],
    /// The positions in `rows` of the collection's items, in display order.
    displayed: Vec<usize>,
}

impl<K: Clone + Eq> RowMap<'_, K> {
    /// The host's place of a place in the browser's collection; `None` where no drop lands: on a group, an action row
    /// or the separator, or past the presets.
    fn host(&self, at: &DropAt<BrowserKey<K>>) -> Option<DropAt<K>> {
        match at {
            DropAt::Onto(BrowserKey::Preset(key)) => {
                self.rows.iter().any(|row| row.key == *key && !row.is_group()).then(|| DropAt::Onto(key.clone()))
            }
            DropAt::Gap { key, insert, .. } if *insert <= self.displayed.len() => {
                let over = match key {
                    Some(BrowserKey::Action(_) | BrowserKey::Separator) => return None,
                    Some(BrowserKey::Preset(key)) => Some(key.clone()),
                    None => None,
                };
                Some(DropAt::gap(over, full_position(self.rows, &self.displayed, &[], *insert)))
            }
            _ => None,
        }
    }
}

/// The host's test of a payload, with the payload type erased so the drop's lifetime stays covariant: a browser that
/// borrows its rows for less than the host's closure lives takes the drop as it is.
type DropAccepts<'a, K> = Rc<dyn Fn(&dyn Any, &DropAt<K>) -> bool + 'a>;

/// Builds the drop kind of one payload type over the rows of one browser pass.
type BuildDrop<K> = for<'m> fn(DropPlace, DropAccepts<'m, K>, Rc<RowMap<'m, K>>) -> DropKind<'m, BrowserKey<K>>;

/// A drop target on the presets: a host payload of type `P` dropped onto a preset or between presets.
pub struct PresetDrop<'a, K> {
    place: DropPlace,
    accepts: DropAccepts<'a, K>,
    /// The drop kind of the payload type the drop was made for, over the rows of one browser pass.
    build: BuildDrop<K>,
}

impl<'a, K: Clone + Eq + 'a> PresetDrop<'a, K> {
    /// Takes payloads of type `P` at `place` where `accepts` agrees; the drop comes back as `PresetEvent::Dropped`.
    pub fn new<P: Send + Sync + 'static>(place: DropPlace, accepts: impl Fn(&P, &DropAt<K>) -> bool + 'a) -> Self {
        PresetDrop {
            place,
            accepts: Rc::new(move |payload, at| payload.downcast_ref::<P>().is_some_and(|payload| accepts(payload, at))),
            build: drop_kind::<P, K>,
        }
    }

    /// The drop kind over the rows of `map`.
    fn kind<'m>(&self, map: Rc<RowMap<'m, K>>) -> DropKind<'m, BrowserKey<K>>
    where
        'a: 'm,
    {
        (self.build)(self.place, Rc::clone(&self.accepts), map)
    }
}

fn drop_kind<'m, P: Send + Sync + 'static, K: Clone + Eq>(
    place: DropPlace,
    accepts: DropAccepts<'m, K>,
    map: Rc<RowMap<'m, K>>,
) -> DropKind<'m, BrowserKey<K>> {
    DropKind::new::<P>(place, move |payload, at| map.host(at).is_some_and(|at| accepts(payload, &at)))
}

type BrowserToolsFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// The host's controls at the reading end of the toolbar, after the search field: a picker's buttons and menus.
pub struct BrowserTools<'a>(BrowserToolsFn<'a>);

impl<'a> BrowserTools<'a> {
    /// The controls that `show` adds.
    pub fn new(show: impl FnMut(&mut Ui) + 'a) -> Self {
        BrowserTools(Box::new(show))
    }

    /// Adds the controls: how a picker hands its own to the browser it opens.
    pub(super) fn show(&mut self, ui: &mut Ui) {
        (self.0)(ui);
    }
}

/// A library of named presets shown as thumbnails.
///
/// - **Rows.** The host's flat display list: groups with their depth, then presets. The grid view shows each group as
///   a header over its presets; the list view shows the tree.
/// - **State.** `state` is the view, the thumbnail zoom, the open groups and the search text. Bind it with
///   `Bind::bound`: the browser writes a changed state through the binding.
/// - **Keys and pointer.** The arrows move the focus without selecting; a click or Space selects, Enter selects and
///   applies, a double click applies, F2 renames, Left and Right close and open a group, and a drag or Cmd/Ctrl+] and [
///   (Shift to the ends) reorder by the `reorder` rules. The host performs the delete, rename and move it is told of.
/// - **Search.** The toolbar's field keeps the presets whose name matches, with their groups, all open; Up, Down and
///   Enter go from the field to the presets.
/// - **Selection.** The host's `selection`, in the collection's modes (`select`): `Single` by default, `Multi` for
///   Shift ranges and Cmd/Ctrl toggles. `PresetEvent::Select` reports a plain pick only.
/// - **Row menu.** Rename, Move to, Delete and the host's `row_items` (`row_menu`) act on the row the menu was asked
///   for, which the browser keeps until the menu closes, else on the selection's lead.
/// - **Actions.** Rows after the presets, below a divider (`actions`): Down from the last preset reaches them, and a
///   click or Enter reports `PresetEvent::Action`. They are never selected, dragged or renamed.
///
/// ```ignore
/// let out = PresetBrowser::new("Swatches", Bind::bound(&mut state), &source).selection(Bind::given(Selection::single(chosen.clone()))).toolbar(true).show(ui);
/// for event in out.events { /* the host applies Select, Activate, Reorder, ... */ }
/// ```
#[must_use = "show the browser with `show`"]
#[derive(Pattern, Props)]
#[pattern(probe(PresetBrowser<'static, u32>))]
pub struct PresetBrowser<'a, K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> {
    label: Label,
    /// The browser's own id, which `PresetBrowser::request_rename` names; the scope id of the `Ui` it is shown in
    /// when none is given.
    #[prop]
    browser_id: Option<Id>,
    state: Bind<'a, PresetState<K>>,
    source: &'a PresetSource<'a, K>,
    /// The host's selection. With `Bind::given`, every change comes out as `PresetEvent::Selection`; with
    /// `Bind::bound`, the browser writes the binding.
    #[prop]
    selection: Option<Bind<'a, Selection<K>>>,
    /// How picks select: `Single` by default; `Multi` gives the collection rules in both views.
    #[prop]
    select: SelectMode,
    /// Lets selection interactions include titled groups in both views. Group disclosure remains a separate control.
    #[prop]
    select_groups: bool,
    /// Scrolls this preset into view once.
    #[prop]
    reveal: Option<K>,
    /// The shape of a grid tile's picture.
    #[prop]
    cell_shape: CellShape,
    /// How large the grid view's tiles are; the list view is the same for both.
    #[prop]
    tiles: PresetTiles,
    /// The zoom the thumbnails span and how it steps; the footer's slider uses the same.
    #[prop]
    thumb_range: TileZoom,
    /// Which groups are open before the user toggles any.
    #[prop]
    default_open: DefaultOpen,
    /// A row above the presets with the search field and the grid and list buttons.
    #[prop]
    toolbar: bool,
    /// The toolbar's search field; off, the toolbar keeps the view buttons and the host's tools, and the search is
    /// cleared.
    #[prop]
    search: bool,
    /// The host's controls at the reading end of the toolbar.
    #[prop(func = "FnMut(&mut Ui)")]
    tools: Option<BrowserTools<'a>>,
    /// Places the host's tools at the reading start, with the search and view controls at the end.
    /// Off by default, preserving the existing toolbar order.
    #[prop]
    tools_at_start: bool,
    /// What a secondary click or the context-menu key opens: the standard row menu, the host's `row_items` alone, or
    /// nothing.
    #[prop]
    row_menu: PresetRowMenu,
    /// The host's rows of the row menu for a target, after the standard ones (or alone with `PresetRowMenu::Host`).
    #[prop(func = "Fn(&K) -> Vec<Entry<String>>")]
    row_items: Option<RowItems<'a, K>>,
    /// Presets drag out as `ListDrag<K>`, without reorder or a drop zone.
    #[prop]
    drags: bool,
    /// How group heads show: titled rows, or full-width rules between the groups, which are always open.
    #[prop]
    group_heads: GroupHeads,
    /// Fixes the view, whatever the state says; the toolbar then has no view buttons.
    #[prop]
    view_lock: Option<PresetView>,
    /// Presets (and groups) drag and move by these rules.
    #[prop]
    reorder: Option<PresetReorder>,
    /// An area outside the browser (a canvas, a timeline) where a dragged preset drops: a release there reports
    /// `PresetEvent::DropAt` with the pointer in the zone's layer space. Groups dropped there report nothing. Without
    /// `reorder` presets still drag, to the zone and to the host's own drop targets only.
    #[prop]
    drop_zone: Option<DropZone>,
    actions: Vec<Label>,
    /// Paints the tip that shows on hover over a tile or row.
    #[prop(boxed = "Fn(&mut Ui, &K) + 'a")]
    tip: Option<TipFn<'a, K>>,
    drops: Vec<PresetDrop<'a, K>>,
    /// The host's payloads a preset drag carries beside its `ListDrag<K>`.
    payloads: Vec<DragPayload>,
    /// The search field is its scope's default stop (`Initial::Default`): a picker's popover opens with the focus in
    /// it, so typing filters the presets.
    search_first: bool,
    /// A footer under the presets, made from these options with the browser's zoom bound to its slider and the count
    /// of presets.
    #[prop]
    footer: Option<PresetFooterOptions<'a>>,
    /// The most rows the presets show before they scroll, in either view: a library that can grow long shown inside a
    /// body that scrolls (a dialog page) keeps its own scroll this way.
    #[prop]
    max_rows: Option<u16>,
    /// The browser takes the presets' height, up to `room_height` (then they scroll), instead of the available
    /// height: for a host that sizes to its content, such as a floating panel.
    #[prop(quantity = "room the host's panel offers, in points")]
    room_height: Option<f32>,
    /// The columns of tiles the grid asks for where it takes its width from its content (a popover): its width holds
    /// this many tiles. 0 asks for none.
    #[prop]
    columns: u16,
    /// What the browser shows when `rows` is empty, in place of the default words.
    #[prop(into)]
    empty: Option<Label>,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> PresetBrowser<'a, K> {
    /// A browser named `label` of the presets of `source`, keeping its view settings in `state`: a grid of square
    /// thumbnails, no toolbar, with a context menu.
    pub fn new(label: impl Into<Label>, state: Bind<'a, PresetState<K>>, source: &'a PresetSource<'a, K>) -> Self {
        PresetBrowser {
            label: label.into(),
            browser_id: None,
            state,
            source,
            selection: None,
            select: SelectMode::Single,
            select_groups: false,
            reveal: None,
            cell_shape: CellShape::Square,
            tiles: PresetTiles::Thumbnails,
            thumb_range: TileZoom::Ladder,
            default_open: DefaultOpen::All,
            toolbar: false,
            search: true,
            tools: None,
            tools_at_start: false,
            row_menu: PresetRowMenu::Standard,
            row_items: None,
            drags: false,
            group_heads: GroupHeads::Titled,
            view_lock: None,
            reorder: None,
            drop_zone: None,
            actions: Vec::new(),
            tip: None,
            drops: Vec::new(),
            payloads: Vec::new(),
            search_first: false,
            footer: None,
            max_rows: None,
            room_height: None,
            columns: 0,
            empty: None,
        }
    }

    /// Takes host payloads dropped onto a preset or between presets; each drop comes back as `PresetEvent::Dropped`.
    pub fn takes_drop(mut self, drop: PresetDrop<'a, K>) -> Self {
        self.drops.push(drop);
        self
    }

    /// A preset drag carries `map` of it too, for the host's drop targets of type `P` (an app's paint payload), beside
    /// the `ListDrag<K>` the browser's own targets take. Presets the map gives a payload for drag without `drags`.
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.payloads.push(DragPayload::new(move |drag: &ListDrag<BrowserKey<K>>| map(&app_drag(drag)?)));
        self
    }

    /// The id of the browser's list, the `ListDrag::list` of its drags: a footer action accepts only drags from its own
    /// browser.
    pub fn list_id(browser: Id) -> Id {
        rows_id(browser)
    }

    /// Makes the search field, when shown, its scope's default stop.
    pub(super) fn search_first(mut self) -> Self {
        self.search_first = true;
        self
    }

    /// Rows after the presets, below a divider, as a preset menu lists its commands ("Create preset", "Detach"). They
    /// are rows of the browser's collection in both views: the lead reaches them with Down from the last preset, and a
    /// click or Enter on one reports `PresetEvent::Action` with its index in `labels`.
    pub fn actions(mut self, labels: impl IntoIterator<Item = Label>) -> Self {
        self.actions = labels.into_iter().collect();
        self
    }
}

/// The rows the browser shows, as a display order over `rows`, then `actions` action rows.
struct Shown<'a, K> {
    rows: &'a [PresetRow<K>],
    at: Vec<usize>,
    open: Vec<bool>,
    tree: bool,
    actions: usize,
}

impl<K> Shown<'_, K> {
    /// The preset row at position `at`, `None` for the separator or an action row.
    fn row(&self, at: usize) -> Option<&PresetRow<K>> {
        self.at.get(at).map(|&row| &self.rows[row])
    }

    /// Whether a line sets the action rows off from the presets: both are shown.
    fn separated(&self) -> bool {
        self.actions > 0 && !self.at.is_empty()
    }
}

impl<K: Clone> DisplayOrder<BrowserKey<K>> for Shown<'_, K> {
    fn len(&self) -> usize {
        self.at.len() + usize::from(self.separated()) + self.actions
    }

    fn key(&self, at: usize) -> BrowserKey<K> {
        match self.row(at) {
            Some(row) => BrowserKey::Preset(row.key.clone()),
            None if self.separated() && at == self.at.len() => BrowserKey::Separator,
            None => BrowserKey::Action(at - self.at.len() - usize::from(self.separated())),
        }
    }

    fn is_section(&self, at: usize) -> bool {
        self.separated() && at == self.at.len()
    }

    fn depth(&self, at: usize) -> usize {
        self.row(at).filter(|_| self.tree).map_or(0, |row| row.depth)
    }

    fn has_children(&self, at: usize) -> bool {
        self.tree && self.row(at).is_some_and(PresetRow::is_group)
    }

    fn open(&self, at: usize) -> bool {
        self.open.get(at).copied().unwrap_or(false)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.row(at).map(|row| std::borrow::Cow::Borrowed(row.label.as_str()))
    }
}

impl<K: Clone> ItemSource<BrowserKey<K>> for Shown<'_, K> {}

/// The positions of the rows to show: without a search, the rows outside closed groups; with one, the presets whose
/// label or `search_name` matches and their groups.
fn shown_rows<K>(rows: &[PresetRow<K>], open: &[bool], matcher: Option<&Matcher>) -> Vec<usize> {
    if let Some(matcher) = matcher {
        let found = |text: &str| matcher.score(text).is_some();
        let mut keep = vec![false; rows.len()];
        let mut path: Vec<usize> = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            path.truncate(row.depth);
            if !row.is_group() && (found(&row.label) || row.search_name.as_deref().is_some_and(found)) {
                keep[at] = true;
                for &ancestor in &path {
                    keep[ancestor] = true;
                }
            }
            path.push(at);
        }
        return (0..rows.len()).filter(|&at| keep[at]).collect();
    }
    let open = |at: usize| open.get(at).copied().unwrap_or(false);
    let mut out = Vec::with_capacity(rows.len());
    let mut at = 0;
    while at < rows.len() {
        out.push(at);
        at = if rows[at].is_group() && !open(at) { children_end(rows, at) } else { at + 1 };
    }
    out
}

/// The glyph of a group row in both views: an open or a closed folder.
fn folder(open: bool) -> IconId {
    IconId::new(if open { "folder_open" } else { "folder" })
}

/// The grid's tiles and group headers from the shown rows: each header covers the presets that follow it inside its
/// group.
struct Plan<K> {
    tiles: Vec<usize>,
    groups: Vec<TileGroup<BrowserKey<K>>>,
    group_rows: Vec<usize>,
}

fn grid_plan<K: Clone>(
    rows: &[PresetRow<K>],
    shown: &[usize],
    open: impl Fn(usize) -> bool,
    count: impl Fn(usize) -> Option<String>,
    select_groups: bool,
) -> Plan<K> {
    let mut plan = Plan { tiles: Vec::new(), groups: Vec::new(), group_rows: Vec::new() };
    let mut counting = false;
    for &at in shown {
        let row = &rows[at];
        if row.is_group() {
            let open = open(at);
            let mut group = TileGroup::new(BrowserKey::Preset(row.key.clone()), plan.tiles.len(), 0, row.label.clone())
                .open(open)
                .depth(row.depth)
                .icon(folder(open));
            if select_groups {
                group = group.selectable();
            }
            if let Some(detail) = count(at) {
                group = group.detail(detail);
            }
            plan.groups.push(group);
            plan.group_rows.push(at);
            counting = true;
            continue;
        }
        plan.tiles.push(at);
        if counting
            && let (Some(group), Some(&head)) = (plan.groups.last_mut(), plan.group_rows.last())
            && row.depth > rows[head].depth
        {
            group.count += 1;
        } else {
            counting = false;
        }
    }
    plan
}

/// The position in the full row list that a place `to` among the shown rows means: `to` counts the rows that remain
/// shown once `moved` are out (none for a drop); the result counts all rows that remain once the moved rows and the
/// rows inside moved groups are out, as the library's reorder takes it.
fn full_position<K: Eq>(rows: &[PresetRow<K>], shown: &[usize], moved: &[K], to: usize) -> usize {
    let mut gone = vec![false; rows.len()];
    for key in moved {
        if let Some(at) = rows.iter().position(|row| row.key == *key) {
            gone[at..children_end(rows, at)].fill(true);
        }
    }
    let anchor = shown.iter().copied().filter(|&at| !moved.contains(&rows[at].key)).nth(to);
    match anchor {
        Some(anchor) => (0..anchor).filter(|&at| !gone[at]).count(),
        None => gone.iter().filter(|gone| !**gone).count(),
    }
}

/// The rows stamp of a query: another query starts the lead over.
fn query_stamp(query: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    query.hash(&mut hasher);
    hasher.finish()
}

fn list_thumb(zoom: f64) -> ThumbSize {
    if zoom >= LARGE_FROM {
        ThumbSize::Large
    } else if zoom < SMALL_UNTIL {
        ThumbSize::Small
    } else {
        ThumbSize::Medium
    }
}

fn card_aspect(shape: CellShape) -> f32 {
    match shape {
        CellShape::Square => 1.0,
        CellShape::Ratio { wide, high } => f32::from(wide) / f32::from(high.max(1)),
    }
}

/// What a pass of either view reported that the browser acts on.
struct Pass<K> {
    seen: Vec<Seen<K>>,
    grid: Vec<GridEvent<K>>,
}

enum Seen<K> {
    Selection(SelectionChange<K>),
    Select(K),
    Dropped(DropAt<K>, DropOperation, DropPayload),
    Activate(K),
    Open(OpenChange<K>),
    Menu(K),
    Reorder(EditEvent<ReorderMove<K>>),
    Rename(EditEvent<Rename<K>>),
    Delete(Vec<K>),
}

/// The events of a collection that the browser acts on, whatever its items report.
fn seen<K: Clone, E>(events: Vec<CollectionEvent<K, E>>) -> Vec<Seen<K>> {
    let mut out = Vec::with_capacity(events.len());
    for event in events {
        match event {
            CollectionEvent::Selection { change, .. } => {
                let picked = match &change {
                    SelectionChange::Set(keys) => keys.last().cloned(),
                    _ => None,
                };
                out.push(Seen::Selection(change));
                out.extend(picked.map(Seen::Select));
            }
            // A click on the sole selected preset picks it again, as `Select` says.
            CollectionEvent::Picked { key, .. } => out.push(Seen::Select(key)),
            CollectionEvent::Activate(key) => out.push(Seen::Activate(key)),
            CollectionEvent::Open(change) => out.push(Seen::Open(change)),
            CollectionEvent::ContextMenu(request) => out.push(Seen::Menu(request.key)),
            CollectionEvent::Reorder(edit) => out.push(Seen::Reorder(edit)),
            CollectionEvent::Rename(edit) => out.push(Seen::Rename(edit)),
            CollectionEvent::Delete(keys) => out.push(Seen::Delete(keys)),
            CollectionEvent::Dropped { at, op, payload, .. } => out.push(Seen::Dropped(at, op, payload)),
            _ => {}
        }
    }
    out
}

/// What the browser keeps in its container's memory: the row the open menu was asked for, and the state as read and
/// as the pass changes it, which the body and the footer region share. The binding is written once, by the last
/// region that runs.
#[derive(Clone)]
struct BrowserMemory<K> {
    menu_target: Option<K>,
    before: PresetState<K>,
    local: PresetState<K>,
}

impl<K> Default for BrowserMemory<K> {
    fn default() -> Self {
        BrowserMemory { menu_target: None, before: PresetState::new(), local: PresetState::new() }
    }
}

type StateBind<'a, K> = RefCell<Bind<'a, PresetState<K>>>;

/// Writes the state the pass changed through the binding.
fn commit<K: Clone + Eq + Hash + Send + Sync + 'static>(cx: &mut BodyCx<'_>, state: &StateBind<'_, K>, rows: &[PresetRow<K>], name: &str) {
    let (before, mut local) = {
        let memory = cx.keep::<BrowserMemory<K>>();
        (memory.before.clone(), memory.local.clone())
    };
    local.retain_groups(rows);
    if local != before {
        let event = EditEvent::set(Mixed::Same(before), local, Arc::from(name), None);
        let _ = state.borrow_mut().reborrow().write(cx, &event);
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static> PresetBrowser<'_, K> {
    /// Asks the browser with the id `browser` (the one given to `browser_id`, or the `scope_id` of the `Ui`
    /// it is shown in) to start renaming the row `key` at its next pass, as the row menu's Rename does. A key the
    /// browser lacks or may not rename is ignored; the request is taken once.
    pub fn request_rename(ctx: &egui::Context, browser: Id, key: &K) {
        request_rename(ctx, rows_id(browser), &BrowserKey::Preset(key.clone()));
    }

    /// Shows the browser across the available width and reports what the user did, in order.
    pub fn show(self, ui: &mut Ui) -> PresetOutput<K> {
        let PresetBrowser {
            label,
            browser_id,
            state,
            source,
            selection,
            select,
            select_groups,
            reveal,
            cell_shape,
            tiles: tile_size,
            thumb_range,
            default_open,
            toolbar,
            search,
            mut tools,
            tools_at_start,
            row_menu,
            row_items,
            drags,
            group_heads,
            view_lock,
            reorder,
            drop_zone,
            actions,
            tip: tip_painter,
            drops,
            payloads,
            search_first,
            footer,
            max_rows,
            room_height,
            columns,
            empty: empty_text,
        } = self;

        let env = Env::of(ui);
        let words = env.words::<PresetWords>();
        let list_id = rows_id(browser_id.unwrap_or_else(|| ui.scope_id()));
        let name = label.as_str().to_owned();
        let has_footer = footer.is_some();
        let state = RefCell::new(state);
        let selection = RefCell::new(selection);
        let footer_events: RefCell<Vec<PresetFooterEvent<K>>> = RefCell::new(Vec::new());
        let kept = source.rows(ui.ctx(), &env);
        let rows = kept.as_slice();

        let body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let before = match state.borrow_mut().reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => PresetState::new(),
            };
            let mut local = before.clone();
            local.zoom = local.zoom_in(&thumb_range);
            let menu_target = {
                let memory = cx.keep::<BrowserMemory<K>>();
                memory.before = before;
                memory.local = local.clone();
                memory.menu_target.clone()
            };

            if toolbar {
                let modes = [
                    Entry::new(PresetView::Grid, words.grid_view.clone()).icon(IconId::new("layout_grid")),
                    Entry::new(PresetView::List, words.list_view.clone()).icon(IconId::new("list")),
                ];
                let locked = view_lock.is_some();
                let bar = Bar::new(Label::Empty, BarKind::Tool);
                let mut controls = |ui: &mut Ui| {
                    if search {
                        let field = TextField::search(words.search.clone(), Bind::bound(&mut local.filter)).placeholder(words.search.clone());
                        let field = field.drives(list_id);
                        let field = if search_first { field.default_stop() } else { field };
                        field.show(ui);
                    }
                    if !locked {
                        Segmented::new(words.view.clone(), Bind::bound(&mut local.view), &modes).show(ui);
                    }
                };
                if let Some(tools) = tools.as_mut() {
                    if tools_at_start {
                        let _ = bar.trailing(|ui, _| controls(ui)).show(ui, |ui, _| tools.show(ui));
                    } else {
                        let _ = bar.trailing(|ui, _| tools.show(ui)).show(ui, |ui, _| controls(ui));
                    }
                } else {
                    let _ = bar.show(ui, |ui, _| controls(ui));
                }
            }
            // A hidden search field filters nothing: a search typed before hiding it would otherwise keep presets
            // out of the list with no field to show why.
            if toolbar && !search {
                local.filter.clear();
            }
            // Under `Rule` every group is open and the state's open groups are not read.
            let open: Vec<bool> = if group_heads == GroupHeads::Rule {
                vec![true; rows.len()]
            } else {
                let test = local.open_test(rows, default_open);
                (0..rows.len()).map(test).collect()
            };
            let mut held: Selection<K> = match selection.borrow_mut().as_mut().map(|bind| bind.reborrow().read(cx)) {
                Some(Some(Mixed::Same(held))) => held,
                _ => Selection::default(),
            };
            let mut sent = Vec::new();
            let props = ListProps {
                source,
                label: label.clone(),
                list_id,
                open,
                query: local.filter.trim().to_owned(),
                view: view_lock.unwrap_or(local.view),
                zoom: local.zoom,
                selection: held.clone(),
                select,
                select_groups,
                reveal: reveal.clone(),
                menu_target,
                cell_shape,
                tiles: tile_size,
                columns,
                max_rows,
                reorder,
                drop_zone,
                actions: &actions,
                row_menu,
                drags,
                group_heads,
                host: (tip_painter.is_some() || row_items.is_some() || !drops.is_empty() || !payloads.is_empty()).then(|| HostFns {
                    tip: tip_painter.as_ref(),
                    row_items: row_items.as_ref(),
                    drops: &drops,
                    payloads: &payloads,
                }),
                empty: empty_text.clone(),
                sent: Outbox::new(&mut sent),
            };
            let _ = MemoView::new("presets", props, list_view::<K>).show(ui);
            let mut events = Vec::new();
            let mut menu_target = cx.keep::<BrowserMemory<K>>().menu_target.clone();
            for run in sent {
                for event in run.events {
                    match event {
                        // A bound selection takes the change; a given one reports it.
                        PresetEvent::Selection(change) => {
                            let before = held.clone();
                            held.apply(change.clone());
                            let edit = EditEvent::set(Mixed::Same(before), held.clone(), Arc::from(name.as_str()), None);
                            let written = selection.borrow_mut().as_mut().is_some_and(|bind| bind.reborrow().write(cx, &edit));
                            if !written {
                                events.push(PresetEvent::Selection(change));
                            }
                        }
                        event => {
                            if let PresetEvent::Open(change) = &event {
                                local.apply_open(rows, change, default_open);
                            }
                            events.push(event);
                        }
                    }
                }
                menu_target = run.menu_target;
            }

            let memory = cx.keep::<BrowserMemory<K>>();
            memory.local = local;
            memory.menu_target = menu_target;
            if !has_footer {
                commit(cx, &state, rows, &name);
            }
            events
        };

        let footer_region = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let Some(options) = footer.as_ref() else { return };
            let mut zoom = cx.keep::<BrowserMemory<K>>().local.zoom;
            let count = rows.iter().filter(|row| !row.is_group()).count();
            let bar = PresetFooter::new(Bind::bound(&mut zoom)).range(thumb_range).count(count).options(options.clone());
            *footer_events.borrow_mut() = bar.show(ui).events;
            cx.keep::<BrowserMemory<K>>().local.zoom = zoom;
            commit(cx, &state, rows, &name);
        };

        let stack = room_height.map_or_else(|| Stack::new().fill(), |height| Stack::new().fill_width().room_height(height));
        let stack = if has_footer { stack.footer(footer_region) } else { stack };
        let shown = stack.show(ui, body);
        let mut events = shown.inner.unwrap_or_default();
        events.extend(footer_events.take().into_iter().map(PresetEvent::Footer));
        let drag = crate::runtime::drag::active::<ListDrag<K>>(ui.ctx()).filter(|drag| drag.list == list_id).map(|drag| (*drag).clone());
        PresetOutput::new(shown.output.clone(), events, drag)
    }
}

/// The host's closures a run of the presets view calls: the tip painter, the row menu's rows and the drop targets.
#[derive(crate::contract::MemoProps)]
#[memo_props(every_pass = "a closure reads what it captured, which no key can feed")]
struct HostFns<'p, 'a, K> {
    tip: Option<&'p TipFn<'a, K>>,
    row_items: Option<&'p RowItems<'a, K>>,
    drops: &'p [PresetDrop<'a, K>],
    payloads: &'p [DragPayload],
}

/// Every input of the browser's presets view: the source, the state the toolbar left (the open groups, the search,
/// the view and the zoom), the host's selection, the row menu's target, the browser's plain settings and the host's
/// closures, and an outbox for what a run reports.
#[derive(crate::contract::MemoProps)]
struct ListProps<'p, 'a, K: 'static> {
    source: &'p PresetSource<'a, K>,
    label: Label,
    list_id: Id,
    /// Whether each row is open, by its index in the rows.
    open: Vec<bool>,
    query: String,
    view: PresetView,
    zoom: f64,
    selection: Selection<K>,
    select: SelectMode,
    select_groups: bool,
    reveal: Option<K>,
    menu_target: Option<K>,
    cell_shape: CellShape,
    tiles: PresetTiles,
    columns: u16,
    max_rows: Option<u16>,
    reorder: Option<PresetReorder>,
    drop_zone: Option<DropZone>,
    actions: &'p [Label],
    row_menu: PresetRowMenu,
    drags: bool,
    group_heads: GroupHeads,
    /// The host's closures; `None` without any, so the view keys on its plain props alone.
    host: Option<HostFns<'p, 'a, K>>,
    empty: Option<Label>,
    sent: Outbox<'p, ListRun<K>>,
}

/// What a run of the presets view reports: the events, in order (each open change and each selection change once,
/// which the browser applies to its state and to the host's selection), and the row the menu acts on after the run.
struct ListRun<K> {
    events: Vec<PresetEvent<K>>,
    menu_target: Option<K>,
}

/// The presets view: the grid or the list of the shown rows, the drop zone and the row menu. The pointer is read only
/// in a run that has collection events to place, so a pointer moving elsewhere does not run the view.
#[expect(
    clippy::cognitive_complexity,
    reason = "the presets view maps both collections' events onto the browser's from one set of shared locals, so a split would pass them all through"
)]
fn list_view<K: Clone + Eq + Hash + Send + Sync + MemoProps + 'static>(props: &mut ListProps<'_, '_, K>, ui: &mut Ui) {
    let env = Env::of(ui);
    let words = env.words::<PresetWords>();
    let source = props.source;
    let kept = source.rows(ui.ctx(), &env);
    let rows = kept.as_slice();
    let ListProps {
        label,
        list_id,
        view,
        zoom,
        cell_shape,
        tiles: tile_size,
        columns,
        max_rows,
        reorder,
        drop_zone,
        row_menu,
        select,
        drags,
        group_heads,
        ..
    } = &*props;
    let (list_id, view, zoom, cell_shape, tile_size, columns, max_rows, reorder, drop_zone, row_menu, select, drags, group_heads) =
        (*list_id, *view, *zoom, *cell_shape, *tile_size, *columns, *max_rows, *reorder, *drop_zone, *row_menu, *select, *drags, *group_heads);
    let label = label.clone();
    let actions = props.actions;
    let (tip_painter, row_items, drops, payloads) =
        props.host.as_ref().map_or((None, None, &[][..], &[][..]), |host| (host.tip, host.row_items, host.drops, host.payloads));
    let reveal = props.reveal.clone();
    let mut held = props.selection.clone();
    let mut menu_target = props.menu_target.clone();
    let mut open_flags = props.open.clone();
    open_flags.resize(rows.len(), false);
    let mut events: Vec<PresetEvent<K>> = Vec::new();

    let query = props.query.clone();
    let matcher = (!query.is_empty()).then(|| Matcher::new(&env.folder(), &query));
    let shown = shown_rows(rows, &open_flags, matcher.as_ref());
    let counted = |at: usize| rows[at].count.map(|n| words.group_count.text(n as u64, &env.format().integer(n as u64)));
    let ruled_heads = group_heads == GroupHeads::Rule;
    let select_groups = props.select_groups && !ruled_heads && select != SelectMode::None;
    let shown_selection = Selection::new(
        held.items.iter().cloned().map(BrowserKey::Preset).collect(),
        held.lead.clone().map(BrowserKey::Preset),
        held.anchor.clone().map(BrowserKey::Preset),
    );
    let payload_drag = reorder.is_some() || drop_zone.is_some() || !drops.is_empty() || !payloads.is_empty() || drags;
    let stamp = query_stamp(&query);

    let can_drag = |key: &BrowserKey<K>| {
        rows.iter().find(|row| key.preset() == Some(&row.key)).is_some_and(|row| !row.is_group() || reorder.is_some_and(|rules| rules.moves_groups()))
    };
    // No drop lands on an action row; the list also keeps every drop before them.
    let accepts = |drag: &ListDrag<BrowserKey<K>>, over: Option<&BrowserKey<K>>, insert: usize| {
        let (Some(rules), Some(drag)) = (reorder, app_drag(drag)) else { return false };
        match over {
            Some(BrowserKey::Action(_) | BrowserKey::Separator) => false,
            over => rules.allows(source, rows, &drag, over.and_then(BrowserKey::preset), insert),
        }
    };
    let list_accepts =
        |drag: &ListDrag<BrowserKey<K>>, over: Option<&BrowserKey<K>>, insert: usize| insert <= shown.len() && accepts(drag, over, insert);
    let rename_row = |key: &BrowserKey<K>| key.preset().is_some_and(|key| source.can_rename(key));
    let paint = |row: &PresetRow<K>, selected: bool| {
        let key = row.key.clone();
        let preview = row.preview.clone();
        move |cx: &ContentPainterCx, rect: Rect| {
            if !source.thumb(cx, rect, ThumbInfo::new(&key, selected, false))
                && let Some(preview) = &preview
            {
                preview.paint.paint(cx, rect);
            }
        }
    };

    let (response, map, pass) = if shown.is_empty() && actions.is_empty() {
        let message = if query.is_empty() { props.empty.clone().unwrap_or_else(|| words.empty.clone()) } else { words.no_match.clone() };
        let response = Notice::new(Tone::Neutral, message).layout(NoticeLayout::Centred).show(ui).response;
        (response, Rc::new(RowMap { rows, displayed: Vec::new() }), Pass { seen: Vec::new(), grid: Vec::new() })
    } else if view == PresetView::Grid {
        let plan = grid_plan(rows, &shown, |at| open_flags[at], counted, select_groups);
        let tiles = Shown { rows, at: plan.tiles.clone(), open: vec![false; plan.tiles.len()], tree: false, actions: 0 };
        // The grid's positions count its group header rows as the list's count its group rows: the shown rows.
        let map = Rc::new(RowMap { rows, displayed: shown.clone() });
        let tail: Vec<TailRow<BrowserKey<K>>> =
            actions.iter().enumerate().map(|(at, label)| TailRow::new(BrowserKey::Action(at), label.clone())).collect();
        let aspect = card_aspect(cell_shape);
        let grid = TileGrid::<BrowserKey<K>>::new(label.clone())
            .id(list_id)
            .select(select)
            .arrows_select(false)
            .selection(Bind::given(shown_selection))
            .aspect(aspect);
        let grid = match tile_size {
            PresetTiles::Thumbnails => {
                let captions = if has_captions(rows) { CaptionPlace::Below } else { CaptionPlace::None };
                grid.caption(captions).thumb_size(ThumbSize::Large).zoom(GridZoom::Fixed(zoom))
            }
            // The zoom is a factor on the picture's width: a wide chip is the small thumbnail high.
            PresetTiles::Chips => grid
                .caption(CaptionPlace::None)
                .thumb_size(ThumbSize::Small)
                .zoom(GridZoom::Fixed(f64::from(aspect.max(1.0))))
                .row_height(RowHeight::Text),
        };
        let mut grid = grid
            .groups(&plan.groups)
            .tail(&tail)
            .group_heads(group_heads)
            .rename(true)
            .deletable(true)
            .rows_stamp(stamp)
            .can_rename(rename_row)
            .can_drag(can_drag)
            .accepts(accepts)
            .card_columns(usize::from(columns));
        if reorder.is_some() {
            grid = grid.reorder(ReorderRule::new());
        }
        if !ruled_heads {
            grid = grid.collapsible();
        }
        if payload_drag {
            grid = grid.drag_payload(app_drag::<K>);
        }
        for payload in payloads {
            grid = grid.carry(payload.clone());
        }
        for drop in drops {
            grid = grid.takes_drop(drop.kind(Rc::clone(&map)));
        }
        if let Some(key) = &reveal {
            grid = grid.reveal(BrowserKey::Preset(key.clone()));
        }
        if let Some(rows) = max_rows {
            grid = grid.max_rows(rows);
        }
        let aspect = cell_shape.aspect();
        let out = grid.show_items(ui, &tiles, &mut |key, icx| {
            let Some(row) = tiles.index_of(key).and_then(|at| tiles.row(at)) else { return Tile::new(Label::default()) };
            if let Some(tip) = tip_painter {
                let key = row.key.clone();
                icx.tip(move |ui| tip(ui, &key));
            }
            let mut tile = Tile::new(row.label.clone());
            if let Some(detail) = &row.caption {
                tile = tile.detail(detail.clone());
            }
            if row.dimmed {
                tile = tile.dimmed();
            }
            if let Some(tip) = &row.tooltip {
                tile = tile.tooltip(tip.clone());
            }
            if let Some(name) = &row.accessible_name {
                tile = tile.accessible_name(name.clone());
            }
            let draw = paint(row, icx.row.selected);
            tile.thumb(row.preview.as_ref().map_or(aspect, |preview| preview.aspect), move |cx, rect| draw(cx, rect))
        });
        (out.response.response.clone(), map, Pass { seen: seen(out.response.events), grid: out.events })
    } else {
        let open: Vec<bool> = shown.iter().map(|&at| open_flags[at]).collect();
        let source_rows = Shown { rows, at: shown.clone(), open, tree: true, actions: actions.len() };
        let map = Rc::new(RowMap { rows, displayed: shown.clone() });
        let thumb_size = list_thumb(zoom);
        let mut list = List::<BrowserKey<K>>::new(label.clone())
            .id(list_id)
            .tree(true)
            .group_heads(group_heads)
            .select(select)
            .arrows_select(false)
            .selection(Bind::given(shown_selection))
            .rename(true)
            .deletable(true)
            .rows_stamp(stamp)
            .can_rename(rename_row)
            .can_drag(can_drag)
            .accepts(list_accepts);
        if reorder.is_some() {
            list = list.reorder(ReorderRule::new().into_groups());
        }
        if payload_drag {
            list = list.drag_payload(app_drag::<K>);
        }
        for payload in payloads {
            list = list.carry(payload.clone());
        }
        for drop in drops {
            list = list.takes_drop(drop.kind(Rc::clone(&map)));
        }
        if let Some(key) = &reveal {
            list = list.reveal(BrowserKey::Preset(key.clone()));
        }
        if let Some(rows) = max_rows {
            list = list.max_rows(rows);
        }
        let aspect = cell_shape.aspect();
        let out = list.show_items(ui, &source_rows, &mut |key, icx| {
            let Some(&at) = source_rows.at.get(icx.at) else {
                return match key {
                    BrowserKey::Action(action) => ListRow::new(actions[*action].clone()),
                    _ => ListRow::separator(),
                };
            };
            let row = &rows[at];
            if row.is_group() {
                let mut line =
                    ListRow::new(row.label.clone()).heading().depth(row.depth).group(icx.row.open).icon(IconSpec::svg(folder(icx.row.open)));
                if let Some(detail) = counted(at) {
                    line = line.detail(detail);
                }
                return line;
            }
            if let Some(tip) = tip_painter {
                let key = row.key.clone();
                icx.tip(move |ui| tip(ui, &key));
            }
            let mut line = ListRow::new(row.label.clone()).depth(row.depth);
            if let Some(caption) = &row.caption {
                line = line.detail(caption.clone());
            }
            if row.dimmed {
                line = line.ink(RowInk::Dimmed);
            }
            if let Some(tip) = &row.tooltip {
                line = line.tooltip(tip.clone());
            }
            if let Some(name) = &row.accessible_name {
                line = line.accessible_name(name.clone());
            }
            for (icon, name) in &row.trailing {
                line = line.trailing_icon(icon.clone(), name.clone());
            }
            let draw = paint(row, icx.row.selected);
            line.thumb_aspect(thumb_size, row.preview.as_ref().map_or(aspect, |preview| preview.aspect), move |cx, rect| draw(cx, rect))
        });
        (out.response.clone(), map, Pass { seen: seen(out.events), grid: Vec::new() })
    };

    let row_of = |key: &K| rows.iter().position(|row| row.key == *key);
    if let Some(zone) = drop_zone {
        let target = ZoneDrop { source: response.id, rows, zone };
        if let Some((key, at)) = drag::zone_target(ui, list_id.with("drop zone"), zone, &target) {
            events.push(PresetEvent::DropAt { key, at, mods: crate::runtime::pointer::frame(ui.ctx()).pick_mods });
        }
    }
    let toggle = |flags: &mut Vec<bool>, events: &mut Vec<PresetEvent<K>>, at: usize, open: Option<bool>| {
        if ruled_heads {
            return;
        }
        let open = open.unwrap_or_else(|| !flags[at]);
        flags[at] = open;
        events.push(PresetEvent::Open(OpenChange::new(rows[at].key.clone(), open)));
    };
    for event in pass.grid {
        match event {
            GridEvent::GroupOpen { group, open } => {
                if let Some(&at) = grid_group_row(&shown, rows, group) {
                    toggle(&mut open_flags, &mut events, at, Some(open));
                }
            }
            GridEvent::GroupPicked { group, .. } if !select_groups => {
                if let Some(&at) = grid_group_row(&shown, rows, group) {
                    toggle(&mut open_flags, &mut events, at, None);
                }
            }
            GridEvent::GroupMenu { group, .. } => {
                if let Some(&at) = grid_group_row(&shown, rows, group) {
                    menu_target = Some(rows[at].key.clone());
                }
            }
            _ => {}
        }
    }
    if !pass.seen.is_empty() {
        // A secondary press or release selects the row it lands on so that the row's menu acts on it; that selection
        // is not a pick, and a picker would commit it.
        let frame = crate::runtime::pointer::frame(ui.ctx());
        let secondary = frame.buttons.contains(craft_behavior::Buttons::SECONDARY) || frame.release == Some(craft_behavior::PointerButton::Secondary);
        let mods = frame.pick_mods;
        // An action row of the list is a plain row: a single press picks it, and Enter activates it. Its selection by
        // an arrow key, and the activation by the second click of a double click, are not a choice. The grid's
        // tail rows report Enter and a click as their activation.
        let list = view == PresetView::List;
        let first_press = frame.press.as_ref().is_some_and(|press| press.clicks == 1);
        let released = frame.release.is_some();
        let action = |events: &mut Vec<PresetEvent<K>>, at: usize| {
            if !events.contains(&PresetEvent::Action(at)) {
                events.push(PresetEvent::Action(at));
            }
        };
        for event in pass.seen {
            match event {
                Seen::Selection(change) => {
                    let selectable = |key: &K| row_of(key).is_some_and(|at| select_groups || !rows[at].is_group());
                    if let Some(change) = app_change(change, selectable) {
                        held.apply(change.clone());
                        events.push(PresetEvent::Selection(change));
                    }
                }
                Seen::Select(BrowserKey::Preset(key)) => {
                    if !secondary && !mods.shift && !mods.cmd && row_of(&key).is_some_and(|at| !rows[at].is_group()) {
                        events.push(PresetEvent::Select { key, mods });
                    }
                }
                Seen::Dropped(at, op, payload) => {
                    if let Some(at) = map.host(&at) {
                        events.push(PresetEvent::Dropped { at, op, payload, mods });
                    }
                }
                Seen::Select(BrowserKey::Action(at)) => {
                    if list && first_press && !secondary {
                        action(&mut events, at);
                    }
                }
                Seen::Activate(BrowserKey::Preset(key)) => match row_of(&key) {
                    Some(at) if rows[at].is_group() => toggle(&mut open_flags, &mut events, at, None),
                    Some(_) => events.push(PresetEvent::Activate { key, mods }),
                    None => {}
                },
                Seen::Activate(BrowserKey::Action(at)) => {
                    if !(list && released) {
                        action(&mut events, at);
                    }
                }
                Seen::Select(BrowserKey::Separator) | Seen::Activate(BrowserKey::Separator) => {}
                Seen::Open(change) => {
                    if !ruled_heads && let BrowserKey::Preset(key) = change.key {
                        events.push(PresetEvent::Open(OpenChange { key, open: change.open, all: change.all }));
                    }
                }
                Seen::Menu(key) => {
                    if let BrowserKey::Preset(key) = key {
                        menu_target = Some(key);
                    }
                }
                Seen::Reorder(edit) => {
                    let moved = |mv: ReorderMove<BrowserKey<K>>| {
                        let mv = app_move(mv)?;
                        let to = full_position(rows, &map.displayed, &mv.keys, mv.to);
                        Some(ReorderMove { to, ..mv })
                    };
                    events.extend(edit.filter_map(moved).map(PresetEvent::Reorder));
                }
                // Delete and Backspace give the same event as the row menu's Delete and the footer button; an
                // open rename field takes the keys itself.
                Seen::Delete(keys) => {
                    for key in keys {
                        if let BrowserKey::Preset(key) = key
                            && source.can_delete(&key)
                        {
                            events.push(PresetEvent::Delete(key));
                        }
                    }
                }
                Seen::Rename(edit) => {
                    let ended = matches!(edit.phase(), Some(GesturePhase::Commit) | None);
                    if let (true, Some(rename)) = (ended, edit.value())
                        && let BrowserKey::Preset(key) = &rename.key
                    {
                        let text = rename.name.trim();
                        let changed = row_of(key).is_some_and(|at| rows[at].label != text);
                        if changed && !text.is_empty() {
                            events.push(PresetEvent::Rename { key: key.clone(), text: text.to_owned() });
                        }
                    }
                }
            }
        }
    }

    if row_menu != PresetRowMenu::Off
        && let Some(target) = menu_target.clone().or_else(|| held.lead.clone()).filter(|key| row_of(key).is_some())
    {
        let at = row_of(&target).unwrap_or_default();
        let group = rows[at].is_group();
        let deletable = source.can_delete(&target);
        let renamable = source.can_rename(&target) && (!group || view == PresetView::List);
        let movable = !group && source.can_move(&target);
        let home = rows[..at].iter().rposition(|row| row.is_group() && row.depth < rows[at].depth);
        let destinations: Vec<usize> = (0..rows.len()).filter(|&to| rows[to].is_group() && Some(to) != home).collect();
        let standard = row_menu == PresetRowMenu::Standard;
        let host_rows = row_items.map(|items| items.items(&target)).unwrap_or_default();
        let mut ran: Vec<PresetEvent<K>> = Vec::new();
        let menu = Menu::context(words.menu.clone(), &response).show(ui, |ui, _| {
            let (rename, delete) = if group {
                (PresetItemCommands::RenameGroup, PresetItemCommands::DeleteGroup)
            } else {
                (PresetItemCommands::Rename, PresetItemCommands::Delete)
            };
            if standard && MenuItem::command(rename).enabled(renamable).show(ui).ran {
                request_rename(ui.ctx(), list_id, &BrowserKey::Preset(target.clone()));
            }
            if standard && movable && !destinations.is_empty() {
                let _ = Menu::submenu_row(ui, words.move_to.clone(), |ui, _| {
                    for &to in &destinations {
                        if MenuItem::new(Label::from(rows[to].label.clone())).show(ui).ran {
                            ran.push(PresetEvent::MoveTo { key: target.clone(), group: rows[to].key.clone() });
                        }
                    }
                });
            }
            if standard && MenuItem::command(delete).enabled(deletable).show(ui).ran {
                ran.push(PresetEvent::Delete(target.clone()));
            }
            if standard && !host_rows.is_empty() {
                Divider::horizontal().show(ui);
            }
            for entry in &host_rows {
                if entry.is_separator() {
                    Divider::horizontal().show(ui);
                } else if let Some(value) = entry.value() {
                    let mut row = MenuItem::new(entry.label().clone()).enabled(entry.is_enabled()).entry_checked(entry.is_checked());
                    if let Some(reason) = entry.why_disabled() {
                        row = row.tooltip(TooltipSpec::new(reason.clone()).disabled_text(reason.clone()));
                    }
                    if row.show(ui).ran {
                        ran.push(PresetEvent::Menu { target: target.clone(), command: value.clone() });
                    }
                }
            }
        });
        if menu.output.closed.is_some() {
            menu_target = None;
        }
        events.extend(ran);
    }
    props.sent.send(ListRun { events, menu_target });
}

/// The row of the header `group` of the grid's plan over the shown rows.
fn grid_group_row<'s, K>(shown: &'s [usize], rows: &[PresetRow<K>], group: usize) -> Option<&'s usize> {
    shown.iter().filter(|&&at| rows[at].is_group()).nth(group)
}

/// The id of the browser's list, derived from the browser's id.
fn rows_id(scope: Id) -> Id {
    scope.with("browser-rows")
}

//! The item step (17.3): the collection's side of steps 4 to 17 and its content sub-steps I1 to I5.

pub(super) mod rows;

use super::collection::{CollectionEvent, ColumnLayout, LeadSource, ShownRow};
use super::{Collection, ItemFn};
use crate::runtime::egui::areas;
use craft_behavior::{Axis, GestureId, ItemSource, ScrollPos, TypeAhead};
use egui::{Pos2, Rect, Ui, Vec2};
use std::ops::Range;
use std::sync::Arc;

/// Runs `body` with the item facts of the item at `at` of a set installed and the id that position gives the item, so
/// measuring and showing an item see one id and one environment.
fn with_item_facts<R>(ui: &mut Ui, at: crate::runtime::access::SetPosition, body: impl FnOnce(&mut Ui, egui::Id) -> R) -> R {
    use crate::runtime::env::{Env, ItemFacts};
    // The id comes from the position so two items of one set never share one, and an item keeps its id while its
    // position holds.
    let id = ui.scope_id().with(("shown item", at.index));
    let facts = ItemFacts {
        item: id,
        flags: crate::style::Flags::empty(),
        position: at,
        columns: Vec::new(),
        toggles: Vec::new(),
        toggle_keys: Vec::new(),
        aligns: Vec::new(),
        pressed: None,
        menu_open: false,
    };
    let installed = Env::install_for(ui, Env::of(ui).derive_local(|scoped| scoped.item_facts = Some(Arc::new(facts))));
    let shown = body(ui, id);
    Env::restore(ui.ctx(), installed);
    shown
}

/// Measures `item` as the item at `at` of a set, under the id and the item facts [`show_item`] shows it with.
pub fn measure_item<C>(ui: &mut Ui, item: &mut C, at: crate::runtime::access::SetPosition, proposal: craft_behavior::Proposal) -> Vec2
where
    C: super::Component + super::Props + super::Show,
{
    with_item_facts(ui, at, |ui, id| super::run::measure_under(item, ui, Some(id), proposal, None))
}

/// Shows `item` in `rect` as the item at `at` of a set, with the item facts a collection installs.
pub fn show_item<C>(ui: &mut Ui, rect: Rect, item: C, at: crate::runtime::access::SetPosition) -> C::Output
where
    C: super::Component + super::Props + super::Show,
{
    with_item_facts(ui, at, |ui, id| super::Props::id(item, id).show_at(ui, rect))
}

/// Shows `item` in the flow of `ui` as the item at `at` of a set, taking the room the flow gives it.
pub fn show_item_in_flow<C>(ui: &mut Ui, item: C, at: crate::runtime::access::SetPosition) -> C::Output
where
    C: super::Component + super::Props + super::Show,
{
    with_item_facts(ui, at, |ui, id| super::Props::id(item, id).show(ui))
}

mod drag;
mod edge;
pub(super) mod empty_text;
pub(crate) mod header;
mod marquee;
mod request;

pub use drag::{SweepApply, SweepStep};
pub use header::ColumnCommands;
pub(crate) use header::HeaderFacts;
#[doc(hidden)]
pub use header::put_toggle_names;
pub use press::item_part_open;
#[doc(hidden)]
pub use request::lead_position;
pub use request::request_rename;

use drag::Drags;
use header::Header;

mod access;
mod geometry;
mod hooks;
mod keys;
mod lead;
mod measure;
mod members;
pub use members::item_node;
mod output;
mod press;
mod rename;
mod reveal;

/// The rows the item step builds in a pass and where it puts them: what `PartsCx::rect_of`, `offset` and `rows_view`
/// read.
pub(super) struct Rows<K> {
    pub(super) rects: Vec<(K, Rect)>,
    pub(super) offset: ScrollPos,
    /// The rows' rect less the scroll bar's track while the bar shows.
    pub(super) view: Rect,
}

const SCROLL: craft_behavior::PartKey = craft_behavior::PartKey::new("items_scroll");
const SECTION: craft_behavior::PartKey = craft_behavior::PartKey::new("section");
/// The band of the line between two rows that an edge press takes (`Collection::edge_press`).
const EDGE: craft_behavior::PartKey = craft_behavior::PartKey::new("items_edge");
/// The sub-node of a toggle cell of an item row; the index is the hash of the row's key and the column.
const TOGGLE_CELL: craft_behavior::PartKey = craft_behavior::PartKey::new("toggle_cell");
/// The part an item row's own access node stands for in the collection: AccessKit requests on the row's node reach
/// the collection under this part, the index being the hash of the row's key.
const ITEM_NODE: craft_behavior::PartKey = craft_behavior::PartKey::new("item_node");

/// The part of a collection that stands for the section row `key` (`ItemSource::section_stop`): the item step builds
/// no row for it, so the access node that AccessKit focus goes to while the section holds the lead, and the pointer
/// part over it, are this part of the collection. A component that senses and reports its own section headers
/// declares them as this part, so the lead, the pointer and the access tree treat them as the collection's section
/// rows. One that only paints them paints each as the face of a sense-less part of its own over `PartsCx::rect_of`,
/// and the collection keeps their press, keys and node.
pub fn section_part<K: std::hash::Hash>(key: &K) -> craft_behavior::PartRef {
    SECTION.at(key_hash(key))
}

#[derive(Clone, Default)]
struct Scrolled {
    y: f64,
    x: f64,
    /// The same position named by content: the offset follows it while the heights of rows before its row change.
    anchor: Option<crate::runtime::lazy::Anchor>,
}

#[derive(Clone)]
struct Lead<K> {
    key: Option<K>,
    at: usize,
    /// A sheet's lead cell column.
    column: usize,
    /// The `CollectionProps.rows_stamp` of the pass that last set the lead.
    stamp: Option<u64>,
    /// The lead `CollectionEvent::Rested` was reported for: one report per lead change.
    rested: Option<K>,
}

impl<K> Default for Lead<K> {
    fn default() -> Self {
        Lead { key: None, at: 0, column: 0, stamp: None, rested: None }
    }
}

#[derive(Clone)]
struct Renaming<K> {
    key: Option<K>,
    draft: String,
    gesture: GestureId,
    focused: bool,
    /// The field has held the focus: losing it afterwards ends the rename.
    held: bool,
    /// The field opened with a draft the collection proposed that differs from the row's text.
    proposed: bool,
}

impl<K> Default for Renaming<K> {
    fn default() -> Self {
        Renaming { key: None, draft: String::new(), gesture: GestureId::default(), focused: false, held: false, proposed: false }
    }
}

#[derive(Clone, Default)]
struct Typing(TypeAhead);

/// The item layout of the last pass, kept per collection in session memory.
#[derive(Clone)]
struct LastLayout<L>(Option<Arc<L>>);

impl<L> Default for LastLayout<L> {
    fn default() -> Self {
        LastLayout(None)
    }
}

#[derive(Clone)]
struct Held<S>(S);

impl<S: Default> Default for Held<S> {
    fn default() -> Self {
        Held(S::default())
    }
}

/// The rows the item step walks in a pass: the source the caller gave, or the view `Collection::filtered` made of it,
/// which the hooks own for the pass.
enum Source<'s, K> {
    Given(&'s dyn ItemSource<K>),
    Filtered(Box<dyn ItemSource<K> + 's>),
}

impl<'s, K> std::ops::Deref for Source<'s, K> {
    type Target = dyn ItemSource<K> + 's;

    fn deref(&self) -> &(dyn ItemSource<K> + 's) {
        match self {
            Source::Given(given) => *given,
            Source::Filtered(view) => &**view,
        }
    }
}

pub(super) struct ItemsHooks<'s, 'r, C: Collection> {
    /// The rows of the pass: the read step applies the collection's filter once (`Collection::filtered`).
    source: Source<'s, C::Key>,
    item: &'s mut ItemFn<'r, C>,
    layout: Option<Arc<C::ItemLayout>>,
    /// The layout's column count this pass, which a row pick's whole-row cells span.
    columns: u32,
    /// The layout of the last pass, which `Collection::item_layout_from` may reuse.
    previous: Option<Arc<C::ItemLayout>>,
    content: Vec2,
    room: Vec2,
    /// One step of a tree's indent: the icon side at the collection's size, as its rows indent.
    indent_step: f32,
    /// The axes (x, y) the collection takes from its content under a body sized from its content: the height in every
    /// run, the width in every run except the real run of a collection held in a content-sized column, which shows
    /// across the width the column settles on.
    from_content: (bool, bool),
    scrolls: bool,
    /// The collection senses the wheel itself (a tab strip stepping its tabs): the wheel is its own, not the rows'
    /// scroll.
    own_wheel: bool,
    /// The collection shows at most `max_rows` rows: its height is bounded by its rows, whatever its sizing.
    capped: bool,
    /// The distance one wheel notch scrolls the rows, from `CollectionProps::notch`.
    notch: craft_behavior::WheelNotch,
    /// The axis the rows overflow along.
    overflow: Axis,
    selection: C::Selection,
    selection_at_read: C::Selection,
    events: Vec<CollectionEvent<C::Key, C::ItemEvent>>,
    /// The click count of the press behind this pass's pointer activation, 0 when no pointer activated.
    activation_clicks: u8,
    /// The row the activation key activated this pass, which reads it as `RowState::activated_by_key`.
    keyed_activation: Option<C::Key>,
    visible: Range<usize>,
    /// The rows built this pass with their rects, for `collection_response`.
    shown: Vec<ShownRow<C::Key>>,
    /// The row under the pointer this pass, for `collection_response`.
    hovered: Option<C::Key>,
    offset: f64,
    offset_at_read: f64,
    offset_x: f64,
    offset_x_at_read: f64,
    /// The scroll position named by content, kept with an offset the framework holds (`Scrolled`).
    anchor: Option<crate::runtime::lazy::Anchor>,
    anchor_at_read: Option<crate::runtime::lazy::Anchor>,
    /// The bound column layout as the read step found it; `None` while the header keeps its own memory.
    column_layout_at_read: Option<ColumnLayout>,
    lead: Lead<C::Key>,
    /// The lead row and the renaming row as step 4 read them: a row's measure at step 5 reads them
    /// (`known_row`).
    lead_at_read: Option<C::Key>,
    renaming_at_read: Option<C::Key>,
    renaming: Renaming<C::Key>,
    ended_rename: bool,
    /// The editor of the rename that ended still held focus, so focus goes back to the collection.
    refocus: bool,
    /// The mapping between the screen and the rows when `ContainerProps.transform` zooms them.
    xf: Option<areas::LayerMap>,
    header: Header,
    drags: Drags<C::Key>,
    /// The row a key move brought to the lead in a collection without its own scroll: the page's scroll owner shows it.
    page_reveal: Option<usize>,
    /// The row this pass scrolls into view, placed before the rows are built (`place_reveal`).
    revealing: Option<usize>,
    /// The row the collection's own press is held on this pass, and where on it.
    pressed: Option<(usize, crate::contract::collection::RowPressed)>,
    /// The rows above and below the edge-press band declared this pass.
    edge: Option<(C::Key, C::Key)>,
}

/// The primary-ness, click count, position in the layout's coordinates and scroll offset of a press on a row.
#[derive(Clone, Copy)]
struct RowPress {
    primary: bool,
    clicks: u32,
    origin: Pos2,
    offset: ScrollPos,
}

/// Where the rows sit in the collection's pass: the rows rect, the scroll offset and the reading direction.
#[derive(Clone, Copy)]
struct Placement {
    rect: Rect,
    offset: ScrollPos,
    rtl: bool,
}

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    pub(super) fn new(source: &'s dyn ItemSource<C::Key>, item: &'s mut ItemFn<'r, C>) -> Self {
        ItemsHooks {
            source: Source::Given(source),
            item,
            layout: None,
            columns: 0,
            previous: None,
            content: Vec2::ZERO,
            room: Vec2::ZERO,
            indent_step: 0.0,
            from_content: (false, false),
            scrolls: true,
            own_wheel: false,
            capped: false,
            notch: craft_behavior::WheelNotch::DEFAULT,
            overflow: Axis::Y,
            selection: C::Selection::default(),
            selection_at_read: C::Selection::default(),
            events: Vec::new(),
            activation_clicks: 0,
            keyed_activation: None,
            visible: 0..0,
            shown: Vec::new(),
            hovered: None,
            offset: 0.0,
            offset_at_read: 0.0,
            offset_x: 0.0,
            offset_x_at_read: 0.0,
            anchor: None,
            anchor_at_read: None,
            column_layout_at_read: None,
            lead: Lead::default(),
            renaming: Renaming::default(),
            ended_rename: false,
            refocus: false,
            xf: None,
            header: Header::default(),
            drags: Drags::default(),
            page_reveal: None,
            revealing: None,
            lead_at_read: None,
            renaming_at_read: None,
            pressed: None,
            edge: None,
        }
    }
}

fn key_hash<K: std::hash::Hash>(key: &K) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

// `StyleCx::face` sits here, with the measuring it calls, not in `style`: `style` is below `contract`.
impl crate::style::StyleCx<'_> {
    /// The size `face` has, measured from the `Ui` the collection hook fills this context from, so a collection's
    /// `item_layout` measures a button's label and icon in the pass it lays out in. Zero in a context no collection
    /// filled, which has nothing to measure in.
    pub fn face<S: crate::contract::Show>(&self, mut face: S) -> egui::Vec2 {
        let Some(ui) = self.measuring else { return egui::Vec2::ZERO };
        crate::contract::measure_under(&mut face, ui, None, craft_behavior::Proposal::Ideal, None)
    }
}

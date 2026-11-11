//! The item step (17.3): the collection's side of steps 4 to 17 and its content sub-steps I1 to I5.

pub(super) mod rows;

use super::collection::{CollectionEvent, LeadSource, ShownRow};
use super::{Collection, ItemFn};
use crate::runtime::egui::areas;
use craft_behavior::{Axis, GestureId, ItemSource, ScrollPos, TypeAhead};
use egui::{Pos2, Rect, Vec2};
use std::ops::Range;
use std::sync::Arc;

mod drag;
pub(super) mod empty_text;
mod header;
mod marquee;
mod request;

pub use drag::{SweepApply, SweepStep};
pub use header::ColumnCommands;
#[doc(hidden)]
pub use header::put_toggle_names;
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
mod output;
mod press;
mod rename;
mod reveal;

/// The rows the item step builds in a pass and where it puts them: what `PartsCx::rect_of` and `offset` read.
pub(super) struct Rows<K> {
    pub(super) rects: Vec<(K, Rect)>,
    pub(super) offset: ScrollPos,
}

const SCROLL: craft_behavior::PartKey = craft_behavior::PartKey::new("items_scroll");
const SECTION: craft_behavior::PartKey = craft_behavior::PartKey::new("section");
/// The sub-node of a toggle cell of an item row; the index is the hash of the row's key and the column.
const TOGGLE_CELL: craft_behavior::PartKey = craft_behavior::PartKey::new("toggle_cell");
/// The part an item row's own access node stands for in the collection: AccessKit requests on the row's node reach
/// the collection under this part, the index being the hash of the row's key.
const ITEM_NODE: craft_behavior::PartKey = craft_behavior::PartKey::new("item_node");

/// The part of a collection that stands for the section row `key` (`ItemSource::section_stop`): the item step builds
/// no row for it, so the access node that AccessKit focus goes to while the section holds the lead, and the pointer
/// part over it, are this part of the collection. It is the collection contract's one section mechanism: a component
/// that draws its own section headers lays them out as this part, so the lead, the pointer and the access tree treat
/// them as the collection's section rows.
pub fn section_part<K: std::hash::Hash>(key: &K) -> craft_behavior::PartRef {
    SECTION.at(key_hash(key))
}

#[derive(Clone, Default)]
struct Scrolled {
    y: f64,
    x: f64,
}

#[derive(Clone, Default)]
struct View {
    extent: f32,
    view: Option<[f32; 4]>,
}

#[derive(Clone)]
struct Lead<K> {
    key: Option<K>,
    at: usize,
    /// A sheet's lead cell column.
    column: usize,
    /// The `CollectionProps.rows_stamp` of the pass that last set the lead.
    stamp: Option<u64>,
}

impl<K> Default for Lead<K> {
    fn default() -> Self {
        Lead { key: None, at: 0, column: 0, stamp: None }
    }
}

#[derive(Clone)]
struct Renaming<K> {
    key: Option<K>,
    draft: String,
    gesture: GestureId,
    focused: bool,
}

impl<K> Default for Renaming<K> {
    fn default() -> Self {
        Renaming { key: None, draft: String::new(), gesture: GestureId::default(), focused: false }
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

pub(super) struct ItemsHooks<'s, 'r, C: Collection> {
    source: &'s dyn ItemSource<C::Key>,
    item: &'s mut ItemFn<'r, C>,
    layout: Option<Arc<C::ItemLayout>>,
    /// The layout's column count this pass, which a row pick's whole-row cells span.
    columns: u32,
    /// The layout of the last pass, which `Collection::item_layout_from` may reuse.
    previous: Option<Arc<C::ItemLayout>>,
    content: Vec2,
    room: Vec2,
    /// A measuring run under a body sized from its content: the axes (x, y) the collection takes from its content.
    from_content: (bool, bool),
    scrolls: bool,
    /// The axis the rows overflow along.
    overflow: Axis,
    selection: C::Selection,
    selection_at_read: C::Selection,
    events: Vec<CollectionEvent<C::Key, C::ItemEvent>>,
    /// The click count of the press behind this pass's pointer activation, 0 when no pointer activated.
    activation_clicks: u8,
    visible: Range<usize>,
    /// The rows built this pass with their rects, for `collection_response`.
    shown: Vec<ShownRow<C::Key>>,
    offset: f64,
    offset_at_read: f64,
    offset_x: f64,
    offset_x_at_read: f64,
    last: View,
    lead: Lead<C::Key>,
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
            source,
            item,
            layout: None,
            columns: 0,
            previous: None,
            content: Vec2::ZERO,
            room: Vec2::ZERO,
            from_content: (false, false),
            scrolls: true,
            overflow: Axis::Y,
            selection: C::Selection::default(),
            selection_at_read: C::Selection::default(),
            events: Vec::new(),
            activation_clicks: 0,
            visible: 0..0,
            shown: Vec::new(),
            offset: 0.0,
            offset_at_read: 0.0,
            offset_x: 0.0,
            offset_x_at_read: 0.0,
            last: View::default(),
            lead: Lead::default(),
            renaming: Renaming::default(),
            ended_rename: false,
            refocus: false,
            xf: None,
            header: Header::default(),
            drags: Drags::default(),
            page_reveal: None,
        }
    }
}

fn key_hash<K: std::hash::Hash>(key: &K) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

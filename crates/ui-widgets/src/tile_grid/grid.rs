//! `TileGrid`: a collection of `Tile`s in wrapped cards or justified rows, under optional group headers. The item step
//! does the work: virtualisation, lead and selection rules, arrow, Home, End and page keys over the layout, type-ahead,
//! drag with drop marks, OS file drops, the rename lifecycle and the context-menu request. The grid adds the layout,
//! the pinned group headers and the tail rows after the tiles, which are rows of the grid's outline (section rows of
//! the item step that hold the lead), the insert gaps between tiles and the rating keys. It owns what its cards have
//! in common: where the caption goes, whether a detail line is reserved, where the hover actions sit, and the zoom.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::headers::Outline;
use super::layout::{Around, ArrangeSpacing, GridLayout, Recipe, Slot, TailRow, TileGroup, Tiling, arrange, aspects_of, slot_at};
use super::tile::AroundPicture;
use super::{CaptionPlace, CardText, Tile, TileActionPlace, TilePartEvent, TileZoom, ZoomStep};
use crate::contract::{
    Access, Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, ColumnKind, ColumnSpec, Common, Component,
    ContainerProps, Cx, DragPayload, DropKind, EditResponse, Focus as _, ListDrag, Meta as _, NoValue, Notch, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Role, RowHeight, RowState, SelectMode, Show as _, Sizing, SpaceAction, SweepApply, SweepStep, ToggleColumn,
    section_part,
};
use crate::controls::disclosure::Disclosure;
use crate::list::{DropTest, ListRow, ListRowStyle, RowTest, ThumbSize, ToggleState};
use crate::primitives::{IconSpec, PaintShape, RectSide, StrokeWidth};
use crate::runtime::access::Name;
use crate::runtime::commands::FireSource;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, RuleKind, StyleColor, StyleCx};
use craft_behavior::{
    ColumnKey, ColumnWidth, EditEvent, HitLevel, HitPriority, Intent, ItemLayout as _, ItemSource, Metrics, Mixed, NavShape, PartKey, PointerButton,
    PressPhase, Roving, Selection, Sense,
};
use craft_keymap::{Chord, Key, Mods};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2, pos2, vec2};
use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::Hash;
use std::sync::Arc;

/// The heading level of a group header: the grid's groups are its top-level sections.
const GROUP_LEVEL: usize = 1;
const TOGGLE: PartKey = PartKey::new("group_toggle");
/// An insert gap, indexed by the display position it inserts before.
const GAP: PartKey = PartKey::new("insert_gap");
/// The keyboard path of the header parts: the header is a row of the outline, and the lead on it takes these keys.
const HEADER_KEYS: &str = "Up and Down move to a group header; Enter picks it and Left and Right close and open it";
/// The keyboard path of a tail row's part: the row is a row of the outline, and the lead on it takes Enter.
const TAIL_KEYS: &str = "Down from the last line of tiles moves to the row; Enter activates it";
/// The toggle column that carries the tiles' check boxes, so Space toggles the lead tile's check.
const CHECKS: ColumnKey = ColumnKey::new("check");

/// Which keys rate the selected tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RatingKeys {
    /// No key rates.
    #[default]
    None,
    /// The digits 1 to 5 rate and 0 clears. Other typed text still moves by type-ahead.
    Digits,
    /// Cmd/Ctrl with 1 to 5 rates and with 0 clears (Bridge).
    CommandDigits,
}

/// The size of a grid's tiles.
#[non_exhaustive]
pub enum GridZoom<'a> {
    /// A picture side factor the host holds; the grid takes no zoom keys or wheel.
    Fixed(f64),
    /// A factor the grid steps itself: Cmd/Ctrl `=` and `+`, `-` and `0` through `TileZoom::step`, and Ctrl/Cmd with
    /// the wheel or a pinch by the gesture's factor, inside the zoom's range, keeping the tile under the pointer in
    /// place. Cmd/Ctrl+0 is a rating key under `RatingKeys::CommandDigits`, so it clears the rating there instead of
    /// resetting the size. A binding that holds no single value reads as the zoom's default.
    Bound(Bind<'a, f64>, TileZoom),
}

impl Default for GridZoom<'_> {
    fn default() -> Self {
        GridZoom::Fixed(1.0)
    }
}

/// What the grid reports besides the collection's events.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GridEvent<K> {
    /// A tile's stars were clicked (the selected tiles when the tile is one of them, else that tile), or a rating key was
    /// pressed (the selected tiles, or the lead when none is selected): the rating, 0 clears. The one route to a rating; the tile's own `TilePartEvent::Rate` does not
    /// reach the collection's events.
    #[non_exhaustive]
    Rate {
        /// The tiles to rate.
        keys: Vec<K>,
        /// The rating.
        rating: u8,
    },
    /// A group header was clicked or took Enter.
    #[non_exhaustive]
    GroupPicked {
        /// The group's index in the list the grid was given.
        group: usize,
        /// The modifiers held.
        mods: Mods,
    },
    /// The disclosure of a group header, or AccessKit Expand or Collapse on it, asks for a state.
    GroupOpen {
        /// The group's index.
        group: usize,
        /// The state asked for.
        open: bool,
    },
    /// The insert gap before display position `at` was activated ([`TileGrid::insert_gaps`]).
    #[non_exhaustive]
    Insert {
        /// The display position the insertion goes before.
        at: usize,
    },
    /// A group header asks for its context menu: a secondary click, the context-menu key or Shift+F10.
    #[non_exhaustive]
    GroupMenu {
        /// The group's index.
        group: usize,
        /// Where the menu opens: the header's bottom start corner.
        at: Pos2,
    },
}

/// What a `TileGrid` reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the grid's events"]
pub struct TileGridOutput<K> {
    /// The collection's response and events.
    pub response: CollectionResponse<K, TilePartEvent>,
    /// The grid's own events, in the order they happened.
    pub events: Vec<GridEvent<K>>,
}

impl<K> Outcome for TileGridOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.response.story_facts()
    }
}

/// The look of the pinned group headers and of the headers themselves, which are heading rows.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TileGridStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A header pinned over its group's tiles.
    pinned: StyleColor,
    /// The look of a group header: a heading row, which places the disclosure the grid declares.
    header: ListRowStyle,
    /// The line down the middle of an insert gap under the pointer.
    insert: StyleColor,
    /// The plus of an insert gap at rest, before the pointer reaches it: muted, so the gaps do not compete with the tiles.
    insert_idle: StyleColor,
    /// The width of an insert gap, centred on the edge of the tile it precedes.
    insert_width: f32,
}

/// The sizes the grid lays its tiles out with.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GridSizes {
    /// The picture side of a card at zoom 1 when a grid names no thumbnail size: the card thumbnail.
    card: f32,
    /// The picture side of a card at zoom 1 for each thumbnail size.
    thumbs: [f32; 3],
    /// What a tile adds around its picture.
    around: AroundPicture,
    /// The gaps of the arrangement and the height of a group header (a heading row, as high as the grid's rows: the
    /// collection's row height), for captioned tiles and for bare ones.
    spacing: ArrangeSpacing,
    bare_spacing: ArrangeSpacing,
}

impl GridSizes {
    fn of(cx: &StyleCx<'_>, card: bool, text: CardText, rows: RowHeight, regular_card_padding: bool, action_place: TileActionPlace) -> GridSizes {
        let spacing =
            ArrangeSpacing::new(if card { cx.tokens.space.gap.section } else { cx.tokens.space.gap.stack }, cx.tokens.space.gap.inline, rows.of(cx));
        GridSizes {
            card: cx.tokens.size.thumb.card,
            thumbs: [cx.tokens.size.thumb.sm, cx.tokens.size.thumb.md, cx.tokens.size.thumb.lg],
            around: Tile::around(cx, card, text, regular_card_padding, action_place),
            spacing,
            // Bare tiles sit with no gap between them: the room around each picture keeps them apart.
            bare_spacing: if card { spacing } else { ArrangeSpacing { gap: 0.0, ..spacing } },
        }
    }

    /// The spacing of a grid whose tiles have their caption in `place`.
    fn spacing(&self, place: CaptionPlace) -> ArrangeSpacing {
        if place == CaptionPlace::None { self.bare_spacing } else { self.spacing }
    }

    /// The side of a picture at `zoom`, a factor on the thumbnail of `size`, or on the card thumbnail for `None`.
    fn picture_side(&self, zoom: f32, size: Option<ThumbSize>) -> f32 {
        size.map_or(self.card, |size| size.pick(self.thumbs)) * zoom.max(f32::EPSILON)
    }

    /// An equal card: a picture `side` wide with the aspect `aspect`, and what the tile adds around it.
    fn card(&self, side: f32, aspect: f32, around: Around) -> egui::Vec2 {
        vec2(side, side / aspect) + around.size()
    }
}

/// The share of the grid's view one wheel notch scrolls (LightCraft's filmstrip).
const WHEEL_SHARE: f32 = 1.0 / 3.0;

/// A grid of tiles: one Tab stop, the selection rules of the collection contract, type-ahead on the tiles' names (with
/// `type_ahead`), rename in place (F2, or a slow second click on the caption of the only selected tile), drag to reorder, files dropped from the OS, and the request for the app's context
/// menu. A double click on the background reports `CollectionEvent::EmptyActivated` (an import, in FilmCraft). One wheel
/// notch scrolls a third of the grid's view.
///
/// The picture side is `zoom` times the thumbnail side (the card thumbnail unless `thumb_size` names one): a factor the
/// app holds, or one the grid steps itself through a binding (`GridZoom`). A card is the
/// picture plus what the tile adds around it, whatever the layout. Pictures are `aspect` wide over high; with
/// `justified` the tiles keep their own aspects (the source's `ItemSource::size_hint` as `SizeHint::Aspect`) and fill
/// rows. `groups` put headers over runs of tiles; a header stays pinned at the top while its group scrolls. The grid
/// decides where every tile's caption goes and whether the cards hold a detail line.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, probe(TileGrid<'static, usize>))]
pub struct TileGrid<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    /// The selection the caller owns.
    #[prop]
    selection: Option<Bind<'a, Selection<K>>>,
    /// Scrolls the tile of `key` into view once.
    #[prop]
    reveal: Option<K>,
    /// Tiles keep their own aspect and fill rows, instead of equal cards.
    #[prop(flag)]
    justified: bool,
    /// Equal cards grow to fill their row: the columns are those that fit at the zoomed card width, and the cards share
    /// the width between them. The row starts at the start edge, and what the columns leave over lies after the last card.
    #[prop(flag)]
    fill_row: bool,
    /// The width of a card over its picture's height.
    #[prop(quantity = "aspect ratio")]
    aspect: f32,
    /// The picture side as a multiple of the thumbnail side the grid sizes from; 1 is that side.
    zoom: GridZoom<'a>,
    /// The zoom's factor as this pass was built with it: the layout of the pass reads it, and a step the pass takes
    /// shows in the next pass's layout, so the anchor is named against the layout the step changes from.
    factor: f64,
    /// Where the pointer was when `update` took a zoom chord in this pass: the point a key zoom keeps in place.
    chord_at: Option<Pos2>,
    /// Where every tile's hover actions sit.
    #[prop]
    action_place: TileActionPlace,
    /// Insert gaps between tiles, keyed by display position; `label` names the gap before a position. None show
    /// while a drag is held. A press on one reports [`GridEvent::Insert`].
    ///
    /// The gap before position `at` sits on the reading-start edge of tile `at`, between tiles `at - 1` and `at`; the
    /// gap `at` = the tile count sits on the end edge of the last tile. A gap shows while its tile is in view, so the
    /// end gap shows only with the last tile in view.
    #[prop]
    insert_gaps: Option<&'a dyn Fn(usize) -> Label>,
    /// The thumbnail size the zoom is a factor of; `None` is the card thumbnail.
    #[prop]
    thumb_size: Option<ThumbSize>,
    /// Cards are as large as the largest tile measures (a non-card icon tile measures its icon), instead of the zoomed
    /// thumbnail side; `zoom` and `thumb_size` do not apply.
    #[prop(flag)]
    content_cells: bool,
    /// The columns of cards a grid that takes its width from its content asks for: its width holds this many cards and
    /// their gaps. 0 asks for one card, and the content decides the rest.
    #[prop]
    card_columns: usize,
    /// Where every tile's caption goes; `CaptionPlace::None` keeps no room for one.
    #[prop]
    caption: CaptionPlace,
    /// Cards keep a line under the name for the tile's detail, whether a tile has one or not.
    #[prop(flag)]
    detail_line: bool,
    /// Tiles are framed cards: the picture inset in a raised surface with the name and detail under it as `card_text` sets them.
    #[prop(flag)]
    card: bool,
    /// How framed cards set their name and detail: the strong, start-aligned name of a file, or the centred regular
    /// name of a preset.
    #[prop]
    card_text: CardText,
    /// Framed cards use the theme's regular padding around their picture and caption.
    #[prop(flag)]
    regular_card_padding: bool,
    /// The headers over runs of tiles, in display order.
    #[prop]
    groups: &'a [TileGroup<K>],
    /// Group headers show a disclosure that opens and closes their group.
    #[prop(flag)]
    collapsible: bool,
    /// Full-width rows after the tiles, below a divider: actions such as "Create preset". A click or Enter on one
    /// reports `CollectionEvent::Activate` with its key.
    #[prop]
    tail: &'a [TailRow<K>],
    /// Which keys rate the selected tiles.
    rating_keys: RatingKeys,
    /// Tiles show a check box and Space toggles the lead tile's check.
    checks: bool,
    /// Which tiles may be renamed; a rename request on another tile does nothing.
    #[prop(func = "Fn(&K) -> bool")]
    can_rename: Option<RowTest<'a, K>>,
    /// Which tiles may be dragged.
    #[prop(func = "Fn(&K) -> bool")]
    can_drag: Option<RowTest<'a, K>>,
    /// What the grid takes from drags that did not start in it, in order.
    drops: Vec<DropKind<'a, K>>,
    /// Where the grid's own drag may land: the test gets the drag, the tile under the pointer and the position before which the
    /// dragged tiles would go.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
    /// How the groups are set apart: `Rule` starts every group after the first below a full-width divider row in place
    /// of its titled head.
    #[prop]
    group_heads: GroupHeads,
    /// The layout of this pass, kept for the headers' parts and paint.
    laid: RefCell<Option<GridLayout>>,
    /// The headers this pass shows, in the grid's layer space: the group, its rect and whether it is pinned.
    placed: Vec<Placed>,
    /// The tail rows this pass shows, in the grid's layer space: the row's index and its rect.
    placed_tail: Vec<(usize, Rect)>,
    /// The insert gaps this pass shows, in the grid's layer space: the display position and the gap's rect.
    placed_gaps: Vec<(usize, Rect)>,
    /// The tail rows clicked this pass, in order.
    tail_clicked: Vec<K>,
    events: Vec<GridEvent<K>>,
}

/// How the groups of a grid or a list are set apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum GroupHeads {
    /// Every group has its titled head.
    #[default]
    Titled,
    /// Every group after the first starts below a full-width divider row with no title, no disclosure and no Tab
    /// stop; the group is always open.
    Rule,
}

/// A group header as it shows this pass.
#[derive(Clone, Copy, Debug)]
struct Placed {
    group: usize,
    rect: Rect,
    pinned: bool,
    /// The header has a disclosure part.
    toggle: bool,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> TileGrid<'a, K> {
    /// A grid named `label`, with multiple selection and equal square cards of the card thumbnail. Space picks the lead and
    /// no key rates or zooms; Bridge's keys are `rating_keys(RatingKeys::CommandDigits)`, `space(SpaceAction::Preview)`
    /// and a `GridZoom::Bound` zoom.
    pub fn new(label: impl Into<Label>) -> Self {
        TileGrid {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Multi).slow_click_rename(true).notch(Notch::ViewShare(WHEEL_SHARE)),
            selection: None,
            reveal: None,
            justified: false,
            fill_row: false,
            aspect: 1.0,
            zoom: GridZoom::Fixed(1.0),
            factor: 1.0,
            chord_at: None,
            action_place: TileActionPlace::Corner,
            insert_gaps: None,
            thumb_size: None,
            content_cells: false,
            card_columns: 0,
            caption: CaptionPlace::Below,
            detail_line: false,
            card: false,
            card_text: CardText::File,
            regular_card_padding: false,
            groups: &[],
            collapsible: false,
            tail: &[],
            rating_keys: RatingKeys::None,
            checks: false,
            can_rename: None,
            can_drag: None,
            drops: Vec::new(),
            accepts: None,
            group_heads: GroupHeads::Titled,
            laid: RefCell::new(None),
            placed: Vec::new(),
            placed_tail: Vec::new(),
            placed_gaps: Vec::new(),
            tail_clicked: Vec::new(),
            events: Vec::new(),
        }
        .label(label)
    }

    /// The size of the tiles: a factor the host holds, or one the grid steps itself through a binding.
    #[must_use]
    pub fn zoom(mut self, mut zoom: GridZoom<'a>) -> Self {
        self.factor = match &mut zoom {
            GridZoom::Fixed(factor) => *factor,
            GridZoom::Bound(bind, steps) => Self::bound_factor(bind, *steps),
        };
        self.zoom = zoom;
        self
    }

    /// The value a bound zoom holds; the zoom's default while the binding holds no single value.
    fn bound_factor(bind: &mut Bind<'_, f64>, steps: TileZoom) -> f64 {
        match bind.reborrow().get() {
            Some(Mixed::Same(factor)) if factor.is_finite() => factor,
            _ => steps.default_value(),
        }
    }

    /// Writes a bound zoom's next factor, `to` of its value and the zoom, inside the zoom's range, as a `Set` named
    /// `label`. A fixed zoom is the host's: it takes nothing. Several steps in one pass chain on the written value.
    fn step_zoom(&mut self, label: &Arc<str>, to: impl FnOnce(f64, TileZoom) -> f64) {
        let GridZoom::Bound(bind, steps) = &mut self.zoom else { return };
        let steps = *steps;
        let before = Self::bound_factor(bind, steps);
        let range = steps.range();
        let after = to(before, steps).clamp(*range.start(), *range.end());
        if !after.is_finite() || after == before {
            return;
        }
        bind.reborrow().apply(&EditEvent::set(Mixed::Same(before), after, label.clone(), None));
    }

    /// Which keys rate the selected tiles. Digit keys reach the grid as digits only while this is on.
    #[must_use]
    pub fn rating_keys(mut self, keys: RatingKeys) -> Self {
        self.rating_keys = keys;
        self.items = self.items.digit_keys(keys == RatingKeys::Digits);
        self
    }

    /// Tiles show a check box (`Tile::check`) and Space toggles the lead tile's check. A toggle of a check reports
    /// `CollectionEvent::Item` with `TilePartEvent::Check`, as a click on the check box does.
    #[must_use]
    pub fn checks(mut self, on: bool) -> Self {
        self.checks = on;
        let columns = if on {
            vec![ColumnSpec::new(CHECKS, Label::default(), ColumnWidth::Fixed(0.0)).kind(ColumnKind::Toggle(ToggleColumn::default()))]
        } else {
            Vec::new()
        };
        self.items = self.items.columns(columns).header(!on);
        if on {
            self.items = self.items.space(SpaceAction::ToggleCell);
        } else if self.items.space == SpaceAction::ToggleCell {
            self.items = self.items.space(SpaceAction::default());
        }
        self
    }

    /// A tile drag carries `map` of it, for the drop targets of type `P`; a tile the map gives `None` for does not drag
    /// unless a reorder rule is set. The drag stays one session with one ghost.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
        self
    }

    /// A tile drag carries `payload` of it too, beside the payloads given before (`CollectionProps::carry`).
    #[must_use]
    pub fn carry(mut self, payload: DragPayload) -> Self {
        self.items = self.items.carry(payload);
        self
    }

    /// Takes the drops of `kind` from drags that did not start in the grid; kinds given earlier come first.
    #[must_use]
    pub fn takes_drop(mut self, kind: DropKind<'a, K>) -> Self {
        self.drops.push(kind);
        self
    }

    /// The selected tiles, or the lead when none is selected: what a rating key rates. The caller's binding when it
    /// holds one, else the selection the item step keeps.
    fn rated(&mut self, cx: &Cx<'_, Self>) -> Vec<K> {
        let held = self.selection.as_mut().map(Bind::reborrow).and_then(|bind| bind.get());
        let selection = match held {
            Some(Mixed::Same(selection)) => Some(selection),
            Some(_) => None,
            None => cx.collection_selection().cloned(),
        };
        match selection {
            Some(selection) if !selection.items.is_empty() => selection.items,
            Some(selection) => selection.lead.into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// The layout of this pass: `previous` when it was made from the same recipe, else a new arrangement. A card is
    /// the picture plus what the tile adds around it in every tiling.
    fn laid_out(&self, previous: Option<GridLayout>, source: &dyn ItemSource<K>, width: f32, item: Option<Vec2>, cx: &StyleCx<'_>) -> GridLayout {
        let style = GridSizes::of(cx, self.card, self.card_text, self.items.row_height, self.regular_card_padding, self.action_place);
        let spacing = style.spacing(self.caption);
        let around = style.around.get(self.caption, self.detail_line);
        // A card never outgrows the grid: in a column narrower than a card the card takes the column.
        let mut side = self.picture_side(&style, item, around).min(width - 2.0 * spacing.inset - around.size().x).max(1.0);
        if self.fill_row && !self.justified {
            let (free, gap, card) = (width, spacing.gap, side + around.size().x);
            let columns = ((free + gap) / (card + gap)).floor().max(1.0);
            side = ((free - (columns - 1.0) * gap) / columns - around.size().x).max(1.0);
        }
        let aspect = if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 };
        let tail_height = self.items.row_height.of(cx);
        let groups = self.shown_groups();
        let recipe = self.items.rows_stamp.map(|stamp| Recipe {
            stamp,
            width,
            tiles: source.len(),
            side,
            aspect,
            justified: self.justified,
            around,
            group_header: spacing.group_header,
            groups: groups.iter().map(|group| (group.start, group.count, group.open)).collect(),
            tail: (self.tail.len(), tail_height),
        });
        if let (Some(previous), Some(recipe)) = (previous, &recipe)
            && previous.is_made_from(recipe)
        {
            *self.laid.borrow_mut() = Some(previous.clone());
            return previous;
        }
        let layout = if self.justified || !self.groups.is_empty() || !self.tail.is_empty() {
            let tiling = if self.justified { Tiling::justified(side) } else { Tiling::cards(side, aspect) };
            let mut arrangement = arrange(&aspects_of(source), &groups, width, tiling, spacing, around);
            arrangement.add_tail(self.tail.len(), tail_height, width, spacing);
            GridLayout::arranged(arrangement, width, around)
        } else {
            if self.fill_row {
                // The columns fill the whole width, and the row starts at the start edge.
                GridLayout::equal(source, width, 0.0, style.card(side, aspect, around), spacing.gap)
            } else {
                GridLayout::equal(source, width, spacing.inset, style.card(side, aspect, around), spacing.gap)
            }
        };
        let layout = match recipe {
            Some(recipe) => layout.made_from(recipe),
            None => layout,
        };
        *self.laid.borrow_mut() = Some(layout.clone());
        layout
    }

    /// The side of a card's picture: the largest tile's (`item`, as the items step measured it) less what a tile adds
    /// around its picture, with `content_cells`; else the zoomed thumbnail side.
    fn picture_side(&self, style: &GridSizes, item: Option<Vec2>, around: Around) -> f32 {
        match item.filter(|_| self.content_cells) {
            Some(item) => (item.x - around.size().x).max(1.0),
            None => style.picture_side(self.factor as f32, self.thumb_size),
        }
    }

    /// The row that holds the lead while the grid has focus, when the grid has an arrangement.
    fn lead_slot(&self, cx: &Cx<'_, Self>) -> Option<Slot> {
        if !cx.is_focused() {
            return None;
        }
        let at = cx.collection_lead()?;
        let laid = self.laid.borrow();
        let arrangement = laid.as_ref()?.arrangement()?;
        Some(slot_at(&arrangement.starts, arrangement.tiles.len(), at))
    }

    /// The outline position of the first tail row in this pass's layout, when the grid has one.
    fn tail_from(&self) -> Option<usize> {
        self.laid.borrow().as_ref().and_then(|layout| layout.arrangement().map(|arrangement| arrangement.tail_from()))
    }

    /// The groups as they show: under `Rule` every group is open, whatever the app declared.
    fn shown_groups(&self) -> Cow<'a, [TileGroup<K>]> {
        match self.group_heads {
            GroupHeads::Titled => Cow::Borrowed(self.groups),
            GroupHeads::Rule => Cow::Owned(self.groups.iter().map(|group| group.clone().open(true)).collect()),
        }
    }

    /// Group headers open and close their group: only titled heads do.
    fn collapses(&self) -> bool {
        self.collapsible && self.group_heads == GroupHeads::Titled
    }

    fn group_rect(rect: Rect, view: Rect, rtl: bool) -> Rect {
        if rtl {
            Rect::from_min_max(pos2(view.min.x + view.max.x - rect.max.x, rect.min.y), pos2(view.min.x + view.max.x - rect.min.x, rect.max.y))
        } else {
            rect
        }
    }

    /// Declares the insert gaps of the tiles in view, none while the grid's own drag is held: the gap before each tile
    /// in view, and the end gap with the last tile in view. The gaps are named by display position, so rows scrolled
    /// out of view change no position.
    fn place_gaps(&mut self, clip: Rect, cx: &mut PartsCx<'_, Self>) {
        let dragging = crate::runtime::drag::active::<ListDrag<K>>(cx.ui.ctx()).is_some_and(|drag| drag.list == cx.id);
        if self.insert_gaps.is_none() || dragging {
            return;
        }
        let gaps = {
            let laid = self.laid.borrow();
            let Some(layout) = laid.as_ref() else { return };
            let (view, offset) = (cx.content_rect, cx.offset());
            let mut shown = Vec::new();
            layout.visible(offset, view.size(), &mut shown);
            let tiles: Vec<usize> = shown.into_iter().filter_map(|at| layout.tile_at(at)).collect();
            let count = layout.tiles();
            let end = (count > 0 && tiles.last() == Some(&(count - 1))).then_some(count);
            let width = cx.style.insert_width;
            let rtl = cx.flow.is_rtl();
            tiles
                .into_iter()
                .chain(end)
                .filter_map(|at| layout.gap(at, offset, width).map(|gap| (at, Self::group_rect(gap.translate(view.min.to_vec2()), view, rtl))))
                .filter(|(_, rect)| rect.intersects(clip))
                .collect::<Vec<_>>()
        };
        let priority = HitPriority::new(HitLevel::Edge, 0);
        for &(at, rect) in &gaps {
            cx.part(GAP.at(at as u64), rect).sense(Sense::CLICK | Sense::FOCUS).priority(priority).over().focusable().state_layer();
        }
        if !gaps.is_empty() {
            cx.roving(Roving::new(NavShape::Flat));
        }
        self.placed_gaps = gaps;
    }

    /// The header row of `group`: a heading at the group's depth, with its icon and its detail at the end, and a
    /// disclosure column the grid owns when the groups open and close.
    fn header_row(&self, group: &TileGroup<K>, state: RowState) -> ListRow<'static> {
        let mut row = ListRow::new(group.title.clone()).heading().rows(self.items.row_height).node(false).depth(group.depth).state(state);
        if self.collapses() {
            row = row.group(group.open).disclosure_by_host();
        }
        if let Some(icon) = &group.icon {
            row = row.icon(IconSpec::svg(icon.clone()));
        }
        if !group.detail.is_empty() {
            row = row.trailing(group.detail.clone());
        }
        row
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Component for TileGrid<'_, K> {
    const ROLE: Role = Role::LIST_BOX;
    const OVERLAYS: bool = false;
    type Style = TileGridStyle;
    type Output = TileGridOutput<K>;

    fn style(cx: &StyleCx<'_>, _: TileGridStyleKey<K>) -> TileGridStyle {
        TileGridStyle {
            metrics: Metrics::new(),
            pinned: cx.color(cx.tokens.color.surface.panel),
            header: ListRow::style(cx, ListRow::new(Label::default()).heading().style_key()),
            insert: cx.color(cx.tokens.color.accent.solid),
            insert_idle: cx.color(cx.tokens.color.text.muted),
            insert_width: cx.tokens.size.icon.lg,
        }
    }

    /// One pointer part per header in view, over the tiles, and a disclosure part in it when headers collapse; one
    /// pointer part per tail row in view; one focusable part per insert gap in view.
    fn parts(&mut self, area: Rect, cx: &mut PartsCx<'_, Self>) {
        self.placed.clear();
        self.placed_tail.clear();
        self.placed_gaps.clear();
        self.place_gaps(area, cx);
        let Some(arrangement) = self.laid.borrow().as_ref().and_then(|layout| layout.arrangement().cloned()) else { return };
        let (view, offset, rtl) = (cx.content_rect, cx.offset(), cx.flow.is_rtl());
        let rule = self.group_heads == GroupHeads::Rule;
        // A rule has nothing to pin.
        let pinned = if rule { None } else { arrangement.pinned(offset.y as f32) };
        let shift = view.min.to_vec2() - vec2(offset.x as f32, offset.y as f32);
        let clip = area;
        for (group, header) in arrangement.headers.iter().enumerate() {
            let (rect, held) = match pinned {
                Some((at, lifted)) if at == group => (lifted, true),
                _ => (*header, false),
            };
            let rect = Self::group_rect(rect.translate(shift), view, rtl);
            let Some(declared) = self.groups.get(group) else { continue };
            if !rect.intersects(clip) {
                continue;
            }
            if rule {
                // A divider takes no press and no stop: its part only carries the heading node.
                self.placed.push(Placed { group, rect, pinned: false, toggle: false });
                cx.part(section_part(&declared.key), rect);
                continue;
            }
            let column = self
                .header_row(declared, RowState::default())
                .disclosure_at(&cx.style.header, rect, rtl, &|run| cx.layout_at(run, f32::INFINITY).size().x);
            self.placed.push(Placed { group, rect, pinned: held, toggle: column.is_some() });
            let priority = HitPriority::new(HitLevel::Edge, 0);
            let sense = Sense::CLICK | Sense::TAKE_FOCUS;
            cx.part(section_part(&declared.key), rect).sense(sense).priority(priority).over().pointer_only(HEADER_KEYS);
            if let Some(column) = column {
                cx.part(TOGGLE.at(group as u64), column).sense(Sense::CLICK).priority(priority).over().pointer_only(HEADER_KEYS);
            }
        }
        for (at, (row, rect)) in self.tail.iter().zip(&arrangement.tails).enumerate() {
            let rect = Self::group_rect(rect.translate(shift), view, rtl);
            if !rect.intersects(clip) {
                continue;
            }
            self.placed_tail.push((at, rect));
            let priority = HitPriority::new(HitLevel::Edge, 0);
            cx.part(section_part(&row.key), rect).sense(Sense::CLICK | Sense::TAKE_FOCUS).priority(priority).over().pointer_only(TAIL_KEYS);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for rating in 0..=MAX_RATING {
            let mut pressed = 0;
            match self.rating_keys {
                RatingKeys::None => {}
                RatingKeys::Digits => {
                    while cx.intents().take_exact(&Intent::Digit(rating)) {
                        pressed += 1;
                    }
                }
                RatingKeys::CommandDigits => {
                    let chord = Chord::new(Mods::CMD, DIGITS[usize::from(rating)]);
                    cx.claim(chord);
                    while cx.take_claimed(chord) {
                        pressed += 1;
                    }
                }
            }
            for _ in 0..pressed {
                let keys = self.rated(cx);
                if !keys.is_empty() {
                    self.events.push(GridEvent::Rate { keys, rating });
                }
            }
        }
        if matches!(self.zoom, GridZoom::Bound(..)) {
            // A chord is input: a second pass of the frame takes none, so it neither steps again nor names a point.
            let reset = (self.rating_keys != RatingKeys::CommandDigits).then_some((Key::Num0, ZoomStep::Reset));
            let label = cx.core.label_arc();
            for (key, step) in [(Key::Equals, ZoomStep::In), (Key::Plus, ZoomStep::In), (Key::Minus, ZoomStep::Out)].into_iter().chain(reset) {
                let chord = Chord::new(Mods::CMD, key);
                cx.claim(chord);
                while cx.take_claimed(chord) {
                    self.step_zoom(&label, |value, steps| steps.step(value, step));
                    self.chord_at = crate::runtime::pointer::frame(&cx.core.ctx).pos;
                }
            }
        }
        for (at, _) in self.placed_gaps.clone() {
            if cx.part_activated(GAP.at(at as u64)).is_some() {
                self.events.push(GridEvent::Insert { at });
            }
        }
        let placed = if self.group_heads == GroupHeads::Rule { Vec::new() } else { self.placed.clone() };
        for Placed { group, rect, .. } in placed {
            let Some(declared) = self.groups.get(group) else { continue };
            let part = section_part(&declared.key);
            let toggle = TOGGLE.at(group as u64);
            let open = declared.open;
            if let Some(result) = cx.part_if_declared(part)
                && let PressPhase::Release { start, click: true, .. } = result.press.phase
            {
                let mods = crate::runtime::pointer::frame(&cx.core.ctx).pick_mods;
                self.events.push(match start.button {
                    PointerButton::Secondary => {
                        GridEvent::GroupMenu { group, at: if cx.flow.is_rtl() { rect.right_bottom() } else { rect.left_bottom() } }
                    }
                    _ => GridEvent::GroupPicked { group, mods },
                });
            } else if cx.part_activated(part) == Some(FireSource::Access) {
                self.events.push(GridEvent::GroupPicked { group, mods: Mods::NONE });
            }
            if self.collapses() && cx.part_activated(toggle).is_some() {
                self.events.push(GridEvent::GroupOpen { group, open: !open });
            }
        }
        for (at, _) in self.placed_tail.clone() {
            let key = &self.tail[at].key;
            let part = section_part(key);
            let clicked = cx.part_if_declared(part).is_some_and(
                |result| matches!(result.press.phase, PressPhase::Release { start, click: true, .. } if start.button == PointerButton::Primary),
            );
            if clicked || cx.part_activated(part) == Some(FireSource::Access) {
                self.tail_clicked.push(key.clone());
            }
        }
        if self.collapses() {
            while let Some(request) = cx.intents().take_access() {
                let Some(target) = request.target else { continue };
                let wanted = match request.action {
                    craft_behavior::AccessAction::Expand => true,
                    craft_behavior::AccessAction::Collapse => false,
                    _ => continue,
                };
                let owner = self
                    .placed
                    .iter()
                    .map(|placed| placed.group)
                    .find(|group| section_part(&self.groups[*group].key) == target || TOGGLE.at(*group as u64) == target);
                if let Some(group) = owner {
                    self.events.push(GridEvent::GroupOpen { group, open: wanted });
                }
            }
        }
    }

    /// A heading node per header, a list item node per tail row and a button per insert gap in view.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(label) = self.insert_gaps {
            for &(at, _) in &self.placed_gaps {
                access.node(GAP.at(at as u64), Role::BUTTON).name = Name::Text(cx.env.resolve(&label(at)).as_str().to_owned());
            }
        }
        let headers = self.placed.iter().map(|placed| Slot::Header(placed.group));
        let tails = self.placed_tail.iter().map(|&(at, _)| Slot::Tail(at));
        for slot in headers.chain(tails) {
            match slot {
                Slot::Header(group) => {
                    let Some(group) = self.groups.get(group) else { continue };
                    let node = access.node(section_part(&group.key), Role::HEADING);
                    node.level(GROUP_LEVEL);
                    node.name = Name::Text(if group.detail.is_empty() { group.title.clone() } else { format!("{}, {}", group.title, group.detail) });
                    if self.collapses() {
                        node.expanded = Some(group.open);
                    }
                }
                Slot::Tail(at) => {
                    let row = &self.tail[at];
                    access.node(section_part(&row.key), Role::LIST_ITEM).name = Name::Text(cx.env.resolve(&row.label).as_str().to_owned());
                }
                Slot::Tile(_) => {}
            }
        }
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let lead = self.lead_slot(cx);
        for placed in &self.placed {
            let Some(group) = self.groups.get(placed.group) else { continue };
            let rect = placed.rect;
            if self.group_heads == GroupHeads::Rule {
                if placed.group > 0 {
                    ui.draw().edge_rule(RuleKind::Divider, rect, RectSide::Top);
                }
                continue;
            }
            if placed.pinned {
                let draw = ui.draw();
                draw.fill(&PaintShape::Rect(rect, CornerRadius::ZERO), style.pinned);
                draw.edge_rule(RuleKind::Divider, rect, RectSide::Bottom);
            }
            let lead = RowState { emphasized: lead == Some(Slot::Header(placed.group)), ..RowState::default() };
            ui.face(section_part(&group.key), self.header_row(group, lead));
            if placed.toggle {
                let disclosure = Disclosure::new(Bind::given(group.open)).node(false).label(group.title.clone());
                ui.face(TOGGLE.at(placed.group as u64), disclosure);
            }
        }
        for &(at, rect) in &self.placed_tail {
            let row = &self.tail[at];
            if at == 0 {
                ui.draw().edge_rule(RuleKind::Divider, rect, RectSide::Top);
            }
            let state = RowState { emphasized: lead == Some(Slot::Tail(at)), ..RowState::default() };
            ui.face(section_part(&row.key), ListRow::new(row.label.clone()).rows(self.items.row_height).node(false).state(state));
        }
        // The gap under the pointer shows where an insertion lands; the part's state layer shows hover and focus.
        for &(at, rect) in &self.placed_gaps {
            let hovered = cx.hover_pos(GAP.at(at as u64)).is_some();
            if hovered {
                let line = PaintShape::Path(vec![rect.center_top(), rect.center_bottom()]);
                ui.draw().stroke(&line, StrokeWidth::Thick, style.insert);
            }
            let ink = if hovered { style.insert } else { style.insert_idle };
            // The insertion target must be discoverable before the pointer is over it.
            let center = rect.center();
            let radius = (rect.width() * 0.4).min(9.0);
            let arm = radius * 0.45;
            let draw = ui.draw();
            draw.fill(&PaintShape::Circle(center, radius), style.pinned);
            draw.stroke(&PaintShape::Circle(center, radius), StrokeWidth::Thin, ink);
            draw.stroke(&PaintShape::Path(vec![center - vec2(arm, 0.0), center + vec2(arm, 0.0)]), StrokeWidth::Medium, ink);
            draw.stroke(&PaintShape::Path(vec![center - vec2(0.0, arm), center + vec2(0.0, arm)]), StrokeWidth::Medium, ink);
        }
    }

    /// The grid's events are the digits' and the stars' ratings, in that order, besides the collection's own. The
    /// collection's events on a header row are the group events instead: Enter picks the group, the context-menu
    /// key asks for its menu, Left and Right open and close it, and the lead on it is not reported. A tail row keeps its
    /// Enter and the lead on it, and adds its click as `CollectionEvent::Activate`; it has no menu. A bound zoom takes
    /// the wheel's and the pinch's `CollectionEvent::Zoom` here, after `content` named the point it keeps in place, and
    /// the event is not reported.
    fn finish(mut self, mut cx: Cx<'_, Self>) -> TileGridOutput<K> {
        let selected = self.rated(&cx);
        let mut response = cx.collection_response();
        let label = cx.core.label_arc();
        for event in &response.events {
            if let CollectionEvent::Zoom { factor, .. } = event {
                self.step_zoom(&label, |value, _| value * f64::from(*factor));
            }
        }
        if matches!(self.zoom, GridZoom::Bound(..)) {
            response.events.retain(|event| !matches!(event, CollectionEvent::Zoom { .. }));
        }
        let mut events = self.events;
        // Only a pass with events reads the pointer, so a pass without one does not depend on it.
        let mods = if response.events.is_empty() { Mods::NONE } else { crate::runtime::pointer::frame(&cx.core.ctx).pick_mods };
        let group_of = |key: &K| self.groups.iter().position(|group| group.key == *key);
        let is_tail = |key: &K| self.tail.iter().any(|row| row.key == *key);
        let mut kept = Vec::with_capacity(response.events.len());
        for event in std::mem::take(&mut response.events) {
            let tail_menu = matches!(&event, CollectionEvent::ContextMenu(request) if is_tail(&request.key));
            if tail_menu {
                continue;
            }
            let header = match &event {
                CollectionEvent::Activate(key) => group_of(key),
                CollectionEvent::ContextMenu(request) => group_of(&request.key),
                CollectionEvent::Open(change) => group_of(&change.key),
                _ => None,
            };
            if let Some(group) = header {
                match event {
                    CollectionEvent::Activate(_) => events.push(GridEvent::GroupPicked { group, mods }),
                    CollectionEvent::ContextMenu(request) => events.push(GridEvent::GroupMenu { group, at: request.at }),
                    CollectionEvent::Open(change) => events.push(GridEvent::GroupOpen { group, open: change.open }),
                    _ => {}
                }
                continue;
            }
            match event {
                CollectionEvent::Item(key, TilePartEvent::Rate(rating)) => {
                    // Stars on a selected tile rate the whole selection.
                    let keys = if selected.contains(&key) { selected.clone() } else { vec![key] };
                    events.push(GridEvent::Rate { keys, rating });
                }
                CollectionEvent::Toggle(edit) if self.checks => {
                    // The check column does not sweep, so each flip arrives whole as a `Set`.
                    if let Some(SweepStep::Flip(toggle)) = SweepApply::default().feed(&edit) {
                        kept.push(CollectionEvent::Item(toggle.key, TilePartEvent::Check { mods: Mods::NONE }));
                    }
                }
                other => kept.push(other),
            }
        }
        for key in self.tail_clicked {
            let reported = kept.iter().any(|event| matches!(event, CollectionEvent::Activate(done) if *done == key));
            if !reported {
                kept.push(CollectionEvent::Activate(key));
            }
        }
        response.events = kept;
        // A tail row has no menu: its request is dropped above, and so is the request the grid would carry for it.
        response.menu = response.menu.filter(|request| !is_tail(&request.key));
        TileGridOutput { response, events }
    }
}

/// The highest rating a digit key sets.
const MAX_RATING: u8 = 5;
/// The keys of the digits 0 to `MAX_RATING`.
const DIGITS: [Key; 6] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5];

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Collection for TileGrid<'_, K> {
    type Key = K;
    type ItemLayout = GridLayout;
    type Selection = Selection<K>;
    type Item<'r> = Tile<'r>;
    type ItemEvent = TilePartEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<K>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn reveal(&self) -> Option<&K> {
        self.reveal.as_ref()
    }

    /// A bound zoom takes the wheel's zoom share; a fixed one is the host's.
    fn zooms(&self) -> bool {
        matches!(self.zoom, GridZoom::Bound(..))
    }

    /// While the zoom is bound: the point of this pass's wheel or pinch zoom, else the pointer a zoom chord was taken
    /// at in this pass's `update`; `None` keeps the first row in view.
    fn zoom_anchor(&self, wheel: Option<Pos2>) -> Option<Pos2> {
        match self.zoom {
            GridZoom::Bound(..) => wheel.or(self.chord_at),
            GridZoom::Fixed(_) => None,
        }
    }

    /// The tiles with a header row before each group, then the tail rows: the rows the item step walks and the layout
    /// places.
    fn filtered<'s>(&self, source: &'s dyn ItemSource<K>, _: &crate::runtime::env::Env, _: Option<&K>) -> Option<Box<dyn ItemSource<K> + 's>> {
        (!self.groups.is_empty() || !self.tail.is_empty()).then(|| {
            Box::new(Outline::new(source, &self.shown_groups(), self.tail, self.collapses(), self.group_heads)) as Box<dyn ItemSource<K> + 's>
        })
    }

    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> GridLayout {
        self.laid_out(None, source, width, None, cx)
    }

    fn measures_items(&self) -> bool {
        self.content_cells
    }

    /// The caption place, the detail line, the card look, the actions' place and whether checks show, which `dress`
    /// gives every tile.
    fn dress_inputs(&self) -> u64 {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.caption, self.detail_line, self.card, self.card_text, self.regular_card_padding, self.action_place, self.checks).hash(&mut hasher);
        hasher.finish()
    }

    /// The last layout again while the width, the rows stamp and everything the arrangement reads are the same.
    fn ideal_width(&self, item: Option<Vec2>, _rows: usize, cx: &StyleCx<'_>) -> f32 {
        let style = GridSizes::of(cx, self.card, self.card_text, self.items.row_height, self.regular_card_padding, self.action_place);
        let aspect = if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 };
        let around = style.around.get(self.caption, self.detail_line);
        let card = style.card(self.picture_side(&style, item, around), aspect, around).x;
        // At least one card: in a narrower column the cards shrink, and the grid would measure shorter than it shows
        // in the wider column it is given.
        let columns = self.card_columns.max(1) as f32;
        // Half a point over, so the division that counts the columns back never rounds below `columns`.
        let spacing = style.spacing(self.caption);
        columns * card + (columns - 1.0) * spacing.gap + 2.0 * spacing.inset + 0.5
    }

    fn item_layout_from(
        &self,
        previous: Option<GridLayout>,
        source: &dyn ItemSource<K>,
        width: f32,
        item: Option<Vec2>,
        cx: &StyleCx<'_>,
    ) -> GridLayout {
        self.laid_out(previous, source, width, item, cx)
    }

    /// Every tile gets the grid's caption place, detail line and actions' place, the card look, and its row's rename
    /// state. Under `checks` a tile's check is its own selection, so the app keeps no second copy of it.
    fn dress<'r>(&self, tile: Self::Item<'r>, row: &RowState) -> Self::Item<'r> {
        let tile =
            tile.caption(self.caption).detail_line(self.detail_line).action_place(self.action_place).renaming(row.renaming).selected(row.selected);
        let tile = if self.checks { tile.check(if row.selected { ToggleState::On } else { ToggleState::Off }) } else { tile };
        if self.card { tile.card().card_text(self.card_text).regular_card_padding(self.regular_card_padding) } else { tile }
    }

    fn can_rename(&self, key: &K) -> bool {
        self.can_rename.as_ref().is_none_or(|test| test.passes(key))
    }

    fn can_drag(&self, key: &K) -> bool {
        self.can_drag.as_ref().is_none_or(|test| test.passes(key))
    }

    fn drops(&self) -> &[DropKind<'_, K>] {
        &self.drops
    }

    /// No drop lands on a tail row or after one; the app's test decides the rest.
    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        let on_tail = over.is_some_and(|key| self.tail.iter().any(|row| row.key == *key));
        let past_tail = self.tail_from().is_some_and(|from| !self.tail.is_empty() && insert > from);
        !on_tail && !past_tail && self.accepts.as_ref().is_none_or(|test| test.passes(drag, over, insert))
    }

    fn rename_editor(&mut self, ui: &mut Ui, draft: Bind<'_, String>, label: Rect, name: &str) -> Option<EditResponse<String>> {
        Some(crate::text::rename_field(draft, name).show_at(ui, label))
    }
}

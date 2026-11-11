//! `TileGrid`: a collection of `Tile`s in wrapped cards or justified rows, under optional group headers. The item step
//! does the work: virtualisation, lead and selection rules, arrow, Home, End and page keys over the layout, type-ahead,
//! drag with drop marks, OS file drops, the rename lifecycle and the context-menu request. The grid adds the layout,
//! the pinned group headers and the tail rows after the tiles, which are rows of the grid's outline (section rows of
//! the item step that hold the lead), and the rating keys. It owns what its cards have in common: where the
//! caption goes, whether a detail line is reserved, and the zoom.

#[cfg(feature = "testing")]
mod stories;

use super::headers::Outline;
use super::layout::{Around, ArrangeSpacing, GridLayout, Recipe, Slot, TailRow, TileGroup, Tiling, arrange, aspects_of, slot_at};
use super::tile::AroundPicture;
use super::{CaptionPlace, Tile, TilePartEvent};
use crate::contract::{
    Access, Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, ColumnKind, ColumnSpec, Common, Component,
    ContainerProps, Cx, EditResponse, Focus as _, ListDrag, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, RowState, SelectMode,
    Sizing, SpaceAction, SweepApply, SweepStep, ToggleColumn,
};
use crate::contract::{lead_position, section_part};
use crate::controls::disclosure::Disclosure;
use crate::list::{DropTest, ListRow, RowTest, SourceTest, ThumbSize};
use crate::primitives::PaintShape;
use crate::runtime::access::Name;
use crate::runtime::commands::FireSource;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, RuleKind, StyleColor, StyleCx};
use craft_behavior::{
    ColumnKey, ColumnWidth, HitLevel, HitPriority, Intent, ItemSource, Metrics, Mixed, PartKey, PointerButton, PressPhase, Selection, Sense,
};
use craft_keymap::Mods;
use egui::{CornerRadius, Id, Pos2, Rect, Response, Ui, pos2, vec2};
use std::cell::RefCell;
use std::hash::Hash;

/// The heading level of a group header: the grid's groups are its top-level sections.
const GROUP_LEVEL: usize = 1;
const TOGGLE: PartKey = PartKey::new("group_toggle");
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
}

/// What the grid reports besides the collection's events.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GridEvent<K> {
    /// A tile's stars were clicked (that tile), or a rating key was pressed (the selected tiles, or the lead when none
    /// is selected): the rating, 0 clears. The one route to a rating; the tile's own `TilePartEvent::Rate` does not
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

/// The sizes of the grid and the look of the pinned group headers; the headers' own look is the heading row's.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TileGridStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A header pinned over its group's tiles.
    pinned: StyleColor,
    /// The side of the disclosure column and the space beside the header's content.
    toggle: f32,
    pad: f32,
    /// The picture side of a card at zoom 1 when a grid names no thumbnail size: the card thumbnail.
    card: f32,
    /// The picture side of a card at zoom 1 for each thumbnail size.
    thumbs: [f32; 3],
    /// What a tile adds around its picture.
    around: AroundPicture,
    /// The gaps of the arrangement and the height of a group header.
    spacing: ArrangeSpacing,
}

impl TileGridStyle {
    fn build(cx: &StyleCx<'_>, card: bool) -> TileGridStyle {
        let colors = &cx.tokens.color;
        TileGridStyle {
            metrics: Metrics::new(),
            pinned: cx.color(colors.surface.panel),
            toggle: cx.tokens.size.icon.sm,
            pad: cx.tokens.space.gap.inline,
            card: cx.tokens.size.thumb.card,
            thumbs: [cx.tokens.size.thumb.sm, cx.tokens.size.thumb.md, cx.tokens.size.thumb.lg],
            around: Tile::around(cx, card),
            spacing: ArrangeSpacing::new(
                if card { cx.tokens.space.gap.section } else { cx.tokens.space.gap.stack },
                cx.tokens.space.gap.inline,
                cx.tokens.size.panel_header,
            ),
        }
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

/// A grid of tiles: one Tab stop, the selection rules of the collection contract, type-ahead on the tiles' names,
/// rename in place, drag to reorder, files dropped from the OS, and the request for the app's context menu.
///
/// The picture side is `zoom` times the thumbnail side (the card thumbnail unless `thumb_size` names one): the factor the app's zoom control holds. A card is the
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
    #[prop(quantity = "zoom factor")]
    zoom: f32,
    /// The thumbnail size the zoom is a factor of; `None` is the card thumbnail.
    #[prop]
    thumb_size: Option<ThumbSize>,
    /// The columns of cards a grid that takes its width from its content asks for: its width holds this many cards and
    /// their gaps. 0 asks for none, so the content decides.
    #[prop]
    card_columns: usize,
    /// Where every tile's caption goes; `CaptionPlace::None` keeps no room for one.
    #[prop]
    caption: CaptionPlace,
    /// Cards keep a line under the name for the tile's detail, whether a tile has one or not.
    #[prop(flag)]
    detail_line: bool,
    /// Tiles are framed cards: the picture inset in a raised surface with the name and detail at the reading start.
    #[style_key]
    #[prop(flag)]
    card: bool,
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
    /// Which collections a drag may come from: the test gets the id of the collection the drag started in.
    #[prop(func = "Fn(Id) -> bool")]
    accept_from: Option<SourceTest<'a>>,
    /// Where a drag may land: the test gets the drag, the tile under the pointer and the position before which the
    /// dragged tiles would go.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
    /// Files dragged in from the OS may drop on the grid; a drop reports `CollectionEvent::FilesDropped`.
    #[prop]
    takes_files: bool,
    /// The layout of this pass, kept for the headers' parts and paint.
    laid: RefCell<Option<GridLayout>>,
    /// The headers this pass shows, in the grid's layer space: the group, its rect and whether it is pinned.
    placed: Vec<Placed>,
    /// The tail rows this pass shows, in the grid's layer space: the row's index and its rect.
    placed_tail: Vec<(usize, Rect)>,
    /// The tail rows clicked this pass, in order.
    tail_clicked: Vec<K>,
    events: Vec<GridEvent<K>>,
}

/// A group header as it shows this pass.
#[derive(Clone, Copy, Debug)]
struct Placed {
    group: usize,
    rect: Rect,
    pinned: bool,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> TileGrid<'a, K> {
    /// A grid named `label`, with multiple selection and equal square cards of the card thumbnail.
    pub fn new(label: impl Into<Label>) -> Self {
        TileGrid {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Multi),
            selection: None,
            reveal: None,
            justified: false,
            fill_row: false,
            aspect: 1.0,
            zoom: 1.0,
            thumb_size: None,
            card_columns: 0,
            caption: CaptionPlace::Below,
            detail_line: false,
            card: false,
            groups: &[],
            collapsible: false,
            tail: &[],
            rating_keys: RatingKeys::None,
            checks: false,
            can_rename: None,
            can_drag: None,
            accept_from: None,
            accepts: None,
            takes_files: false,
            laid: RefCell::new(None),
            placed: Vec::new(),
            placed_tail: Vec::new(),
            tail_clicked: Vec::new(),
            events: Vec::new(),
        }
        .label(label)
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
        self.items = self.items.columns(columns).header(!on).space(if on { SpaceAction::ToggleCell } else { SpaceAction::default() });
        self
    }

    /// A tile drag carries `map` of it, for the drop targets of type `P`; a tile the map gives `None` for does not drag
    /// unless a reorder rule is set. The drag stays one session with one ghost.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
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
    fn laid_out(&self, previous: Option<GridLayout>, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> GridLayout {
        let style = TileGridStyle::build(cx, self.card);
        let spacing = style.spacing;
        let around = style.around.get(self.caption, self.detail_line);
        // A card never outgrows the grid: in a column narrower than a card the card takes the column.
        let mut side = style.picture_side(self.zoom, self.thumb_size).min(width - 2.0 * spacing.inset - around.size().x).max(1.0);
        if self.fill_row && !self.justified {
            let (free, gap, card) = (width, spacing.gap, side + around.size().x);
            let columns = ((free + gap) / (card + gap)).floor().max(1.0);
            side = ((free - (columns - 1.0) * gap) / columns - around.size().x).max(1.0);
        }
        let aspect = if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 };
        let tail_height = ListRow::row_height(cx, ThumbSize::None, false);
        let recipe = self.items.rows_stamp.map(|stamp| Recipe {
            stamp,
            width,
            tiles: source.len(),
            side,
            aspect,
            justified: self.justified,
            around,
            groups: self.groups.iter().map(|group| (group.start, group.count, group.open)).collect(),
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
            let mut arrangement = arrange(&aspects_of(source), self.groups, width, tiling, spacing, around);
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

    /// The row that holds the lead while the grid has focus, when the grid has an arrangement.
    fn lead_slot(&self, cx: &Cx<'_, Self>) -> Option<Slot> {
        if !cx.is_focused() {
            return None;
        }
        let at = lead_position(&cx.core.ctx, cx.core.id)?;
        let laid = self.laid.borrow();
        let arrangement = laid.as_ref()?.arrangement()?;
        Some(slot_at(&arrangement.starts, arrangement.tiles.len(), at))
    }

    /// The outline position of the first tail row in this pass's layout, when the grid has one.
    fn tail_from(&self) -> Option<usize> {
        self.laid.borrow().as_ref().and_then(|layout| layout.arrangement().map(|arrangement| arrangement.tail_from()))
    }

    fn group_rect(rect: Rect, view: Rect, rtl: bool) -> Rect {
        if rtl {
            Rect::from_min_max(pos2(view.min.x + view.max.x - rect.max.x, rect.min.y), pos2(view.min.x + view.max.x - rect.min.x, rect.max.y))
        } else {
            rect
        }
    }

    /// The disclosure column of the header in `rect`, at its reading start.
    fn toggle_rect(rect: Rect, style: &TileGridStyle, rtl: bool) -> Rect {
        let x = if rtl { rect.max.x - style.pad - style.toggle } else { rect.min.x + style.pad };
        Rect::from_min_size(pos2(x, rect.min.y), vec2(style.toggle, rect.height()))
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Component for TileGrid<'_, K> {
    const ROLE: Role = Role::LIST_BOX;
    const OVERLAYS: bool = false;
    type Style = TileGridStyle;
    type Output = TileGridOutput<K>;

    fn style(cx: &StyleCx<'_>, key: TileGridStyleKey<K>) -> TileGridStyle {
        TileGridStyle::build(cx, key.card)
    }

    /// One pointer part per header in view, over the tiles, and a disclosure part in it when headers collapse; one
    /// pointer part per tail row in view.
    fn parts(&mut self, area: Rect, cx: &mut PartsCx<'_, Self>) {
        self.placed.clear();
        self.placed_tail.clear();
        let Some(arrangement) = self.laid.borrow().as_ref().and_then(|layout| layout.arrangement().cloned()) else { return };
        let (view, offset, rtl) = (cx.content_rect, cx.offset(), cx.flow.is_rtl());
        let pinned = arrangement.pinned(offset.y as f32);
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
            self.placed.push(Placed { group, rect, pinned: held });
            let priority = HitPriority::new(HitLevel::Edge, 0);
            let sense = Sense::CLICK | Sense::TAKE_FOCUS;
            cx.part(section_part(&declared.key), rect).sense(sense).priority(priority).over().pointer_only(HEADER_KEYS);
            if self.collapsible {
                let column = Self::toggle_rect(rect, cx.style, rtl);
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
        if self.rating_keys == RatingKeys::Digits {
            for rating in 0..=MAX_RATING {
                while cx.intents().take_exact(&Intent::Digit(rating)) {
                    let keys = self.rated(cx);
                    if !keys.is_empty() {
                        self.events.push(GridEvent::Rate { keys, rating });
                    }
                }
            }
        }
        let placed = self.placed.clone();
        for Placed { group, rect, .. } in placed {
            let Some(declared) = self.groups.get(group) else { continue };
            let part = section_part(&declared.key);
            let toggle = TOGGLE.at(group as u64);
            let open = declared.open;
            if let Some(result) = cx.part_if_declared(part)
                && let PressPhase::Release { start, click: true, .. } = result.press.phase
            {
                let mods = result.press.mods;
                self.events.push(match start.button {
                    PointerButton::Secondary => {
                        GridEvent::GroupMenu { group, at: if cx.flow.is_rtl() { rect.right_bottom() } else { rect.left_bottom() } }
                    }
                    _ => GridEvent::GroupPicked { group, mods },
                });
            } else if cx.part_activated(part) == Some(FireSource::Access) {
                self.events.push(GridEvent::GroupPicked { group, mods: Mods::NONE });
            }
            if self.collapsible && cx.part_activated(toggle).is_some() {
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
        if self.collapsible {
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

    /// A heading node per header and a list item node per tail row in view.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let headers = self.placed.iter().map(|placed| Slot::Header(placed.group));
        let tails = self.placed_tail.iter().map(|&(at, _)| Slot::Tail(at));
        for slot in headers.chain(tails) {
            match slot {
                Slot::Header(group) => {
                    let Some(group) = self.groups.get(group) else { continue };
                    let node = access.node(section_part(&group.key), Role::HEADING);
                    node.level(GROUP_LEVEL);
                    node.name = Name::Text(if group.detail.is_empty() { group.title.clone() } else { format!("{}, {}", group.title, group.detail) });
                    if self.collapsible {
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
            if placed.pinned {
                let draw = ui.draw();
                draw.fill(&PaintShape::Rect(rect, CornerRadius::ZERO), style.pinned);
                draw.rule(RuleKind::Divider, rect.left_bottom(), rect.right_bottom());
            }
            let lead = RowState { emphasized: lead == Some(Slot::Header(placed.group)), ..RowState::default() };
            let mut heading = ListRow::new(group.title.clone()).heading().node(false).state(lead);
            if !group.detail.is_empty() {
                heading = heading.trailing(group.detail.clone());
            }
            ui.face(section_part(&group.key), heading);
            if self.collapsible {
                let disclosure = Disclosure::new(Bind::given(group.open)).node(false).label(group.title.clone());
                ui.face(TOGGLE.at(placed.group as u64), disclosure);
            }
        }
        for &(at, rect) in &self.placed_tail {
            let row = &self.tail[at];
            if at == 0 {
                ui.draw().rule(RuleKind::Divider, rect.left_top(), rect.right_top());
            }
            let state = RowState { emphasized: lead == Some(Slot::Tail(at)), ..RowState::default() };
            ui.face(section_part(&row.key), ListRow::new(row.label.clone()).node(false).state(state));
        }
    }

    /// The grid's events are the digits' and the stars' ratings, in that order, besides the collection's own. The
    /// collection's events on a header row are the group events instead: Enter picks the group, the context-menu
    /// key asks for its menu, Left and Right open and close it, and the lead on it is not reported. A tail row keeps its
    /// Enter and the lead on it, and adds its click as `CollectionEvent::Activate`; it has no menu.
    fn finish(self, mut cx: Cx<'_, Self>) -> TileGridOutput<K> {
        let mut response = cx.collection_response();
        let mut events = self.events;
        let mods = crate::runtime::pointer::frame(&cx.core.ctx).mods;
        let group_of = |key: &K| self.groups.iter().position(|group| group.key == *key);
        let is_tail = |key: &K| self.tail.iter().any(|row| row.key == *key);
        let mut kept = Vec::with_capacity(response.events.len());
        for event in std::mem::take(&mut response.events) {
            let tail_menu = matches!(&event, CollectionEvent::ContextMenu(request) if is_tail(&request.key));
            if tail_menu {
                continue;
            }
            let header = match &event {
                CollectionEvent::Activate(key) | CollectionEvent::LeadMoved { key, .. } => group_of(key),
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
                CollectionEvent::Item(key, TilePartEvent::Rate(rating)) => events.push(GridEvent::Rate { keys: vec![key], rating }),
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
        TileGridOutput { response, events }
    }
}

/// The highest rating a digit key sets.
const MAX_RATING: u8 = 5;

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

    /// The tiles with a header row before each group, then the tail rows: the rows the item step walks and the layout
    /// places.
    fn filtered<'s>(&self, source: &'s dyn ItemSource<K>, _: &crate::runtime::env::Env) -> Option<Box<dyn ItemSource<K> + 's>> {
        (!self.groups.is_empty() || !self.tail.is_empty())
            .then(|| Box::new(Outline::new(source, self.groups, self.tail, self.collapsible)) as Box<dyn ItemSource<K> + 's>)
    }

    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> GridLayout {
        self.laid_out(None, source, width, cx)
    }

    /// The last layout again while the width, the rows stamp and everything the arrangement reads are the same.
    fn ideal_width(&self, cx: &StyleCx<'_>) -> f32 {
        if self.card_columns == 0 {
            return 0.0;
        }
        let style = TileGridStyle::build(cx, self.card);
        let aspect = if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 };
        let card = style.card(style.picture_side(self.zoom, self.thumb_size), aspect, style.around.get(self.caption, self.detail_line)).x;
        let columns = self.card_columns as f32;
        // Half a point over, so the division that counts the columns back never rounds below `columns`.
        columns * card + (columns - 1.0) * style.spacing.gap + 2.0 * style.spacing.inset + 0.5
    }

    fn item_layout_from(&self, previous: Option<GridLayout>, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> GridLayout {
        self.laid_out(previous, source, width, cx)
    }

    /// Every tile gets the grid's caption place and detail line, the card look, and its row's rename state.
    fn dress<'r>(&self, tile: Self::Item<'r>, row: &RowState) -> Self::Item<'r> {
        let tile = tile.caption(self.caption).detail_line(self.detail_line).renaming(row.renaming);
        if self.card { tile.card() } else { tile }
    }

    fn can_rename(&self, key: &K) -> bool {
        self.can_rename.as_ref().is_none_or(|test| test.passes(key))
    }

    fn can_drag(&self, key: &K) -> bool {
        self.can_drag.as_ref().is_none_or(|test| test.passes(key))
    }

    fn accept_from(&self, from: Id) -> bool {
        self.accept_from.as_ref().is_none_or(|test| test.passes(from))
    }

    /// No drop lands on a tail row or after one; the app's test decides the rest.
    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        let on_tail = over.is_some_and(|key| self.tail.iter().any(|row| row.key == *key));
        let past_tail = self.tail_from().is_some_and(|from| !self.tail.is_empty() && insert > from);
        !on_tail && !past_tail && self.accepts.as_ref().is_none_or(|test| test.passes(drag, over, insert))
    }

    fn takes_files(&self) -> bool {
        self.takes_files
    }

    fn rename_editor(&mut self, ui: &mut Ui, draft: Bind<'_, String>, label: Rect, name: &str) -> Option<EditResponse<String>> {
        Some(crate::text::rename_field(ui, draft, label, name))
    }
}

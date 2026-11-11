//! `FreeBin`: a collection of `Tile`s at positions the host stores, with the flow of unplaced cards, moves of the
//! selection together with its stacks, the marquee, spatial arrow keys and drops of foreign drags.
//!
//! The item step does the pointer and key work of a free layout: selection, the lead, arrows over the cards, Home and
//! End, the marquee, the move gesture, rename, delete and the context menu. `FreeBin` adds the placement and turns the
//! move gesture into the cards' new positions.

#[cfg(feature = "screen-testing")]
mod stories;

use std::cell::RefCell;
use std::collections::HashMap;

use craft_behavior::{GesturePhase, ItemSource, Metrics, Selection};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, DropMark, DropTarget,
    EditResponse, ListDrag, NoValue, Outcome, PartsCx, Pointer as _, Props as _, Role, RowState, SelectMode, Show as _, Sizing,
};
use craft_ui::primitives::textures::TextureKey;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::tile_grid::{Around, CaptionPlace, Tile, TilePartEvent};
use egui::{Id, Pos2, Rect, Response, Ui, Vec2, pos2, vec2};

/// A card in the free bin, where the host stores it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FreeformCard<K> {
    /// The item or bin.
    pub key: K,
    /// Its top-left corner in the view, in points, counted from the reading start edge.
    pub pos: Vec2,
    /// The stack it belongs to: the cards of one stack move together.
    pub stack: Option<u32>,
}

impl<K> FreeformCard<K> {
    /// A card for `key` at `pos`, in no stack.
    pub fn new(key: K, pos: Vec2) -> Self {
        FreeformCard { key, pos, stack: None }
    }

    /// The card in the stack `stack`.
    #[must_use]
    pub fn in_stack(mut self, stack: u32) -> Self {
        self.stack = Some(stack);
        self
    }
}

/// Which cards take a drop of a foreign drag: the test `FreeBin::drop_into` is given.
pub struct TakesDrop<'a, K>(Box<dyn Fn(&K) -> bool + 'a>);

impl<'a, K> TakesDrop<'a, K> {
    /// The test that runs `test`.
    pub fn new(test: impl Fn(&K) -> bool + 'a) -> Self {
        TakesDrop(Box::new(test))
    }

    fn passes(&self, key: &K) -> bool {
        (self.0)(key)
    }
}

/// The cards a drag is carrying and how far, while the pointer holds them. The output gives it each pass; the host
/// hands it back with `FreeBin::moving` so the cards show where the drag has them, and stops when the output gives
/// none.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FreeBinMove<K> {
    /// The cards that move: the selection and the stacks of its cards.
    pub keys: Vec<K>,
    /// How far they have moved from the host's positions, in points from the reading start edge.
    pub by: Vec2,
}

/// Where a card sits before any drag: its place in the host's list or in the flow.
#[derive(Clone, Debug)]
struct Slot<K> {
    key: K,
    pos: Vec2,
    stack: Option<u32>,
}

/// The sizes of a card and the space between cards.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FreeBinStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The picture side of a card at zoom 1: the card thumbnail.
    thumb: f32,
    /// What a tile adds around its picture.
    around: Around,
    /// The space between flowed cards and from the view's edges.
    gap: f32,
}

impl FreeBinStyle {
    fn build(cx: &StyleCx<'_>) -> FreeBinStyle {
        FreeBinStyle {
            metrics: Metrics::new(),
            thumb: cx.tokens.size.thumb.md,
            around: Tile::around_picture(cx, CaptionPlace::Below, false),
            gap: cx.tokens.space.gap.inline,
        }
    }

    /// An equal card: a picture `side` wide with the aspect `aspect`, and what the tile adds around it.
    fn card(&self, side: f32, aspect: f32) -> Vec2 {
        vec2(side, side / aspect) + vec2(self.around.width(), self.around.height())
    }
}

/// A collection of cards at stored positions. A card without a position takes the next free cell of a flow, and with
/// `flow_all` every card does.
///
/// The picture side is `zoom` times the card thumbnail side and pictures are `aspect` wide over high, as for
/// `TileGrid`. Dragging a selected card moves the selection and the stacks of its cards; the output reports the new
/// positions when the drag ends, and Escape leaves the cards where the host has them.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(collection, probe(FreeBin<'static, usize>))]
pub struct FreeBin<'a, K: TextureKey + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    /// The selection the caller owns.
    #[prop]
    selection: Option<Bind<'a, Selection<K>>>,
    /// Scrolls the card of `key` into view once.
    #[prop]
    reveal: Option<K>,
    /// The picture side as a multiple of the card thumbnail side; 1 is the card thumbnail.
    #[prop(quantity = "zoom factor")]
    zoom: f32,
    /// The width of a card's picture over its height.
    #[prop(quantity = "aspect ratio")]
    aspect: f32,
    /// Committed positions round to multiples of this step.
    #[prop(quantity = "snap step")]
    snap: Option<f32>,
    /// Every card is placed by the flow, whatever the host stores.
    #[prop]
    flow_all: bool,
    /// The drag in progress, as the last pass's output gave it.
    #[prop]
    moving: Option<FreeBinMove<K>>,
    /// Which cards take a drop of a foreign drag.
    #[prop(func = "Fn(&K) -> bool")]
    drop_into: Option<TakesDrop<'a, K>>,
    cards: &'a [FreeformCard<K>],
    /// The cards of this pass in display order with their places before any drag.
    slots: RefCell<Vec<Slot<K>>>,
    /// The rects of the cards that take a foreign drop, from the item step of this pass.
    takers: Vec<Rect>,
    /// A foreign drag released off the cards that take it.
    background: Option<ListDrag<K>>,
}

impl<'a, K: TextureKey> FreeBin<'a, K> {
    /// A bin named `label` showing the cards placed by `cards`, with multiple selection, the marquee, rename and delete.
    pub fn new(label: impl Into<Label>, cards: &'a [FreeformCard<K>]) -> Self {
        FreeBin {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Multi).marquee(true).rename(true).deletable(true),
            selection: None,
            reveal: None,
            zoom: 1.0,
            aspect: 1.0,
            snap: None,
            flow_all: false,
            moving: None,
            drop_into: None,
            cards,
            slots: RefCell::new(Vec::new()),
            takers: Vec::new(),
            background: None,
        }
        .label(label)
    }

    /// The layout of this pass: every card at its stored place, else in the next cell of the flow, shifted by the drag
    /// in progress. Rects count from the screen's left, so a right-to-left view mirrors the positions.
    fn laid_out(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> craft_behavior::FreeLayout {
        let style = FreeBinStyle::build(cx);
        let aspect = if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 };
        let cell = style.card(style.thumb * self.zoom.max(f32::EPSILON), aspect);
        let columns = (((width - style.gap) / (cell.x + style.gap)).floor() as usize).max(1);
        let stored: HashMap<&K, &FreeformCard<K>> =
            if self.flow_all { HashMap::new() } else { self.cards.iter().map(|card| (&card.key, card)).collect() };
        let rtl = cx.direction.is_rtl();
        // A card without a place takes the next cell of the flow that no placed card covers (Finder).
        let occupied: Vec<Rect> = stored.values().map(|card| Rect::from_min_size(pos2(card.pos.x, card.pos.y), cell)).collect();
        let flow_cell = |place: usize| {
            vec2(style.gap + (place % columns) as f32 * (cell.x + style.gap), style.gap + (place / columns) as f32 * (cell.y + style.gap))
        };
        let mut next_cell = 0usize;
        let mut slots = Vec::with_capacity(source.len());
        let mut rects = Vec::with_capacity(source.len());
        for at in 0..source.len() {
            let key = source.key(at);
            let card = stored.get(&key).copied();
            let pos = match card {
                Some(card) => card.pos,
                None => loop {
                    let at = flow_cell(next_cell);
                    next_cell += 1;
                    if !occupied.iter().any(|taken| taken.intersects(Rect::from_min_size(pos2(at.x, at.y), cell))) {
                        break at;
                    }
                },
            };
            let shown = match &self.moving {
                Some(live) if live.keys.contains(&key) => pos + live.by,
                _ => pos,
            };
            let x = if rtl { width - shown.x - cell.x } else { shown.x };
            rects.push(Rect::from_min_size(pos2(x, shown.y), cell));
            slots.push(Slot { key, pos, stack: card.and_then(|card| card.stack) });
        }
        *self.slots.borrow_mut() = slots;
        craft_behavior::FreeLayout::new(rects)
    }

    /// The cards a drag of `selection` moves, in display order: the selected cards and the cards of their stacks.
    fn carried(&self, selection: &Selection<K>) -> Vec<Slot<K>> {
        let slots = self.slots.borrow();
        let stacks: Vec<u32> = slots.iter().filter(|slot| selection.items.contains(&slot.key)).filter_map(|slot| slot.stack).collect();
        slots.iter().filter(|slot| selection.items.contains(&slot.key) || slot.stack.is_some_and(|stack| stacks.contains(&stack))).cloned().collect()
    }

    /// How far the carried cards move for a pointer offset `raw` in screen space: the grabbed card snaps to the grid
    /// and the rest keep their distance from it. Positions count from the reading start edge, so a right-to-left
    /// view flips the horizontal offset.
    fn displacement(&self, grabbed: Vec2, raw: Vec2, rtl: bool) -> Vec2 {
        let by = if rtl { vec2(-raw.x, raw.y) } else { raw };
        match self.snap {
            Some(step) if step > 0.0 => {
                let target = grabbed + by;
                vec2((target.x / step).round() * step, (target.y / step).round() * step) - grabbed
            }
            _ => by,
        }
    }
}

/// Where a foreign drag may drop off the cards: anywhere in the view outside the cards that take drops.
struct Background {
    own: Id,
    view: Rect,
    takers: Vec<Rect>,
}

impl<K> DropTarget<ListDrag<K>> for Background {
    type Drop = ();

    fn accept(&self, payload: &ListDrag<K>, at: Pos2, _: craft_ui::contract::DropOperation) -> Option<()> {
        (payload.list != self.own && !self.takers.iter().any(|rect| rect.contains(at))).then_some(())
    }

    fn mark(&self, _: &()) -> DropMark {
        DropMark::Into(self.view)
    }
}

/// What a `FreeBin` reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the bin's events"]
pub struct FreeBinOutput<K> {
    /// The collection's response and events: selection, activation, rename, delete, context menu, a drop on a card.
    pub response: CollectionResponse<K, TilePartEvent>,
    /// The cards a drag carries now and how far; `None` when no drag runs. Hand it back with `FreeBin::moving`.
    pub moving: Option<FreeBinMove<K>>,
    /// On a committed drag: every moved card with its new position.
    pub moved: Vec<(K, Vec2)>,
    /// A foreign drag released off every card that takes it.
    pub dropped_on_background: Option<ListDrag<K>>,
}

impl<K> Outcome for FreeBinOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.response.story_facts()
    }
}

impl<K: TextureKey> Component for FreeBin<'_, K> {
    const ROLE: Role = Role::LIST_BOX;
    type Style = FreeBinStyle;
    type Output = FreeBinOutput<K>;

    fn style(cx: &StyleCx<'_>, _: FreeBinStyleKey<K>) -> FreeBinStyle {
        FreeBinStyle::build(cx)
    }

    /// The rects of the cards that take a foreign drop.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        self.takers.clear();
        let Some(test) = &self.drop_into else { return };
        for slot in self.slots.borrow().iter().filter(|slot| test.passes(&slot.key)) {
            if let Some(rect) = cx.rect_of(&slot.key) {
                self.takers.push(rect);
            }
        }
    }

    /// A foreign drag released in the view off the cards that take it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some((payload, _)) = cx.dragging::<ListDrag<K>>() else { return };
        let target = Background { own: cx.id, view: cx.rect, takers: self.takers.clone() };
        if cx.drop_target::<ListDrag<K>, _>(&target).is_some() {
            self.background = Some((*payload).clone());
        }
    }

    /// The drag's carried cards and their positions from the collection's move gesture.
    fn finish(self, mut cx: Cx<'_, Self>) -> FreeBinOutput<K> {
        let rtl = cx.flow.is_rtl();
        let selection = cx.collection_selection().cloned().unwrap_or_default();
        let response = cx.collection_response();
        let carried = self.carried(&selection);
        let grabbed = selection
            .lead
            .as_ref()
            .and_then(|lead| carried.iter().find(|slot| slot.key == *lead))
            .or(carried.first())
            .map_or(Vec2::ZERO, |slot| slot.pos);
        let live = |raw: Vec2| FreeBinMove { keys: carried.iter().map(|slot| slot.key.clone()).collect(), by: self.displacement(grabbed, raw, rtl) };
        let mut moving = self.moving.clone();
        let mut moved = Vec::new();
        for event in &response.events {
            let CollectionEvent::Move(edit) = event else { continue };
            match edit.phase() {
                Some(GesturePhase::Begin) => moving = Some(live(Vec2::ZERO)),
                Some(GesturePhase::Update) => moving = edit.value().map(|raw| live(*raw)),
                Some(GesturePhase::Commit) => {
                    if let Some(raw) = edit.value() {
                        let by = self.displacement(grabbed, *raw, rtl);
                        moved = carried.iter().map(|slot| (slot.key.clone(), slot.pos + by)).collect();
                    }
                    moving = None;
                }
                Some(GesturePhase::Cancel) => moving = None,
                _ => {}
            }
        }
        FreeBinOutput { response, moving, moved, dropped_on_background: self.background }
    }
}

impl<K: TextureKey> Collection for FreeBin<'_, K> {
    type Key = K;
    type ItemLayout = craft_behavior::FreeLayout;
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

    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> craft_behavior::FreeLayout {
        self.laid_out(source, width, cx)
    }

    /// Every tile gets the bin's caption place and its row's rename state.
    fn dress<'r>(&self, tile: Self::Item<'r>, row: &RowState) -> Self::Item<'r> {
        tile.caption(CaptionPlace::Below).renaming(row.renaming)
    }

    fn accepts(&self, _: &ListDrag<K>, over: Option<&K>, _: usize) -> bool {
        over.is_none_or(|over| self.drop_into.as_ref().is_some_and(|test| test.passes(over)))
    }

    fn rename_editor(&mut self, ui: &mut Ui, draft: Bind<'_, String>, label: Rect, name: &str) -> Option<EditResponse<String>> {
        Some(craft_ui::text::TextField::new(name.to_owned(), draft).show_at(ui, label))
    }
}

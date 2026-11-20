//! The collection's keys: Space actions, ragged-grid moves, hidden items, digits against type-ahead, the header
//! flag, the lead's source and the first sort order.

use super::kit::{Count, Dismissed, ROW_H, Row};
use craft_behavior::{ItemLayout, ItemSource, NavShape, ScrollPos, Selection};
use craft_ui::contract::{
    Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, Focus as _, LeadSource, Props as _,
    Role, SelectMode, ShowCollection as _,
};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, input, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(400.0, 240.0);
const TILE: Vec2 = Vec2::new(30.0, ROW_H);

/// What a collection reported and where its items were built.
#[derive(Default)]
struct Seen {
    id: Cell<Option<Id>>,
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
    digits: RefCell<Vec<u8>>,
}

impl Seen {
    fn take(&self, out: CollectionResponse<usize, Dismissed>) {
        self.id.set(Some(out.response.id));
        self.events.borrow_mut().extend(out.events);
    }

    fn note(&self, key: usize, rect: Rect) {
        let mut rows = self.rows.borrow_mut();
        rows.retain(|(held, _)| *held != key);
        rows.push((key, rect));
    }

    fn leads(&self) -> Vec<(usize, LeadSource)> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| if let CollectionEvent::LeadMoved { key, by, .. } = event { Some((*key, *by)) } else { None })
            .collect()
    }

    fn lead_keys(&self) -> Vec<usize> {
        self.leads().into_iter().map(|(key, _)| key).collect()
    }

    fn id(&self) -> Id {
        self.id.get().expect("the collection ran")
    }
}

fn press(frames: &mut Frames, key: Key, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![input::key_press(key, Modifiers::NONE)], &mut *show);
    frames.frame(vec![craft_ui::testing::focus::key_release(key, Modifiers::NONE)], &mut *show);
}

fn focus(frames: &mut Frames, seen: &Seen, show: &mut impl FnMut(&mut Ui)) {
    request(&frames.ctx, FocusTarget::Id(seen.id()));
    frames.idle(2, &mut *show);
}

/// A collection that lays its items in rows of different lengths, the last items hidden as a closed group's are.
#[derive(Clone)]
struct Ragged {
    rows: Vec<usize>,
    hidden_from: usize,
}

impl Ragged {
    /// The row and column of item `at`.
    fn place(&self, at: usize) -> (usize, usize) {
        let mut start = 0;
        for (row, len) in self.rows.iter().enumerate() {
            if at < start + len {
                return (row, at - start);
            }
            start += len;
        }
        (self.rows.len(), 0)
    }

    fn count(&self) -> usize {
        self.rows.iter().sum()
    }
}

impl ItemLayout for Ragged {
    fn content_size(&self) -> ScrollPos {
        let widest = self.rows.iter().copied().max().unwrap_or(0);
        ScrollPos { x: f64::from(widest as f32 * TILE.x), y: f64::from(self.rows.len() as f32 * TILE.y) }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        let view = Rect::from_min_size(Pos2::ZERO, view);
        out.extend((0..self.count()).filter(|&at| self.shown(at) && self.rect(at, offset).intersects(view)));
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        if !self.shown(at) {
            return Rect::NOTHING;
        }
        let (row, column) = self.place(at);
        Rect::from_min_size(Pos2::new(column as f32 * TILE.x - offset.x as f32, row as f32 * TILE.y - offset.y as f32), TILE)
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        (0..self.count()).find(|&at| self.shown(at) && self.rect(at, offset).contains(point))
    }

    fn nav(&self) -> NavShape {
        NavShape::RaggedGrid
    }

    fn row_len(&self, row: usize) -> Option<usize> {
        self.rows.get(row).copied()
    }

    fn shown(&self, at: usize) -> bool {
        at < self.hidden_from
    }
}

/// A collection over `Ragged`, handing the digits it receives to the trial.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
struct Probe<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    layout: Ragged,
    digits: &'a RefCell<Vec<u8>>,
}

impl<'a> Probe<'a> {
    fn new(items: CollectionProps, layout: Ragged, digits: &'a RefCell<Vec<u8>>) -> Self {
        Probe { common: Common::new(), props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill), items, layout, digits }
    }
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        while let Some(digit) = cx.intents().take_digit() {
            self.digits.borrow_mut().push(digit);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Probe<'_> {
    type Key = usize;
    type ItemLayout = Ragged;
    type Selection = Selection<usize>;
    type Item<'r> = Row;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn item_layout(&self, _: &dyn ItemSource<usize>, _: f32, _: &StyleCx<'_>) -> Ragged {
        self.layout.clone()
    }
}

fn show_probe(ui: &mut Ui, props: CollectionProps, layout: &Ragged, source: &dyn ItemSource<usize>, seen: &Seen) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
        seen.take(Probe::new(props, layout.clone(), &seen.digits).id_salt("probe").show_items(ui, source, &mut |key, cx| {
            seen.note(*key, cx.rect);
            Row::new()
        }));
    });
}

/// End lands on the last item that shows, and no item of a closed group is built with an unplaced rect.
#[trial]
fn end_skips_the_items_of_a_closed_group() {
    let (seen, layout) = (Seen::default(), Ragged { rows: vec![3, 2, 3], hidden_from: 5 });
    let source = Count(8);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_probe(ui, CollectionProps::new().select(SelectMode::Single), &layout, &source, &seen);
    frames.idle(3, &mut show);
    focus(&mut frames, &seen, &mut show);
    press(&mut frames, Key::End, &mut show);
    press(&mut frames, Key::ArrowDown, &mut show);
    assert_eq!(seen.lead_keys(), vec![4], "End takes the last shown item, and Down past it stays");
    let rows = seen.rows.borrow();
    assert!(rows.iter().all(|(key, rect)| *key < 5 && rect.is_finite()), "only shown items are built: {rows:?}");
}

#[trial]
fn unused_moves_reach_the_regions_collection_in_either_declaration_order() {
    use craft_ui::contract::{Show as _, ShowContainer as _};
    use craft_ui_test_kit::{
        body::Slab,
        contract::{Probe as Button, Seen as ButtonSeen},
    };
    for collection_first in [false, true] {
        let (seen, layout) = (Seen::default(), Ragged { rows: vec![3, 2], hidden_from: 5 });
        let source = Count(5);
        let button_seen = RefCell::new(ButtonSeen::default());
        let list_id = Id::unique("fallback collection");
        let button_id = Id::unique("fallback button");
        let mut frames = Frames::new();
        let mut show = |ui: &mut Ui| {
            in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
                Slab::new(ContainerProps::new().fallback_collection(list_id)).show(ui, |ui, _| {
                    let list = |ui: &mut Ui| {
                        seen.take(
                            Probe::new(CollectionProps::new().select(SelectMode::Single), layout.clone(), &seen.digits).id(list_id).show_items(
                                ui,
                                &source,
                                &mut |_, _| Row::new(),
                            ),
                        );
                    };
                    if collection_first {
                        list(ui);
                    }
                    Button::new(&button_seen).id(button_id).show(ui);
                    if !collection_first {
                        list(ui);
                    }
                });
            });
        };
        frames.idle(3, &mut show);
        request(&frames.ctx, FocusTarget::Id(button_id));
        frames.idle(2, &mut show);
        press(&mut frames, Key::ArrowDown, &mut show);
        frames.idle(2, &mut show);
        assert_eq!(seen.lead_keys().len(), 1, "the move reaches the collection once across later passes");
    }
}

#[derive(craft_ui::contract::Component)]
struct MoveControl<'a> {
    #[common]
    common: Common,
    taken: &'a Cell<usize>,
}

impl Component for MoveControl<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: MoveControlStyleKey) -> PlainStyle {
        PlainStyle::default()
    }
    fn keys(&self) -> craft_ui::runtime::focus::KeyUse {
        craft_ui::runtime::focus::KeyUse::of(craft_behavior::Input::Pointer).with_moves()
    }
    fn measure(&mut self, _: &craft_ui::contract::MeasureCx<'_, Self>) -> Vec2 {
        TILE
    }
    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::CLICK | craft_behavior::Sense::FOCUS
    }
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        while cx.intents().take_move().is_some() {
            self.taken.set(self.taken.get() + 1);
        }
    }
    fn finish(self, cx: Cx<'_, Self>) -> Self::Output {
        cx.response.clone()
    }
}

#[trial]
fn a_focused_control_keeps_the_moves_it_uses_before_the_fallback() {
    use craft_ui::contract::{Show as _, ShowContainer as _};
    use craft_ui_test_kit::body::Slab;
    let (seen, layout) = (Seen::default(), Ragged { rows: vec![3, 2], hidden_from: 5 });
    let source = Count(5);
    let taken = Cell::new(0);
    let list_id = Id::unique("fallback first refusal list");
    let control_id = Id::unique("fallback first refusal control");
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().fallback_collection(list_id)).show(ui, |ui, _| {
                MoveControl { common: Common::new(), taken: &taken }.id(control_id).show(ui);
                seen.take(Probe::new(CollectionProps::new().select(SelectMode::Single), layout.clone(), &seen.digits).id(list_id).show_items(
                    ui,
                    &source,
                    &mut |_, _| Row::new(),
                ));
            });
        })
    };
    frames.idle(3, &mut show);
    request(&frames.ctx, FocusTarget::Id(control_id));
    frames.idle(2, &mut show);
    press(&mut frames, Key::ArrowDown, &mut show);
    frames.idle(2, &mut show);
    assert_eq!(taken.get(), 1);
    assert!(seen.lead_keys().is_empty(), "a move used by the control never reaches the fallback");
}

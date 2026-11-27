//! Collections over a free layout, where items sit at points the document stores: Shift and Cmd picks that add and
//! toggle without a range, drop kinds that take a drop at a point (`DropPlace::Spot`) and report the empty space's
//! double click at its screen point, and a move of the selection started from a part the collection declares
//! (`Collection::moves_from_part`).

use crate::collection::kit::{Count, Dismissed, List, Row, Words};
use craft_behavior::{CanvasTransform, EditEvent, FreeLayout, HitLevel, HitPriority, ItemSource, PartKey, PartRef, Selection};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, DropAt, DropKind,
    DropPlace, ListDrag, PartsCx, Pointer as _, Props as _, Role, SelectMode, ShowCollection as _,
};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A part a press on which moves the selection.
const GRIP: PartKey = PartKey::new("grip");

/// A part the board does not move from.
const KNOB: PartKey = PartKey::new("knob");

/// A part in the rows' space (`PartsCx::in_body`).
const PAD: PartKey = PartKey::new("pad");

/// Places that differ by less are the same place: the rounding of the layout.
const SAME: f32 = 0.5;

type Events = Vec<CollectionEvent<usize, Dismissed>>;

/// Items at the rects the trial gives, in the board's own space from its top left.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
struct Board<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    rects: Vec<Rect>,
    selection: Option<Bind<'a, Selection<usize>>>,
    drops: Vec<DropKind<'a, usize>>,
    /// Declares `GRIP` and `KNOB` at these rects, in the board's own space from its top left.
    grips: Option<(Rect, Rect)>,
    /// Declares `PAD` in the rows' space at this rect from the board's top left, and notes where a press on it begins.
    pad: Option<(Rect, &'a Cell<Option<Pos2>>)>,
    /// `Collection::body_is_document`.
    document: bool,
}

impl<'a> Board<'a> {
    fn new(rects: Vec<Rect>) -> Self {
        Board {
            common: Common::new(),
            props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Multi),
            rects,
            selection: None,
            drops: Vec::new(),
            grips: None,
            pad: None,
            document: false,
        }
    }

    fn with_body_part(mut self, pad: Rect, pressed: &'a Cell<Option<Pos2>>) -> Self {
        self.pad = Some((pad, pressed));
        self
    }

    fn selection(mut self, selection: Bind<'a, Selection<usize>>) -> Self {
        self.selection = Some(selection);
        self
    }

    fn takes_drop(mut self, kind: DropKind<'a, usize>) -> Self {
        self.drops.push(kind);
        self
    }

    fn transformed(mut self, transform: CanvasTransform) -> Self {
        self.props = self.props.transform(transform);
        self
    }

    fn with_parts(mut self, grip: Rect, knob: Rect) -> Self {
        self.grips = Some((grip, knob));
        self
    }
}

impl Component for Board<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: BoardStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    /// The grip and the knob sit over the empty space at `Edge` priority, as a node graph's wires do.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        if let Some((grip, knob)) = self.grips {
            for (key, at) in [(GRIP, grip), (KNOB, knob)] {
                cx.part(key, at.translate(rect.min.to_vec2()))
                    .sense(craft_behavior::Sense::DRAG)
                    .priority(HitPriority::new(HitLevel::Edge, 0))
                    .pointer_only("the part is dragged with the pointer");
            }
        }
        if let Some((pad, _)) = self.pad {
            cx.in_body(PAD, pad.translate(rect.min.to_vec2()))
                .sense(craft_behavior::Sense::CLICK)
                .pointer_only("the trial presses the part")
                .node(Role::BUTTON, "Pad");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some((_, pressed)) = self.pad
            && let Some(result) = cx.part_if_declared(PAD)
            && let craft_behavior::PressPhase::Down { start, .. } = result.press.phase
        {
            pressed.set(Some(start.origin));
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Board<'_> {
    type Key = usize;
    type ItemLayout = FreeLayout;
    type Selection = Selection<usize>;
    type Item<'r> = Row;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn drops(&self) -> &[DropKind<'_, usize>] {
        &self.drops
    }

    fn moves_from_part(&self, part: PartRef) -> bool {
        part.key == GRIP
    }

    fn body_is_document(&self) -> bool {
        self.document
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, _: f32, _: &StyleCx<'_>) -> FreeLayout {
        FreeLayout::new((0..source.len()).map(|at| self.rects.get(source.key(at)).copied().unwrap_or(Rect::NOTHING)).collect())
    }
}

/// Where each item was last shown, in its collection's own space.
#[derive(Default)]
struct Shown(RefCell<Vec<(usize, Rect)>>);

impl Shown {
    fn note(&self, key: usize, rect: Rect) {
        let mut rows = self.0.borrow_mut();
        rows.retain(|(held, _)| *held != key);
        rows.push((key, rect));
    }

    fn rect(&self, key: usize) -> Rect {
        self.0.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the item was shown")
    }
}

/// Three items apart from each other, at the board's top left, low on the right and in the middle.
fn three_items() -> Vec<Rect> {
    vec![
        Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::splat(40.0)),
        Rect::from_min_size(Pos2::new(150.0, 120.0), Vec2::splat(40.0)),
        Rect::from_min_size(Pos2::new(60.0, 60.0), Vec2::splat(40.0)),
    ]
}

fn sorted(selection: &RefCell<Selection<usize>>) -> Vec<usize> {
    let mut items = selection.borrow().items.clone();
    items.sort_unstable();
    items
}

/// S-G4 (ge.C11): over a free layout Shift adds the item and never the items between it and the anchor in display
/// order, and keeps an item already selected; Cmd toggles and so does Shift+Cmd. In a flow list beside it Shift still
/// selects the range.
#[trial]
fn shift_click_in_a_free_layout_adds_without_a_range() {
    let (board_sel, list_sel) = (Rc::new(RefCell::new(Selection::<usize>::default())), Rc::new(RefCell::new(Selection::<usize>::default())));
    let (board_at, list_at) = (Rc::new(Shown::default()), Rc::new(Shown::default()));
    let words = Rc::new(Words::flat(&["a", "b", "c"]));
    let mut fixture = Fixture::new({
        let (board_sel, list_sel, board_at, list_at, words) = (board_sel.clone(), list_sel.clone(), board_at.clone(), list_at.clone(), words.clone());
        move |ui: &mut Ui| {
            in_region(ui, Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(240.0, 200.0)), |ui| {
                let mut held = board_sel.borrow().clone();
                let _ = Board::new(three_items()).selection(Bind::bound(&mut held)).id_salt("board").show_items(ui, &Count(3), &mut |key, cx| {
                    board_at.note(*key, cx.rect);
                    Row::new()
                });
                *board_sel.borrow_mut() = held;
            });
            in_region(ui, Rect::from_min_size(Pos2::new(260.0, 0.0), Vec2::new(120.0, 100.0)), |ui| {
                let mut held = list_sel.borrow().clone();
                let _ = List::new(SelectMode::Multi).selection(Bind::bound(&mut held)).id_salt("list").show_items(ui, &*words, &mut |key, cx| {
                    list_at.note(*key, cx.rect);
                    Row::new()
                });
                *list_sel.borrow_mut() = held;
            });
        }
    });
    // The PC keys: Ctrl is the toggle key.
    craft_ui::testing::windows::set_os(fixture.ctx(), craft_host::target::Os::Linux);
    fixture.idle(3);
    let item = |key: usize| board_at.rect(key).center();
    // Each click waits past the double-click time, so none is the second click of a double click.
    let click = |fixture: &mut Fixture, at: Pos2, mods: Modifiers| {
        fixture.hold(mods).click_at(at).hold(Modifiers::NONE).advance(1.0).idle(1);
    };
    // Item 1 lies between items 0 and 2 in display order: a range from 0 to 2 would hold it.
    click(&mut fixture, item(0), Modifiers::NONE);
    assert_eq!(sorted(&board_sel), [0]);
    click(&mut fixture, item(2), Modifiers::SHIFT);
    assert_eq!(sorted(&board_sel), [0, 2], "Shift adds the item and not the item between them in display order");
    click(&mut fixture, item(2), Modifiers::SHIFT);
    assert_eq!(sorted(&board_sel), [0, 2], "Shift on a selected item keeps it");
    click(&mut fixture, item(0), Modifiers::COMMAND);
    assert_eq!(sorted(&board_sel), [2], "Cmd toggles a selected item off");
    click(&mut fixture, item(0), Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(sorted(&board_sel), [0, 2], "Shift+Cmd toggles an item on, with no range");
    click(&mut fixture, item(2), Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(sorted(&board_sel), [0], "and off");

    click(&mut fixture, list_at.rect(0).center(), Modifiers::NONE);
    click(&mut fixture, list_at.rect(2).center(), Modifiers::SHIFT);
    assert_eq!(sorted(&list_sel), [0, 1, 2], "a flow list still selects the range");
}

/// A payload another list's row drag also carries, which the board takes at a point.
#[derive(Clone, Debug, PartialEq)]
struct Photo(usize);

/// ge.C10 and ge.C9 under a body transform: a `Spot` kind reports the drop at the point in the items' own space with
/// the item under it, none over the empty space; an `Onto` kind on the same board is unchanged; a double click on the
/// empty space reports its screen point, the one a secondary click there reports.
#[trial]
fn a_drop_in_a_free_layout_reports_its_spot_and_the_empty_space_its_screen_point() {
    let (scale, offset) = (0.5, Vec2::new(30.0, 20.0));
    let words = Words::flat(&["a", "b", "c"]);
    let (photos, rows, board_at) = (Shown::default(), Shown::default(), Shown::default());
    let events = RefCell::new(Events::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(150.0, 100.0)), |ui| {
            let from = List::new(SelectMode::Single).drag_payload(|drag: &ListDrag<usize>| Some(Photo(drag.grabbed)));
            let _ = from.id_salt("photos").show_items(ui, &words, &mut |key, cx| {
                photos.note(*key, cx.rect);
                Row::new()
            });
        });
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 130.0), Vec2::new(150.0, 100.0)), |ui| {
            let _ = List::new(SelectMode::Single).reorder().id_salt("rows").show_items(ui, &words, &mut |key, cx| {
                rows.note(*key, cx.rect);
                Row::new()
            });
        });
        in_region(ui, Rect::from_min_size(Pos2::new(200.0, 10.0), Vec2::new(400.0, 300.0)), |ui| {
            let spot = DropKind::new(DropPlace::Spot, |_: &Photo, _: &DropAt<usize>| true);
            let onto = DropKind::new(DropPlace::Onto, |_: &ListDrag<usize>, _: &DropAt<usize>| true);
            let board = Board::new(three_items()).takes_drop(spot).takes_drop(onto).transformed(CanvasTransform::new(scale, offset));
            let out = board.id_salt("board").show_items(ui, &Count(3), &mut |key, cx| {
                board_at.note(*key, cx.rect);
                Row::new()
            });
            events.borrow_mut().extend(out.events);
        });
    };
    frames.idle(3, &mut show);
    // The board's own space starts at its first item; the transform scales about it and moves by `offset`.
    let origin = board_at.rect(0).min;
    let screen = |own: Pos2| origin + (own - origin) * scale + offset;
    let drop_at = |frames: &mut Frames, from: Pos2, own: Pos2, show: &mut dyn FnMut(&mut Ui)| {
        let to = screen(own);
        frames.frame(vec![moved(from)], &mut *show);
        frames.frame(vec![press(from)], &mut *show);
        for step in 1..=4 {
            frames.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))], &mut *show);
        }
        frames.idle(1, &mut *show);
        frames.frame(vec![release(to)], &mut *show);
        frames.idle(2, &mut *show);
    };
    let drops = |events: &RefCell<Events>| -> Vec<DropAt<usize>> {
        let taken = events
            .borrow()
            .iter()
            .filter_map(|event| if let CollectionEvent::Dropped { at, .. } = event { Some(at.clone()) } else { None })
            .collect();
        events.borrow_mut().clear();
        taken
    };

    let on_item = board_at.rect(1).center() + Vec2::new(5.0, -5.0);
    drop_at(&mut frames, photos.rect(0).center(), on_item, &mut show);
    let got = drops(&events);
    assert!(
        matches!(got.as_slice(), [DropAt::Spot { over: Some(1), at, .. }] if (*at - on_item).length() < SAME),
        "the drop over an item is at its point in the items' own space, over the item: {got:?}"
    );
    assert!((screen(on_item) - on_item).length() > 10.0, "the transform moves the point, so a forgotten mapping shows");

    let empty = origin + Vec2::new(300.0, 40.0);
    drop_at(&mut frames, photos.rect(1).center(), empty, &mut show);
    let got = drops(&events);
    assert!(
        matches!(got.as_slice(), [DropAt::Spot { over: None, at, .. }] if (*at - empty).length() < SAME),
        "the drop on the empty space is at its point, over no item: {got:?}"
    );

    drop_at(&mut frames, rows.rect(0).center(), board_at.rect(2).center(), &mut show);
    assert_eq!(drops(&events), [DropAt::Onto(2)], "an `Onto` kind of a free layout still drops onto the item");

    let space = screen(empty);
    frames.advance(1.0);
    for _ in 0..2 {
        frames.frame(vec![moved(space)], &mut show);
        frames.frame(vec![press(space)], &mut show);
        frames.frame(vec![release(space)], &mut show);
    }
    frames.idle(1, &mut show);
    frames.advance(1.0);
    frames.frame(vec![secondary(space, true)], &mut show);
    frames.frame(vec![secondary(space, false)], &mut show);
    frames.idle(1, &mut show);
    let events = events.borrow();
    let activated: Vec<Pos2> =
        events.iter().filter_map(|event| if let CollectionEvent::EmptyActivated { at, .. } = event { Some(*at) } else { None }).collect();
    let menu: Vec<Pos2> =
        events.iter().filter_map(|event| if let CollectionEvent::EmptyContextMenu { at, .. } = event { Some(*at) } else { None }).collect();
    assert_eq!(activated.len(), 1, "the double click on the empty space is reported once: {events:?}");
    assert!((activated[0] - space).length() < SAME, "at its screen point, not mapped into the items' space: {activated:?}");
    assert!(menu.iter().any(|at| (*at - activated[0]).length() < SAME), "the point a secondary click there reports: {menu:?}");
}

/// What the `Move` events of a run were: begins, updates, commits with their offset, and cancels.
#[derive(Debug, Default, PartialEq)]
struct Moves {
    begins: usize,
    updates: usize,
    commits: Vec<Vec2>,
    cancels: usize,
}

fn moves(events: &RefCell<Events>) -> Moves {
    let mut seen = Moves::default();
    for event in events.borrow_mut().drain(..) {
        match event {
            CollectionEvent::Move(EditEvent::Begin { .. }) => seen.begins += 1,
            CollectionEvent::Move(EditEvent::Update { .. }) => seen.updates += 1,
            CollectionEvent::Move(EditEvent::Commit { after, .. }) => seen.commits.push(after),
            CollectionEvent::Move(EditEvent::Cancel { .. }) => seen.cancels += 1,
            _ => {}
        }
    }
    seen
}

/// ge.C32 (q78): a primary drag pressed on a part the board moves from reports the selection's move, one gesture whose
/// commit carries the pointer's travel, also when a pass of the drag reruns; Escape cancels it; a drag pressed on a
/// part the board does not move from reports no move.
#[trial]
fn a_press_on_a_declared_part_moves_the_selection_when_the_collection_says_so() {
    let (grip, knob) =
        (Rect::from_min_size(Pos2::new(200.0, 20.0), Vec2::new(40.0, 20.0)), Rect::from_min_size(Pos2::new(200.0, 80.0), Vec2::new(40.0, 20.0)));
    let selection = Rc::new(RefCell::new(Selection::<usize>::default()));
    let (shown, events, discard) = (Rc::new(Shown::default()), Rc::new(RefCell::new(Events::new())), Rc::new(Cell::new(false)));
    let mut fixture = Fixture::new({
        let (selection, shown, events, discard) = (selection.clone(), shown.clone(), events.clone(), discard.clone());
        move |ui: &mut Ui| {
            // A pass the trial discards reports nothing the app keeps: its rerun is the frame's pass.
            let discarded = discard.take();
            if discarded {
                craft_ui::runtime::frame::discard_pass(ui.ctx(), "the trial reruns a pass of the drag");
            }
            let mut held = selection.borrow().clone();
            let board = Board::new(three_items()).with_parts(grip, knob).selection(Bind::bound(&mut held));
            let out = board.id_salt("board").show_items(ui, &Count(3), &mut |key, cx| {
                shown.note(*key, cx.rect);
                Row::new()
            });
            *selection.borrow_mut() = held;
            if !discarded {
                events.borrow_mut().extend(out.events);
            }
        }
    });
    fixture.idle(3);
    // The parts sit at their rects from the board's top left, where its first item sits.
    let origin = shown.rect(0).min;
    let (grip_at, knob_at) = (origin + grip.center().to_vec2(), origin + knob.center().to_vec2());
    fixture.click_at(shown.rect(0).center()).advance(1.0).idle(1);
    assert_eq!(sorted(&selection), [0]);
    moves(&events);

    let by = Vec2::new(40.0, 30.0);
    fixture.frame(vec![moved(grip_at)]).frame(vec![press(grip_at)]);
    for step in 1..=4 {
        // The third step's frame runs two passes; only its first gets the input.
        discard.set(step == 3);
        fixture.frame(vec![moved(grip_at + by * (step as f32 / 4.0))]);
    }
    fixture.frame(vec![release(grip_at + by)]).idle(1);
    let grip_drag = moves(&events);
    assert_eq!((grip_drag.begins, grip_drag.cancels), (1, 0), "one gesture: {grip_drag:?}");
    assert!(grip_drag.updates > 0, "the move follows the pointer: {grip_drag:?}");
    assert!(
        matches!(grip_drag.commits.as_slice(), [after] if (*after - by).length() < SAME),
        "the release commits the pointer's travel: {grip_drag:?}"
    );

    fixture.advance(1.0).drag_hold_at(grip_at, by);
    fixture.key(Key::Escape, Modifiers::NONE).release_at(grip_at + by);
    let cancelled = moves(&events);
    assert_eq!((cancelled.begins, cancelled.cancels), (1, 1), "Escape cancels the move: {cancelled:?}");
    assert!(cancelled.commits.is_empty(), "and nothing commits: {cancelled:?}");

    fixture.advance(1.0).drag_at(knob_at, by);
    assert_eq!(moves(&events), Moves::default(), "a part the board does not move from moves nothing");
    assert_eq!(sorted(&selection), [0], "the selection is as it was");
}

/// q151: a press-drag on an item outside the selection selects it alone, before the move begins, and moves it in the
/// same gesture; a press-drag on an item of a multiple selection moves all of it and keeps it whole.
#[trial]
fn a_drag_on_an_unselected_item_selects_and_moves_it() {
    let selection = Rc::new(RefCell::new(Selection::<usize>::default()));
    let (shown, events) = (Rc::new(Shown::default()), Rc::new(RefCell::new(Events::new())));
    let mut fixture = Fixture::new({
        let (selection, shown, events) = (selection.clone(), shown.clone(), events.clone());
        move |ui: &mut Ui| {
            let mut held = selection.borrow().clone();
            let out = Board::new(three_items()).selection(Bind::bound(&mut held)).id_salt("board").show_items(ui, &Count(3), &mut |key, cx| {
                shown.note(*key, cx.rect);
                Row::new()
            });
            *selection.borrow_mut() = held;
            events.borrow_mut().extend(out.events);
        }
    });
    fixture.idle(3);
    let item = |key: usize| shown.rect(key).center();
    fixture.click_at(item(0)).advance(1.0).idle(1);
    events.borrow_mut().clear();

    let by = Vec2::new(30.0, 20.0);
    fixture.drag_at(item(2), by).advance(1.0).idle(1);
    assert_eq!(sorted(&selection), [2], "the drag selects the item it started on, in place of the selection");
    let picked = events.borrow().iter().position(|event| matches!(event, CollectionEvent::Selection { .. }));
    let began = events.borrow().iter().position(|event| matches!(event, CollectionEvent::Move(EditEvent::Begin { .. })));
    assert!(
        matches!((picked, began), (Some(picked), Some(began)) if picked < began),
        "the selection changes before the move begins, so the move carries the item"
    );
    let dragged = moves(&events);
    assert!(matches!(dragged.commits.as_slice(), [after] if (*after - by).length() < SAME), "one move of the pointer's travel: {dragged:?}");

    fixture.hold(Modifiers::SHIFT).click_at(item(0)).hold(Modifiers::NONE).advance(1.0).idle(1);
    assert_eq!(sorted(&selection), [0, 2]);
    events.borrow_mut().clear();
    fixture.drag_at(item(0), by).advance(1.0).idle(1);
    assert_eq!(sorted(&selection), [0, 2], "a drag on a selected item keeps the selection whole");
    assert_eq!(moves(&events).commits.len(), 1, "and moves it");
}

/// q131: a collection's body part (`PartsCx::in_body`) lies in its rows' space. A press finds it where the body's
/// transform shows it, after a pan and after a zoom, and not where it would sit untransformed, and the press reads in
/// the rows' space.
#[trial]
fn a_body_part_is_hit_where_the_transform_shows_it_after_a_pan_and_a_zoom() {
    // Near the origin, so each transform below still shows it inside the board, which clips its body.
    let pad = Rect::from_min_size(Pos2::new(100.0, 20.0), Vec2::new(40.0, 20.0));
    let transform = Rc::new(Cell::new(CanvasTransform::new(2.0, Vec2::new(30.0, 20.0))));
    let (shown, pressed) = (Rc::new(Shown::default()), Rc::new(Cell::new(None::<Pos2>)));
    let mut fixture = Fixture::new({
        let (transform, shown, pressed) = (transform.clone(), shown.clone(), pressed.clone());
        move |ui: &mut Ui| {
            let board = Board::new(three_items()).with_body_part(pad, &pressed).transformed(transform.get());
            let _ = board.id_salt("board").show_items(ui, &Count(3), &mut |key, cx| {
                shown.note(*key, cx.rect);
                Row::new()
            });
        }
    });
    fixture.idle(3);
    // The rows' space starts at the first item; the transform scales about it and moves by its offset.
    let origin = shown.rect(0).min;
    let (own, inside) = (origin + pad.center().to_vec2(), pad.translate(origin.to_vec2()));
    let panned = CanvasTransform::new(2.0, Vec2::new(-40.0, 35.0));
    for (at, what) in [(transform.get(), "as shown"), (panned, "after a pan"), (CanvasTransform::new(0.75, panned.offset), "after a zoom")] {
        transform.set(at);
        fixture.advance(1.0).idle(2);
        let screen = origin + (own - origin) * at.scale + at.offset;
        assert!((screen - own).length() > 10.0, "{what}: the transform moves the part, so a forgotten mapping shows");
        pressed.set(None);
        fixture.click_at(own).advance(1.0).idle(1);
        assert_eq!(pressed.get(), None, "{what}: no press where the part would sit untransformed");
        fixture.click_at(screen).advance(1.0).idle(1);
        let got = pressed.get();
        assert!(got.is_some_and(|got| inside.contains(got)), "{what}: the press takes the part, read in the rows' space: {got:?}");
    }
}

/// A press on a collection's body part over its empty space (a graph's wire) moves key focus to the collection, as a
/// press on an item does, so the keys that act on what the part's press picked reach it.
#[trial]
fn a_press_on_a_body_part_focuses_the_collection() {
    let pad = Rect::from_min_size(Pos2::new(100.0, 20.0), Vec2::new(40.0, 20.0));
    let (shown, pressed) = (Rc::new(Shown::default()), Rc::new(Cell::new(None::<Pos2>)));
    let mut fixture = Fixture::new({
        let (shown, pressed) = (shown.clone(), pressed.clone());
        move |ui: &mut Ui| {
            let _ = Board::new(three_items()).with_body_part(pad, &pressed).id_salt("board").show_items(ui, &Count(3), &mut |key, cx| {
                shown.note(*key, cx.rect);
                Row::new()
            });
        }
    });
    fixture.idle(3);
    assert_eq!(fixture.focused_stop(), None, "nothing has focus before the press");
    fixture.click_at(shown.rect(0).min + pad.center().to_vec2()).idle(1);
    assert!(pressed.get().is_some(), "the press takes the part");
    assert!(fixture.focused_stop().is_some(), "the press on the part focuses the board");
}

/// A board with no items and its pad at the start of its body, which says whether it is document content.
fn pad_board(ui: &mut Ui, document: bool) -> craft_ui::testing::story::StoryOutput {
    let pressed = Cell::new(None);
    let mut board = Board::new(Vec::new()).with_body_part(Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 20.0)), &pressed);
    board.document = document;
    board.id_salt("board").show_items(ui, &Count(0), &mut |_, _| Row::new()).into()
}

/// q135: a body part keeps its place under right-to-left, as a free layout's items do. The suite's right-to-left
/// case takes it as document content when the collection says its body is (`Collection::body_is_document`), and
/// fails the same part when the collection does not.
#[trial]
fn the_rtl_case_leaves_a_document_body_part_where_it_is() {
    use craft_ui::testing::story::Story;
    use craft_ui::testing::suite::rtl_case;
    let failed = rtl_case(&Story::new("mirrored-body", |ui, _| pad_board(ui, false))).expect_err("the pad does not mirror");
    assert!(failed.contains("not its reflection"), "{failed}");
    rtl_case(&Story::new("document-body", |ui, _| pad_board(ui, true))).expect("a document body part keeps its place");
}

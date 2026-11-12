//! The list trials: wheel chaining, keys and selection, and large sources.

use super::kit::{Count, Dismissed, List, ROW_H, Row, Words};
use craft_behavior::{Selection, SelectionChange};
use craft_ui::contract::{Bind, CollectionEvent, CollectionResponse, Props as _, SelectMode, Show as _, ShowCollection as _};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, input, pointer, trial};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const ROWS: usize = 40;
const SAME: f32 = 0.5;
const WHEEL: f32 = 60.0;

type Out = CollectionResponse<usize, Dismissed>;

/// What a frame of a list reported.
struct Seen {
    id: Cell<Option<Id>>,
    rect: Cell<Rect>,
    visible: Cell<(usize, usize)>,
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
}

impl Default for Seen {
    fn default() -> Self {
        Seen { id: Cell::new(None), rect: Cell::new(Rect::NOTHING), visible: Cell::new((0, 0)), events: RefCell::new(Vec::new()) }
    }
}

impl Seen {
    fn range(&self) -> std::ops::Range<usize> {
        self.visible.get().0..self.visible.get().1
    }

    fn take(&self, out: Out) {
        self.id.set(Some(out.response.id));
        self.rect.set(out.response.rect);
        self.visible.set((out.visible.start, out.visible.end));
        self.events.borrow_mut().extend(out.events);
    }

    fn id(&self) -> Id {
        self.id.get().expect("the list ran")
    }

    fn leads(&self) -> Vec<usize> {
        self.events.borrow().iter().filter_map(|event| if let CollectionEvent::LeadMoved { key, .. } = event { Some(*key) } else { None }).collect()
    }

    fn selections(&self) -> Vec<SelectionChange<usize>> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| if let CollectionEvent::Selection(change) = event { Some(change.clone()) } else { None })
            .collect()
    }
}

fn labels() -> Vec<&'static str> {
    vec!["apple", "apricot", "banana", "blueberry", "cherry", "date"]
}

fn show_words(ui: &mut Ui, list: List<'_>, words: &Words, seen: &Seen) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
        seen.take(list.id_salt("list").show_items(ui, words, &mut |_, _| Row::new()));
    });
}

fn press(frames: &mut Frames, key: Key, mods: Modifiers, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![input::key_press(key, mods)], &mut *show);
    frames.frame(vec![craft_ui::testing::focus::key_release(key, mods)], &mut *show);
}

fn focus_list(frames: &mut Frames, seen: &Seen, show: &mut impl FnMut(&mut Ui)) {
    request(&frames.ctx, FocusTarget::Id(seen.id()));
    frames.idle(2, &mut *show);
}

/// A list inside a scrolling panel takes the wheel while it can scroll and hands it to the panel at its end.
#[trial]
fn a_list_at_its_end_hands_the_wheel_to_the_enclosing_panel() {
    use craft_ui::contract::{ContainerProps, Scroll, ShowContainer as _};
    use craft_ui::testing::layout::Block;
    use craft_ui_test_kit::body::Slab;
    let words = Count(ROWS);
    let seen = Seen::default();
    let first_top = Cell::new(0.0f32);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                for line in 0..ROWS {
                    if line == 0 {
                        first_top.set(ui.next_widget_position().y);
                    }
                    Block::new(Vec2::new(VIEW.x, ROW_H)).show(ui);
                }
            });
        });
        in_region(ui, Rect::from_min_size(ORIGIN + Vec2::new(120.0, 10.0), Vec2::new(150.0, 100.0)), |ui| {
            seen.take(List::new(SelectMode::None).id_salt("list").show_items(ui, &words, &mut |_, _| Row::new()));
        });
    };
    frames.idle(3, &mut show);
    let (list, top0) = (seen.rect.get(), first_top.get());
    frames.frame(vec![pointer::moved(list.center())], &mut show);
    frames.frame(vec![pointer::wheel(-WHEEL)], &mut show);
    assert!((first_top.get() - top0).abs() < SAME, "the list takes the wheel while it has room");
    for _ in 0..3 {
        frames.frame(vec![pointer::wheel(-100_000.0)], &mut show);
    }
    assert_eq!(seen.range().end, ROWS, "the list reached its last row");
    frames.frame(vec![pointer::wheel(-WHEEL)], &mut show);
    assert!(top0 - first_top.get() > SAME, "at its end the list hands the wheel to the panel: {top0} -> {}", first_top.get());
}

/// A list with a row range is as tall as its rows between the least and the most it shows; past the most it scrolls
/// them, inside a scrolling panel too.
#[trial]
fn a_row_range_bounds_the_list_and_it_scrolls_past_the_cap() {
    use craft_ui::contract::{ContainerProps, Scroll, ShowContainer as _};
    use craft_ui_test_kit::body::Slab;
    let cases: [(usize, u16, Option<u16>); 5] = [(ROWS, 0, Some(3)), (3, 0, Some(3)), (2, 0, Some(3)), (0, 0, None), (0, 2, None)];
    let seen: Vec<Seen> = cases.iter().map(|_| Seen::default()).collect();
    let empty: Vec<Cell<Rect>> = cases.iter().map(|_| Cell::new(Rect::NOTHING)).collect();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, Vec2::new(VIEW.x, 1000.0)), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                for (at, (rows, min, max)) in cases.iter().enumerate() {
                    let list = List::new(SelectMode::None).rows(*min, *max).on_empty(&empty[at]).id_salt(("list", at));
                    seen[at].take(list.show_items(ui, &Count(*rows), &mut |_, _| Row::new()));
                }
            });
        });
    };
    frames.idle(3, &mut show);
    let height = |at: usize| seen[at].rect.get().height();
    assert!((height(0) - height(1)).abs() < SAME, "past the cap the list is as tall as its first rows: {} vs {}", height(0), height(1));
    assert!(height(2) < height(1) - SAME, "under the cap the list is as tall as its rows");
    let (least, plain) = (empty[4].get().height(), empty[3].get().height());
    assert!(least > plain + SAME, "the least rows keep an empty list taller than its empty state: {least} vs {plain}");
    let first = seen[0].range().start;
    frames.frame(vec![pointer::moved(seen[0].rect.get().center())], &mut show);
    frames.frame(vec![pointer::wheel(-WHEEL)], &mut show);
    assert!(seen[0].range().start > first, "the capped list scrolls its rows under the wheel");
}

/// One tab stop; the lead moves by arrows, Home and End, Page and type-ahead; selection by Space, Shift and Ctrl.
#[trial]
fn a_list_with_one_tab_stop_moves_its_lead_and_selects() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let selection = RefCell::new(Selection::<usize>::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        let list = List::new(SelectMode::Multi).type_ahead().selection(Bind::bound(&mut held));
        show_words(ui, list, &words, &seen);
        *selection.borrow_mut() = held;
    };
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    let last = |seen: &Seen| *seen.leads().last().expect("the lead moved");

    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    assert_eq!(last(&seen), 1);
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(seen.id()), "the rows take no focus: the list is the one stop");
    press(&mut frames, Key::End, Modifiers::NONE, &mut show);
    assert_eq!(last(&seen), 5);
    press(&mut frames, Key::Home, Modifiers::NONE, &mut show);
    assert_eq!(last(&seen), 0);
    press(&mut frames, Key::PageDown, Modifiers::NONE, &mut show);
    assert!(last(&seen) > 1, "a page moves several rows: {:?}", seen.leads());
    frames.frame(vec![Event::Text("c".into())], &mut show);
    assert_eq!(last(&seen), 4, "type-ahead picks the word starting with the letter");

    assert_eq!(selection.borrow().items, vec![4], "arrows, Home, End, Page and type-ahead move the selection with the lead");
    press(&mut frames, Key::ArrowUp, Modifiers::COMMAND, &mut show);
    assert_eq!((selection.borrow().items.clone(), last(&seen)), (vec![4], 3), "Ctrl moves only the lead");
    press(&mut frames, Key::Space, Modifiers::NONE, &mut show);
    let mut items = selection.borrow().items.clone();
    items.sort_unstable();
    assert_eq!(items, vec![3, 4], "Space toggles the lead");
    press(&mut frames, Key::ArrowUp, Modifiers::SHIFT, &mut show);
    press(&mut frames, Key::ArrowUp, Modifiers::SHIFT, &mut show);
    let mut items = selection.borrow().items.clone();
    items.sort_unstable();
    assert_eq!(items, vec![1, 2, 3], "Shift extends from the anchor");
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    assert_eq!((selection.borrow().items.clone(), last(&seen)), (vec![2], 2), "a plain arrow replaces the range");
    assert!(seen.selections().len() >= 3);
}

/// Where the arrows move the lead only, an arrow leaves the selection and Enter selects the lead and activates it.
#[trial]
fn enter_picks_the_lead_where_arrows_leave_the_selection() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let selection = RefCell::new(Selection::<usize>::single(0));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        show_words(ui, List::new(SelectMode::Single).arrows_move_only().selection(Bind::bound(&mut held)), &words, &seen);
        *selection.borrow_mut() = held;
    };
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    assert_eq!(selection.borrow().items, vec![0], "the arrows leave the selection");
    let lead = *seen.leads().last().expect("the lead moved");
    seen.events.borrow_mut().clear();
    press(&mut frames, Key::Enter, Modifiers::NONE, &mut show);
    assert_eq!(selection.borrow().items, vec![lead], "Enter selects the lead");
    let activated = seen.events.borrow().iter().any(|event| matches!(event, CollectionEvent::Activate(key) if *key == lead));
    assert!(activated, "Enter activates the lead");
}

/// Where Space previews, Space reports the lead's preview and leaves the selection; Ctrl+Space toggles the lead's pick.
#[trial]
fn space_previews_and_ctrl_space_toggles_the_lead() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let selection = RefCell::new(Selection::<usize>::single(0));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        let list = List::new(SelectMode::Multi).space(craft_ui::contract::SpaceAction::Preview).selection(Bind::bound(&mut held));
        show_words(ui, list, &words, &seen);
        *selection.borrow_mut() = held;
    };
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::COMMAND, &mut show);
    let lead = *seen.leads().last().expect("the lead moved");
    seen.events.borrow_mut().clear();
    press(&mut frames, Key::Space, Modifiers::NONE, &mut show);
    let previewed = seen.events.borrow().iter().any(|event| matches!(event, CollectionEvent::Preview(key) if *key == lead));
    assert!(previewed, "Space previews the lead");
    assert_eq!(selection.borrow().items, vec![0], "Space leaves the selection");
    press(&mut frames, Key::Space, Modifiers::CTRL, &mut show);
    let mut items = selection.borrow().items.clone();
    items.sort_unstable();
    assert_eq!(items, vec![0, lead], "Ctrl+Space adds the lead");
    press(&mut frames, Key::Space, Modifiers::CTRL, &mut show);
    assert_eq!(selection.borrow().items, vec![0], "Ctrl+Space again takes it out");
}

/// In a list that wraps, Down on the last row goes to the first and Up on the first to the last.
#[trial]
fn arrows_wrap_at_the_ends_of_a_wrapping_list() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_words(ui, List::new(SelectMode::Single).wrap(), &words, &seen);
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::End, Modifiers::NONE, &mut show);
    let last = *seen.leads().last().expect("the lead moved");
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    assert_eq!(seen.leads().last(), Some(&0), "Down on the last row wraps to the first");
    press(&mut frames, Key::ArrowUp, Modifiers::NONE, &mut show);
    assert_eq!(seen.leads().last(), Some(&last), "Up on the first row wraps to the last");
}

/// A list without type-ahead leaves typed letters to the app's tool keys: the lead stays where it is.
#[trial]
fn a_list_without_type_ahead_leaves_typed_letters_alone() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_words(ui, List::new(SelectMode::Single), &words, &seen);
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    let before = seen.leads();
    frames.frame(vec![Event::Text("c".into())], &mut show);
    frames.idle(1, &mut show);
    assert_eq!(seen.leads(), before, "a typed letter moves no lead");
}

/// The stacking keys move the selected rows as a block: Cmd+] one place toward the top, Shift+Cmd+[ to the bottom.
/// Alt+Down is no reorder key.
#[trial]
fn stacking_keys_move_the_selected_rows() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let selection = RefCell::new(Selection::<usize>::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        let list = List::new(SelectMode::Multi).reorder().selection(Bind::bound(&mut held));
        show_words(ui, list, &words, &seen);
        *selection.borrow_mut() = held;
    };
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::SHIFT, &mut show);
    let moves = |seen: &Seen| -> Vec<(Vec<usize>, usize)> {
        let events: Vec<_> = seen.events.borrow_mut().drain(..).collect();
        events
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Reorder(edit) => edit.value().map(|mv| (mv.keys.clone(), mv.to)),
                _ => None,
            })
            .collect()
    };
    moves(&seen);
    press(&mut frames, Key::ArrowDown, Modifiers::ALT, &mut show);
    assert_eq!(moves(&seen), Vec::new(), "Alt+Down moves nothing");
    press(&mut frames, Key::CloseBracket, Modifiers::COMMAND, &mut show);
    assert_eq!(moves(&seen), vec![(vec![2, 3], 1)], "Cmd+] moves both selected rows one place up");
    press(&mut frames, Key::OpenBracket, Modifiers::COMMAND | Modifiers::SHIFT, &mut show);
    assert_eq!(moves(&seen), vec![(vec![2, 3], 4)], "Shift+Cmd+[ moves them to the bottom");
}

/// A million rows: the last row places and hits exactly through the runtime.
#[trial]
fn a_million_row_layout_places_and_hits_the_last_row_exactly() {
    const COUNT: usize = 1_048_576;
    let words = Count(COUNT);
    let (seen, last) = (Seen::default(), Cell::new(Rect::NOTHING));
    let last = &*Box::leak(Box::new(last));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = List::new(SelectMode::Single).id_salt("list").show_items(ui, &words, &mut |key, cx| {
                if *key == COUNT - 1 {
                    last.set(cx.rect);
                }
                Row::new()
            });
            seen.take(list);
        });
    };
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::End, Modifiers::NONE, &mut show);
    frames.idle(2, &mut show);
    assert_eq!(seen.range().end, COUNT);
    let view = seen.rect.get();
    assert_eq!(last.get().bottom(), view.bottom(), "the last row's edge is exact: {:?} in {view:?}", last.get());
    let click = last.get().center();
    pointer::click(&mut frames, click, &mut show);
    assert!(seen.leads().contains(&(COUNT - 1)), "a press at the last row's centre hits it");
}

/// A press the focused list takes as a move or a stacking move is gone from egui's input, where the app reads its
/// own shortcuts: one press, one action. A chord the list does not take still reaches the app.
#[trial]
fn keys_the_list_takes_do_not_reach_the_app() {
    let words = Words::flat(&labels());
    let seen = Seen::default();
    let selection = RefCell::new(Selection::<usize>::default());
    // Every key the app could read in a frame, before the list runs: a press event or a key held down.
    let app_saw = RefCell::new(Vec::<Key>::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        ui.input(|input| {
            let mut saw = app_saw.borrow_mut();
            saw.extend(input.events.iter().filter_map(|event| match event {
                Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            }));
            saw.extend(input.keys_down.iter().copied());
        });
        let mut held = selection.borrow().clone();
        let list = List::new(SelectMode::Multi).reorder().selection(Bind::bound(&mut held));
        show_words(ui, list, &words, &seen);
        *selection.borrow_mut() = held;
    };
    frames.idle(2, &mut show);
    focus_list(&mut frames, &seen, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    let reorders = |seen: &Seen| seen.events.borrow_mut().drain(..).filter(|event| matches!(event, CollectionEvent::Reorder(_))).count();
    reorders(&seen);
    assert!(selection.borrow().contains(&1), "the arrows moved the selection");
    assert!(!app_saw.borrow().contains(&Key::ArrowDown), "the app reads no arrow the list took");
    press(&mut frames, Key::CloseBracket, Modifiers::COMMAND, &mut show);
    assert_eq!(reorders(&seen), 1, "Cmd+] moves the row once");
    assert!(!app_saw.borrow().contains(&Key::CloseBracket), "the app reads no Cmd+] the list took");
    press(&mut frames, Key::E, Modifiers::COMMAND, &mut show);
    assert!(app_saw.borrow().contains(&Key::E), "a chord the list leaves reaches the app");
}

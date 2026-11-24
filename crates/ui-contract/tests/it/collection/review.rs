//! The item step under the layers around it: a multiple selection dragged as one, a row outside it dragged alone, and
//! a press that shares its frame with a wheel scroll.

use super::kit::{Count, Dismissed, List, ROW_H, Row};
use craft_behavior::Selection;
use craft_ui::contract::{Bind, CollectionEvent, CollectionResponse, Props as _, SelectMode, ShowCollection as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);

/// What a list reported and where its rows were.
#[derive(Default)]
struct Run {
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
}

impl Run {
    fn take(&self, out: CollectionResponse<usize, Dismissed>) {
        self.events.borrow_mut().extend(out.events);
    }

    fn note(&self, key: usize, rect: Rect) {
        let mut rows = self.rows.borrow_mut();
        rows.retain(|(held, _)| *held != key);
        rows.push((key, rect));
    }

    fn row(&self, key: usize) -> Rect {
        self.rows.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    fn leads(&self) -> Vec<usize> {
        self.events.borrow().iter().filter_map(|event| if let CollectionEvent::LeadMoved { key, .. } = event { Some(*key) } else { None }).collect()
    }
}

/// Dragging one row of a selected range moves the whole range, and the event names every moved row.
#[trial]
fn dragging_a_selected_range_reorders_every_row_of_it() {
    let words = Count(8);
    let (run, selection) = (Run::default(), RefCell::new(Selection::new(vec![1, 2], Some(2), Some(1))));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let mut held = selection.borrow().clone();
            let out =
                List::new(SelectMode::Multi).reorder().selection(Bind::bound(&mut held)).id_salt("list").show_items(ui, &words, &mut |key, cx| {
                    run.note(*key, cx.rect);
                    Row::new()
                });
            *selection.borrow_mut() = held;
            run.take(out);
        });
    };
    frames.idle(3, &mut show);
    let (from, below) = (run.row(1).center(), run.row(4));
    let target = Pos2::new(from.x, below.max.y - 1.0);
    frames.frame(vec![pointer::moved(from)], &mut show);
    frames.frame(vec![pointer::press(from)], &mut show);
    for step in 1..=4 {
        frames.frame(vec![pointer::moved(from + (target - from) * (step as f32 / 4.0))], &mut show);
    }
    frames.frame(vec![pointer::release(target)], &mut show);
    frames.idle(2, &mut show);
    let moves: Vec<(Vec<usize>, usize)> = run
        .events
        .borrow()
        .iter()
        .filter_map(|event| if let CollectionEvent::Reorder(edit) = event { edit.value().map(|moved| (moved.keys.clone(), moved.to)) } else { None })
        .collect();
    assert_eq!(moves, [(vec![1, 2], 3)], "rows 1 and 2 land together before the row that followed row 4");
    assert_eq!(selection.borrow().items, [1, 2], "the press on a selected row kept the range for the drag");
}

/// A plain press on a row outside the selection picks it only when it ends as a click: a drag of the row carries it
/// alone and leaves the selection as it was, as the old apps picked a row on the click's release.
#[trial]
fn dragging_an_unselected_row_moves_it_alone_and_a_click_on_it_picks_it_at_the_release() {
    let words = Count(8);
    let (run, selection) = (Run::default(), RefCell::new(Selection::new(vec![1, 2], Some(2), Some(1))));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let mut held = selection.borrow().clone();
            let out =
                List::new(SelectMode::Multi).reorder().selection(Bind::bound(&mut held)).id_salt("list").show_items(ui, &words, &mut |key, cx| {
                    run.note(*key, cx.rect);
                    Row::new()
                });
            *selection.borrow_mut() = held;
            run.take(out);
        });
    };
    frames.idle(3, &mut show);
    let (from, below) = (run.row(5).center(), run.row(6));
    let target = Pos2::new(from.x, below.max.y - 1.0);
    frames.frame(vec![pointer::moved(from)], &mut show);
    frames.frame(vec![pointer::press(from)], &mut show);
    assert_eq!(selection.borrow().items, [1, 2], "the press leaves the selection");
    for step in 1..=4 {
        frames.frame(vec![pointer::moved(from + (target - from) * (step as f32 / 4.0))], &mut show);
    }
    frames.frame(vec![pointer::release(target)], &mut show);
    frames.idle(2, &mut show);
    let moves: Vec<Vec<usize>> = run
        .events
        .borrow()
        .iter()
        .filter_map(|event| if let CollectionEvent::Reorder(edit) = event { edit.value().map(|moved| moved.keys.clone()) } else { None })
        .collect();
    assert_eq!(moves, [vec![5]], "the drag moves the pressed row alone");
    assert_eq!(selection.borrow().items, [1, 2], "the drag leaves the selection");

    let at = run.row(0).center();
    frames.frame(vec![pointer::moved(at)], &mut show);
    frames.frame(vec![pointer::press(at)], &mut show);
    assert_eq!(selection.borrow().items, [1, 2], "the press of a click leaves the selection");
    frames.frame(vec![pointer::release(at)], &mut show);
    frames.idle(1, &mut show);
    assert_eq!(selection.borrow().items, [0], "the click's release picks the row");
}

/// A press in the frame of a wheel scroll lands on the row the user saw under the pointer.
#[trial]
fn a_press_in_the_frame_of_a_wheel_scroll_hits_the_row_the_user_saw() {
    let words = Count(40);
    let run = Run::default();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            run.take(List::new(SelectMode::Single).id_salt("list").show_items(ui, &words, &mut |_, _| Row::new()));
        });
    };
    frames.idle(3, &mut show);
    let at = ORIGIN + Vec2::new(30.0, ROW_H * 2.5);
    frames.frame(vec![pointer::moved(at)], &mut show);
    frames.frame(vec![pointer::wheel(-60.0), pointer::press(at)], &mut show);
    assert_eq!(run.leads().last(), Some(&2), "the press hit row 2 as it stood before the scroll");
}

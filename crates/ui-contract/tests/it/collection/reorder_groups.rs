//! Reordering into the groups of a tree: where a drop lands below an open group, and that a group does not drop
//! inside itself.

use super::kit::{Dismissed, List, ROW_H, Row, Words};
use craft_ui::contract::{CollectionEvent, CollectionResponse, Props as _, SelectMode, ShowCollection as _};
use craft_ui::runtime::drag;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(300.0, 200.0);
/// Far enough from the leading edge to read as the child indent, and near enough to read as the group's own.
const DEEP: f32 = 60.0;
const SHALLOW: f32 = 2.0;

/// `a`, an open group `G` holding `c` and `d`, `e`, and an open group `H` with no rows.
fn grove() -> Words {
    let mut words = Words::flat(&["a", "G", "c", "d", "e", "H"]);
    words.tree = vec![(0, false), (0, true), (1, false), (1, false), (0, false), (0, true)];
    words.open = RefCell::new(vec![false, true, false, false, false, true]);
    words
}

#[derive(Default)]
struct Run {
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
}

impl Run {
    fn row(&self, key: usize) -> Rect {
        self.rows.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    fn take(&self, out: CollectionResponse<usize, Dismissed>) {
        self.events.borrow_mut().extend(out.events);
    }

    /// Every reorder reported: the moved keys, the position and the group.
    fn moves(&self) -> Vec<(Vec<usize>, usize, Option<usize>)> {
        self.events
            .borrow()
            .iter()
            .filter_map(
                |event| if let CollectionEvent::Reorder(edit) = event { edit.value().map(|m| (m.keys.clone(), m.to, m.parent)) } else { None },
            )
            .collect()
    }
}

fn show_grove(ui: &mut Ui, list: List<'_>, words: &Words, run: &Run) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
        let out = list.id_salt("list").show_items(ui, words, &mut |key, cx| {
            let mut rows = run.rows.borrow_mut();
            rows.retain(|(held, _)| held != key);
            rows.push((*key, cx.rect));
            Row::new()
        });
        run.take(out);
    });
}

/// Presses on row `from`, drags to `to` and holds there; the caller reads the mark and releases.
fn drag_to(frames: &mut Frames, run: &Run, from: usize, to: Pos2, show: &mut impl FnMut(&mut Ui)) -> Pos2 {
    let start = run.row(from).center();
    frames.frame(vec![moved(start)], &mut *show);
    frames.frame(vec![press(start)], &mut *show);
    for step in 1..=4 {
        frames.frame(vec![moved(start + (to - start) * (step as f32 / 4.0))], &mut *show);
    }
    frames.idle(1, &mut *show);
    to
}

fn release_at(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![release(at)], &mut *show);
    frames.idle(2, &mut *show);
}

/// The drop point `dy` below the top of row `key`, `dx` right of its leading edge.
fn point(run: &Run, key: usize, dx: f32, dy: f32) -> Pos2 {
    let row = run.row(key);
    Pos2::new(row.min.x + dx, row.min.y + dy)
}

/// Below the last row of an open group, the pointer's indent picks the group or the level above it.
#[trial]
fn a_drop_below_the_last_child_lands_at_the_depth_the_pointer_is_at() {
    let (words, run) = (grove(), Run::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_grove(ui, List::new(SelectMode::Single).tree().reorder_into_groups(), &words, &run);
    frames.idle(3, &mut show);
    for (dx, group) in [(DEEP, Some(1)), (SHALLOW, None)] {
        let at = point(&run, 3, dx, ROW_H - 2.0);
        drag_to(&mut frames, &run, 0, at, &mut show);
        release_at(&mut frames, at, &mut show);
        assert_eq!(run.moves().last(), Some(&(vec![0], 3, group)), "dropped at indent {dx}");
    }
}

/// A group cannot be dropped inside itself.
#[trial]
fn a_group_does_not_drop_inside_itself() {
    let (words, run) = (grove(), Run::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_grove(ui, List::new(SelectMode::Single).tree().reorder_into_groups(), &words, &run);
    frames.idle(3, &mut show);
    let at = point(&run, 2, DEEP, ROW_H / 2.0);
    drag_to(&mut frames, &run, 1, at, &mut show);
    assert!(drag::mark(&frames.ctx).is_none(), "no drop spot inside the dragged group's rows");
    release_at(&mut frames, at, &mut show);
    assert!(run.moves().is_empty());
}

//! Drop kinds: a list takes a drag that started elsewhere, of its declared payload type, onto a row or into a gap.

use super::kit::{Dismissed, List, Row, Words};
use craft_ui::contract::{
    CollectionEvent, CollectionResponse, DropAt, DropKind, DropMark, DropPlace, ListDrag, PendingDrop, Props as _, SelectMode, ShowCollection as _,
};
use craft_ui::runtime::drag;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const SIZE: Vec2 = Vec2::new(150.0, 100.0);

/// A payload of another type than the rows' own drag: what the source list's row drag also carries.
#[derive(Clone, Debug, PartialEq)]
struct Photo(usize);

#[derive(Default)]
struct Run {
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
    pending: RefCell<Option<PendingDrop<usize>>>,
}

impl Run {
    fn row(&self, key: usize) -> Rect {
        self.rows.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    fn take(&self, out: CollectionResponse<usize, Dismissed>) {
        self.events.borrow_mut().extend(out.events);
        *self.pending.borrow_mut() = out.pending_drop;
    }

    /// Every drop reported: the place and the payload's `P`.
    fn drops<P: Clone + 'static>(&self) -> Vec<(DropAt<usize>, Option<P>)> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Dropped { at, payload, .. } => Some((at.clone(), payload.get::<P>().cloned())),
                _ => None,
            })
            .collect()
    }
}

fn show(ui: &mut Ui, list: List<'_>, salt: &str, at: Pos2, words: &Words, run: &Run) {
    in_region(ui, Rect::from_min_size(at, SIZE), |ui| {
        let out = list.id_salt(salt).show_items(ui, words, &mut |key, cx| {
            let mut rows = run.rows.borrow_mut();
            rows.retain(|(held, _)| held != key);
            rows.push((*key, cx.rect));
            Row::new()
        });
        run.take(out);
    });
}

/// Presses at `from`, drags to `to` in steps and holds there.
fn drag_to(frames: &mut Frames, from: Pos2, to: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![moved(from)], &mut *show);
    frames.frame(vec![press(from)], &mut *show);
    for step in 1..=4 {
        frames.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))], &mut *show);
    }
    frames.idle(1, &mut *show);
}

fn hold_at(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![moved(at)], &mut *show);
    frames.idle(1, &mut *show);
}

fn release_at(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![release(at)], &mut *show);
    frames.idle(2, &mut *show);
}

/// A row drag that also carries a `Photo` drops onto a row of another list whose kind takes photos there: the row
/// shows the `Into` mark and is the pending drop, a row the kind refuses shows nothing, and the release reports the
/// row and the photo.
#[trial]
fn a_payload_of_another_type_drops_onto_a_row_the_kind_takes() {
    let words = Words::flat(&["a", "b", "c", "d"]);
    let (source, target) = (Run::default(), Run::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let from = List::new(SelectMode::Single).drag_payload(|drag: &ListDrag<usize>| Some(Photo(drag.grabbed)));
        show(ui, from, "source", Pos2::new(10.0, 10.0), &words, &source);
        let onto = DropKind::new(DropPlace::Onto, |_: &Photo, at: &DropAt<usize>| *at != DropAt::Onto(1));
        show(ui, List::new(SelectMode::Single).takes_drop(onto), "target", Pos2::new(200.0, 10.0), &words, &target);
    };
    frames.idle(3, &mut show);
    let row = target.row(2);
    drag_to(&mut frames, source.row(0).center(), row.center(), &mut show);
    assert!(matches!(drag::mark(&frames.ctx), Some(DropMark::Into(rect)) if rect.contains(row.center())), "the row under the pointer is outlined");
    assert_eq!(*target.pending.borrow(), Some(PendingDrop::Drop(DropAt::Onto(2))));
    hold_at(&mut frames, target.row(1).center(), &mut show);
    assert!(drag::mark(&frames.ctx).is_none(), "a row the kind refuses shows no mark");
    assert_eq!(*target.pending.borrow(), None);
    hold_at(&mut frames, row.center(), &mut show);
    release_at(&mut frames, row.center(), &mut show);
    assert_eq!(target.drops::<Photo>(), vec![(DropAt::Onto(2), Some(Photo(0)))]);
    assert!(source.drops::<Photo>().is_empty(), "the source takes no drop of its own drag");
}

/// Rows dragged from one list drop into a gap of a list whose kind takes them between rows; a list without a kind
/// takes nothing.
#[trial]
fn rows_from_another_list_drop_into_a_gap_only_where_a_kind_takes_them() {
    let words = Words::flat(&["a", "b", "c", "d"]);
    let (source, target, other) = (Run::default(), Run::default(), Run::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        show(ui, List::new(SelectMode::Single).reorder(), "source", Pos2::new(10.0, 10.0), &words, &source);
        let between = DropKind::new(DropPlace::Between, |_: &ListDrag<usize>, _: &DropAt<usize>| true);
        show(ui, List::new(SelectMode::Single).takes_drop(between), "target", Pos2::new(200.0, 10.0), &words, &target);
        show(ui, List::new(SelectMode::Single), "other", Pos2::new(390.0, 10.0), &words, &other);
    };
    frames.idle(3, &mut show);
    let elsewhere = other.row(2).center();
    drag_to(&mut frames, source.row(0).center(), elsewhere, &mut show);
    assert!(drag::mark(&frames.ctx).is_none(), "a list without a kind shows no mark");
    release_at(&mut frames, elsewhere, &mut show);
    assert!(other.events.borrow().iter().all(|event| !matches!(event, CollectionEvent::Dropped { .. })));

    let gap = target.row(2).min + Vec2::new(10.0, 2.0);
    drag_to(&mut frames, source.row(0).center(), gap, &mut show);
    assert!(matches!(drag::mark(&frames.ctx), Some(DropMark::Line { .. })), "a gap shows the line mark");
    release_at(&mut frames, gap, &mut show);
    let keys: Vec<(DropAt<usize>, Option<Vec<usize>>)> =
        target.drops::<ListDrag<usize>>().into_iter().map(|(at, drag)| (at, drag.map(|drag| drag.keys))).collect();
    assert_eq!(keys, vec![(DropAt::gap(Some(2), 2), Some(vec![0]))]);
}

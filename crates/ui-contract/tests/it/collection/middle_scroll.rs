//! Middle-button auto-scroll on a collection's rows: the latch the items step runs under `MiddleButton::AutoScroll`.

use super::kit::{Count, Dismissed, ROW_H, Row};
use super::table::Table;
use craft_ui::contract::{CollectionEvent, CollectionProps, MiddleButton, Props as _, SelectMode, ShowCollection as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ROWS: usize = 200;
const SAME: f32 = 0.5;
/// How far below the latch the pointer pulls: past the 15 pt dead zone, still over the rows.
const PULL: f32 = 80.0;

/// A scrolling list of `ROWS` rows with the middle button `button`, what it reported, and a switch that reruns the
/// next pass.
struct Rig {
    button: MiddleButton,
    source: Count,
    rect: Cell<Rect>,
    /// The first row the last pass built, and its top.
    first: Cell<Option<(usize, f32)>>,
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    discard: Cell<bool>,
}

impl Rig {
    fn new(button: MiddleButton) -> Self {
        Rig {
            button,
            source: Count(ROWS),
            rect: Cell::new(Rect::NOTHING),
            first: Cell::new(None),
            events: RefCell::new(Vec::new()),
            discard: Cell::new(false),
        }
    }

    fn show(&self, ui: &mut Ui) {
        self.first.set(None);
        let items = CollectionProps::new().select(SelectMode::Single).middle_button(self.button);
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let out = Table::new(items).id_salt("rows").show_items(ui, &self.source, &mut |key, cx| {
                if self.first.get().is_none_or(|(first, _)| *key < first) {
                    self.first.set(Some((*key, cx.rect.top())));
                }
                Row::new()
            });
            self.rect.set(out.response.rect);
            self.events.borrow_mut().extend(out.events);
        });
        if self.discard.take() {
            ui.ctx().request_discard("the trial reruns the pass");
        }
    }

    /// The rows' scroll offset, read from where the first built row lies: the rows are `ROW_H` apart.
    fn offset(&self) -> f32 {
        let (key, top) = self.first.get().expect("the list built a row");
        key as f32 * ROW_H + self.rect.get().top() - top
    }

    /// The reports of a row press: a pick, a lead move or an activation.
    fn row_presses(&self) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|event| matches!(event, CollectionEvent::Selection { .. } | CollectionEvent::LeadMoved { .. } | CollectionEvent::Activate(_)))
            .count()
    }
}

/// A middle click at `at`: hover, press and release in three frames.
fn middle_click(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![pointer::moved(at)], &mut *show);
    frames.frame(vec![pointer::middle(at, true)], &mut *show);
    frames.frame(vec![pointer::middle(at, false)], &mut *show);
}

#[trial]
fn one_frame_middle_click_latches_at_the_press_and_the_stopping_click_does_not_pick() {
    let rig = Rig::new(MiddleButton::AutoScroll);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| rig.show(ui);
    frames.idle(2, &mut show);
    let at = rig.rect.get().center();
    let below = at + Vec2::new(0.0, PULL);
    frames.frame(vec![pointer::moved(at), pointer::middle(at, true), pointer::moved(below), pointer::middle(below, false)], &mut show);
    let before = rig.offset();
    frames.idle(10, &mut show);
    assert!(rig.offset() > before + SAME, "the latch uses the press point");
    let picks = rig.row_presses();
    frames.frame(vec![pointer::press(below)], &mut show);
    frames.frame(vec![pointer::release(below)], &mut show);
    frames.idle(3, &mut show);
    assert_eq!(rig.row_presses(), picks, "the click ending the latch does not select a row");
}

#[trial]
fn a_latch_does_not_resume_when_the_pointer_leaves_and_returns() {
    let rig = Rig::new(MiddleButton::AutoScroll);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| rig.show(ui);
    frames.idle(2, &mut show);
    let at = rig.rect.get().center();
    middle_click(&mut frames, at, &mut show);
    frames.frame(vec![egui::Event::PointerGone], &mut show);
    frames.frame(vec![pointer::moved(at + Vec2::new(0.0, PULL))], &mut show);
    let before = rig.offset();
    frames.idle(5, &mut show);
    assert!((rig.offset() - before).abs() < SAME, "the latch stays ended after the pointer returns");
}

/// Under `AutoScroll` a middle click on the rows latches: the pointer held below the latch scrolls the rows toward the
/// end, once per frame however many passes the frame runs, no row is picked, and a second middle click ends it. Under
/// the default `Pan` the same events scroll nothing and the press picks its row as any press does. Guards the items
/// step's wiring of the latch: the row press taking a latching press, or the offset not following the latch.
#[trial]
fn a_latched_middle_press_scrolls_the_rows_and_presses_nothing() {
    let rig = Rig::new(MiddleButton::AutoScroll);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| rig.show(ui);
    frames.idle(2, &mut show);
    let at = rig.rect.get().center();
    let below = at + Vec2::new(0.0, PULL);
    middle_click(&mut frames, at, &mut show);
    let before = rig.offset();
    frames.frame(vec![pointer::moved(below)], &mut show);
    frames.idle(10, &mut show);
    assert!(rig.offset() > before + SAME, "the latch scrolls the rows toward the end: {before} -> {}", rig.offset());

    // A frame that runs a second pass steps once: the rerun pass carries no time of its own.
    let start = rig.offset();
    frames.idle(1, &mut show);
    let step = rig.offset() - start;
    let start = rig.offset();
    rig.discard.set(true);
    assert_eq!(frames.frame(Vec::new(), &mut show), 2, "the frame reran its pass");
    let rerun = rig.offset() - start;
    assert!(step > SAME, "one frame of the latch scrolls: {step}");
    assert!(rerun > SAME && rerun < step * 1.5, "a frame of two passes scrolls one step: {rerun} against {step}");

    middle_click(&mut frames, below, &mut show);
    let ended = rig.offset();
    frames.idle(5, &mut show);
    assert!((rig.offset() - ended).abs() < SAME, "a second middle press ends the latch: {ended} -> {}", rig.offset());
    assert_eq!(rig.row_presses(), 0, "no middle press picked, led to or activated a row");

    let pan = Rig::new(MiddleButton::Pan);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| pan.show(ui);
    frames.idle(2, &mut show);
    let at = pan.rect.get().center();
    middle_click(&mut frames, at, &mut show);
    let before = pan.offset();
    frames.frame(vec![pointer::moved(at + Vec2::new(0.0, PULL))], &mut show);
    frames.idle(10, &mut show);
    assert!((pan.offset() - before).abs() < SAME, "under Pan a middle click latches nothing: {before} -> {}", pan.offset());
    assert!(pan.row_presses() > 0, "under Pan the middle press picks its row");
}

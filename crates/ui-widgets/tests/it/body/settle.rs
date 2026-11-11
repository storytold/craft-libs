//! Settling children (`Lazy::settles`): a settled child adds its last run to the pass in place of running, and runs
//! when something it shows may have changed: the pointer over it, a bound value it read, its room, an animation.

use crate::values::kit::{AT, INITIAL, Scrub, Seen};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Lazy, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Spinner;
use craft_ui::layout_components::Stack;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, last_pass_measuring_runs, trial};
use egui::{Id, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 60.0);
const ROOM: Vec2 = Vec2::new(300.0, 200.0);
/// Where the bound scrub of the reader child is shown, inside that child's room.
const READER: Rect = Rect { min: Pos2::new(150.0, 70.0), max: Pos2::new(250.0, 90.0) };
/// Settled frames a trial looks at.
const SETTLED: usize = 4;

/// Two settling children, `reader` and `other`, under a scrub bound to the value the reader's own scrub shows. The
/// scrub that edits comes before the children in the pass when `editor_first`, else after them.
struct Scene {
    value: RefCell<f32>,
    room: Cell<Vec2>,
    editor_first: bool,
    spinner: bool,
    runs: [Cell<usize>; 2],
    buttons: [Cell<(Id, Rect)>; 2],
    seen: Seen,
}

impl Scene {
    fn new(editor_first: bool, spinner: bool) -> Scene {
        Scene {
            value: RefCell::new(INITIAL),
            room: Cell::new(ROOM),
            editor_first,
            spinner,
            runs: Default::default(),
            buttons: [Cell::new((Id::NULL, Rect::NOTHING)), Cell::new((Id::NULL, Rect::NOTHING))],
            seen: Seen::default(),
        }
    }

    fn show(&self, ui: &mut Ui) {
        if self.editor_first {
            self.editor(ui);
        }
        in_region(ui, Rect::from_min_size(ORIGIN, self.room.get()), |ui| {
            Lazy::new("reader", 0.0).settles(true).show(ui, |ui| {
                self.runs[0].set(self.runs[0].get() + 1);
                let response = Button::new("Alpha").show(ui).response;
                self.buttons[0].set((response.id, response.rect));
                Scrub::new(Bind::bound(&mut *self.value.borrow_mut())).id_salt("reader").seen(&self.seen).show_at(ui, READER);
            });
            Lazy::new("other", 0.0).settles(true).show(ui, |ui| {
                self.runs[1].set(self.runs[1].get() + 1);
                // In a body, whose measure a run that is not settled takes from what its last run kept.
                Stack::new().show(ui, |ui, _| {
                    let response = Button::new("Beta").show(ui).response;
                    self.buttons[1].set((response.id, response.rect));
                });
                if self.spinner {
                    Spinner::new().show(ui);
                }
            });
        });
        if !self.editor_first {
            self.editor(ui);
        }
    }

    fn editor(&self, ui: &mut Ui) {
        Scrub::new(Bind::bound(&mut *self.value.borrow_mut())).id_salt("editor").show_at(ui, AT);
    }

    /// The runs of each child in one frame with `events`.
    fn frame(&self, frames: &mut Frames, events: Vec<egui::Event>) -> [usize; 2] {
        self.runs.iter().for_each(|runs| runs.set(0));
        frames.frame(events, |ui| self.show(ui));
        [self.runs[0].get(), self.runs[1].get()]
    }

    /// Frames with no input until both children are settled.
    fn settle(&self, frames: &mut Frames) {
        frames.idle(30, |ui| self.show(ui));
    }
}

fn painted(frames: &Frames, text: &str) -> bool {
    frames.texts().iter().any(|run| run.text == text)
}

/// An untouched settled child runs no component, yet its paint shows in each frame and its accessibility nodes stay
/// in the tree, under the same parents.
#[trial]
fn a_settled_child_does_not_run_and_still_paints() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    frames.frame(vec![], |ui| scene.show(ui));
    let ids = scene.buttons.each_ref().map(|button| button.get().0);
    let tree = frames.tree();
    let parents = ids.map(|id| tree.parent_of(id));
    assert!(parents.iter().all(Option::is_some), "the buttons have nodes in a frame that runs them");
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "settled frame {frame} ran a child");
        assert!(painted(&frames, "Alpha") && painted(&frames, "Beta"), "settled frame {frame} paints both children");
        let tree = frames.tree();
        assert_eq!(
            ids.map(|id| tree.name(id).map(str::to_owned)),
            [Some("Alpha".to_owned()), Some("Beta".to_owned())],
            "settled frame {frame} keeps the nodes"
        );
        assert_eq!(ids.map(|id| tree.parent_of(id)), parents, "settled frame {frame} keeps the nodes' parents");
    }
}

/// The pointer over a settled child runs it, and only it, in the frame it arrives, from the measures its last run
/// kept; leaving, the child runs until its hover has faded out, then settles again.
#[trial]
fn hovering_a_settled_child_runs_it_alone() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let over = scene.buttons[1].get().1.center();
    let [reader, other] = scene.frame(&mut frames, vec![moved(over)]);
    assert!(other > 0, "the hovered child runs in the frame the pointer arrives");
    let measured = last_pass_measuring_runs(&frames.ctx);
    assert!(measured.is_empty(), "the hovered child's body takes its kept measure: {measured:?}");
    assert_eq!(reader, 0, "the child beside it stays settled");
    let away = Pos2::new(ORIGIN.x + ROOM.x - 2.0, ORIGIN.y + ROOM.y - 2.0);
    let [_, other] = scene.frame(&mut frames, vec![moved(away)]);
    assert!(other > 0, "the child the pointer left runs to show its hover leaving");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "once its hover faded, the child settles again");
}

/// Drags the editor right in steps; returns per drag frame the passes, the runs of each child, and whether the
/// reader showed the dragged value.
fn drag(scene: &Scene) -> Vec<(usize, [usize; 2], bool)> {
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let from = Pos2::new(AT.left() + 10.0, AT.center().y);
    scene.frame(&mut frames, vec![moved(from)]);
    scene.frame(&mut frames, vec![press(from)]);
    let mut seen = Vec::new();
    for step in 1..=4 {
        scene.runs.iter().for_each(|runs| runs.set(0));
        let passes = frames.frame(vec![moved(from + Vec2::new(10.0 * step as f32 + 10.0, 0.0))], |ui| scene.show(ui));
        let shows = scene.seen.shown.get() == Some(*scene.value.borrow());
        seen.push((passes, [scene.runs[0].get(), scene.runs[1].get()], shows));
    }
    scene.frame(&mut frames, vec![release(from + Vec2::new(50.0, 0.0))]);
    seen
}

/// A drag of a control outside both children, bound to a value one of them reads, runs that child in each drag frame
/// and leaves the other settled, with the editor before the children in the pass and after them. The frame after the
/// press runs both: input unsettles every child for its frame and the next.
#[trial]
fn an_edit_runs_the_settled_child_that_read_the_value() {
    for editor_first in [true, false] {
        let frames = drag(&Scene::new(editor_first, false));
        for (passes, [reader, other], shows) in &frames[1..] {
            assert!(*reader > 0, "the reader runs in each drag frame (editor first: {editor_first}): {frames:?}");
            assert_eq!(*other, 0, "the other child stays settled (editor first: {editor_first}): {frames:?}");
            assert!(*shows, "the reader shows the dragged value (editor first: {editor_first}): {frames:?}");
            let expected = if editor_first { 1 } else { 2 };
            assert_eq!(*passes, expected, "passes per drag frame (editor first: {editor_first}): {frames:?}");
        }
    }
}

/// A settled child whose room changes runs in that frame.
#[trial]
fn a_settled_child_runs_when_its_room_changes() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.room.set(ROOM + Vec2::new(40.0, 0.0));
    assert_eq!(scene.frame(&mut frames, vec![]), [1, 1], "both children run at the new room");
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "and settle at it");
}

/// A child with a running animation (a spinner) runs every frame; the child beside it stays settled.
#[trial]
fn a_child_with_a_running_animation_runs_every_frame() {
    let scene = Scene::new(false, true);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 1], "frame {frame}: the spinner's child runs, the other does not");
    }
}

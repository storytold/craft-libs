//! The undo adapter: one undo step per gesture, named by the component's label.

use super::kit::{AT, DRAGGED, INITIAL, Scrub, near};
use craft_behavior::DocKey;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::edit::{GestureHost, StepName, UndoMarks};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Ui, Vec2};

/// A document with the history of an app: every applied edit records a step, and the history can fold or take back steps.
#[derive(Default)]
struct Doc {
    value: f32,
    steps: Vec<(f32, Option<String>)>,
}

impl Doc {
    fn set(&mut self, value: f32) {
        self.steps.push((self.value, None));
        self.value = value;
    }
}

impl UndoMarks for Doc {
    type Mark = usize;

    fn undo_mark(&self) -> usize {
        self.steps.len()
    }

    fn revert_to(&mut self, mark: usize) {
        if let Some((value, _)) = self.steps.get(mark) {
            self.value = *value;
        }
        self.steps.truncate(mark);
    }

    fn squash_since(&mut self, mark: usize, name: &StepName) {
        if self.steps.len() > mark {
            self.steps.truncate(mark + 1);
            self.steps[mark].1 = Some(name.label.to_string());
        }
    }
}

/// Shows a given-bound `Scrub` over the document and feeds its events to the host, frame by frame.
struct Session {
    frames: Frames,
    doc: Doc,
    host: GestureHost<Doc>,
    salt: &'static str,
}

impl Session {
    fn new(value: f32) -> Self {
        Session { frames: Frames::new(), doc: Doc { value, steps: Vec::new() }, host: GestureHost::new(), salt: "gain" }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let (doc, host, salt) = (&mut self.doc, &mut self.host, self.salt);
        self.frames.frame(events, |ui: &mut Ui| {
            let out = Scrub::new(Bind::given(doc.value)).label("Gain").doc_key(DocKey::new("trial.gain")).id_salt(salt).show_at(ui, AT);
            for event in out.events {
                host.handle(doc, event, |doc, value| doc.set(value));
            }
        });
    }

    fn drag(&mut self) -> Pos2 {
        let at = Pos2::new(AT.left() + 10.0, AT.center().y);
        for _ in 0..3 {
            self.frame(vec![]);
        }
        self.frame(vec![moved(at)]);
        self.frame(vec![press(at)]);
        self.frame(vec![moved(at + Vec2::new(30.0, 0.0))]);
        self.frame(vec![moved(at + Vec2::new(40.0, 0.0))]);
        self.frame(vec![]);
        at + Vec2::new(40.0, 0.0)
    }
}

/// A drag edits the document on every proposal and still leaves one undo step, named by the component's label.
#[trial]
fn a_gesture_leaves_one_named_undo_step() {
    let mut session = Session::new(INITIAL);
    let end = session.drag();
    assert!(session.doc.steps.len() > 1, "the preview edits recorded steps while dragging: {}", session.doc.steps.len());
    session.frame(vec![release(end)]);
    session.frame(vec![]);
    assert_eq!(session.doc.steps.len(), 1, "one step for the gesture");
    assert_eq!(session.doc.steps[0], (INITIAL, Some("Gain".to_owned())));
    assert!(near(session.doc.value, DRAGGED));
}

/// Gesture ids are shared by every component: an undo during one component's drag does not make the host drop the
/// next component's gesture.
#[trial]
fn an_undo_during_one_components_drag_does_not_drop_anothers_edit() {
    let mut session = Session::new(INITIAL);
    let end = session.drag();
    session.host.before_undo(&session.frames.ctx, &mut session.doc);
    session.frame(vec![release(end)]);
    session.frame(vec![]);
    session.salt = "pan";
    session.drag();
    session.frame(vec![release(Pos2::new(AT.left() + 50.0, AT.center().y))]);
    session.frame(vec![]);
    assert!(near(session.doc.value, DRAGGED), "the second component's edit reached the document: {}", session.doc.value);
    assert_eq!(session.doc.steps.len(), 1, "one step for the second gesture");
}

//! Tests of the Liquify workspace: its session, tools and keys, and the canvas gestures.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::edit_history::Step;
use craft_behavior::{CancelReason, PixelSize};
use craft_imaging::ui::liquify::{
    LiquifyBrush, LiquifyEvent, LiquifyMesh, LiquifyOpen, LiquifySource, LiquifyTool, LiquifyWorkspace, MeshSize, StrokeEvent, StrokeRecorder,
};
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Color32, ColorImage, Event, Pos2, Vec2};

/// The document's size in pixels.
const DOCUMENT: [u32; 2] = [800, 600];

/// A solid picture.
struct Solid {
    size: PixelSize,
    colour: Color32,
}

impl RasterSource for Solid {
    fn size(&self) -> PixelSize {
        self.size
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(self.colour);
    }
}

/// The app's side: a picture and two layers.
struct Source {
    image: Solid,
}

impl LiquifySource for Source {
    fn picture(&self) -> &dyn RasterSource {
        &self.image
    }

    fn freeze_mask(&self) -> Option<(&dyn RasterSource, u32)> {
        None
    }

    fn mesh(&self, _: MeshSize) -> Option<LiquifyMesh> {
        None
    }

    fn backdrop(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn layers(&self) -> Vec<(u64, String)> {
        vec![(7, "Sky".to_owned()), (9, "Lake".to_owned())]
    }
}

/// What the app keeps of the workspace and what it saw.
struct App {
    workspace: LiquifyWorkspace,
    source: Source,
    events: Vec<LiquifyEvent>,
}

/// The workspace over a solid picture, settled.
fn open() -> (Fixture, Rc<RefCell<App>>) {
    let size = PixelSize::new(DOCUMENT[0], DOCUMENT[1]);
    let app = Rc::new(RefCell::new(App {
        workspace: LiquifyWorkspace::new(LiquifyOpen::new(DOCUMENT, "Photo")),
        source: Source { image: Solid { size, colour: Color32::from_rgb(40, 120, 200) } },
        events: Vec::new(),
    }));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shown.borrow_mut();
        let App { workspace, source, .. } = &mut *app;
        let out = workspace.show(ui, source);
        app.events.extend(out.events);
    });
    fixture.room(by(1000.0, 900.0));
    fixture.settle();
    (fixture, app)
}

/// The offset `x` points right and `y` points down.
fn by(x: f32, y: f32) -> Vec2 {
    Vec2::X * x + Vec2::Y * y
}

/// The middle of the canvas, moved by `by`.
fn canvas_point(fixture: &Fixture, offset: Vec2) -> Pos2 {
    let canvas = fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas");
    canvas.center() + offset
}

fn primary(at: Pos2, down: bool) -> Event {
    if down { pointer::press(at) } else { pointer::release(at) }
}

/// Press at `from` with `button`, move to `to` in two steps, release.
fn drag(fixture: &mut Fixture, button: fn(Pos2, bool) -> Event, from: Pos2, to: Pos2) {
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![button(from, true)]);
    fixture.frame(vec![pointer::moved(from + (to - from) * 0.5)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![button(to, false)]);
    fixture.frame(Vec::new());
}

/// Presses at `at` and moves 30 points away, leaving the stroke open.
fn open_stroke(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::press(at)]);
    fixture.frame(vec![pointer::moved(at + by(30.0, 0.0))]);
}

#[trial]
fn liquify_undo_mid_stroke_cancels_the_stroke_then_steps() {
    let (mut fixture, app) = open();
    let left = canvas_point(&fixture, by(-40.0, 0.0));
    drag(&mut fixture, primary, left, left + by(80.0, 10.0));
    assert_eq!(app.borrow().workspace.session().len(), 1);
    let below = canvas_point(&fixture, by(0.0, 40.0));
    open_stroke(&mut fixture, below);
    app.borrow_mut().events.clear();

    assert!(app.borrow_mut().workspace.step(Step::Undo), "the step waits for the next pass");
    fixture.frame(Vec::new());
    let events = app.borrow().events.clone();
    assert!(
        matches!(
            events.as_slice(),
            [LiquifyEvent::Stroke(StrokeEvent::Cancel(CancelReason::UndoDuringGesture)), LiquifyEvent::Replay(steps)] if steps.is_empty()
        ),
        "{events:?}"
    );
    assert!(app.borrow().workspace.session().is_empty(), "the earlier stroke is undone");
}

// A press, two drags and a release report Begin, Update, Update, Commit; each Update carries only its new point and
// the Commit the whole stroke; a press and release one pixel apart is recorded whole with both points.
#[trial]
fn liquify_stroke_events_begin_update_commit_and_short_strokes_are_recorded_whole() {
    let mut recorder = StrokeRecorder::default();
    let brush = LiquifyBrush::default();
    let StrokeEvent::Begin(first) = recorder.begin(LiquifyTool::Pucker, &brush, [10.0, 10.0, 1.0], true) else { panic!("Begin") };
    assert!(recorder.is_active());
    assert_eq!((first.tool, first.points.len()), (LiquifyTool::Bloat, 1), "Alt paints with the alternate tool");
    let Some(StrokeEvent::Update(two)) = recorder.extend([20.0, 10.0, 0.5]) else { panic!("Update") };
    assert_eq!(two.points, [[20.0, 10.0, 0.5]], "an Update carries the new point only");
    assert_eq!(two.tool, LiquifyTool::Bloat, "and the stroke's tool and brush");
    assert!(recorder.extend([20.0, 10.0, 0.9]).is_none(), "an equal point is skipped");
    let Some(StrokeEvent::Update(three)) = recorder.extend([30.0, 12.0, 0.5]) else { panic!("Update") };
    assert_eq!(three.points, [[30.0, 12.0, 0.5]]);
    for held in [4, 5] {
        let Some(StrokeEvent::Update(value)) = recorder.hold([30.0, 12.0, 0.5]) else { panic!("Update") };
        assert_eq!(value.points, [[30.0, 12.0, 0.5]], "a held dab {held} is its own point");
    }
    let Some(StrokeEvent::Commit(whole)) = recorder.end([40.0, 12.0]) else { panic!("Commit") };
    assert!(!recorder.is_active());
    assert_eq!(whole.points.last(), Some(&[40.0, 12.0, 0.5]), "the release point keeps the last pressure");
    assert_eq!(whole.points.len(), 6);

    recorder.begin(LiquifyTool::ForwardWarp, &brush, [5.0, 5.0, 1.0], false);
    let Some(StrokeEvent::Commit(short)) = recorder.end([6.0, 5.0]) else { panic!("Commit") };
    assert_eq!(short.points, [[5.0, 5.0, 1.0], [6.0, 5.0, 1.0]]);

    recorder.begin(LiquifyTool::ForwardWarp, &brush, [5.0, 5.0, 1.0], false);
    assert!(matches!(recorder.cancel(CancelReason::Escape), Some(StrokeEvent::Cancel(CancelReason::Escape))));
    assert!(recorder.end([1.0, 1.0]).is_none());
}

// A closed Freeze Lasso records one stroke with amount 1 (0 with Alt at the press) and the first point again at its
// end; fewer than 3 points record nothing.

// Cmd+I and Cmd+D record the global step in the order pressed; Restore All empties the session.

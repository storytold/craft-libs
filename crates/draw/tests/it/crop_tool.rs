//! The Crop tool and its session through the public API: the pass, the keys the overlay leaves to the tool, the
//! session's box history, and one drag through the overlay on a canvas.

use craft_behavior::CanvasView;
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_draw::ui::crop::{CropDocKey, CropDocument, CropEvent, CropInput, CropSession, CropSettings, CropTool};
use craft_ui::canvas::{Canvas, ToolHost};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::testing::{Fixture, Frames, pointer, trial};
use egui::{Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const SIZE: Size = Size::new(400.0, 300.0);

/// The units of the default scope (PostScript points).
fn units() -> Units {
    let mut frames = Frames::new();
    let mut units = None;
    frames.frame(vec![], |ui| units = Some(Env::of(ui).units()));
    units.expect("the frame ran")
}

fn document(key: u64) -> CropDocument {
    CropDocument::new(CropDocKey::new(key), SIZE, 72.0)
}

fn upright(x0: f64, y0: f64, x1: f64, y1: f64) -> CropBox {
    CropBox::upright(Rect::new(x0, y0, x1, y1))
}

fn classic() -> CropSettings {
    let mut settings = CropSettings::default();
    settings.shield.classic_mode = true;
    settings
}

// The session cases.

fn sized(key: u64, width: f64, height: f64) -> CropDocument {
    CropDocument::new(CropDocKey::new(key), Size::new(width, height), 72.0)
}

#[trial]
fn box_undo_redo_steps_committed_gestures_only() {
    let mut session = CropSession::new();
    let ctx = egui::Context::default();
    session.ensure_frame(&sized(1, 200.0, 100.0), false);
    let start = session.frame();
    assert!(!session.can_undo());

    session.nudge(&ctx, upright(10.0, 0.0, 100.0, 100.0), "Crop");
    session.nudge(&ctx, upright(20.0, 0.0, 100.0, 100.0), "Crop");
    session.nudge(&ctx, upright(30.0, 0.0, 100.0, 100.0), "Crop");
    assert!(session.can_undo(), "an open run counts, so Undo never falls through to the document");
    session.settle();
    assert!(session.pending());
    session.set(upright(31.0, 0.0, 100.0, 100.0), false, "Crop");

    assert!(session.undo());
    assert_eq!(session.frame(), Some(upright(30.0, 0.0, 100.0, 100.0)), "the run of nudges is one step");
    assert!(session.undo());
    assert_eq!(session.frame(), start);
    assert!(session.is_default(), "the default mark comes back with the step");
    assert!(!session.can_undo() && !session.undo());
    assert!(session.can_redo() && session.redo());
    assert_eq!(session.frame(), Some(upright(30.0, 0.0, 100.0, 100.0)));
    assert!(!session.is_default());

    session.set(upright(5.0, 5.0, 60.0, 60.0), false, "Crop");
    assert!(!session.can_redo() && !session.redo(), "a new step drops the redo side");
}

// One drag through the overlay on a canvas.

struct Host {
    tool: CropTool,
    settings: CropSettings,
    document: CropDocument,
    ctx: egui::Context,
    events: Vec<CropEvent>,
}

struct Rig {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
}

impl Rig {
    fn new(settings: CropSettings) -> Self {
        let host =
            Rc::new(RefCell::new(Host { tool: CropTool::new(), settings, document: document(1), ctx: egui::Context::default(), events: Vec::new() }));
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| {
            let mut guard = shown.borrow_mut();
            let host = &mut *guard;
            let room = ui.available_rect_before_wrap();
            let view = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
            let mut pass = host.tool.pass(CropInput::new(&host.ctx, &host.settings, &host.document));
            let canvas = Canvas::new("Document").document(SIZE).view(view).show(ui, |ui, body| pass.show_frame(ui, body.rect));
            let _ = ToolHost::new().run(ui, &canvas.output, &mut pass);
            let events = pass.finish().events;
            for event in &events {
                if let CropEvent::Settings(next) = event {
                    host.settings = next.clone();
                }
            }
            host.events.extend(events);
        });
        fixture.settle();
        Rig { fixture, host }
    }

    /// The screen point of document point `(x, y)`: the view shows (200, 150) at the canvas's centre.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        let canvas = self.fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas");
        canvas.center() + Vec2::new(x as f32 - 200.0, y as f32 - 150.0)
    }

    fn drag(&mut self, from: (f64, f64), to: (f64, f64)) {
        let (from, to) = (self.at(from.0, from.1), self.at(to.0, to.1));
        self.fixture.frame(vec![pointer::moved(from)]);
        self.fixture.frame(vec![pointer::press(from)]);
        for step in 1..=4 {
            self.fixture.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 4.0))]);
        }
        self.fixture.frame(vec![pointer::release(to)]);
        self.fixture.frame(Vec::new());
    }
}

#[trial]
fn leaving_during_a_drag_waits_for_the_release() {
    let mut rig = Rig::new(classic());
    rig.drag((100.0, 80.0), (300.0, 220.0));
    let from = rig.at(200.0, 150.0);
    rig.fixture.frame(vec![pointer::moved(from)]);
    rig.fixture.frame(vec![pointer::press(from)]);
    rig.fixture.frame(vec![pointer::moved(from + Vec2::new(30.0, 10.0))]);
    rig.fixture.frame(vec![pointer::moved(from + Vec2::new(40.0, 20.0))]);
    {
        let mut host = rig.host.borrow_mut();
        let Host { tool, settings, document, .. } = &mut *host;
        assert!(tool.dragging());
        assert_eq!(tool.leave(settings, document, &units(), false), None, "a drag keeps the frame pending");
    }
    rig.fixture.frame(vec![pointer::release(from + Vec2::new(40.0, 20.0))]);
    rig.fixture.frame(Vec::new());
    let mut host = rig.host.borrow_mut();
    let Host { tool, settings, document, .. } = &mut *host;
    assert!(matches!(tool.leave(settings, document, &units(), false), Some(CropEvent::Apply(_))), "after the release the drawn frame applies");
}

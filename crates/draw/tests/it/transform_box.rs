//! `TransformBox`, `TransformOverlay` and `TransformTool`: the parts the pointer takes, what each drag does, the
//! keys, the readout and the overlay's edits.

use craft_behavior::geometry::quad::{BoxPart, BoxRules, BoxState};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::ui::transform::{RotateZone, TransformBox};
use craft_keymap::Mods;
use craft_ui::canvas::Canvas;
use craft_ui::canvas::{Guide, GuideKey};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The box before any drag, in document units.
const RECT: Rect = Rect::new(100.0, 80.0, 300.0, 220.0);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    Begin,
    Update,
    Commit,
    Cancel,
    Set,
}

#[derive(Clone)]
struct Config {
    rules: BoxRules,
    zone: RotateZone,
    inside_moves: bool,
    reference_point: bool,
    pixel_grid: bool,
    mods: Option<Mods>,
    rotation: f64,
    flip: bool,
    guides: bool,
}

/// What the host keeps of the box and what the passes reported.
struct Host {
    state: BoxState,
    before: Option<BoxState>,
    events: Vec<Seen>,
    applied: usize,
    discarded: usize,
    picked: Option<BoxPart>,
    hovered: Option<BoxPart>,
    id: egui::Id,
    view: CanvasView,
    guide_events: usize,
}

struct Harness {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
}

fn defaults() -> Config {
    Config {
        rules: BoxRules::default(),
        zone: RotateZone::Outside,
        inside_moves: true,
        reference_point: true,
        pixel_grid: false,
        mods: None,
        rotation: 0.0,
        flip: false,
        guides: false,
    }
}

fn harness(state: BoxState, setup: impl FnOnce(&mut Config)) -> Harness {
    let mut config = defaults();
    setup(&mut config);
    let shown_config = Rc::new(RefCell::new(config));
    let host = Rc::new(RefCell::new(Host {
        state,
        before: None,
        events: Vec::new(),
        applied: 0,
        discarded: 0,
        picked: None,
        hovered: None,
        id: egui::Id::NULL,
        view: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
        guide_events: 0,
    }));
    let shown_host = Rc::clone(&host);
    let mut guides = vec![Guide::new(GuideKey::new(1), craft_behavior::Axis::Y, 300.0)];
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let config = shown_config.borrow().clone();
        let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
            .rotation(config.rotation)
            .flip_x(config.flip);
        let state = {
            let mut host = shown_host.borrow_mut();
            host.view = view;
            host.state
        };
        let canvas = Canvas::new("Document").view(view).show(ui, |ui, body| {
            let mut guide_edits = 0;
            if config.guides {
                let output = Guides::new("Guides", Bind::bound(&mut guides)).interactive(true).show_at(ui, body.rect);
                guide_edits = usize::from(!output.edit.events.is_empty());
            }
            let mut control = TransformBox::new("Free Transform", Bind::given(state), RECT, 0.0)
                .rules(config.rules)
                .rotate_zone(config.zone)
                .inside_moves(config.inside_moves)
                .reference_point(config.reference_point)
                .pixel_grid(config.pixel_grid);
            if let Some(mods) = config.mods {
                control = control.mods(mods);
            }
            (control.show_at(ui, body.rect), guide_edits)
        });
        if let Some((output, guide_edits)) = canvas.inner {
            let mut host = shown_host.borrow_mut();
            host.guide_events += guide_edits;
            host.id = output.edit.response.id;
            host.applied += usize::from(output.apply);
            host.discarded += usize::from(output.discard);
            (host.picked, host.hovered) = (output.picked, output.hovered);
            for event in &output.edit.events {
                match event {
                    EditEvent::Begin { .. } => {
                        host.events.push(Seen::Begin);
                        host.before = Some(host.state);
                    }
                    EditEvent::Update { value, .. } => {
                        host.events.push(Seen::Update);
                        host.state = *value;
                    }
                    EditEvent::Commit { after, .. } => {
                        host.events.push(Seen::Commit);
                        host.state = *after;
                    }
                    EditEvent::Set { after, .. } => {
                        host.events.push(Seen::Set);
                        host.state = *after;
                    }
                    EditEvent::Cancel { .. } => {
                        host.events.push(Seen::Cancel);
                        host.state = host.before.take().unwrap_or(host.state);
                    }
                    _ => {}
                }
            }
        }
    });
    fixture.settle();
    Harness { fixture, host }
}

fn upright() -> Harness {
    harness(BoxState::from_rect(RECT), |_| {})
}

fn frame(h: &mut Harness, events: Vec<egui::Event>) {
    h.fixture.frame(events);
}

impl Harness {
    fn screen(&self, p: Point) -> Pos2 {
        self.host.borrow().view.to_screen(p)
    }

    fn state(&self) -> BoxState {
        self.host.borrow().state
    }
}

/// A screen displacement.
fn by(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

#[trial]
fn a_pixel_grid_box_fed_back_as_state_does_not_drift_over_a_drag() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.pixel_grid = true);
    let from = h.screen(Point::new(300.0, 220.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    for step in 1..=12 {
        frame(&mut h, vec![pointer::moved(from + by(0.4 * step as f32, 0.3 * step as f32))]);
    }
    let held = h.state();
    frame(&mut h, vec![pointer::release(from + by(4.8, 3.6))]);
    frame(&mut h, Vec::new());
    for corner in h.state().quad {
        assert!((corner.x - corner.x.round()).abs() < 1e-9 && (corner.y - corner.y.round()).abs() < 1e-9, "{corner:?} is on whole units");
    }
    assert!((h.state().quad[2].x - held.quad[2].x).abs() <= 1.0, "no drift from the held box to the committed one");
}

#[trial]
fn a_release_with_the_pointer_gone_is_not_a_click_outside() {
    let mut h = upright();
    let at = h.screen(Point::new(340.0, 260.0));
    frame(&mut h, vec![pointer::moved(at)]);
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, vec![pointer::gone()]);
    frame(&mut h, vec![pointer::release(at)]);
    frame(&mut h, Vec::new());
    assert_eq!(h.host.borrow().applied, 0);
}

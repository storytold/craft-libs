//! `PathTextMarkers`: which mark a press grabs, what a body drag does at the end of an open path, and the flip of the
//! text when the pointer crosses the path.

use craft_behavior::kurbo::{Point, Vec2 as PathVec};
use craft_behavior::{CanvasView, EditEvent};
use craft_text::ui::path_text::{PathGrab, PathRange, PathTextMarkers, PathTrack};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Vec2};
use std::cell::{Cell, RefCell};
use std::f64::consts::TAU;
use std::rc::Rc;

/// The radius of the test circle.
const RADIUS: f64 = 40.0;

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// A horizontal line at y = 100 from x = 20.
    Line,
    /// A circle round the origin, clockwise on the screen from its rightmost point.
    Circle,
}

/// A test path.
struct TestTrack {
    shape: Shape,
    len: f64,
    /// The `near` of the last projection.
    near: Rc<Cell<Option<f64>>>,
}

impl TestTrack {
    fn point_at(&self, u: f64) -> (Point, PathVec) {
        match self.shape {
            Shape::Line => (Point::new(20.0 + u, 100.0), PathVec::new(1.0, 0.0)),
            Shape::Circle => {
                let angle = u / RADIUS;
                (Point::new(RADIUS * angle.cos(), RADIUS * angle.sin()), PathVec::new(-angle.sin(), angle.cos()))
            }
        }
    }
}

impl PathTrack for TestTrack {
    fn length(&self) -> f64 {
        self.len
    }

    fn closed(&self) -> bool {
        self.shape == Shape::Circle
    }

    fn at(&self, s: f64) -> Option<(Point, PathVec)> {
        Some(self.point_at(s))
    }

    fn project(&self, p: Point, near: Option<f64>) -> Option<f64> {
        self.near.set(near);
        Some(match self.shape {
            Shape::Line => (p.x - 20.0).max(0.0).min(self.len),
            Shape::Circle => p.y.atan2(p.x).rem_euclid(TAU) * RADIUS,
        })
    }
}

#[derive(Clone)]
struct Config {
    shape: Shape,
    len: f64,
    anchor: f64,
    boxes: Vec<[Point; 4]>,
    range: PathRange,
}

impl Config {
    fn line(len: f64, range: PathRange) -> Self {
        Config { shape: Shape::Line, len, anchor: 0.0, boxes: Vec::new(), range }
    }

    fn circle(range: PathRange) -> Self {
        Config { shape: Shape::Circle, len: TAU * RADIUS, anchor: 0.0, boxes: Vec::new(), range }
    }

    fn track(&self, near: &Rc<Cell<Option<f64>>>) -> TestTrack {
        TestTrack { shape: self.shape, len: self.len, near: Rc::clone(near) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    Begin,
    Update,
    Commit,
}

struct Host {
    range: PathRange,
    hovered: Option<PathGrab>,
    events: Vec<Seen>,
    view: CanvasView,
}

struct Harness {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
    config: Config,
    near: Rc<Cell<Option<f64>>>,
}

fn harness(config: Config) -> Harness {
    let near = Rc::new(Cell::new(None));
    let host = Rc::new(RefCell::new(Host {
        range: config.range,
        hovered: None,
        events: Vec::new(),
        view: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
    }));
    let (shown_host, shown_config, shown_near) = (Rc::clone(&host), config.clone(), Rc::clone(&near));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let center = match shown_config.shape {
            Shape::Line => Point::new(20.0 + shown_config.len / 2.0, 100.0),
            Shape::Circle => Point::ZERO,
        };
        let view = CanvasView::new(room, center, 1.0);
        let range = {
            let mut host = shown_host.borrow_mut();
            host.view = view;
            host.range
        };
        let track = shown_config.track(&shown_near);
        let canvas = Canvas::new("Document").view(view).show(ui, |ui, body| {
            PathTextMarkers::new("Text on path", Bind::given(range), &track)
                .anchor(shown_config.anchor)
                .body(&shown_config.boxes)
                .show_at(ui, body.rect)
        });
        if let Some(output) = canvas.inner {
            let mut host = shown_host.borrow_mut();
            host.hovered = output.hovered;
            for event in &output.range.events {
                match event {
                    EditEvent::Begin { .. } => host.events.push(Seen::Begin),
                    EditEvent::Update { value, .. } => {
                        host.events.push(Seen::Update);
                        host.range = *value;
                    }
                    EditEvent::Commit { after, .. } => {
                        host.events.push(Seen::Commit);
                        host.range = *after;
                    }
                    EditEvent::Set { after, .. } => host.range = *after,
                    _ => {}
                }
            }
        }
    });
    fixture.settle();
    Harness { fixture, host, config, near }
}

impl Harness {
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.fixture.frame(events);
    }

    fn track(&self) -> TestTrack {
        self.config.track(&self.near)
    }

    /// The screen position of the path at arc length `s`.
    fn at(&self, s: f64) -> Pos2 {
        let point = self.track().at(s).map_or(Point::ZERO, |(p, _)| p);
        self.host.borrow().view.to_screen(point)
    }

    /// The path's direction at arc length `s` on the screen (the view is upright).
    fn direction(&self, s: f64) -> Vec2 {
        let tangent = self.track().at(s).map_or(PathVec::ZERO, |(_, t)| t);
        Vec2::new(tangent.x as f32, tangent.y as f32)
    }

    fn range(&self) -> PathRange {
        self.host.borrow().range
    }

    /// Moves onto `from`, presses, drags to `to` in five steps and releases.
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![pointer::moved(from)]);
        self.frame(vec![pointer::press(from)]);
        for step in 1..=5 {
            self.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 5.0))]);
        }
        self.frame(vec![pointer::release(to)]);
        self.frame(Vec::new());
    }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[trial]
fn coincident_markers_pick_their_end_by_the_first_motion() {
    let lap = TAU * RADIUS;
    // On a full lap: a press ahead of the markers grabs the start, behind them the end, between them the body.
    for (offset, grab, direction) in [(3.0, PathGrab::Start, 1.0), (-3.0, PathGrab::End, -1.0), (0.0, PathGrab::Body, 1.0)] {
        let mut h = harness(Config::circle(PathRange::new(0.0, lap, false)));
        let tangent = h.direction(0.0);
        let from = h.at(0.0) + tangent * offset;
        h.frame(vec![pointer::moved(from)]);
        assert_eq!(h.host.borrow().hovered, Some(grab), "hover at {offset}");
        h.drag(from, from + tangent * (10.0 * direction));
        let range = h.range();
        match grab {
            PathGrab::Start => assert!(close(range.start, 10.0, 1.5) && close(range.end, lap, 1e-9), "start moved: {range:?}"),
            PathGrab::End => assert!(close(range.end, lap - 10.0, 1.5) && close(range.start, 0.0, 1e-9), "end moved: {range:?}"),
            _ => assert!(close(range.start, 10.0, 1.5) && close(range.end, lap + 10.0, 1.5), "both moved: {range:?}"),
        }
    }
    // Not on a full lap: the first motion picks the end (forward) or the start (backward).
    for forward in [true, false] {
        let mut h = harness(Config::circle(PathRange::new(50.0, 50.0, false)));
        let tangent = h.direction(50.0);
        let from = h.at(50.0);
        let sign = if forward { 1.0 } else { -1.0 };
        h.drag(from, from + tangent * (10.0 * sign));
        let range = h.range();
        if forward {
            assert!(close(range.start, 50.0, 1e-9) && close(range.end, 60.0, 1.5), "forward moves the end: {range:?}");
        } else {
            assert!(close(range.end, 50.0, 1e-9) && close(range.start, 40.0, 1.5), "backward moves the start: {range:?}");
        }
    }
}

#[trial]
fn a_body_drag_at_an_open_end_keeps_the_paragraph_anchor() {
    // The body's glyph box covers arc lengths 25 to 55 of a path 100 long; the text spans 20 to 60.
    let boxes = vec![[Point::new(45.0, 92.0), Point::new(75.0, 92.0), Point::new(75.0, 108.0), Point::new(45.0, 108.0)]];
    // (anchor, the arc length the pointer is dragged to from 40, the range afterwards)
    let cases = [(0.0, 100.0, (80.0, 100.0)), (1.0, 100.0, (60.0, 100.0)), (0.5, 90.0, (80.0, 100.0))];
    for (anchor, to, (start, end)) in cases {
        let mut config = Config::line(100.0, PathRange::new(20.0, 60.0, false));
        config.anchor = anchor;
        config.boxes = boxes.clone();
        let mut h = harness(config);
        let from = h.at(40.0);
        h.drag(from, h.at(to));
        let range = h.range();
        assert!(close(range.start, start, 1e-6) && close(range.end, end, 1e-6), "anchor {anchor}: {range:?}");
    }
    // A click with no drag changes nothing.
    let mut config = Config::line(100.0, PathRange::new(20.0, 60.0, false));
    config.boxes = boxes;
    let mut h = harness(config);
    let at = h.at(40.0);
    h.frame(vec![pointer::moved(at)]);
    h.frame(vec![pointer::press(at)]);
    h.frame(vec![pointer::release(at)]);
    h.frame(Vec::new());
    assert_eq!(h.range(), PathRange::new(20.0, 60.0, false));
    assert!(h.host.borrow().events.is_empty(), "no edit: {:?}", h.host.borrow().events);
}

#[trial]
fn crossing_the_path_flips_the_text_and_keeps_its_span() {
    let start = PathRange::new(20.0, 60.0, false);
    // Within 4 points of the path nothing flips.
    let mut h = harness(Config::line(120.0, start));
    let from = h.at(60.0);
    h.drag(from, from + Vec2::new(10.0, -3.0));
    let range = h.range();
    assert!(!range.flip && range.end > 60.0, "3 points across the path is not a crossing: {range:?}");

    // Dragging the end 20 along and 10 across: the span 20..80 is seen from the other end: 40..100.
    let mut h = harness(Config::line(120.0, start));
    let from = h.at(60.0);
    h.drag(from, from + Vec2::new(20.0, -10.0));
    let range = h.range();
    assert!(range.flip && close(range.start, 40.0, 1e-6) && close(range.end, 100.0, 1e-6), "flipped and remapped: {range:?}");
    assert_eq!(h.host.borrow().events.first(), Some(&Seen::Begin));
    assert_eq!(h.host.borrow().events.last(), Some(&Seen::Commit));
    assert!(h.near.get().is_some(), "the projection continues from the previous arc length");
}

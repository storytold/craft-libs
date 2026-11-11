//! The Blur Gallery's pins: the model's handles and drags, the overlay's pointer and keys, and the panel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Cursor, CursorIcon};
use craft_imaging::ui::blur_pins::{
    AXIS_HANDLES, BlurGrab, BlurKind, BlurPin, BlurPins, BlurPinsOverlay, DEFAULT_AMOUNT, IrisShape, PinReach, drag, feather_handles, grab,
    iris_point, round_handle,
};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Rect, pos2};

const BOUNDS: [f64; 4] = [0.0, 0.0, 1000.0, 800.0];

fn reach() -> PinReach {
    PinReach::new(9.0, 24.0, 5.0, 5.0)
}

/// One screen point per document pixel.
fn view() -> CanvasView {
    CanvasView::new(Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 800.0)), Point::new(500.0, 400.0), 1.0)
}

fn start(kind: BlurKind) -> BlurPins {
    BlurPins::new(kind, BOUNDS, BlurPin::new(0.0, 0.0, DEFAULT_AMOUNT))
}

fn screen(point: Point) -> Pos2 {
    view().to_screen(point)
}

fn dragged(pins: &mut BlurPins, grab: BlurGrab, to: Point) {
    let start = pins.clone();
    let from = pins.to_document(&start.pins[0]);
    drag(pins, grab, &start, from, to);
}

// Iris handles: an axis handle sets its radius, the square the roundness, a feather dot the feather; `clean` puts each
// back in range.
#[trial]
fn blur_pins_iris_handles_resize_turn_round_and_feather() {
    let mut pins = start(BlurKind::Iris);
    let pin = pins.pins[0];
    let Some(shape) = pin.iris else { panic!("an Iris pin has a shape") };
    assert_eq!(shape, IrisShape::iris());
    let centre = pins.to_document(&pin);

    // The press grabs each handle of the selected pin.
    let at = |u: f64, v: f64| screen(iris_point(&pins, &pin, shape, u, v));
    assert_eq!(grab(&pins, &view(), at(AXIS_HANDLES[0].0, AXIS_HANDLES[0].1), &reach()), BlurGrab::axis(0, 0));
    assert_eq!(grab(&pins, &view(), at(round_handle(shape).0, round_handle(shape).1), &reach()), BlurGrab::round(0));
    let dot = feather_handles(shape)[2];
    assert_eq!(grab(&pins, &view(), at(dot.0, dot.1), &reach()), BlurGrab::feather(0, 2));

    // Dragging +u out to 0.5 of the short side (400 px) turns nothing and sets radius_x.
    dragged(&mut pins, BlurGrab::axis(0, 0), Point::new(centre.x + 400.0, centre.y));
    let Some(resized) = pins.pins[0].iris else { panic!("shape") };
    assert!((resized.radius_x - 0.5).abs() < 1e-9 && resized.angle.abs() < 1e-9);
    // Dragging +v straight right turns the shape by a quarter turn (its +v axis points right).
    dragged(&mut pins, BlurGrab::axis(0, 1), Point::new(centre.x + 200.0, centre.y));
    let Some(turned) = pins.pins[0].iris else { panic!("shape") };
    assert!((turned.radius_y - 0.25).abs() < 1e-9 && (turned.angle + 90.0).abs() < 1e-9);

    // The square far out is a rectangle (roundness 100), at the centre an ellipse.
    let mut pins = start(BlurKind::Iris);
    dragged(&mut pins, BlurGrab::round(0), Point::new(centre.x + 5000.0, centre.y - 5000.0));
    assert!((pins.pins[0].iris.map_or(0.0, |s| s.roundness) - 100.0).abs() < 1e-9);
    dragged(&mut pins, BlurGrab::round(0), centre);
    assert!(pins.pins[0].iris.map_or(1.0, |s| s.roundness).abs() < 1e-9);

    // A feather dot at the shape's edge sets 1, clamped to 0.99; at the centre 0.
    dragged(&mut pins, BlurGrab::feather(0, 0), Point::new(centre.x + 5000.0, centre.y));
    assert_eq!(pins.pins[0].iris.map(|s| s.feather), Some(0.99));
    dragged(&mut pins, BlurGrab::feather(0, 0), centre);
    assert_eq!(pins.pins[0].iris.map(|s| s.feather), Some(0.0));

    let wild = IrisShape::new(9.0, f64::NAN, 270.0, -4.0, 3.0).clean();
    assert_eq!(wild, IrisShape::new(1.0, 0.25, -90.0, 0.0, 0.99));
}

// A whole turn of the ring sweeps the full range (Spin: 360°), clamped at the ends.

// A new pin copies the selected pin's amount and shape.

// The cursor over each grab: Alias, Move, ResizeVertical, Grab, Crosshair (Default for Tilt-Shift).

const CANVAS: [f64; 4] = [0.0, 0.0, 400.0, 300.0];

fn pins_of(kind: BlurKind, amounts: &[f64]) -> BlurPins {
    let mut pins = BlurPins::new(kind, CANVAS, BlurPin::new(0.5, 0.5, 15.0));
    pins.pins = amounts.iter().enumerate().map(|(i, &amount)| BlurPin::new(0.25 + 0.5 * i as f64 / amounts.len() as f64, 0.5, amount)).collect();
    pins.selected = Some(0);
    pins.clean();
    pins
}

struct Overlay {
    fixture: Fixture,
    pins: Rc<RefCell<BlurPins>>,
    view: Rc<Cell<Option<CanvasView>>>,
}

fn overlay(pins: BlurPins) -> Overlay {
    let pins = Rc::new(RefCell::new(pins));
    let view = Rc::new(Cell::new(None));
    let (shown_pins, shown_view) = (Rc::clone(&pins), Rc::clone(&view));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let canvas_view = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
        shown_view.set(Some(canvas_view));
        Canvas::new("Document").view(canvas_view).show(ui, |ui, body| {
            let mut held = shown_pins.borrow_mut();
            BlurPinsOverlay::new("Blur pins", Bind::bound(&mut *held)).show_at(ui, body.rect)
        });
    });
    fixture.settle();
    Overlay { fixture, pins, view }
}

impl Overlay {
    fn at(&self, x: f64, y: f64) -> Pos2 {
        self.view.get().expect("shown").to_screen(Point::new(x, y))
    }

    fn press(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
        self.fixture.frame(vec![pointer::press(at)]);
    }

    fn move_to(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
    }

    fn release(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::release(at)]);
        self.fixture.frame(Vec::new());
    }
}

// Page picks a pin as a keyboard part, arrows move it, and the part is named "Pin 2, Blur 15 px".

// A Spin Blur pin's part is named with its angle and the degree unit.

// Add Pin puts a pin at the middle of the bounds, selected: the keyboard path to creating a pin.

// The panel shows Blur (Spin: Blur Angle) for the selected pin, "No pin selected" without one, and the kind's hints.

// A drag on the selected pin's ring turns the amount frame by frame: a quarter turn clockwise on screen adds a quarter
// of the range, and the turns add up past a half turn.
#[trial]
fn blur_pins_ring_drag_turns_the_amount_over_several_frames() {
    let mut o = overlay(pins_of(BlurKind::Field, &[15.0]));
    let centre = o.at(100.0, 150.0);
    assert!((o.pins.borrow().pins[0].x - 0.25).abs() < 1e-9);
    // The ring is where the pointer shows the ring's cursor.
    let mut radius = None;
    for step in 1..=200u16 {
        let distance = f32::from(step) / 2.0;
        o.move_to(centre + egui::Vec2::new(distance, 0.0));
        if craft_ui::runtime::cursor::current(o.fixture.ctx()) == Some(Cursor::System(CursorIcon::Alias)) {
            radius = Some(distance);
            break;
        }
    }
    let radius = radius.expect("the pointer meets the ring");
    let on_ring = |quarter: u8| centre + egui::Vec2::angled(std::f32::consts::FRAC_PI_2 * f32::from(quarter)) * radius;
    o.press(on_ring(0));
    for quarter in 1..=3 {
        o.move_to(on_ring(quarter));
    }
    o.release(on_ring(3));
    // Three quarter turns: 15 + 0.75 * 500.
    let amount = o.pins.borrow().pins[0].amount;
    assert!((amount - 390.0).abs() < 1.0, "{amount}");
}

//! What a press on the pins grabs, and the drag that follows.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, CursorIcon};
use egui::Pos2;

use super::model::{BlurKind, BlurPin, BlurPins, IrisShape, TiltBand};

/// How near a press grabs each mark, in screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PinReach {
    /// How near a press grabs a pin's dot or a handle.
    pub grab: f32,
    /// The ring's radius.
    pub ring: f32,
    /// How far from the ring's radius a press still grabs the ring.
    pub ring_band: f32,
    /// How near a press grabs a Tilt-Shift line.
    pub line_grab: f32,
}

impl PinReach {
    /// The reach of the given sizes.
    pub fn new(grab: f32, ring: f32, ring_band: f32, line_grab: f32) -> Self {
        Self { grab, ring, ring_band, line_grab }
    }
}

/// The distance from `p` to `q`.
fn point_distance(p: Pos2, q: Pos2) -> f32 {
    p.distance(q)
}

/// The distance from `p` to the segment from `a` to `b`.
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length = ab.length_sq();
    let t = if length > 0.0 { ((p - a).dot(ab) / length).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// What a press grabs, by pin index. Handles belong to the selected pin only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlurGrab {
    /// An Iris or Spin outline handle: axis 0..=3 (+u, +v, −u, −v) resizes that radius.
    #[non_exhaustive]
    Axis {
        /// The pin's index.
        pin: usize,
        /// The axis, 0 to 3.
        axis: usize,
    },
    /// Iris's rounding square.
    #[non_exhaustive]
    Round {
        /// The pin's index.
        pin: usize,
    },
    /// One of Iris's four feather dots.
    #[non_exhaustive]
    Feather {
        /// The pin's index.
        pin: usize,
        /// The dot, 0 to 3.
        dot: usize,
    },
    /// A Tilt-Shift turn dot, on one side of the band (−1 or 1).
    #[non_exhaustive]
    Turn {
        /// The pin's index.
        pin: usize,
        /// The side of the band, 1 or -1.
        side: i8,
    },
    /// A Tilt-Shift solid line: the sharp band's width.
    #[non_exhaustive]
    Focus {
        /// The pin's index.
        pin: usize,
    },
    /// A Tilt-Shift dashed line: the transition's width.
    #[non_exhaustive]
    Transition {
        /// The pin's index.
        pin: usize,
    },
    /// A pin's ring: the amount (one turn is the full range).
    #[non_exhaustive]
    Ring {
        /// The pin's index.
        pin: usize,
    },
    /// A pin's dot: moves it.
    #[non_exhaustive]
    Pin {
        /// The pin's index.
        pin: usize,
    },
    /// Nothing: a click adds a pin (not Tilt-Shift).
    New,
}

impl BlurGrab {
    /// Outline handle `axis` of pin `pin`.
    pub fn axis(pin: usize, axis: usize) -> Self {
        BlurGrab::Axis { pin, axis }
    }

    /// The rounding square of pin `pin`.
    pub fn round(pin: usize) -> Self {
        BlurGrab::Round { pin }
    }

    /// Feather dot `dot` of pin `pin`.
    pub fn feather(pin: usize, dot: usize) -> Self {
        BlurGrab::Feather { pin, dot }
    }

    /// The turn dot on `side` (1 or -1) of pin `pin`.
    pub fn turn(pin: usize, side: i8) -> Self {
        BlurGrab::Turn { pin, side }
    }

    /// The solid lines of pin `pin`.
    pub fn focus(pin: usize) -> Self {
        BlurGrab::Focus { pin }
    }

    /// The dashed lines of pin `pin`.
    pub fn transition(pin: usize) -> Self {
        BlurGrab::Transition { pin }
    }

    /// The ring of pin `pin`.
    pub fn ring(pin: usize) -> Self {
        BlurGrab::Ring { pin }
    }

    /// The dot of pin `pin`.
    pub fn pin(pin: usize) -> Self {
        BlurGrab::Pin { pin }
    }

    /// The cursor over this grab: Alias for the ring, rounding and turn dots; Move for the axis and
    /// feather handles; ResizeVertical for the Tilt-Shift lines; Grab for a pin; Crosshair over
    /// nothing (Default for Tilt-Shift, which adds no pin).
    pub fn cursor(self, tilt_shift: bool) -> CursorIcon {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:539-546.
        match self {
            BlurGrab::Ring { .. } | BlurGrab::Round { .. } | BlurGrab::Turn { .. } => CursorIcon::Alias,
            BlurGrab::Axis { .. } | BlurGrab::Feather { .. } => CursorIcon::Move,
            BlurGrab::Focus { .. } | BlurGrab::Transition { .. } => CursorIcon::ResizeVertical,
            BlurGrab::Pin { .. } => CursorIcon::Grab,
            BlurGrab::New if tilt_shift => CursorIcon::Default,
            BlurGrab::New => CursorIcon::Crosshair,
        }
    }
}

/// The bounds are usable: a positive width and height.
fn valid(pins: &BlurPins) -> bool {
    let b = pins.bounds;
    b[2] > b[0] && b[3] > b[1]
}

/// A shaped pin's local point `(u, v)` (1 on the outline along that axis) in document pixels.
pub fn iris_point(pins: &BlurPins, pin: &BlurPin, shape: IrisShape, u: f64, v: f64) -> Point {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:322 `iris_point`.
    let c = pins.to_document(pin);
    let side = pins.short_side();
    let (x, y) = (u * shape.radius_x * side, v * shape.radius_y * side);
    let (sin, cos) = shape.angle.to_radians().sin_cos();
    Point::new(c.x + x * cos - y * sin, c.y + x * sin + y * cos)
}

/// A document point in a shaped pin's local coordinates (the engine's `u`, `v`).
fn iris_local(pins: &BlurPins, pin: &BlurPin, shape: IrisShape, at: Point) -> (f64, f64) {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:331 `iris_local`.
    let c = pins.to_document(pin);
    let side = pins.short_side();
    let (dx, dy) = (at.x - c.x, at.y - c.y);
    let (sin, cos) = shape.angle.to_radians().sin_cos();
    ((dx * cos + dy * sin) / (shape.radius_x * side), (-dx * sin + dy * cos) / (shape.radius_y * side))
}

/// The superellipse's diagonal point `t` (on the shape at (t, t)): 2^(−1/m).
fn diagonal(shape: IrisShape) -> f64 {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:340 `diagonal`.
    0.5f64.powf(1.0 / shape.exponent())
}

/// The four outline handles' local points (+u, +v, −u, −v).
pub const AXIS_HANDLES: [(f64, f64); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];

/// Iris's rounding square, local (upper right).
pub fn round_handle(shape: IrisShape) -> (f64, f64) {
    let t = diagonal(shape);
    (t, -t)
}

/// Iris's four feather dots, local (on the diagonals).
pub fn feather_handles(shape: IrisShape) -> [(f64, f64); 4] {
    let k = shape.feather * diagonal(shape);
    [(k, k), (-k, k), (-k, -k), (k, -k)]
}

/// The point of the Tilt-Shift band `offset` pixels across it (along its normal) and `along`
/// pixels along it from the centre, in document pixels.
pub fn tilt_point(pins: &BlurPins, pin: &BlurPin, band: TiltBand, offset: f64, along: f64) -> Point {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:362 `tilt_point`.
    let c = pins.to_document(pin);
    let (n, d) = band.axes();
    Point::new(c.x + n[0] * offset + d[0] * along, c.y + n[1] * offset + d[1] * along)
}

/// How far a document point is across the band from its centre line, in pixels (signed).
fn tilt_offset(pins: &BlurPins, pin: &BlurPin, band: TiltBand, at: Point) -> f64 {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:369 `tilt_offset`.
    let c = pins.to_document(pin);
    let (n, _) = band.axes();
    (at.x - c.x) * n[0] + (at.y - c.y) * n[1]
}

/// A Tilt-Shift turn dot: one on each solid line, beyond the pin's ring and its grab zone (so it
/// never sits on the ring, however narrow the band). `side` is +1 or −1.
pub fn turn_handle(pins: &BlurPins, pin: &BlurPin, band: TiltBand, view: &CanvasView, reach: &PinReach, side: f64) -> Point {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:353 `tilt_turn_handle` (ring + 18 points).
    let scale = view.zoom_x().max(1e-6);
    let along = f64::from(reach.ring + 2.0 * reach.grab) / scale;
    tilt_point(pins, pin, band, side * band.focus * pins.short_side(), along)
}

/// What a press at screen point `at` grabs: the selected pin's handles first (within
/// the grab, the Tilt-Shift lines within `.line_grab`), then its ring (`.ring` ±
/// `.ring_band`), then any pin's dot (`.grab`), else [`BlurGrab::New`].
pub fn grab(pins: &BlurPins, view: &CanvasView, at: Pos2, reach: &PinReach) -> BlurGrab {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:474 `grab`, :468 `line_distance`.
    if !valid(pins) {
        return BlurGrab::New;
    }
    let screen = |p: Point| view.to_screen(p);
    let near = |p: Point| point_distance(at, screen(p)) <= reach.grab;
    if let Some(index) = pins.selected
        && let Some(pin) = pins.pins.get(index)
    {
        if let Some(shape) = pin.iris {
            if let Some(axis) = AXIS_HANDLES.iter().position(|&(u, v)| near(iris_point(pins, pin, shape, u, v))) {
                return BlurGrab::Axis { pin: index, axis };
            }
            if pins.kind == BlurKind::Iris {
                let (u, v) = round_handle(shape);
                if near(iris_point(pins, pin, shape, u, v)) {
                    return BlurGrab::Round { pin: index };
                }
                if let Some(dot) = feather_handles(shape).iter().position(|&(u, v)| near(iris_point(pins, pin, shape, u, v))) {
                    return BlurGrab::Feather { pin: index, dot };
                }
            }
        }
        if let Some(band) = pin.tilt {
            for side in [1i8, -1] {
                if near(turn_handle(pins, pin, band, view, reach, f64::from(side))) {
                    return BlurGrab::Turn { pin: index, side };
                }
            }
        }
        let centre = screen(pins.to_document(pin));
        let from_centre = point_distance(at, centre);
        if (from_centre - reach.ring).abs() <= reach.ring_band {
            return BlurGrab::Ring { pin: index };
        }
        if let Some(band) = pin.tilt
            && from_centre > reach.grab
        {
            let side = pins.short_side();
            let on = |offset: f64| {
                segment_distance(at, screen(tilt_point(pins, pin, band, offset, -side)), screen(tilt_point(pins, pin, band, offset, side)))
                    <= reach.line_grab
            };
            let focus = band.focus * side;
            if on(focus) || on(-focus) {
                return BlurGrab::Focus { pin: index };
            }
            let edge = (band.focus + band.transition) * side;
            if on(edge) || on(-edge) {
                return BlurGrab::Transition { pin: index };
            }
        }
    }
    pins.pins
        .iter()
        .enumerate()
        .map(|(index, pin)| (index, point_distance(at, screen(pins.to_document(pin)))))
        .filter(|(_, distance)| *distance <= reach.grab)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(BlurGrab::New, |(index, _)| BlurGrab::Pin { pin: index })
}

/// A drag's effect on `pins`: `start` is the pins at the press, `from` and `to` the press and
/// the pointer in document pixels. A pin stays inside the bounds; the ring turns the amount by the
/// angle swept from `from` to `to` about the pin, the short way (a whole turn is
/// [`BlurKind::max`]); an axis handle sets its radius and the angle; the rounding square sets the
/// roundness along the diagonal; a feather dot sets the feather; a turn dot the band's angle; the
/// Tilt-Shift lines their widths. No modifier changes a drag (PhotoCraft's). [`BlurGrab::New`]
/// changes nothing: the caller adds the pin ([`BlurPins::add`]) and drags it as a
/// [`BlurGrab::Pin`].
///
/// The ring sweeps less than half a turn between `from` and `to`; to turn the amount through
/// several turns, call with `start` and `from` of the previous frame (PhotoCraft turned it by the
/// angle since the last frame). The other grabs read `to` alone, or `from` as the press.
pub fn drag(pins: &mut BlurPins, grab: BlurGrab, start: &BlurPins, from: Point, to: Point) {
    // pc: crates/ui-egui/src/dialogs/blur_pins.rs:585-672 (the drags), :322 `iris_point`, :331
    // `iris_local`, :340 `diagonal`, :346-360 (handles), :362 `tilt_point`, :369 `tilt_offset`.
    let index = match grab {
        BlurGrab::Axis { pin, .. }
        | BlurGrab::Round { pin }
        | BlurGrab::Feather { pin, .. }
        | BlurGrab::Turn { pin, .. }
        | BlurGrab::Focus { pin }
        | BlurGrab::Transition { pin }
        | BlurGrab::Ring { pin }
        | BlurGrab::Pin { pin } => pin,
        BlurGrab::New => return,
    };
    let Some(&origin) = start.pins.get(index) else { return };
    if index >= pins.pins.len() || !valid(start) {
        return;
    }
    let side = start.short_side();
    let centre = start.to_document(&origin);
    let kind = start.kind;
    let mut pin = origin;
    match grab {
        BlurGrab::Pin { .. } => {
            (pin.x, pin.y) = start.from_document(Point::new(centre.x + to.x - from.x, centre.y + to.y - from.y));
        }
        BlurGrab::Ring { .. } => {
            let angle = |p: Point| (p.y - centre.y).atan2(p.x - centre.x);
            let turn = (angle(to) - angle(from) + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
            pin.amount = (origin.amount + turn / std::f64::consts::TAU * kind.max()).clamp(0.0, kind.max());
        }
        BlurGrab::Axis { axis, .. } => {
            if let Some(mut shape) = origin.iris {
                let (dx, dy) = (to.x - centre.x, to.y - centre.y);
                let radius = dx.hypot(dy) / side;
                if axis % 2 == 0 {
                    shape.radius_x = radius;
                } else {
                    shape.radius_y = radius;
                }
                shape.angle = dy.atan2(dx).to_degrees() - 90.0 * axis as f64;
                pin.iris = Some(shape.clean());
            }
        }
        BlurGrab::Round { .. } => {
            if let Some(shape) = origin.iris {
                // The handle is the diagonal point (t, t), t = 2^(−1/m): t gives the exponent back.
                let (u, v) = iris_local(start, &origin, shape, to);
                let t = ((u.abs() + v.abs()) / 2.0).clamp(0.5f64.sqrt(), 0.5f64.powf(1.0 / 8.0));
                let exponent = -std::f64::consts::LN_2 / t.ln();
                pin.iris = Some(IrisShape { roundness: (exponent - 2.0) / 6.0 * 100.0, ..shape }.clean());
            }
        }
        BlurGrab::Feather { .. } => {
            if let Some(shape) = origin.iris {
                let (u, v) = iris_local(start, &origin, shape, to);
                let m = shape.exponent();
                let extent = (u.abs().powf(m) + v.abs().powf(m)).powf(1.0 / m);
                pin.iris = Some(IrisShape { feather: if extent.is_finite() { extent } else { shape.feather }, ..shape }.clean());
            }
        }
        BlurGrab::Turn { side: dot, .. } => {
            if let Some(band) = origin.tilt {
                // The dot's side of the band points from the centre towards the pointer: the
                // engine's normal is (sin a, cos a).
                let sign = f64::from(dot).signum();
                let (nx, ny) = ((to.x - centre.x) * sign, (to.y - centre.y) * sign);
                if nx.hypot(ny) > 1e-9 {
                    pin.tilt = Some(TiltBand { angle: nx.atan2(ny).to_degrees(), ..band }.clean());
                }
            }
        }
        BlurGrab::Focus { .. } => {
            if let Some(band) = origin.tilt {
                pin.tilt = Some(TiltBand { focus: tilt_offset(start, &origin, band, to).abs() / side, ..band }.clean());
            }
        }
        BlurGrab::Transition { .. } => {
            if let Some(band) = origin.tilt {
                pin.tilt = Some(TiltBand { transition: tilt_offset(start, &origin, band, to).abs() / side - band.focus, ..band }.clean());
            }
        }
        BlurGrab::New => {}
    }
    pins.pins[index] = pin;
    pins.selected = Some(index);
}

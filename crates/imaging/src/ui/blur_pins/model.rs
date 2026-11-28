//! The Blur Gallery's pins as typed data.

use serde::{Deserialize, Serialize};

use craft_behavior::kurbo::Point;

/// The smallest radius (a fraction of the shorter side), as the filter commands bound it.
const MIN_RADIUS: f64 = 0.01;
/// The largest feather.
const MAX_FEATHER: f64 = 0.99;

/// The most pins (PhotoCraft `MAX_PINS`: more than anyone places by hand; it bounds the
/// parameters and the per-pixel loop).
pub const MAX_PINS: usize = 64;

/// A new pin's amount when nothing is remembered: 15.
pub const DEFAULT_AMOUNT: f64 = 15.0;

/// The four Blur Gallery filters with pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BlurKind {
    /// Field Blur: the pins' amounts interpolate over the image.
    Field,
    /// Iris Blur: a superellipse per pin, sharp inside the feather.
    Iris,
    /// Spin Blur: an ellipse per pin; the amount is the blur angle.
    Spin,
    /// Tilt-Shift: one band.
    TiltShift,
}

impl BlurKind {
    /// The amount's top: 500 px of blur, or Spin's 360° blur angle.
    pub fn max(self) -> f64 {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:36-38, :66 `max`.
        if self == BlurKind::Spin { 360.0 } else { 500.0 }
    }
}

/// An Iris or Spin pin's shape.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct IrisShape {
    /// The horizontal radius as a fraction of the bounds' shorter side, 0.01..=1.
    pub radius_x: f64,
    /// The vertical radius, in the same unit.
    pub radius_y: f64,
    /// Degrees, clockwise on screen, −180..180.
    pub angle: f64,
    /// 0 (an ellipse) ..= 100 (a rounded rectangle); Iris only.
    pub roundness: f64,
    /// Where the blur starts, as a fraction of the shape, 0..=0.99; Iris only.
    pub feather: f64,
}

impl IrisShape {
    /// A shape of the given radii (fractions of the bounds' shorter side), angle (degrees), roundness and feather,
    /// as they are, before [`clean`](Self::clean).
    pub fn new(radius_x: f64, radius_y: f64, angle: f64, roundness: f64, feather: f64) -> IrisShape {
        IrisShape { radius_x, radius_y, angle, roundness, feather }
    }

    /// Iris Blur's default: radii 0.35 and 0.25, upright, an ellipse, feather 0.5.
    pub fn iris() -> IrisShape {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:109-114.
        IrisShape { radius_x: 0.35, radius_y: 0.25, angle: 0.0, roundness: 0.0, feather: 0.5 }
    }

    /// Spin Blur's default: a circle of radius 0.3.
    pub fn spin() -> IrisShape {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:118 `spin`.
        IrisShape { radius_x: 0.3, radius_y: 0.3, ..IrisShape::iris() }
    }

    /// The shape with every value finite and in range (non-finite values back to the defaults,
    /// the angle wrapped into −180..180).
    pub fn clean(self) -> IrisShape {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:122 `clean`.
        let finite = |value: f64, default: f64| if value.is_finite() { value } else { default };
        IrisShape {
            radius_x: finite(self.radius_x, 0.35).clamp(MIN_RADIUS, 1.0),
            radius_y: finite(self.radius_y, 0.25).clamp(MIN_RADIUS, 1.0),
            angle: (finite(self.angle, 0.0) + 180.0).rem_euclid(360.0) - 180.0,
            roundness: finite(self.roundness, 0.0).clamp(0.0, 100.0),
            feather: finite(self.feather, 0.5).clamp(0.0, MAX_FEATHER),
        }
    }

    /// The superellipse exponent the engine uses: 2 + roundness / 100 × 6 (2 an ellipse … 8).
    pub fn exponent(self) -> f64 {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:134 `exponent`.
        2.0 + self.roundness / 100.0 * 6.0
    }
}

/// Tilt-Shift's band.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TiltBand {
    /// Degrees, −90..=90; 0 a horizontal band, positive turns it anticlockwise on screen.
    pub angle: f64,
    /// The sharp band's half width beyond the centre line, as a fraction of the bounds' shorter
    /// side, 0..=1 (0.1).
    pub focus: f64,
    /// The transition's width beyond the sharp band, 0.01..=1 (0.15).
    pub transition: f64,
}

impl TiltBand {
    /// A band of the given angle (degrees), focus and transition (fractions of the bounds' shorter side), as they
    /// are, before [`clean`](Self::clean).
    pub fn new(angle: f64, focus: f64, transition: f64) -> TiltBand {
        TiltBand { angle, focus, transition }
    }

    /// The band with every value finite and in range (the angle folded into −90..=90: the band
    /// turned half a turn is the same).
    pub fn clean(self) -> TiltBand {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:150 `clean`.
        let finite = |value: f64, default: f64| if value.is_finite() { value } else { default };
        let mut angle = (finite(self.angle, 0.0) + 180.0).rem_euclid(360.0) - 180.0;
        if angle > 90.0 {
            angle -= 180.0;
        } else if angle < -90.0 {
            angle += 180.0;
        }
        TiltBand { angle, focus: finite(self.focus, 0.1).clamp(0.0, 1.0), transition: finite(self.transition, 0.15).clamp(0.01, 1.0) }
    }

    /// Tilt-Shift's default band: horizontal, focus 0.1, transition 0.15.
    pub fn default_band() -> TiltBand {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:160 (the `unwrap_or` in `set_pins`).
        TiltBand { angle: 0.0, focus: 0.1, transition: 0.15 }
    }

    /// The band's unit normal and its direction along the band (the engine's).
    pub fn axes(self) -> ([f64; 2], [f64; 2]) {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:163 `axes`.
        let (s, c) = self.angle.to_radians().sin_cos();
        ([s, c], [c, -s])
    }
}

/// One pin: its position as fractions of the bounds, its amount (blur in pixels, Spin's blur angle
/// in degrees), and an Iris or Spin pin's shape or the Tilt-Shift band.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct BlurPin {
    /// The horizontal position as a fraction of the bounds' width.
    pub x: f64,
    /// The vertical position as a fraction of the bounds' height.
    pub y: f64,
    /// Blur in pixels (Spin: blur angle in degrees).
    pub amount: f64,
    /// The shape of an Iris or Spin pin; `None` on a Tilt-Shift pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iris: Option<IrisShape>,
    /// The band of a Tilt-Shift pin; `None` on an Iris or Spin pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<TiltBand>,
}

impl BlurPin {
    /// A plain pin at the fractions `x` and `y` of the bounds with `amount` and no shape.
    pub fn new(x: f64, y: f64, amount: f64) -> BlurPin {
        BlurPin { x, y, amount, iris: None, tilt: None }
    }
}

/// The pins of one Blur Gallery filter (lead call 48: typed, where PhotoCraft kept a JSON map in
/// the dialog's fields): the kind, the filter's bounds (the selection's, else the canvas's) in
/// document pixels `[x0, y0, x1, y1]`, the pins, and the selected pin. The dialog's fields keep
/// the remembered amount and shape; the app maps `BlurPins` to the filter's `pins` parameters
/// (Tilt-Shift: `centerX`, `centerY`, `angle`, `focus`, `transition`, `blur`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlurPins {
    /// The filter these pins belong to.
    pub kind: BlurKind,
    /// The filter's bounds in document pixels, `[x0, y0, x1, y1]`.
    pub bounds: [f64; 4],
    /// The pins, in creation order.
    pub pins: Vec<BlurPin>,
    /// Index of the selected pin in `pins`, if any.
    pub selected: Option<usize>,
}

impl BlurPins {
    /// One pin in the middle of `bounds` with `start` (the remembered amount and shape), selected:
    /// what the filter opens with, and what Alt + Cancel (Reset) starts again from.
    pub fn new(kind: BlurKind, bounds: [f64; 4], start: BlurPin) -> BlurPins {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:171 `init`, :182 `reset`.
        let amount = if start.amount.is_finite() { start.amount } else { DEFAULT_AMOUNT };
        let centred = BlurPin { x: 0.5, y: 0.5, amount, iris: None, tilt: None };
        let pin = match kind {
            BlurKind::Field => centred,
            BlurKind::Iris => BlurPin { iris: Some(start.iris.unwrap_or_else(IrisShape::iris)), ..centred },
            BlurKind::Spin => BlurPin { iris: Some(start.iris.unwrap_or_else(IrisShape::spin)), ..centred },
            // Tilt-Shift keeps the remembered centre.
            BlurKind::TiltShift => BlurPin { x: start.x, y: start.y, tilt: Some(start.tilt.unwrap_or_else(TiltBand::default_band)), ..centred },
        };
        let mut pins = BlurPins { kind, bounds, pins: vec![pin], selected: Some(0) };
        pins.clean();
        pins
    }

    /// The bounds' shorter side in document pixels: the unit of radii and widths.
    pub fn short_side(&self) -> f64 {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:309 `short_side`.
        (self.bounds[2] - self.bounds[0]).min(self.bounds[3] - self.bounds[1]).max(1.0)
    }

    /// A pin's position in document pixels.
    pub fn to_document(&self, pin: &BlurPin) -> Point {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:313 `to_doc`.
        let b = self.bounds;
        Point::new(b[0] + pin.x * (b[2] - b[0]), b[1] + pin.y * (b[3] - b[1]))
    }

    /// A document point as fractions of the bounds, clamped to 0..=1 (pins stay inside).
    pub fn from_document(&self, at: Point) -> (f64, f64) {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:317 `from_doc`.
        let b = self.bounds;
        let fraction = |value: f64, low: f64, high: f64| ((value - low) / (high - low).max(f64::EPSILON)).clamp(0.0, 1.0);
        (fraction(at.x, b[0], b[2]), fraction(at.y, b[1], b[3]))
    }

    /// Adds a pin at `at` (inside the bounds; not for Tilt-Shift, not past [`MAX_PINS`]) with the
    /// selected pin's amount and shape, and selects it. Returns whether it was added.
    pub fn add(&mut self, at: Point) -> bool {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:571-583 (`Grab::New`).
        let b = self.bounds;
        let inside = (b[0]..=b[2]).contains(&at.x) && (b[1]..=b[3]).contains(&at.y);
        if self.kind == BlurKind::TiltShift || self.pins.len() >= MAX_PINS || !inside {
            return false;
        }
        let from = self.selected.and_then(|index| self.pins.get(index)).copied();
        let amount = from.map_or(DEFAULT_AMOUNT, |pin| pin.amount).clamp(0.0, self.kind.max());
        let iris = match self.kind {
            BlurKind::Iris => Some(from.and_then(|pin| pin.iris).unwrap_or_else(IrisShape::iris)),
            BlurKind::Spin => Some(from.and_then(|pin| pin.iris).unwrap_or_else(IrisShape::spin)),
            BlurKind::Field | BlurKind::TiltShift => None,
        };
        let (x, y) = self.from_document(at);
        self.pins.push(BlurPin { x, y, amount, iris, tilt: None });
        self.selected = Some(self.pins.len() - 1);
        true
    }

    /// Removes the selected pin unless it is the last; the one before it is selected.
    pub fn remove_selected(&mut self) -> bool {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:385 `remove_selected`.
        let Some(index) = self.selected.filter(|index| *index < self.pins.len()) else { return false };
        if self.kind == BlurKind::TiltShift || self.pins.len() <= 1 {
            return false;
        }
        self.pins.remove(index);
        self.selected = Some(index.saturating_sub(1));
        true
    }

    /// The pins cleaned: positions in 0..=1, amounts in 0..=[`BlurKind::max`], shapes and bands
    /// [`IrisShape::clean`] and [`TiltBand::clean`], at most [`MAX_PINS`], the selection in range.
    pub fn clean(&mut self) {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:228 `pins`.
        let kind = self.kind;
        // Tilt-Shift has one band.
        self.pins.truncate(if kind == BlurKind::TiltShift { 1 } else { MAX_PINS });
        for pin in &mut self.pins {
            let fraction = |value: f64| if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.5 };
            pin.x = fraction(pin.x);
            pin.y = fraction(pin.y);
            pin.amount = if pin.amount.is_finite() { pin.amount } else { DEFAULT_AMOUNT }.clamp(0.0, kind.max());
            pin.iris = match kind {
                BlurKind::Iris => Some(pin.iris.unwrap_or_else(IrisShape::iris).clean()),
                BlurKind::Spin => Some(pin.iris.unwrap_or_else(IrisShape::spin).clean()),
                BlurKind::Field | BlurKind::TiltShift => None,
            };
            pin.tilt = (kind == BlurKind::TiltShift).then(|| pin.tilt.unwrap_or_else(TiltBand::default_band).clean());
        }
        self.selected = self.selected.filter(|index| *index < self.pins.len());
    }
}

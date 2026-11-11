//! Primitive scales: space, radius, stroke, elevation, motion and opacity.
//!
//! Lengths are in logical pixels.

/// The spacing scale; semantic gaps and paddings pick from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Space {
    /// 0 px.
    S0,
    /// 2 px.
    S2,
    /// 4 px.
    S4,
    /// 6 px.
    S6,
    /// 8 px.
    S8,
    /// 12 px.
    S12,
    /// 16 px.
    S16,
    /// 20 px.
    S20,
    /// 24 px.
    S24,
    /// 32 px.
    S32,
}

impl Space {
    /// Every step, smallest first.
    pub const ALL: [Space; 10] = [Space::S0, Space::S2, Space::S4, Space::S6, Space::S8, Space::S12, Space::S16, Space::S20, Space::S24, Space::S32];

    /// The size in logical pixels.
    pub const fn px(self) -> f32 {
        match self {
            Space::S0 => 0.0,
            Space::S2 => 2.0,
            Space::S4 => 4.0,
            Space::S6 => 6.0,
            Space::S8 => 8.0,
            Space::S12 => 12.0,
            Space::S16 => 16.0,
            Space::S20 => 20.0,
            Space::S24 => 24.0,
            Space::S32 => 32.0,
        }
    }
}

/// The corner-radius scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Radius {
    /// Square corners.
    R0,
    /// 2 px.
    R2,
    /// 3 px.
    R3,
    /// 4 px.
    R4,
    /// 6 px.
    R6,
    /// 8 px.
    R8,
    /// 12 px.
    R12,
    /// Fully rounded (pill or circle).
    Full,
}

impl Radius {
    /// Every step, smallest first.
    pub const ALL: [Radius; 8] = [Radius::R0, Radius::R2, Radius::R3, Radius::R4, Radius::R6, Radius::R8, Radius::R12, Radius::Full];

    /// Radius in logical pixels. `Full` is 999, larger than any control, so renderers that clamp
    /// the radius to half the shortest side get a pill.
    pub const fn px(self) -> f32 {
        match self {
            Radius::R0 => 0.0,
            Radius::R2 => 2.0,
            Radius::R3 => 3.0,
            Radius::R4 => 4.0,
            Radius::R6 => 6.0,
            Radius::R8 => 8.0,
            Radius::R12 => 12.0,
            Radius::Full => 999.0,
        }
    }
}

/// The line-width scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stroke {
    /// One physical pixel, whatever the display scale. Resolve with [`Stroke::width`].
    Hairline,
    /// 1 logical px.
    W1,
    /// 1.5 logical px.
    W1_5,
    /// 2 logical px.
    W2,
    /// 3 logical px.
    W3,
}

impl Stroke {
    /// Every step, thinnest first.
    pub const ALL: [Stroke; 5] = [Stroke::Hairline, Stroke::W1, Stroke::W1_5, Stroke::W2, Stroke::W3];

    /// Width in logical pixels at `pixels_per_point` physical pixels per logical pixel.
    pub fn width(self, pixels_per_point: f32) -> f32 {
        match self {
            Stroke::Hairline => 1.0 / pixels_per_point.max(f32::EPSILON),
            Stroke::W1 => 1.0,
            Stroke::W1_5 => 1.5,
            Stroke::W2 => 2.0,
            Stroke::W3 => 3.0,
        }
    }
}

/// A drop shadow: how far it is offset, how soft and large it is, and how opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    /// Horizontal and vertical offset in logical pixels.
    pub offset: [f32; 2],
    /// Blur radius in logical pixels.
    pub blur: f32,
    /// Growth (positive) or shrink (negative) of the shadow rectangle in logical pixels.
    pub spread: f32,
    /// Opacity of the shadow colour, 0..=1.
    pub alpha: f32,
}

impl Elevation {
    /// Levels 0 (flat, no shadow) to 3 (highest), for a `float.shadow` override.
    pub const LEVELS: [Elevation; 4] = [
        Elevation { offset: [0.0, 0.0], blur: 0.0, spread: 0.0, alpha: 0.0 },
        Elevation { offset: [0.0, 1.0], blur: 3.0, spread: 0.0, alpha: 0.14 },
        Elevation { offset: [0.0, 4.0], blur: 12.0, spread: 0.0, alpha: 0.20 },
        Elevation { offset: [0.0, 10.0], blur: 28.0, spread: 0.0, alpha: 0.28 },
    ];

    /// The shadow for `level`; levels above 3 give level 3.
    pub const fn level(level: usize) -> Elevation {
        Self::LEVELS[if level > 3 { 3 } else { level }]
    }
}

/// The motion duration scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Duration {
    /// 80 ms: hover and press feedback.
    Fast,
    /// 150 ms: most state changes.
    Normal,
    /// 250 ms: large surfaces opening or moving.
    Slow,
}

impl Duration {
    /// The duration in milliseconds.
    pub const fn millis(self) -> u32 {
        match self {
            Duration::Fast => 80,
            Duration::Normal => 150,
            Duration::Slow => 250,
        }
    }
}

/// The easing curves for motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Easing {
    /// Movement within the screen.
    Standard,
    /// Elements entering: starts fast, settles slowly.
    Enter,
    /// Elements leaving: starts slowly, ends fast.
    Exit,
}

impl Easing {
    /// CSS-style cubic-bezier control points `[x1, y1, x2, y2]`.
    pub const fn bezier(self) -> [f32; 4] {
        match self {
            Easing::Standard => [0.2, 0.0, 0.0, 1.0],
            Easing::Enter => [0.0, 0.0, 0.0, 1.0],
            Easing::Exit => [0.3, 0.0, 1.0, 1.0],
        }
    }

    /// The curve's progress at time `t` (both 0 to 1), as CSS `cubic-bezier` computes it.
    pub fn at(self, t: f32) -> f32 {
        let [x1, y1, x2, y2] = self.bezier();
        // One coordinate of the curve with control values `a` and `b` at parameter `s`.
        let curve = |a: f32, b: f32, s: f32| ((1.0 - 3.0 * b + 3.0 * a) * s + (3.0 * b - 6.0 * a)) * s * s + 3.0 * a * s;
        let t = t.clamp(0.0, 1.0);
        // x is monotonic in s on [0, 1] for CSS curves (x1, x2 in 0..=1), so bisection finds s.
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if curve(x1, x2, mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        curve(y1, y2, (lo + hi) / 2.0)
    }
}

/// Opacity primitives.
pub struct Opacity;

impl Opacity {
    /// Multiplier for disabled controls.
    pub const DISABLED: f32 = 0.4;
    /// Alpha of a window or layer while it moves over a drop target, so what lies under it shows through.
    pub const MOVING: f32 = 0.75;
    /// Alpha of a mark the app derived, drawn beside the user's own marks of the same kind so those stand out.
    pub const DERIVED: f32 = 0.7;
}

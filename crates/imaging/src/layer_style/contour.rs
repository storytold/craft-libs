//! Contours: the curves that shape a glow, a shadow, a bevel's gloss and Satin.

/// A contour point, input and output in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContourPoint {
    /// Input value, 0..=1 (the horizontal axis).
    pub input: f32,
    /// Output value, 0..=1 (the vertical axis).
    pub output: f32,
    /// A corner point: the curve runs straight into and out of it.
    pub corner: bool,
}

/// A named contour: at least two points sorted by input, at most [`Contour::MAX_POINTS`].
/// Built-in presets carry their preset name; an edited preset is named "Custom"; imported
/// contours keep their own name and points.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    #[allow(missing_docs)]
    pub name: String,
    /// The points, sorted by input.
    pub points: Vec<ContourPoint>,
}

impl Contour {
    /// The most points a contour holds.
    // pc: crates/engine/src/layer_style.rs:370 `MAX_CONTOUR_POINTS`.
    pub const MAX_POINTS: usize = 64;

    /// The identity contour "Linear": `(0, 0)` to `(1, 1)`.
    pub fn linear() -> Contour {
        // pc: crates/engine/src/layer_style.rs:121.
        Contour {
            name: "Linear".into(),
            points: vec![ContourPoint { input: 0.0, output: 0.0, corner: false }, ContourPoint { input: 1.0, output: 1.0, corner: false }],
        }
    }

    /// The built-in presets in the order of Photoshop's default contour picker, each Photoshop's
    /// own curve as PhotoCraft measured it in Photoshop 27.11 (every inner point a corner):
    /// Linear, Cone, Cone - Inverted, Cove - Deep, Cove - Shallow, Gaussian, Half Round, Ring,
    /// Ring - Double, Rolling Slope - Descending, Rounded Steps, Sawtooth 1. Copy the point
    /// lists from the cited table.
    pub fn presets() -> Vec<Contour> {
        // pc: crates/engine/src/layer_style.rs:120-360 `CONTOURS`.
        CONTOURS.iter().map(|(name, points)| from_raw(name, points)).collect()
    }

    /// The preset called `name`, also accepting the names earlier PhotoCraft versions offered
    /// (Cone (Inverted), Domed, Domed (Inverted), Diagonal (Descending)); "linear" in any case is
    /// [`linear`](Self::linear). `None` for a name not shipped, so the host keeps an imported
    /// contour as it is.
    pub fn preset(name: &str) -> Option<Contour> {
        // pc: crates/engine/src/layer_style.rs:362-367 `LEGACY_CONTOURS`, :383-388 `builtin_contour`.
        if name.eq_ignore_ascii_case("Linear") {
            return Some(Contour::linear());
        }
        CONTOURS.iter().skip(1).chain(LEGACY_CONTOURS).find(|(n, _)| *n == name).map(|(n, points)| from_raw(n, points))
    }

    /// Whether `self` is a built-in preset unchanged (its name and points match the preset).
    pub fn is_preset(&self) -> bool {
        // pc: crates/engine/src/layer_style.rs:397 (a preset is stored by its name).
        Contour::preset(&self.name).as_ref() == Some(self)
    }

    /// `points` made valid: `None` for fewer than two or more than [`MAX_POINTS`](Self::MAX_POINTS)
    /// points or a non-finite value; else clamped to 0..=1, sorted by input, points within 1e-6
    /// of the same input collapsed (the first kept), `None` if fewer than two remain.
    pub fn from_points(name: impl Into<String>, points: Vec<ContourPoint>) -> Option<Contour> {
        // pc: crates/engine/src/layer_style.rs:415-444 `contour_from_param`.
        if points.len() < 2 || points.len() > Self::MAX_POINTS {
            return None;
        }
        if points.iter().any(|p| !p.input.is_finite() || !p.output.is_finite()) {
            return None;
        }
        let mut points: Vec<ContourPoint> =
            points.into_iter().map(|p| ContourPoint { input: p.input.clamp(0.0, 1.0), output: p.output.clamp(0.0, 1.0), corner: p.corner }).collect();
        points.sort_by(|a, b| a.input.total_cmp(&b.input));
        points.dedup_by(|a, b| (a.input - b.input).abs() < 1e-6);
        if points.len() < 2 {
            return None;
        }
        Some(Contour { name: name.into(), points })
    }

    /// The contour after an edit in the Contour Editor: a built-in preset's name becomes
    /// "Custom" (an empty name too), so a saved file never calls the edited curve by the preset's
    /// name; another name is kept.
    pub fn edited(mut self) -> Contour {
        // pc: crates/ui-egui/src/dialogs/layer_style/contour.rs:255-266 `store`.
        if self.name.is_empty() || Contour::preset(&self.name).is_some() {
            self.name = "Custom".into();
        }
        self
    }
}

/// A table point as PhotoCraft has it: input, output, corner.
type RawPoint = (f32, f32, bool);

fn from_raw(name: &str, points: &[RawPoint]) -> Contour {
    Contour { name: name.into(), points: points.iter().map(|&(input, output, corner)| ContourPoint { input, output, corner }).collect() }
}

const CONTOURS: &[(&str, &[RawPoint])] = &[
    ("Linear", &[(0.0, 0.0, false), (1.0, 1.0, false)]),
    (
        "Cone",
        &[
            (0.0, 0.0, false),
            (0.172, 0.477, true),
            (0.195, 0.555, true),
            (0.266, 0.711, true),
            (0.383, 0.922, true),
            (0.43, 0.969, true),
            (0.477, 0.992, true),
            (0.523, 0.992, true),
            (0.57, 0.969, true),
            (0.664, 0.859, true),
            (0.703, 0.789, true),
            (0.742, 0.711, true),
            (0.836, 0.477, true),
            (0.859, 0.398, true),
            (0.914, 0.266, true),
            (0.945, 0.156, true),
            (0.977, 0.078, true),
            (1.0, 0.0, false),
        ],
    ),
    (
        "Cone - Inverted",
        &[
            (0.0, 1.0, false),
            (0.031, 0.922, true),
            (0.07, 0.789, true),
            (0.172, 0.523, true),
            (0.195, 0.445, true),
            (0.266, 0.289, true),
            (0.383, 0.078, true),
            (0.43, 0.031, true),
            (0.477, 0.008, true),
            (0.523, 0.008, true),
            (0.57, 0.031, true),
            (0.664, 0.141, true),
            (0.703, 0.211, true),
            (0.742, 0.289, true),
            (0.836, 0.523, true),
            (0.859, 0.602, true),
            (0.914, 0.734, true),
            (0.945, 0.844, true),
            (0.977, 0.922, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Cove - Deep",
        &[
            (0.0, 0.258, false),
            (0.344, 0.289, true),
            (0.383, 0.297, true),
            (0.477, 0.352, true),
            (0.57, 0.43, true),
            (0.617, 0.484, true),
            (0.664, 0.57, true),
            (0.703, 0.711, true),
            (0.742, 0.906, true),
            (0.773, 1.0, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Cove - Shallow",
        &[
            (0.0, 0.0, false),
            (0.008, 0.164, true),
            (0.016, 0.477, true),
            (0.039, 0.508, true),
            (0.172, 0.57, true),
            (0.305, 0.688, true),
            (0.344, 0.734, true),
            (0.477, 0.945, true),
            (0.523, 1.0, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Gaussian",
        &[
            (0.0, 0.0, false),
            (0.117, 0.023, true),
            (0.172, 0.055, true),
            (0.234, 0.117, true),
            (0.305, 0.234, true),
            (0.383, 0.414, true),
            (0.523, 0.688, true),
            (0.617, 0.812, true),
            (0.703, 0.891, true),
            (0.805, 0.953, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Half Round",
        &[
            (0.0, 0.0, false),
            (0.07, 0.18, true),
            (0.195, 0.445, true),
            (0.266, 0.555, true),
            (0.383, 0.703, true),
            (0.523, 0.828, true),
            (0.617, 0.883, true),
            (0.773, 0.945, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Ring",
        &[
            (0.0, 0.0, false),
            (0.031, 0.078, true),
            (0.039, 0.117, true),
            (0.117, 0.383, true),
            (0.172, 0.609, true),
            (0.195, 0.734, true),
            (0.266, 0.953, true),
            (0.305, 1.0, true),
            (0.344, 0.992, true),
            (0.383, 0.922, true),
            (0.43, 0.805, true),
            (0.477, 0.641, true),
            (0.57, 0.281, true),
            (0.617, 0.141, true),
            (0.664, 0.055, true),
            (0.703, 0.016, true),
            (0.742, 0.016, true),
            (0.773, 0.062, true),
            (0.836, 0.234, true),
            (0.859, 0.344, true),
            (0.891, 0.445, true),
            (0.914, 0.547, true),
            (0.945, 0.734, true),
            (0.977, 0.875, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Ring - Double",
        &[
            (0.0, 0.0, false),
            (0.055, 0.086, true),
            (0.094, 0.172, true),
            (0.117, 0.242, true),
            (0.172, 0.477, true),
            (0.195, 0.625, true),
            (0.266, 0.891, true),
            (0.305, 0.945, true),
            (0.344, 0.891, true),
            (0.383, 0.695, true),
            (0.43, 0.367, true),
            (0.477, 0.07, true),
            (0.523, 0.047, true),
            (0.57, 0.32, true),
            (0.617, 0.648, true),
            (0.664, 0.867, true),
            (0.703, 0.93, true),
            (0.742, 0.852, true),
            (0.773, 0.641, true),
            (0.805, 0.375, true),
            (0.836, 0.156, true),
            (0.859, 0.031, true),
            (0.891, 0.023, true),
            (0.914, 0.109, true),
            (0.977, 0.703, true),
            (0.984, 0.82, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Rolling Slope - Descending",
        &[
            (0.0, 1.0, false),
            (0.008, 1.0, true),
            (0.031, 0.93, true),
            (0.039, 0.883, true),
            (0.07, 0.758, true),
            (0.141, 0.539, true),
            (0.195, 0.406, true),
            (0.266, 0.352, true),
            (0.305, 0.352, true),
            (0.523, 0.57, true),
            (0.57, 0.602, true),
            (0.617, 0.617, true),
            (0.703, 0.602, true),
            (0.742, 0.562, true),
            (0.836, 0.391, true),
            (0.914, 0.211, true),
            (0.945, 0.109, true),
            (0.977, 0.039, true),
            (0.984, 0.008, true),
            (1.0, 0.0, false),
        ],
    ),
    (
        "Rounded Steps",
        &[
            (0.0, 0.234, false),
            (0.031, 0.305, true),
            (0.039, 0.336, true),
            (0.094, 0.43, true),
            (0.141, 0.477, true),
            (0.234, 0.492, true),
            (0.266, 0.516, true),
            (0.305, 0.602, true),
            (0.344, 0.664, true),
            (0.383, 0.711, true),
            (0.477, 0.734, true),
            (0.523, 0.797, true),
            (0.57, 0.898, true),
            (0.617, 0.953, true),
            (0.664, 0.984, true),
            (0.773, 0.984, true),
            (0.836, 0.906, true),
            (0.891, 0.891, true),
            (0.914, 0.898, true),
            (1.0, 1.0, false),
        ],
    ),
    (
        "Sawtooth 1",
        &[
            (0.0, 0.0, false),
            (0.031, 0.047, true),
            (0.195, 0.375, true),
            (0.266, 0.492, true),
            (0.477, 0.047, true),
            (0.523, 0.055, true),
            (0.742, 0.469, true),
            (0.773, 0.445, true),
            (0.914, 0.18, true),
            (0.945, 0.102, true),
            (1.0, 0.0, false),
        ],
    ),
];

const LEGACY_CONTOURS: &[(&str, &[RawPoint])] = &[
    ("Cone (Inverted)", &[(0.0, 1.0, false), (0.5, 0.0, false), (1.0, 0.0, false)]),
    ("Domed", &[(0.0, 0.0, false), (0.25, 1.0, false), (0.75, 1.0, false), (1.0, 0.0, false)]),
    ("Domed (Inverted)", &[(0.0, 1.0, false), (0.25, 0.0, false), (0.75, 0.0, false), (1.0, 1.0, false)]),
    ("Diagonal (Descending)", &[(0.0, 1.0, false), (1.0, 0.0, false)]),
];

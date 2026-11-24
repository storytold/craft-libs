//! Per-shape production metadata: what the machine does with a shape (operation, colour and
//! material keys for mats and cut groups), Attach groups, hidden contours, a manual mat, and the
//! retained source shapes of a non-destructive Combine or a Flatten. Everything defaults to
//! "unset" and is skipped when saving, so older files and files without production data are
//! unchanged.

use kurbo::{Affine, BezPath, PathEl};
use serde::{Deserialize, Serialize};

use crate::geom::check_finite_mm;
use crate::{DocError, Shape, Tip};

/// The physical process a pass runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Process {
    Laser,
    Mill,
    Knife,
    Pen,
    Creaser,
    Punch,
    Print,
}

/// The cutting strategy within a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    Line,
    Fill,
    Pocket,
    Drill,
    Score,
    KissCut,
    DieCut,
    Perforate,
    Guide,
}

/// One bridge tab keeping a cut part in the sheet: starts `offset_mm` along the
/// contour, `width_mm` wide.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub offset_mm: f64,
    pub width_mm: f64,
}

/// The complete per-operation settings snapshot a layer carries (D-09). Emission of
/// these settings is a later phase; here they are stored and validated only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OperationSettings {
    /// Laser power, percent.
    pub power: f64,
    pub speed_mm_per_min: f64,
    pub passes: u32,
    pub focus_mm: f64,
    pub kerf_mm: f64,
    pub tabs: Vec<Tab>,
    /// Score/emboss tip, when the pass uses one.
    pub tip: Option<Tip>,
    /// Emboss from the reverse side (deboss) instead of the front.
    pub deboss: bool,
    /// Punch diameter for `Operation::Punch`.
    pub punch_diameter_mm: Option<f64>,
}

impl Default for OperationSettings {
    fn default() -> Self {
        Self {
            power: 0.0,
            speed_mm_per_min: 0.0,
            passes: 1,
            focus_mm: 0.0,
            kerf_mm: 0.0,
            tabs: Vec::new(),
            tip: None,
            deboss: false,
            punch_diameter_mm: None,
        }
    }
}

/// A production pass a layer runs over all of its member shapes: a cooked preset
/// (D-07) tagged with its kind, carrying the full settings snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Operation {
    /// Cut the outline.
    Cut(OperationSettings),
    /// Sticker kiss cut: through the face material only, the backing stays whole.
    KissCut(OperationSettings),
    /// Sticker die cut: through the face material and the backing.
    DieCut(OperationSettings),
    /// Draw the outline with a pen.
    Draw(OperationSettings),
    /// Crease a fold line.
    Score(OperationSettings),
    /// Engrave the surface.
    Engrave(OperationSettings),
    /// A dashed tear line.
    Perforate(OperationSettings),
    /// Print on a home printer, then cut around the print using registration marks.
    PrintThenCut(OperationSettings),
    /// One punch per contour.
    Punch(OperationSettings),
    /// Shown on the canvas only, never sent to the machine or printer.
    Guide,
}

impl Operation {
    /// The settings snapshot; `None` for `Guide`.
    pub fn settings(&self) -> Option<&OperationSettings> {
        match self {
            Operation::Guide => None,
            Operation::Cut(s)
            | Operation::KissCut(s)
            | Operation::DieCut(s)
            | Operation::Draw(s)
            | Operation::Score(s)
            | Operation::Engrave(s)
            | Operation::Perforate(s)
            | Operation::PrintThenCut(s)
            | Operation::Punch(s) => Some(s),
        }
    }

    /// The physical process this pass runs on; `None` for `Guide`.
    pub fn process(&self) -> Option<Process> {
        Some(match self {
            Operation::Cut(_) | Operation::KissCut(_) | Operation::DieCut(_) | Operation::Engrave(_) | Operation::Perforate(_) => Process::Laser,
            Operation::Draw(_) => Process::Pen,
            Operation::Score(_) => Process::Creaser,
            Operation::Punch(_) => Process::Punch,
            Operation::PrintThenCut(_) => Process::Print,
            Operation::Guide => return None,
        })
    }

    /// The cutting strategy within the process.
    pub fn strategy(&self) -> Strategy {
        match self {
            Operation::Cut(_) | Operation::Draw(_) | Operation::PrintThenCut(_) => Strategy::Line,
            Operation::KissCut(_) => Strategy::KissCut,
            Operation::DieCut(_) => Strategy::DieCut,
            Operation::Score(_) => Strategy::Score,
            Operation::Engrave(_) => Strategy::Fill,
            Operation::Perforate(_) => Strategy::Perforate,
            Operation::Punch(_) => Strategy::Drill,
            Operation::Guide => Strategy::Guide,
        }
    }

    /// Whether this pass is canvas-only guidance and excluded from production output.
    pub fn is_guide(&self) -> bool {
        matches!(self, Operation::Guide)
    }

    /// Kebab-case wire name of the pass.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Operation::Cut(_) => "cut",
            Operation::KissCut(_) => "kiss-cut",
            Operation::DieCut(_) => "die-cut",
            Operation::Draw(_) => "draw",
            Operation::Score(_) => "score",
            Operation::Engrave(_) => "engrave",
            Operation::Perforate(_) => "perforate",
            Operation::PrintThenCut(_) => "print-then-cut",
            Operation::Punch(_) => "punch",
            Operation::Guide => "guide",
        }
    }

    /// Rejects non-finite or nonsensical settings before they reach a file or machine.
    pub fn validate(&self) -> Result<(), DocError> {
        let Some(s) = self.settings() else { return Ok(()) };
        let name = self.kind_name();
        let finite = |value: f64, field: &'static str| check_finite_mm(value, field).map_err(|e| DocError::Model(format!("operation {name}: {e}")));
        finite(s.power, "power")?;
        finite(s.speed_mm_per_min, "speed")?;
        finite(s.focus_mm, "focus")?;
        finite(s.kerf_mm, "kerf")?;
        if s.power < 0.0 {
            return Err(DocError::Model(format!("operation {name}: power cannot be negative")));
        }
        if s.speed_mm_per_min < 0.0 {
            return Err(DocError::Model(format!("operation {name}: speed cannot be negative")));
        }
        if s.kerf_mm < 0.0 {
            return Err(DocError::Model(format!("operation {name}: kerf cannot be negative")));
        }
        if s.passes < 1 {
            return Err(DocError::Model(format!("operation {name}: passes must be at least 1")));
        }
        if let Some(diameter) = s.punch_diameter_mm {
            finite(diameter, "punch diameter")?;
            if diameter <= 0.0 {
                return Err(DocError::Model(format!("operation {name}: punch diameter must be positive")));
            }
        }
        for (i, tab) in s.tabs.iter().enumerate() {
            finite(tab.offset_mm, "tab offset")?;
            finite(tab.width_mm, "tab width")?;
            if tab.width_mm < 0.0 {
                return Err(DocError::Model(format!("operation {name}: tab {i} width cannot be negative")));
            }
        }
        Ok(())
    }
}

/// A per-shape tool assignment from the send preview, overriding the group settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendAssign {
    /// Every line of the shape with tool 1.
    Tool1,
    /// Only the shape's outer outline with tool 1 (inner contours are skipped).
    Tool1Outline,
    Tool2,
    Tool2Outline,
    /// Leave the shape out of the job.
    Ignore,
}

/// A Combine's shape operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CombineOp {
    Unite,
    /// The back-most member minus every other member.
    Subtract,
    Intersect,
    /// Area covered by an odd number of members.
    Exclude,
}

/// Why a shape keeps its sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "operation")]
pub enum GroupKind {
    /// Non-destructive Combine: the path is the operation's result; Uncombine restores the
    /// members.
    Combine(CombineOp),
    /// Flatten: the members are the print artwork, the path is its cut outline.
    Flatten,
}

/// The shapes a Combine or Flatten replaced, kept so the action can be undone later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceGroup {
    pub kind: GroupKind,
    pub members: Vec<Shape>,
    /// The grouped shape's path when the group was made. Comparing it with the current path
    /// tells how the group was moved, rotated or scaled since, so the members follow.
    pub path: BezPath,
}

impl SourceGroup {
    /// The members moved the way the grouped shape moved since grouping (`current` is its
    /// path now). An edit that is not an affine map of the old outline (node edits) falls back
    /// to mapping the old bounds onto the new ones.
    pub fn members_now(&self, current: &BezPath) -> Vec<Shape> {
        let map = fit_affine(&self.path, current).unwrap_or_else(|| bounds_map(&self.path, current));
        self.members
            .iter()
            .map(|m| {
                let mut m = m.clone();
                m.path = map * m.path;
                m
            })
            .collect()
    }
}

fn endpoints(path: &BezPath) -> Vec<kurbo::Point> {
    path.elements().iter().filter_map(|el| el.end_point()).collect()
}

/// The affine map taking `old` onto `new` point for point, when the two have the same
/// structure and one exists (within 0.01 mm).
fn fit_affine(old: &BezPath, new: &BezPath) -> Option<Affine> {
    let (a, b) = (endpoints(old), endpoints(new));
    if a.len() != b.len() || a.len() < 3 {
        return None;
    }
    let p0 = *a.first()?;
    // The point farthest from p0, then the one making the largest triangle.
    let i = (0..a.len()).max_by(|&i, &j| (a[i] - p0).hypot2().total_cmp(&(a[j] - p0).hypot2()))?;
    let j = (0..a.len()).max_by(|&j, &k| (a[i] - p0).cross(a[j] - p0).abs().total_cmp(&(a[i] - p0).cross(a[k] - p0).abs()))?;
    let (u, v) = (a[i] - p0, a[j] - p0);
    let det = u.cross(v);
    if det.abs() < 1e-9 {
        // Collinear outline (or no movement information): a pure translation still works.
        let t = Affine::translate(*b.first()? - p0);
        return a.iter().zip(&b).all(|(p, q)| (t * *p - *q).hypot() < 0.01).then_some(t);
    }
    let q0 = *b.first()?;
    let (s, t) = (b[i] - q0, b[j] - q0);
    // M maps u→s and v→t: M = [s t]·[u v]⁻¹.
    let inv = [v.y / det, -v.x / det, -u.y / det, u.x / det];
    let m = [s.x * inv[0] + t.x * inv[2], s.y * inv[0] + t.y * inv[2], s.x * inv[1] + t.x * inv[3], s.y * inv[1] + t.y * inv[3]];
    let linear = Affine::new([m[0], m[1], m[2], m[3], 0.0, 0.0]);
    let map = Affine::translate(q0.to_vec2()) * linear * Affine::translate(-p0.to_vec2());
    a.iter().zip(&b).all(|(p, q)| (map * *p - *q).hypot() < 0.01).then_some(map)
}

fn bounds_map(old: &BezPath, new: &BezPath) -> Affine {
    use kurbo::Shape as _;
    let (a, b) = (old.bounding_box(), new.bounding_box());
    let sx = if a.width() > 1e-9 { b.width() / a.width() } else { 1.0 };
    let sy = if a.height() > 1e-9 { b.height() / a.height() } else { 1.0 };
    Affine::translate((b.x0, b.y0)) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-a.x0, -a.y0))
}

/// Production metadata of one shape. Every field is optional. The v1 `operation`
/// field is gone from v2 (legacy read-only, deserialized by `wire_v1` only).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeProduction {
    /// Canonical `#rrggbb` line colour: the cut-by-line-colour key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_color: Option<String>,
    /// Canonical `#rrggbb` fill colour: the print colour and cut-by-fill-colour key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    /// Material name; shapes on different materials go on different mats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Attach group: shapes sharing it keep their relative positions on one mat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach: Option<u64>,
    /// Manual mat: the piece goes on this mat index instead of its automatic one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mat: Option<u32>,
    /// Tool assignment from the send preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<SendAssign>,
    /// Contours (subpath indices) hidden by Contour: kept in the path, left out of exports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_contours: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Box<SourceGroup>>,
}

impl ShapeProduction {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Shape {
    /// The path the machine sees: the shape's path without the contours Contour hid.
    pub fn cut_path(&self) -> std::borrow::Cow<'_, BezPath> {
        if self.production.hidden_contours.is_empty() {
            return std::borrow::Cow::Borrowed(&self.path);
        }
        let mut out = BezPath::new();
        let mut contour = 0usize;
        let mut started = false;
        for el in self.path.elements() {
            if matches!(el, PathEl::MoveTo(_)) {
                if started {
                    contour += 1;
                }
                started = true;
            }
            if !self.production.hidden_contours.contains(&contour) {
                out.push(*el);
            }
        }
        std::borrow::Cow::Owned(out)
    }
}

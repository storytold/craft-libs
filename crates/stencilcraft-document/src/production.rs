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

/// The kind of a production pass: what the machine does with the member shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationKind {
    /// Cut the outline.
    Cut,
    /// Sticker kiss cut: through the face material only, the backing stays whole.
    KissCut,
    /// Sticker die cut: through the face material and the backing.
    DieCut,
    /// Draw the outline with a pen.
    Draw,
    /// Crease a fold line.
    Score,
    /// Engrave the surface.
    Engrave,
    /// A dashed tear line.
    Perforate,
    /// Print on a home printer, then cut around the print using registration marks.
    PrintThenCut,
    /// One punch per contour.
    Punch,
    /// Shown on the canvas only, never sent to the machine or printer.
    Guide,
}

impl OperationKind {
    /// The physical process this pass runs on; `None` for `Guide`.
    pub fn process(&self) -> Option<Process> {
        Some(match self {
            OperationKind::Cut | OperationKind::KissCut | OperationKind::DieCut | OperationKind::Engrave | OperationKind::Perforate => Process::Laser,
            OperationKind::Draw => Process::Pen,
            OperationKind::Score => Process::Creaser,
            OperationKind::Punch => Process::Punch,
            OperationKind::PrintThenCut => Process::Print,
            OperationKind::Guide => return None,
        })
    }

    /// The cutting strategy within the process.
    pub fn strategy(&self) -> Strategy {
        match self {
            OperationKind::Cut | OperationKind::Draw | OperationKind::PrintThenCut => Strategy::Line,
            OperationKind::KissCut => Strategy::KissCut,
            OperationKind::DieCut => Strategy::DieCut,
            OperationKind::Score => Strategy::Score,
            OperationKind::Engrave => Strategy::Fill,
            OperationKind::Perforate => Strategy::Perforate,
            OperationKind::Punch => Strategy::Drill,
            OperationKind::Guide => Strategy::Guide,
        }
    }

    /// Whether this pass is canvas-only guidance and excluded from production output.
    pub fn is_guide(&self) -> bool {
        matches!(self, OperationKind::Guide)
    }

    /// Kebab-case wire name of the pass.
    pub fn kind_name(&self) -> &'static str {
        match self {
            OperationKind::Cut => "cut",
            OperationKind::KissCut => "kiss-cut",
            OperationKind::DieCut => "die-cut",
            OperationKind::Draw => "draw",
            OperationKind::Score => "score",
            OperationKind::Engrave => "engrave",
            OperationKind::Perforate => "perforate",
            OperationKind::PrintThenCut => "print-then-cut",
            OperationKind::Punch => "punch",
            OperationKind::Guide => "guide",
        }
    }
}

/// A first-class production entity (GEO-05): one pass over its `members`, in
/// `Document::operations` order. Geometry is never duplicated here — `members`
/// holds shape ids only, and a shape may belong to many operations (N:M).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    /// Stable identity; unique within the document.
    pub id: u64,
    pub name: String,
    pub kind: OperationKind,
    /// The full settings snapshot for the pass (D-09). Emission is a later phase.
    #[serde(default)]
    pub settings: OperationSettings,
    /// Material colour label, `#rrggbb`; display/filter only, never identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Whether exports and cuts include this operation.
    #[serde(default = "crate::yes")]
    pub visible: bool,
    /// A locked operation's shapes cannot be selected or edited.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub locked: bool,
    /// Member shape ids, in pass order (D-20). Geometry lives once in
    /// `Document::shapes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<u64>,
    /// Optional icon name for the operations panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl Operation {
    /// Whether exports and cuts include this operation: visible, not Guide.
    pub fn is_output(&self) -> bool {
        self.visible && !self.kind.is_guide()
    }

    /// Kebab-case wire name of the pass kind.
    pub fn kind_name(&self) -> &'static str {
        self.kind.kind_name()
    }

    /// Rejects non-finite or nonsensical settings before they reach a file or machine.
    pub fn validate(&self) -> Result<(), DocError> {
        let s = &self.settings;
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

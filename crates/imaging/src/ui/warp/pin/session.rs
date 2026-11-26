//! [`PinWarpSession`]: the pins of a Puppet Warp while it is open, and the [`PinSolver`] the app
//! plugs in.

use std::collections::BTreeSet;
use std::sync::Arc;

use kurbo::{Point, Rect};

use craft_behavior::edit_history::Step;
use craft_behavior::{EditEvent, EditHistory, Mixed};

/// One pin: where it attaches on the rest mesh and where it holds that point now, in document
/// units (PhotoCraft `PuppetPin { src, dst, rotate, depth }`, VectorCraft `rest`, `at`, `angle`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Pin {
    /// The pin's attachment point on the rest mesh.
    pub rest: Point,
    /// Where the pin holds that point now.
    pub at: Point,
    /// The fixed rotation of the mesh around the pin, in degrees; `None` is Auto (LC15).
    pub angle: Option<f64>,
    /// Where the mesh overlaps itself, triangles nearest a higher pin draw on top (LC16).
    pub depth: i32,
}

impl Pin {
    /// A pin attached at `rest` and held at `at`, with Auto rotation and depth 0.
    pub fn new(rest: Point, at: Point) -> Self {
        Self { rest, at, angle: None, depth: 0 }
    }
}

/// How stiff the mesh is (Photoshop's Mode).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PinMode {
    /// Keeps the mesh stiff.
    Rigid,
    #[default]
    /// The default.
    Normal,
    /// Lets the mesh bend freely.
    Distort,
}

/// How many points the mesh has (Photoshop's Density).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PinDensity {
    /// A coarse mesh.
    Fewer,
    #[default]
    /// The default.
    Normal,
    /// A fine mesh.
    More,
}

/// What the solver reads besides the pins: mode, density and expansion.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct PinWarpOptions {
    /// How stiff the mesh is.
    pub mode: PinMode,
    /// How many points the mesh has.
    pub density: PinDensity,
    /// Grows (or, negative, shrinks) the mesh beyond the opaque pixels, in document pixels; 2 by
    /// default, −50 to 100 in the bar (PhotoCraft).
    pub expansion: f64,
}

impl Default for PinWarpOptions {
    fn default() -> Self {
        Self { mode: PinMode::default(), density: PinDensity::default(), expansion: 2.0 }
    }
}

/// Puppet Warp's persisted section: the options and Show Mesh, kept between runs (Photoshop; LC17).
/// Unknown fields are ignored, missing ones take their defaults.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct PinWarpSettings {
    /// The solver's options.
    #[serde(flatten)]
    pub options: PinWarpOptions,
    /// Show Mesh; on by default.
    pub show_mesh: bool,
}

impl Default for PinWarpSettings {
    fn default() -> Self {
        Self { options: PinWarpOptions::default(), show_mesh: true }
    }
}

/// The app's pin-warp solver (PhotoCraft's `PuppetSolver`, VectorCraft's ARAP): the session calls
/// it whenever the pins or the options change, and the overlay and the preview read its mesh. The
/// solver itself stays in the app until the engine step (step 7 plan, question 1).
pub trait PinSolver {
    /// Pins were added or removed, or a pin's rotation was fixed or freed: rebuild what depends on
    /// the pin set, then solve.
    fn rebind(&mut self, pins: &[Pin], options: &PinWarpOptions);

    /// Pin positions, angles or depths changed: solve again from the rest mesh, never warm-started,
    /// so the result does not depend on the pointer's history.
    fn solve(&mut self, pins: &[Pin], options: &PinWarpOptions);

    /// Density or expansion changed: rebuild the mesh. `Err` keeps the previous mesh; the text is
    /// shown as a notice ("Could not use Puppet Warp: no opaque pixels at this density and
    /// expansion").
    fn remesh(&mut self, options: &PinWarpOptions) -> Result<(), String>;

    /// The mesh as it is now.
    fn mesh(&self) -> PinMeshView<'_>;
}

/// The solver's mesh, lent for one pass: the rest and deformed vertices, the triangles, and the
/// order to draw the triangles in (back to front).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PinMeshView<'a> {
    /// The rest vertices.
    pub rest: &'a [Point],
    /// The deformed vertices, index for index with `rest`.
    pub deformed: &'a [Point],
    /// Triangles as indices into the vertices.
    pub triangles: &'a [[u32; 3]],
    /// The triangle indices in draw order, back to front.
    pub order: &'a [u32],
}

impl<'a> PinMeshView<'a> {
    /// A mesh from the solver's vertices, triangles and draw order.
    pub fn new(rest: &'a [Point], deformed: &'a [Point], triangles: &'a [[u32; 3]], order: &'a [u32]) -> Self {
        Self { rest, deformed, triangles, order }
    }
}

impl PinMeshView<'_> {
    /// The rest position of the deformed mesh's point `at`, where a new pin attaches: the first
    /// triangle containing `at` searched from the top of the draw order, by barycentric
    /// coordinates (degenerate triangles skipped). `None` off the mesh (LC18).
    pub fn rest_of(&self, at: Point) -> Option<Point> {
        // pc: crates/ui-egui/src/tools/puppet_ui.rs:165 (`rest_of`).
        let vertex = |points: &[Point], i: u32| points.get(i as usize).copied();
        for &t in self.order.iter().rev() {
            let Some(tri) = self.triangles.get(t as usize) else { continue };
            let (Some(a), Some(b), Some(c)) = (vertex(self.deformed, tri[0]), vertex(self.deformed, tri[1]), vertex(self.deformed, tri[2])) else {
                continue;
            };
            let det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
            if det.abs() < 1e-12 {
                continue;
            }
            let l1 = ((at.x - a.x) * (c.y - a.y) - (c.x - a.x) * (at.y - a.y)) / det;
            let l2 = ((b.x - a.x) * (at.y - a.y) - (at.x - a.x) * (b.y - a.y)) / det;
            let l0 = 1.0 - l1 - l2;
            if l0 >= 0.0 && l1 >= 0.0 && l2 >= 0.0 {
                let (Some(ra), Some(rb), Some(rc)) = (vertex(self.rest, tri[0]), vertex(self.rest, tri[1]), vertex(self.rest, tri[2])) else {
                    continue;
                };
                return Some(Point::new(l0 * ra.x + l1 * rb.x + l2 * rc.x, l0 * ra.y + l1 * rb.y + l2 * rc.y));
            }
        }
        None
    }

    /// The turn in degrees of the deformed mesh at the rest position `rest`: the rotation of the
    /// triangle containing it (searched from the top of the draw order), by the polar
    /// decomposition of the map from the rest triangle to the deformed one, so a stretch does not
    /// count as a turn. It is the turn an Auto pin has, from which a turn drag starts. `None` off
    /// the rest mesh.
    pub fn turn_at(&self, rest: Point) -> Option<f64> {
        let vertex = |points: &[Point], i: u32| points.get(i as usize).copied();
        for &t in self.order.iter().rev() {
            let Some(tri) = self.triangles.get(t as usize) else { continue };
            let (Some(a), Some(b), Some(c)) = (vertex(self.rest, tri[0]), vertex(self.rest, tri[1]), vertex(self.rest, tri[2])) else {
                continue;
            };
            let (e1, e2) = (b - a, c - a);
            let det = e1.x * e2.y - e2.x * e1.y;
            if det.abs() < 1e-12 {
                continue;
            }
            let p = rest - a;
            let l1 = (p.x * e2.y - e2.x * p.y) / det;
            let l2 = (e1.x * p.y - p.x * e1.y) / det;
            if l1 < 0.0 || l2 < 0.0 || 1.0 - l1 - l2 < 0.0 {
                continue;
            }
            let (Some(a), Some(b), Some(c)) = (vertex(self.deformed, tri[0]), vertex(self.deformed, tri[1]), vertex(self.deformed, tri[2])) else {
                continue;
            };
            let (d1, d2) = (b - a, c - a);
            let m11 = (d1.x * e2.y - d2.x * e1.y) / det;
            let m12 = (d2.x * e1.x - d1.x * e2.x) / det;
            let m21 = (d1.y * e2.y - d2.y * e1.y) / det;
            let m22 = (d2.y * e1.x - d1.y * e2.x) / det;
            return Some((m21 - m12).atan2(m11 + m22).to_degrees());
        }
        None
    }

    /// Every triangle edge once, as vertex index pairs (smaller first), for the mesh painter.
    pub fn edges(&self) -> Vec<[u32; 2]> {
        // pc: crates/ui-egui/src/tools/puppet_ui.rs:368 (each edge once).
        let mut seen = BTreeSet::new();
        for tri in self.triangles {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                seen.insert([a.min(b), a.max(b)]);
            }
        }
        seen.into_iter().collect()
    }

    /// The middle of the rest mesh's bounds: where Insert places a pin when the pointer is
    /// outside the canvas. `None` for an empty mesh.
    pub fn centre(&self) -> Option<Point> {
        // new: the keyboard path of LC14 (Insert).
        let first = *self.rest.first()?;
        let bounds = self.rest.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
        Some(bounds.center())
    }
}

/// An edit of the pin session, from [`PinWarp`](super::PinWarp) or [`PinWarpBar`](super::PinWarpBar).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PinEdit {
    /// The pins as a whole: a drag (Begin, Update, Commit, Cancel) or a one-shot change (Set).
    /// Placing a pin begins a drag whose first proposal holds the new pin, so the placement and the
    /// drag that follows are one step; moving several selected pins, an ⌥-drag turn, a delete, a
    /// depth or angle change and Remove All Pins all report the new pin list.
    Pins(EditEvent<Vec<Pin>>),
    /// The selection, as indices into the pins. Not an undo step.
    Select(Vec<usize>),
    /// Mode, density or expansion. Density and expansion remesh; a refused remesh restores the
    /// previous options.
    Options(PinWarpOptions),
    /// Show Mesh. Not an undo step.
    ShowMesh(bool),
}

impl PinEdit {
    /// A one-step change of the pin list from `before` to `after`, named `label` in the undo list: what the bar and
    /// the commands report.
    pub fn replace(before: &[Pin], after: Vec<Pin>, label: &str) -> PinEdit {
        PinEdit::Pins(EditEvent::set(Mixed::Same(before.to_vec()), after, Arc::from(label), None))
    }
}

/// The pins of an open Puppet Warp, their selection, the options and the session's undo steps.
///
/// - Changes only through [`apply`](Self::apply), which also tells the solver: a new pin count or
///   a pin's rotation fixed or freed is a [`rebind`](PinSolver::rebind), any other pin change a
///   [`solve`](PinSolver::solve), density or expansion a [`remesh`](PinSolver::remesh).
/// - One undo step per drag that moved something and per one-shot change, an arrow nudge included
///   (Photoshop records each). Undo and Redo keep the selection where the selected pins still exist
///   (PhotoCraft `restore`).
#[derive(Clone, Debug)]
pub struct PinWarpSession {
    pins: Vec<Pin>,
    selected: Vec<usize>,
    options: PinWarpOptions,
    show_mesh: bool,
    history: EditHistory<Vec<Pin>>,
    before: Option<Vec<Pin>>,
}

impl PinWarpSession {
    /// A session with no pins and `settings`' options.
    pub fn new(settings: &PinWarpSettings) -> Self {
        Self::from_pins(Vec::new(), settings)
    }

    /// A session on pins the app kept (VectorCraft's pins live in the document).
    pub fn from_pins(pins: Vec<Pin>, settings: &PinWarpSettings) -> Self {
        Self { history: EditHistory::new(&pins), pins, selected: Vec::new(), options: settings.options, show_mesh: settings.show_mesh, before: None }
    }

    /// The pins in their order.
    pub fn pins(&self) -> &[Pin] {
        &self.pins
    }

    /// The selected pins' indices.
    pub fn selected(&self) -> &[usize] {
        &self.selected
    }

    /// The solver's options.
    pub fn options(&self) -> &PinWarpOptions {
        &self.options
    }

    /// Whether Show Mesh is on.
    pub fn show_mesh(&self) -> bool {
        self.show_mesh
    }

    /// The persisted section as the session has it now.
    pub fn settings(&self) -> PinWarpSettings {
        PinWarpSettings { options: self.options, show_mesh: self.show_mesh }
    }

    /// Whether a pin drag is running.
    pub fn dragging(&self) -> bool {
        self.before.is_some()
    }

    /// The pins to commit; `None` when every pin is where it attached and has no turn (a turn of a
    /// whole number of full turns counts as none), so the app skips the command.
    pub fn result(&self) -> Option<&[Pin]> {
        // pc: crates/algo/src/puppet.rs:116 (`is_identity`); crates/ui-egui/src/tools/puppet_ui.rs:236
        // (`commit`).
        let identity = self.pins.iter().all(|p| p.rest == p.at && p.angle.is_none_or(|a| a.rem_euclid(360.0) == 0.0));
        (!identity).then_some(&self.pins)
    }

    /// Applies one edit, telling `solver` what changed. `Err` carries a refused remesh's text for
    /// the bar's notice; the options are then as before.
    ///
    /// A pin list that grows selects its last pin (a placed pin is selected, PhotoCraft); one that
    /// shrinks keeps the selected pins that survive, matched in order by their rest point.
    pub fn apply(&mut self, edit: PinEdit, solver: &mut dyn PinSolver) -> Result<(), String> {
        // pc: crates/ui-egui/src/tools/puppet_ui.rs:293 (`pointer`), :123 (`remesh_or_restore`),
        // :135 (`rebind`), :143 (`solve`), :86 (`delete_selected_pin`).
        match edit {
            PinEdit::Pins(event) => self.pins_event(event, solver),
            PinEdit::Select(indices) => {
                self.selected = Self::clean(indices, self.pins.len());
            }
            PinEdit::Options(options) => return self.set_options(options, solver),
            PinEdit::ShowMesh(show) => self.show_mesh = show,
        }
        Ok(())
    }

    /// Edit › Undo while the session is open (the app routes it here; the session reads no undo
    /// key): steps back one undo step and rebinds `solver`; false while a drag runs or with nothing to
    /// undo.
    pub fn undo(&mut self, solver: &mut dyn PinSolver) -> bool {
        // pc: crates/ui-egui/src/tools/puppet_ui.rs:70 (`undo`), :61 (`restore`).
        self.step(Step::Undo, solver)
    }

    /// Steps forward one undone step and rebinds `solver`.
    pub fn redo(&mut self, solver: &mut dyn PinSolver) -> bool {
        // pc: crates/ui-egui/src/tools/puppet_ui.rs:78 (`redo`).
        self.step(Step::Redo, solver)
    }

    /// Whether an edit can be undone.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether an undone edit can be redone.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Indices in range, each once, in the order given.
    fn clean(indices: Vec<usize>, len: usize) -> Vec<usize> {
        let mut out = Vec::with_capacity(indices.len());
        for i in indices {
            if i < len && !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }

    /// Tells the solver about `new` pins replacing the current ones: a rebind when the pin set or
    /// a pin's fixed rotation changed, a solve otherwise.
    fn replace(&mut self, new: Vec<Pin>, solver: &mut dyn PinSolver) {
        if new == self.pins {
            return;
        }
        let rebind =
            new.len() != self.pins.len() || new.iter().zip(&self.pins).any(|(n, o)| n.rest != o.rest || n.angle.is_some() != o.angle.is_some());
        if new.len() > self.pins.len() {
            self.selected = vec![new.len() - 1];
        } else if new.len() < self.pins.len() {
            let mut kept = Vec::new();
            let mut j = 0;
            for (i, old) in self.pins.iter().enumerate() {
                if new.get(j).is_some_and(|n| n.rest == old.rest) {
                    if self.selected.contains(&i) {
                        kept.push(j);
                    }
                    j += 1;
                }
            }
            self.selected = kept;
        }
        self.pins = new;
        if rebind {
            solver.rebind(&self.pins, &self.options);
        } else {
            solver.solve(&self.pins, &self.options);
        }
    }

    fn pins_event(&mut self, event: EditEvent<Vec<Pin>>, solver: &mut dyn PinSolver) {
        self.history.record(&event);
        match event {
            EditEvent::Begin { .. } => self.before = Some(self.pins.clone()),
            EditEvent::Update { value, .. } => self.replace(value, solver),
            EditEvent::Commit { after, .. } => {
                self.replace(after, solver);
                self.before = None;
            }
            EditEvent::Cancel { .. } => {
                if let Some(before) = self.before.take() {
                    self.replace(before, solver);
                }
            }
            EditEvent::Set { after, .. } => self.replace(after, solver),
            _ => {}
        }
    }

    fn set_options(&mut self, options: PinWarpOptions, solver: &mut dyn PinSolver) -> Result<(), String> {
        if !options.expansion.is_finite() {
            return Ok(());
        }
        let before = self.options;
        self.options = options;
        if options.density != before.density || options.expansion != before.expansion {
            if let Err(text) = solver.remesh(&self.options) {
                self.options = before;
                return Err(text);
            }
            solver.rebind(&self.pins, &self.options);
        } else if options != before {
            solver.solve(&self.pins, &self.options);
        }
        Ok(())
    }

    fn step(&mut self, step: Step, solver: &mut dyn PinSolver) -> bool {
        if self.dragging() {
            return false;
        }
        if !self.history.step(&mut self.pins, step) {
            return false;
        }
        self.selected.retain(|i| *i < self.pins.len());
        solver.rebind(&self.pins, &self.options);
        true
    }
}

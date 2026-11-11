//! The plane editor, without a frame: the editor's state ([`VanishingPointEditor`]), one pass over a session
//! ([`VanishingPointPass`]) and its two faces on a canvas, [`VanishingPointTool`] (Create Plane, the paste and Stamp
//! presses) and [`VanishingPlanes`] (the planes on the canvas, each corner and edge node a part).

mod control;
mod tool;
mod work;

pub use control::{VanishingPlanes, VanishingPlanesOutput};
pub use tool::VanishingPointTool;

use std::cell::RefCell;

use serde::{Deserialize, Serialize};

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Rect;
use craft_behavior::{CancelReason, EditEvent, GestureId};

use super::planes::{TornPlane, VanishingPlane, VanishingScene};
use super::{VanishingPointEvent, VanishingPointSource};
pub(super) use work::{PASTE_CELLS, Work, is_unset};

/// The engine's limits on the clone strokes, given by the app (PhotoCraft `MAX_CLONE_STROKES` 4096 and
/// `MAX_CLONE_DABS` 200 000, engine `vp_cmds.rs:19-20`). The Stamp tool stops there, so OK and the preview never fail
/// on what was painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StampLimits {
    /// The most strokes in a session.
    pub strokes: usize,
    /// The most dabs, over all strokes.
    pub dabs: usize,
}

impl StampLimits {
    /// Limits of `strokes` strokes and `dabs` dabs.
    pub const fn new(strokes: usize, dabs: usize) -> Self {
        Self { strokes, dabs }
    }
}

/// The editor's tools: Edit Plane (V), Create Plane (C), Stamp (S). The workspace's strip adds Hand and Zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VanishingTool {
    /// Drag corner and edge nodes, select a plane.
    EditPlane,
    /// Click four corners to make a plane.
    #[default]
    CreatePlane,
    /// Clone from a source onto the planes.
    Stamp,
}

/// The Stamp tool's options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct StampSettings {
    /// The brush's diameter in pixels where a stroke starts.
    pub diameter: f64,
    /// 0..=100.
    pub hardness: f64,
    /// 0..=100.
    pub opacity: f64,
    /// Later strokes keep the first stroke's source offset instead of restarting at the source.
    pub aligned: bool,
}

impl StampSettings {
    /// The hardness as a share of 1, as the engine takes it.
    pub fn hardness_share(&self) -> f64 {
        self.hardness / 100.0
    }

    /// The opacity as a share of 1, as the engine takes it.
    pub fn opacity_share(&self) -> f64 {
        self.opacity / 100.0
    }
}

impl Default for StampSettings {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:826.
        Self { diameter: 40.0, hardness: 50.0, opacity: 100.0, aligned: true }
    }
}

/// A clone stroke as the engine paints it (`filter.vanishingPoint`'s `clone` entries, PhotoCraft `CloneSpec`): the
/// source, the points, the diameter, and hardness and opacity in 0..=1 (the options' percent / 100).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StampStroke {
    /// The clone source in document pixels where the stroke starts.
    pub source: [f64; 2],
    /// The brush path in document pixels.
    pub points: Vec<[f64; 2]>,
    /// The brush diameter in document pixels.
    pub size: f64,
    /// Hardness, 0..=1.
    pub hardness: f64,
    /// Opacity, 0..=1.
    pub opacity: f64,
}

impl StampStroke {
    /// A stroke from `source` along `points`.
    pub fn new(source: [f64; 2], points: Vec<[f64; 2]>, size: f64, hardness: f64, opacity: f64) -> Self {
        Self { source, points, size, hardness, opacity }
    }
}

/// The pasted image on the selected plane: its top-left in plane coordinates (0..=1) and its width as a fraction of
/// the plane's width.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PasteImage {
    /// The image's top-left in plane coordinates, 0..=1.
    pub at: [f64; 2],
    /// The image's width as a fraction of the plane's width.
    pub width: f64,
}

impl PasteImage {
    /// An image at `at`, `width` of the plane wide.
    pub const fn new(at: [f64; 2], width: f64) -> Self {
        Self { at, width }
    }
}

/// The planes as a gesture reports them: every plane and how it was torn off.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PlaneEdit {
    /// Every plane.
    pub planes: Vec<VanishingPlane>,
    /// Per plane, how it was torn off an earlier plane; `None` for a plane drawn by hand.
    pub torn: Vec<Option<TornPlane>>,
}

impl PlaneEdit {
    /// The edit that leaves `planes`, torn off as `torn`.
    pub fn new(planes: Vec<VanishingPlane>, torn: Vec<Option<TornPlane>>) -> Self {
        Self { planes, torn }
    }
}

/// What OK applies and the history snapshots (PhotoCraft's `VpEdit`): the planes, how they were torn off, where the
/// paste sits, the clone source and strokes. The tool, the pending corners, the selection and the bar's options are
/// outside it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct VanishingPointSession {
    /// Every plane.
    pub planes: Vec<VanishingPlane>,
    /// Per plane, how it was torn off an earlier plane; `None` for a plane drawn by hand.
    pub torn: Vec<Option<TornPlane>>,
    /// Where the pasted image sits on the selected plane.
    pub paste: PasteImage,
    /// The clone source (Alt-click), document pixels.
    pub source: Option<[f64; 2]>,
    /// The clone strokes, in order.
    pub strokes: Vec<StampStroke>,
    /// The first stroke's (source, start) since the source was set: Aligned strokes keep its offset.
    pub aligned_from: Option<([f64; 2], [f64; 2])>,
}

impl Default for VanishingPointSession {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:814-829 (the paste at 0.25, 0.25, half the plane wide).
        Self {
            planes: Vec::new(),
            torn: Vec::new(),
            paste: PasteImage::new([0.25, 0.25], 0.5),
            source: None,
            strokes: Vec::new(),
            aligned_from: None,
        }
    }
}

/// The options bar's values that are not undone: the grid's cell size (100), whether to paste the clipboard (on when it
/// holds an image; the app sets it), whether the result goes to a new layer (off), the stamp options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct VanishingPointSettings {
    /// Grid cell size in pixels at the plane's nearest edge.
    pub grid_size: f64,
    /// Whether to paste the clipboard image.
    pub paste: bool,
    /// Whether the result goes to a new layer.
    pub new_layer: bool,
    /// The Stamp tool's options.
    pub stamp: StampSettings,
}

impl Default for VanishingPointSettings {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:819-826.
        Self { grid_size: 100.0, paste: false, new_layer: false, stamp: StampSettings::default() }
    }
}

/// A drag in progress.
#[derive(Clone, Debug)]
pub(super) enum VanishingDrag {
    /// A corner node and the pointer's offset from it.
    Corner { plane: usize, corner: usize, offset: [f64; 2] },
    /// An edge node: the edge slides along the plane's perspective at the press.
    Edge { plane: usize, edge: usize, at_press: Homography },
    /// The pasted image and the pointer's offset from its top-left in plane coordinates.
    Paste { offset: [f64; 2] },
    /// The far edge node of a torn plane: depth, or with Alt its angle, once it left `start`.
    Torn { plane: usize, rotate: bool, start: [f64; 2], moved: bool },
    /// A clone stroke.
    Stamp,
}

/// The value a drag changes, as it was at the press.
#[derive(Clone, Debug)]
pub(super) enum DragStart {
    Planes(PlaneEdit),
    Paste(PasteImage),
    /// The session's aligned offset before the stroke (the stroke is the last one), and the stroke as it began.
    Stamp(Option<([f64; 2], [f64; 2])>, StampStroke),
}

/// The editor's state between passes: the tool (Create Plane at first, PhotoCraft `vp_ui.rs:816`), the corners clicked
/// so far with Create Plane, the selected plane, the drag, the events not yet taken. It runs on the workspace's canvas
/// and, without a frame, on any canvas: one [`VanishingPointPass`] per pass gives the canvas its
/// [`tool`](VanishingPointPass::tool) and its [`planes`](VanishingPointPass::planes).
#[derive(Debug)]
pub struct VanishingPointEditor {
    pub(super) tool: VanishingTool,
    pub(super) pending: Vec<[f64; 2]>,
    pub(super) selected: Option<usize>,
    pub(super) drag: Option<(GestureId, VanishingDrag)>,
    /// What the drag changes, as it was at the press (Esc puts it back, Commit compares).
    pub(super) before: Option<DragStart>,
    /// The pointer's last document position (the rubber band and the stamp ring follow it).
    pub(super) pointer: Option<[f64; 2]>,
    pub(super) events: Vec<VanishingPointEvent>,
    pub(super) limits: StampLimits,
    /// The message of the last refused Stamp press, for the app's status line.
    pub(super) status: Option<String>,
}

impl VanishingPointEditor {
    /// An editor with Create Plane picked and the app's clone `limits`.
    pub fn new(limits: StampLimits) -> Self {
        Self {
            tool: VanishingTool::CreatePlane,
            pending: Vec::new(),
            selected: None,
            drag: None,
            before: None,
            pointer: None,
            events: Vec::new(),
            limits,
            status: None,
        }
    }

    /// The message of a refused Stamp press (no clone source, too many strokes), translated, for the app's status
    /// line; `None` when there is none since the last call.
    pub fn take_status(&mut self) -> Option<String> {
        self.status.take()
    }

    /// The corners Create Plane has clicked so far.
    pub fn pending(&self) -> &[[f64; 2]] {
        &self.pending
    }

    /// The selected tool.
    pub fn tool(&self) -> VanishingTool {
        self.tool
    }

    /// Picks a tool; Edit Plane and Stamp drop the pending corners.
    pub fn set_tool(&mut self, tool: VanishingTool) {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1052-1059.
        self.tool = tool;
        if tool != VanishingTool::CreatePlane {
            self.pending.clear();
        }
    }

    /// The index of the selected plane, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects plane `index` of `session`; none when it is out of range.
    pub fn select(&mut self, index: Option<usize>, session: &VanishingPointSession) -> &mut Self {
        self.selected = index.filter(|i| *i < session.planes.len());
        self
    }

    /// Esc: drops the pending corners; returns whether there were any (else the workspace cancels).
    pub fn drop_pending(&mut self) -> bool {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1068 `escape`.
        let had = !self.pending.is_empty();
        self.pending.clear();
        had
    }

    /// Delete or Backspace: removes the selected plane from `session`; planes torn off it keep their place as drawn
    /// planes, later parents' indices shift down. Returns the edit, which is also reported (a `Planes` `Set`).
    /// Nothing while a drag runs: the gesture ends first.
    pub fn delete_selected(&mut self, session: &mut VanishingPointSession, scene: &dyn VanishingScene, step: &str) -> Option<PlaneEdit> {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:398 `delete_selected`.
        if self.drag.is_some() {
            return None;
        }
        let i = self.selected.take().filter(|i| *i < session.planes.len())?;
        let before = PlaneEdit::new(session.planes.clone(), session.torn.clone());
        session.torn.resize(session.planes.len(), None);
        session.planes.remove(i);
        session.torn.remove(i);
        for t in &mut session.torn {
            match t {
                Some(c) if c.from == i => *t = None,
                Some(c) if c.from > i => c.from -= 1,
                _ => {}
            }
        }
        if !session.planes.is_empty() {
            session.planes = scene.resolve(&session.planes, &session.torn);
        }
        let edit = PlaneEdit::new(session.planes.clone(), session.torn.clone());
        self.events.push(VanishingPointEvent::Planes(EditEvent::set(craft_behavior::Mixed::Same(before), edit.clone(), step.into(), None)));
        Some(edit)
    }

    /// Adds the plane whose corners are `corners` (the New Plane form's four points) as Create Plane's fourth click
    /// does: the scene normalises it, it is selected and Edit Plane takes over. Returns whether a plane was added
    /// (not past [`MAX_PLANES`](super::MAX_PLANES)). Reported as a `Planes` `Set`.
    pub fn add_plane(&mut self, session: &mut VanishingPointSession, scene: &dyn VanishingScene, corners: [[f64; 2]; 4], step: &str) -> bool {
        if self.drag.is_some() {
            return false;
        }
        work::close_plane(self, session, scene, corners, step)
    }

    /// After undo or redo: no pending corners, the selection clamped. A drag in progress ended before the step
    /// ([`cancel`](Self::cancel) with `CancelReason::UndoDuringGesture`), and the app does not step while
    /// [`busy`](Self::busy).
    pub fn after_step(&mut self, session: &VanishingPointSession) {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:383-396 `step`.
        self.pending.clear();
        self.selected = self.selected.filter(|i| *i < session.planes.len());
    }

    /// Ends the drag in progress without a commit, for `reason`: the session is put back as the press found it and
    /// the drag's `Cancel` is reported. Nothing without a drag.
    pub fn cancel(&mut self, session: &mut VanishingPointSession, reason: CancelReason) {
        // new: Esc, a tool switch or an undo abandons the drag and puts back what the press found.
        let Some((id, drag)) = self.drag.take() else { return };
        let event = match (drag, self.before.take()) {
            (VanishingDrag::Stamp, Some(DragStart::Stamp(aligned, first))) => {
                session.strokes.pop();
                session.aligned_from = aligned;
                Some(VanishingPointEvent::Stamp(EditEvent::cancel(id, reason, Some(craft_behavior::Mixed::Same(first)))))
            }
            (VanishingDrag::Paste { .. }, Some(DragStart::Paste(before))) => {
                session.paste = before;
                Some(VanishingPointEvent::Paste(EditEvent::cancel(id, reason, Some(craft_behavior::Mixed::Same(before)))))
            }
            (_, Some(DragStart::Planes(before))) => {
                session.planes.clone_from(&before.planes);
                session.torn.clone_from(&before.torn);
                let n = session.planes.len();
                self.selected = self.selected.filter(|i| *i < n);
                Some(VanishingPointEvent::Planes(EditEvent::cancel(id, reason, Some(craft_behavior::Mixed::Same(before)))))
            }
            _ => None,
        };
        self.events.extend(event);
    }

    /// One pass of the editor over `session`: the canvas's tool and control.
    pub fn pass<'a>(
        &'a mut self,
        ctx: &'a egui::Context,
        session: &'a mut VanishingPointSession,
        settings: &'a VanishingPointSettings,
        scene: &'a dyn VanishingScene,
        source: &'a dyn VanishingPointSource,
    ) -> VanishingPointPass<'a> {
        VanishingPointPass { ctx, editor: RefCell::new(self), session: RefCell::new(session), settings, scene, source }
    }

    /// Takes the events collected since the last call.
    pub fn take_events(&mut self) -> Vec<VanishingPointEvent> {
        std::mem::take(&mut self.events)
    }

    /// A drag or a stroke runs: the history waits.
    pub fn busy(&self) -> bool {
        self.drag.is_some()
    }
}

/// What the tool and the control of one pass share: the editor and the session behind cells, so the canvas's controls
/// and its tool, which run in the same pass, both change them.
pub(super) trait Shared {
    /// Runs `f` over the pass's editor, session, settings, scene and source.
    fn with(&self, f: &mut dyn FnMut(&mut Work<'_>));
}

/// One pass of the editor over a session, which the app shows on its own canvas.
///
/// ```ignore
/// let pass = editor.pass(ui.ctx(), &mut session, &settings, &scene, &source);
/// FullWindowCanvas::new(bind, image)
///     .controls(|ui, rect| { pass.planes().show_at(ui, rect); })
///     .tool(&mut pass.tool())
///     .show(ui);
/// ```
pub struct VanishingPointPass<'a> {
    ctx: &'a egui::Context,
    editor: RefCell<&'a mut VanishingPointEditor>,
    session: RefCell<&'a mut VanishingPointSession>,
    settings: &'a VanishingPointSettings,
    scene: &'a dyn VanishingScene,
    source: &'a dyn VanishingPointSource,
}

impl Shared for VanishingPointPass<'_> {
    fn with(&self, f: &mut dyn FnMut(&mut Work<'_>)) {
        let mut editor = self.editor.borrow_mut();
        let mut session = self.session.borrow_mut();
        let mut work = Work {
            ctx: self.ctx,
            gesture: None,
            editor: &mut editor,
            session: &mut session,
            scene: self.scene,
            settings: self.settings,
            source: self.source,
        };
        f(&mut work);
    }
}

impl VanishingPointPass<'_> {
    /// The canvas tool of the pass: Create Plane's clicks, the paste's drag and Stamp.
    pub fn tool(&self) -> VanishingPointTool<'_> {
        VanishingPointTool::new(self)
    }

    /// The canvas control of the pass: the planes, their grids and nodes. Show it at the canvas's rect with
    /// `show_at`.
    pub fn planes(&self) -> VanishingPlanes<'_> {
        VanishingPlanes::new(self)
    }

    /// Where the paste lies: the selected plane's frame (the unit square onto the plane, or onto the document) and
    /// the paste's rectangle in plane coordinates. `None` without Paste on, an image, or a usable plane. The app
    /// draws its clipboard image through it (`RasterOverlay::mapped`) unless the stamp preview shows.
    pub fn paste_placement(&self) -> Option<(Homography, Rect)> {
        let mut found = None;
        self.with(&mut |work| found = work.paste_placement());
        found
    }
}

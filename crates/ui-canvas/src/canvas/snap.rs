//! `Snap`: a canvas's snapping of a coordinate the pointer sets.

use crate::runtime::env::{AppValues, EnvValue, EnvView};
use craft_behavior::geometry::{SNAP_PX, SnapLine, SnapOptions, SnapTargets};
use craft_behavior::kurbo::{Point, Vec2};
use craft_behavior::{Axis, CanvasView};
use craft_keymap::{Mods, Platform};
use std::sync::Arc;

/// A canvas's snapping of a coordinate the pointer sets (View > Snap To), as data over the document's [`SnapTargets`]:
/// the axis a guide at the coordinate would run along (an x coordinate is across `Axis::Y`), the document coordinate
/// under the pointer and the modifiers held, to the coordinate to use. The app gives it to the canvas once
/// ([`Canvas::snap`](super::Canvas::snap)); the canvas publishes it to its body, where the rulers snap the zero point
/// and the guides snap a guide ([`canvas_snap`]), and hands it to the tool host through
/// [`CanvasOutput`](super::CanvasOutput), which gives it to the tool ([`ToolContext::snap`](super::ToolContext::snap)).
/// Two snaps are equal when they share their targets and have equal options, tolerance and OS.
#[derive(Clone)]
pub struct Snap {
    targets: Arc<SnapTargets>,
    options: SnapOptions,
    tolerance: Option<f64>,
    os: Platform,
}

/// The gesture's snap options in one argument: whether the gesture reads Control itself, and a kind filter.
#[derive(Clone, Copy, Debug, Default)]
pub struct GestureSnap {
    /// Whether the gesture reads the Control key itself, so Control does not suspend snapping.
    pub reads_control: bool,
    /// The kinds of target the gesture snaps to (Distort corners skip layer edges and centres).
    pub kinds: craft_behavior::geometry::SnapKinds,
}

impl Snap {
    /// The snapping over `targets` with `options`; the tolerance is set by the canvas each pass.
    pub fn new(targets: Arc<SnapTargets>, options: SnapOptions) -> Self {
        Snap { targets, options, tolerance: None, os: Platform::current() }
    }

    /// `position` across a guide running `along`, snapped.
    pub fn apply(&self, along: Axis, position: f64, mods: Mods) -> f64 {
        let Some(tolerance) = self.reach(mods, false) else { return position };
        self.targets.nearest(along, position, tolerance).map_or(position, |target| target.position)
    }

    /// `point` snapped on both axes to the targets of `gesture.kinds`, with the lines to draw; the input and no lines
    /// when snapping is off.
    pub fn point(&self, point: Point, mods: Mods, gesture: GestureSnap) -> (Point, Vec<SnapLine>) {
        match self.reach(mods, gesture.reads_control) {
            Some(tolerance) => self.targets.snap_point_of(point, tolerance, gesture.kinds),
            None => (point, Vec::new()),
        }
    }

    /// The offset that moves `rect` onto a snap target, with the lines to draw; zero when snapping is off.
    pub fn rect(&self, rect: craft_behavior::kurbo::Rect, mods: Mods) -> (Vec2, Vec<SnapLine>) {
        self.rect_of(rect, mods, craft_behavior::geometry::SnapKinds::ALL)
    }

    /// [`rect`](Self::rect) to the targets of a kind in `kinds` only.
    pub fn rect_of(&self, rect: craft_behavior::kurbo::Rect, mods: Mods, kinds: craft_behavior::geometry::SnapKinds) -> (Vec2, Vec<SnapLine>) {
        match self.reach(mods, false) {
            Some(tolerance) => self.targets.snap_rect_of(rect, tolerance, kinds),
            None => (Vec2::ZERO, Vec::new()),
        }
    }

    /// The snapping distance in document units for this pass, or `None` when snapping is off, the canvas has not given
    /// this snap its pass ([`at`](Self::at)) or the Control key suspends it (unless `reads_control`): the one gate of
    /// `apply`, `point` and `rect`, and the reach a gesture's snapped lines were found with.
    pub fn reach(&self, mods: Mods, reads_control: bool) -> Option<f64> {
        self.tolerance.filter(|_| self.options.on && !snap_suspended(mods, self.os, reads_control))
    }

    /// The View > Snap To options this snap was built with.
    pub fn options(&self) -> SnapOptions {
        self.options
    }

    /// This snap with its tolerance for `view` and the OS set.
    pub(super) fn at(mut self, view: &CanvasView, os: Platform) -> Self {
        self.tolerance = Some(view.doc_len(SNAP_PX as f32));
        self.os = os;
        self
    }
}

impl PartialEq for Snap {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.targets, &other.targets) && self.options == other.options && self.tolerance == other.tolerance && self.os == other.os
    }
}

impl std::fmt::Debug for Snap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The targets can number thousands of grid lines, so they are left out.
        f.debug_struct("Snap").field("options", &self.options).field("tolerance", &self.tolerance).field("os", &self.os).finish_non_exhaustive()
    }
}

/// Whether the modifiers held now turn snapping off: the Control key (Control without ⌘ on macOS, Ctrl elsewhere)
/// suspends it while held, Photoshop's rule, unless the gesture reads that key itself (`tool_reads_control`: off macOS
/// Ctrl on a transform box corner distorts). Every snapped drag applies it, whatever its targets are.
pub fn snap_suspended(mods: Mods, os: Platform, tool_reads_control: bool) -> bool {
    let control = if os.is_mac() { mods.ctrl && !mods.cmd } else { mods.cmd };
    control && !tool_reads_control
}

/// How far, in document units under `view`, a dragged point still snaps to a target: [`SNAP_PX`] screen points, or
/// `None` while [`snap_suspended`] holds.
pub fn snap_tolerance(view: &CanvasView, mods: Mods, os: Platform, tool_reads_control: bool) -> Option<f64> {
    (!snap_suspended(mods, os, tool_reads_control)).then(|| view.doc_len(SNAP_PX as f32))
}

#[derive(Clone, PartialEq)]
struct InCanvasSnap(Option<Snap>);

impl EnvValue for InCanvasSnap {
    fn root() -> Self {
        InCanvasSnap(None)
    }
}

/// Publishes `snap` (or none) to a canvas's body.
pub(super) fn publish(values: &mut AppValues, snap: Option<Snap>) {
    values.set(InCanvasSnap(snap));
}

/// The snapping of the innermost `Canvas` around the component, or `None` outside one or when the canvas has none.
pub fn canvas_snap(env: &EnvView<'_>) -> Option<Snap> {
    env.get::<InCanvasSnap>().0
}

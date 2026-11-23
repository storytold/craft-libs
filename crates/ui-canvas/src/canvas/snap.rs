//! `Snap`: a canvas's snapping of a coordinate the pointer sets.

use crate::runtime::env::{AppValues, EnvValue, EnvView};
use craft_behavior::geometry::{SNAP_PX, SnapLine, SnapOptions, SnapSuspend, SnapTargets};
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
    whole_pixels: bool,
}

/// The gesture's snap options in one argument: whether the gesture reads the suspend key itself, and a kind filter.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct GestureSnap {
    /// Whether the gesture reads the suspend key itself, so that key does not suspend snapping.
    pub reads_suspend_key: bool,
    /// The kinds of target the gesture snaps to (an explicit Distort snaps its corners to guides only).
    pub kinds: craft_behavior::geometry::SnapKinds,
}

impl GestureSnap {
    /// These options with the gesture reading the suspend key itself when `reads_suspend_key`.
    pub fn reads_suspend_key(mut self, reads_suspend_key: bool) -> Self {
        self.reads_suspend_key = reads_suspend_key;
        self
    }

    /// These options snapping to `kinds`.
    pub fn kinds(mut self, kinds: craft_behavior::geometry::SnapKinds) -> Self {
        self.kinds = kinds;
        self
    }
}

impl Snap {
    /// The snapping over `targets` with `options`; the tolerance is set by the canvas each pass.
    pub fn new(targets: Arc<SnapTargets>, options: SnapOptions) -> Self {
        Snap { targets, options, tolerance: None, os: Platform::current(), whole_pixels: false }
    }

    /// This snap rounding every point it gives to whole document units when `on`: the pixel grid for drawn points and
    /// transform boxes (Preferences > Tools > Snap Vector Tools and Transforms to Pixel Grid, as the app decides it
    /// applies). The rounding follows the target snap and holds with snapping off or suspended by its key.
    #[must_use]
    pub fn whole_pixels(mut self, on: bool) -> Self {
        self.whole_pixels = on;
        self
    }

    /// Whether this snap rounds its points to whole document units.
    pub fn rounds_to_pixels(&self) -> bool {
        self.whole_pixels
    }

    fn rounded(&self, position: f64) -> f64 {
        if self.whole_pixels { position.round() } else { position }
    }

    /// `position` across a guide running `along`, snapped.
    pub fn apply(&self, along: Axis, position: f64, mods: Mods) -> f64 {
        let snapped = match self.reach(mods, false) {
            Some(tolerance) => self.targets.nearest(along, position, tolerance).map_or(position, |target| target.position),
            None => position,
        };
        self.rounded(snapped)
    }

    /// `point` snapped on both axes to the targets of `gesture.kinds`, with the lines to draw; the input and no lines
    /// when snapping is off.
    pub fn point(&self, point: Point, mods: Mods, gesture: GestureSnap) -> (Point, Vec<SnapLine>) {
        let (at, lines) = match self.reach(mods, gesture.reads_suspend_key) {
            Some(tolerance) => self.targets.snap_point_of(point, tolerance, gesture.kinds),
            None => (point, Vec::new()),
        };
        (Point::new(self.rounded(at.x), self.rounded(at.y)), lines)
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
    /// this snap its pass (`at`) or the suspend key suspends it (unless `reads_key`): the one gate of
    /// `apply`, `point` and `rect`, and the reach a gesture's snapped lines were found with.
    pub fn reach(&self, mods: Mods, reads_key: bool) -> Option<f64> {
        self.tolerance.filter(|_| self.options.on && !snap_suspended(mods, self.os, self.options.suspend, reads_key))
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
        Arc::ptr_eq(&self.targets, &other.targets)
            && self.options == other.options
            && self.tolerance == other.tolerance
            && self.os == other.os
            && self.whole_pixels == other.whole_pixels
    }
}

impl std::fmt::Debug for Snap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The targets can number thousands of grid lines, so they are left out.
        f.debug_struct("Snap")
            .field("options", &self.options)
            .field("tolerance", &self.tolerance)
            .field("os", &self.os)
            .field("whole_pixels", &self.whole_pixels)
            .finish_non_exhaustive()
    }
}

/// Whether the modifiers held now turn snapping off: the `suspend` key ([`SnapSuspend::is_held`]; Control by default,
/// Photoshop's rule) suspends it while held, unless the gesture reads that key itself (`gesture_reads_key`: off macOS
/// Ctrl on a transform box corner distorts). Every snapped drag applies it, whatever its targets are.
pub fn snap_suspended(mods: Mods, os: Platform, suspend: SnapSuspend, gesture_reads_key: bool) -> bool {
    suspend.is_held(mods, os) && !gesture_reads_key
}

/// How far, in document units under `view`, a dragged point still snaps to a target: [`SNAP_PX`] screen points, or
/// `None` while [`snap_suspended`] holds.
pub fn snap_tolerance(view: &CanvasView, mods: Mods, os: Platform, suspend: SnapSuspend, gesture_reads_key: bool) -> Option<f64> {
    (!snap_suspended(mods, os, suspend, gesture_reads_key)).then(|| view.doc_len(SNAP_PX as f32))
}

#[derive(Clone, PartialEq, crate::contract::MemoProps)]
#[memo_props(every_pass = "a context the canvas rebuilds each pass")]
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

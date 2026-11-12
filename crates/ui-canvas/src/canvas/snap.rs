//! `Snap`: a canvas's snapping of a coordinate the pointer sets.

use crate::runtime::env::{AppValues, EnvValue, EnvView};
use craft_behavior::geometry::SNAP_PX;
use craft_behavior::{Axis, CanvasView};
use craft_keymap::{Mods, Platform};
use std::sync::Arc;

/// A canvas's snapping of a coordinate the pointer sets (View > Snap To): the axis a guide at the coordinate would
/// run along (an x coordinate is across `Axis::Y`), the document coordinate under the pointer and the modifiers held,
/// to the coordinate to use. The app gives it to the canvas once ([`Canvas::snap`](super::Canvas::snap)); the canvas
/// publishes it to its body, where the rulers snap the zero point and the guides snap a guide
/// ([`canvas_snap`]), and hands it to the tool host through [`CanvasOutput`](super::CanvasOutput), which gives it to
/// the tool ([`ToolContext::snap`](super::ToolContext::snap)). Clones share one function; two snaps are equal when
/// they are clones of each other.
#[derive(Clone)]
pub struct Snap(Arc<dyn Fn(Axis, f64, Mods) -> f64 + Send + Sync>);

impl Snap {
    /// The snapping that runs `snap`.
    pub fn new(snap: impl Fn(Axis, f64, Mods) -> f64 + Send + Sync + 'static) -> Self {
        Snap(Arc::new(snap))
    }

    /// `position` across a guide running `along`, snapped.
    pub fn apply(&self, along: Axis, position: f64, mods: Mods) -> f64 {
        (self.0)(along, position, mods)
    }
}

impl PartialEq for Snap {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Snap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Snap")
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

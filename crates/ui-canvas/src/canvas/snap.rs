//! `Snap`: a canvas's snapping of a coordinate the pointer sets.

use crate::runtime::env::{AppValues, EnvValue, EnvView};
use craft_behavior::Axis;
use craft_keymap::Mods;
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

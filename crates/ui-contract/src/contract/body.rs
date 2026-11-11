//! The body step (16.3): the regions of a container (header, body with its scroll region, footer), open and
//! gate state, scroll offset, move and resize, and the zoomed canvas body.

use super::cx::BodyCx;
use super::overlay::PlacementChange;
use crate::runtime::memory::SaveKey;
use crate::runtime::motion::Presence;
use crate::runtime::overlay::CloseReason;
use crate::runtime::windows::ResizeEdge;
use craft_behavior::ScrollPos;
use egui::{Rect, Ui, Vec2};

mod hooks;
mod inherit;
mod open;
mod pane;
mod placement;
mod presses;
mod regions;
mod reorder;
mod scroll;
mod size;
mod window;

pub(crate) use placement::shown_placement;
#[doc(hidden)]
pub use regions::pad_of;
#[doc(hidden)]
pub use reorder::REORDER_BAND;
#[cfg(feature = "testing")]
pub(crate) use window::MOVE_BAND;
use window::WindowMode;

/// What the scroll region and the regions measured last pass.
#[derive(Clone, Default)]
struct BodyView {
    extent: (f32, f32),
    view: Option<[f32; 4]>,
}

pub(super) struct BodyHooks<'b> {
    pub(super) body: &'b mut dyn FnMut(&mut Ui, &mut BodyCx<'_>),
    /// The body's size from this pass's measuring run; `None` inside the measuring run itself.
    measured_body: Option<Vec2>,
    /// The header's size from this pass's measuring run; `None` without a header or inside the measuring run.
    measured_header: Option<Vec2>,
    measured_footer: Vec2,
    /// The end region's size from this pass's measuring run; `None` without one or inside the measuring run.
    measured_end: Option<Vec2>,
    /// The trailing controls box as the measuring run found it; `None` in the measuring run and without a record this pass.
    measured_controls: Option<Vec2>,
    /// The measuring run measures a closed layer whose exit fade still shows it as if it were open.
    shown_closed: bool,
    /// A record of this pass may serve instead of a measuring run: in a real pass's step 5 and in a sizing pass, not
    /// in `measure_in`, which measures the content it is given.
    reuse: bool,
    /// `open` and `presence` were read by the real pass's stage and stand for the measuring run: the run's own peek
    /// stage cannot see a slot the real stage holds.
    open_given: bool,
    /// The gap between an inline header and the body, and between a header and its trailing region.
    region_gap: f32,
    /// The space between a stacked header and the body (`HeaderRegion::gap`), zero without one.
    header_gap: f32,
    /// The least width of the content, from `ContainerProps::least_width`.
    least_width: f32,
    /// How far the widest region wants to exceed the width the room allows, from this pass's measure.
    overflow: f32,
    /// The room the mount offers, in which a content-sized body runs.
    room: Vec2,
    open: bool,
    open_at_read: bool,
    open_bound: bool,
    gate: Option<bool>,
    gate_at_read: Option<bool>,
    close: bool,
    /// Why the container closes this pass, the first reason given.
    reason: Option<CloseReason>,
    /// The container is closed and its layer is fading out: the body shows as it was and nothing is written.
    exiting: bool,
    offset: ScrollPos,
    offset_at_read: ScrollPos,
    /// The offset this pass shows, as step 4 left it: what decides which out-of-view children run, in a measuring
    /// run (which lays the content out from offset zero) as in the real run.
    view_offset: ScrollPos,
    scroll_bound: bool,
    moved: bool,
    last: BodyView,
    /// This pass's sizes, recorded at step 17.
    body_size: Option<Vec2>,
    header_size: Option<Vec2>,
    footer_size: Option<Vec2>,
    end_size: Option<Vec2>,
    controls_size: Option<Vec2>,
    extent: Option<Vec2>,
    view: Option<Rect>,
    save: Option<SaveKey>,
    layer: bool,
    /// How open the body is: 0 to 1, animating when the container's `Collapse` does.
    presence: Presence,
    /// The distance one arrow press moves or resizes a window in its keyboard mode.
    nudge: f32,
    /// The window mode the container is in, and what Escape restores.
    mode: WindowMode,
    /// The edge an edge pane's grip sits on this pass, with the least extent it gives; none for other mounts.
    pane_grip: Option<(ResizeEdge, f32)>,
    /// The placement changes of this pass, delivered to `Container::placement_changed` at the end of the update.
    reports: Vec<PlacementChange>,
    /// The host gives the container a rect of its own size: a full window, an OS viewport or a layer the user sized.
    fixed: bool,
    /// The axes (horizontal, vertical) the container is given: it fills them, and its own container was given them
    /// too or it is shown at a rect. Along the others it sizes from its content.
    given: (bool, bool),
    /// A bottom-anchored region is following the end of its content.
    follows_end: bool,
}

impl<'b> BodyHooks<'b> {
    pub(super) fn new(body: &'b mut dyn FnMut(&mut Ui, &mut BodyCx<'_>)) -> Self {
        BodyHooks {
            body,
            measured_body: None,
            measured_header: None,
            measured_footer: Vec2::ZERO,
            measured_end: None,
            measured_controls: None,
            shown_closed: false,
            reuse: false,
            open_given: false,
            region_gap: 0.0,
            header_gap: 0.0,
            least_width: 0.0,
            overflow: 0.0,
            room: Vec2::ZERO,
            open: true,
            open_at_read: true,
            open_bound: false,
            gate: None,
            gate_at_read: None,
            close: false,
            reason: None,
            exiting: false,
            offset: ScrollPos::default(),
            offset_at_read: ScrollPos::default(),
            view_offset: ScrollPos::default(),
            scroll_bound: false,
            moved: false,
            last: BodyView::default(),
            body_size: None,
            header_size: None,
            footer_size: None,
            end_size: None,
            controls_size: None,
            extent: None,
            view: None,
            save: None,
            layer: false,
            presence: Presence { t: 1.0, visible: true, exiting: false },
            nudge: 0.0,
            mode: WindowMode::default(),
            pane_grip: None,
            reports: Vec::new(),
            fixed: false,
            given: (false, false),
            follows_end: false,
        }
    }
}

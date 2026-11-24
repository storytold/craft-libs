//! Chapter 15: the `Overlay` data, its presets, popup handles, `OverlayUi` and the overlay host.
//!
//! One host (`host`) creates the egui layer through `runtime::egui::areas` and applies rank, backdrop, fit, scroll,
//! dismissal and the focus scope. `OverlayUi::show` and the tooltip and toast hosts run it.

pub use crate::runtime::overlay::{LayerRole, OpenFilter, any_open, close_popups, family, is_open, request_close, top};

pub(crate) mod container_open;
mod dismissal;
mod host;
mod opener;
mod place;
mod placement;
pub(crate) mod popups;
mod props;
mod toggles;
mod ui;

pub use container_open::{Popup, opened_from};
pub(crate) use container_open::{opener_open, set_opener_open};
pub use host::paint_frame_layers;
pub(crate) use host::{Layer, forget_layer, host, host_window};
pub(crate) use opener::{LayerFloor, anchor_floor};
pub use opener::{Opener, Trigger};
pub use place::{Anchor, FloatPlace, OverlayFit, Place, Resize, ResizeEdges, SizeLimits, ViewportShare};
pub use placement::{PopupRegion, layer_room};
pub(crate) use placement::{measuring_room, region_for, resolve_anchor, screen_for};
pub(crate) use popups::dwell;
pub use popups::{HOLD, Overlays, PopupHandle};
pub(crate) use popups::{apply_closes, begin_owner, note_layer_closed, open_map, popup_layer_open, popup_shown, popups_shown, write_open};
pub use props::{Dismiss, Outside, Overlay, PlacementChange, PlacementKind};
pub(crate) use toggles::{ToggleDecl, drive_toggles, has_toggle_nodes};
pub(crate) use ui::OverlayOwner;
pub use ui::OverlayUi;

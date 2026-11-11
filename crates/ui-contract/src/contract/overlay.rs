//! Chapter 15: the `Overlay` data, its presets, popup handles, `OverlayUi` and the overlay host.
//!
//! One host (`host`) creates the egui layer through `runtime::egui::areas` and applies rank, backdrop, fit, scroll,
//! dismissal and the focus scope. `OverlayUi::show` and the tooltip and toast hosts run it.

pub use crate::runtime::overlay::{OpenFilter, any_open, family, is_open, request_close, top};

mod container_open;
mod dismissal;
mod host;
mod opener;
mod place;
mod placement;
mod popups;
mod props;
mod toggles;
mod ui;

pub use container_open::Popup;
pub(crate) use container_open::{opener_open, set_opener_open};
pub use host::paint_frame_layers;
pub(crate) use host::{Layer, floor_width, forget_layer, host, host_window};
pub(crate) use opener::opener_floor;
pub use opener::{Opener, Trigger};
pub use place::{Anchor, FloatPlace, OverlayFit, Place, Resize, SizeLimits, ViewportShare};
pub(crate) use placement::{measuring_room, region_for, resolve_anchor, screen_for};
pub use popups::{HOLD, Overlays, PopupHandle};
pub(crate) use popups::{apply_closes, begin_owner, open_map, popup_shown, popups_shown, write_open};
pub use props::{Dismiss, Outside, Overlay, PlacementChange, PlacementKind};
pub(crate) use toggles::{ToggleDecl, drive_toggles, has_toggle_nodes};
pub(crate) use ui::OverlayOwner;
pub use ui::OverlayUi;

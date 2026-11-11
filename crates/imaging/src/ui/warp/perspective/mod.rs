//! Perspective Warp (Edit › Perspective Warp): planes drawn in Layout and dragged in Warp
//! ([`PerspectiveSession`], [`PerspectiveWarp`], [`PerspectiveBar`]). The warp map is the app's;
//! the warped picture is drawn by the transform preview through `PreviewMap::function`.

mod bar;
mod overlay;
mod session;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;

pub use bar::{PerspectiveBar, PerspectiveBarOutput, PerspectiveBarWords};
pub use overlay::{PerspectiveCommands, PerspectiveWarp, PerspectiveWarpOutput, PerspectiveWarpStyle, PerspectiveWarpWords};
pub use session::{
    LINK_DISTANCE, PLANE_MINIMUM, PerspectiveEdit, PerspectiveSession, Plane, PlaneCorner, PlaneMode, Straighten, linked_corners, straighten, unify,
};

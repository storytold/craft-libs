//! Puppet Warp (Edit › Puppet Warp): pins on a deformed mesh ([`PinWarpSession`], [`PinWarp`],
//! [`PinWarpBar`]). The solver is the app's ([`PinSolver`]); the warped picture is drawn by the
//! transform preview through `PreviewMap::pins`.

mod bar;
mod overlay;
mod session;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;

pub use bar::{PinWarpBar, PinWarpBarOutput, PinWarpBarWords};
pub(in crate::ui::warp) use overlay::point_text;
pub use overlay::{PinWarp, PinWarpCommands, PinWarpOutput, PinWarpStyle, PinWarpWords};
pub use session::{Pin, PinDensity, PinEdit, PinMeshView, PinMode, PinSolver, PinWarpOptions, PinWarpSession, PinWarpSettings};

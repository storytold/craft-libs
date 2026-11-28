//! Painting symmetry: the axes ([`Symmetry`]), the guides on the canvas ([`SymmetryOverlay`]), the centre and rotation
//! handle ([`SymmetryHandles`]), the Transform Symmetry session ([`SymmetryTransform`] with its [`SymmetryTool`]) and
//! the options bar's menu button ([`SymmetryMenu`]).
//!
//! Stroke reflection, path symmetry and the brush engine stay in the app's paint engine; the library carries only the
//! axis geometry the screens need.

pub(crate) mod handles;
pub(crate) mod menu;
mod model;
pub(crate) mod overlay;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod transform;

pub use handles::{SymmetryHandles, SymmetryHandlesOutput, SymmetryHandlesStyle};
pub use menu::{SymmetryChoice, SymmetryCommand, SymmetryMenu, SymmetryMenuOutput, SymmetryPathRow, SymmetryWords};
pub use model::{MAX_COORDINATE, Symmetry, SymmetryMode, TURN_STEP_DEGREES};
pub use overlay::{SymmetryOverlay, SymmetryOverlayStyle};
pub use transform::{SymmetryEvent, SymmetryTool, SymmetryTransform};

/// The rotation handle's distance from the centre, as a share of the page's shorter side.
const HANDLE_SHARE: f64 = 0.35;

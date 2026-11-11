//! [`WarpGrid`]: the mesh of a warp (PhotoCraft's Edit › Transform › Warp) on a canvas: patch
//! boundaries, anchors and their handles, dragged by the pointer and moved from the keyboard.
//!
//! Beside it: the Warp group of the options bar ([`WarpFields`]), Puppet Warp ([`pin`]),
//! Perspective Warp ([`perspective`]) and the one painter of a deformed mesh's lines
//! ([`MeshLines`]).

mod fields;
mod grid;
mod mesh;
mod model;
pub mod perspective;
pub mod pin;
#[cfg(feature = "screen-testing")]
mod stories;

pub use fields::{WarpFieldValues, WarpFields, WarpFieldsEdit, WarpFieldsOutput, WarpFieldsWords, WarpGridChoice, WarpStyleEntry};
pub use grid::{POINT, SURFACE, WarpGrid, WarpGridOutput, WarpGridStyle, WarpRule, WarpSurface, WarpWords};
pub(crate) use mesh::MeshLines;
pub use mesh::MeshLinesStyle;
pub use model::{WarpEdit, WarpMesh, WarpSide, WarpSplit};

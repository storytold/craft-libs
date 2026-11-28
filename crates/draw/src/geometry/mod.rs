//! Drawing geometry without egui, shared by the tools and an app's engine.

pub mod artboard;
#[cfg(test)]
pub(crate) mod artboard_tests;
pub mod connector;
#[cfg(test)]
pub(crate) mod connector_tests;
pub mod curvature;
#[cfg(test)]
pub(crate) mod curvature_tests;
pub mod fillet;
#[cfg(test)]
pub(crate) mod fillet_tests;
pub mod presets;
pub mod shape;
pub mod slice;

pub use presets::{PRESETS, ShapePreset};
pub use shape::ShapeGeometry;

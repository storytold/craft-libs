//! Drawing geometry without egui, shared by the tools and an app's engine.

pub mod artboard;
#[cfg(test)]
mod artboard_tests;
pub mod connector;
#[cfg(test)]
mod connector_tests;
pub mod curvature;
#[cfg(test)]
mod curvature_tests;
pub mod fillet;
#[cfg(test)]
mod fillet_tests;
pub mod presets;
pub mod shape;
pub mod slice;

pub use presets::{PRESETS, ShapePreset};
pub use shape::ShapeGeometry;

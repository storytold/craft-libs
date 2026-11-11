//! Android Vector Drawable: a `<vector>` XML drawable written from a small model.
//!
//! Written from Android's public `VectorDrawable` documentation (elements `<vector>`, `<group>`,
//! `<path>`, `<clip-path>`, gradients as inline `GradientColor` resources through `aapt:attr`).
//!
//! - [`model`]: what a drawable holds, in viewport coordinates: paths with a solid or gradient
//!   fill and a stroke Android draws itself, and groups with a scale-rotate-translate transform
//!   and a clip outline.
//! - [`write`]: the XML ([`write::document`]), with its numbers, colours, path data and escaping.
//! - [`paint`]: fitting a gradient drawn through any affine frame to what a drawable can say: a
//!   linear gradient's end point projected, radial and sweep gradients in a `<group>` that maps
//!   the unit circle, a sweep's stops turned.
//!
//! A drawable has no images, patterns, effects, blend modes, masks, dashes or layer opacity, a
//! `<group>` can only scale, rotate and translate, and a `<path>` has no transform at all. What
//! the caller's document has of those is the caller's to bake in, approximate or leave out
//! before it builds the model. There is no reader.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod model;
pub mod paint;
pub mod write;

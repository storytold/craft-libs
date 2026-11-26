//! The gradient stop model and its rules, without egui: [`GradientStops`] and the pure functions of [`stops`].
//!
//! Colour stops and opacity stops live in two rows ([`StopRows::Split`]) or in one, the opacity being the colour's alpha
//! ([`StopRows::Combined`]). The rules are pure functions of the stops, so an on-canvas gradient line and the stop bar
//! cannot disagree. A ramp samples through a [`Sampler`]; the default mixes straight lines in encoded sRGB.

pub mod stops;
mod value;

pub use stops::{Sampler, StopRules, default_sampler};
pub use value::{ColorStop, GradientStops, OpacityStop, StopRow, StopRows};

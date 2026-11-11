//! The counting that scopes share, without egui: the clipping rule, clipping masks, the
//! vectorscope grid, hue and saturation, and histogram statistics. Builds on every target.
//!
//! | Need | Use |
//! |---|---|
//! | Which channels of one pixel clip | [`clipping`] → [`ClippedChannels`] of [`ChannelMask`]s |
//! | A per-pixel clipping image for an overlay | [`ClippingMask::from_rgba`] |
//! | The vectorscope's count grid | [`Vectorscope::from_rgba`], [`HUES`] × [`SATURATIONS`] cells, [`hue_saturation`] |
//! | Mean, Std Dev, Median, Percentile | [`histogram_statistics`] → [`HistogramStatistics`] |
//!
//! Colour management and the histogram count stay in the app: the inputs are straight RGBA in the
//! app's chosen RGB space and counted 256-bin histograms. LightCraft, EffectCraft and PhotoCraft
//! count the same way.

mod clipping;
mod statistics;
#[cfg(test)]
mod tests;
mod vectorscope;

pub use clipping::{ChannelMask, ClippedChannels, ClippingMask, clipping};
pub use statistics::{HistogramStatistics, histogram_statistics};
pub use vectorscope::{HUES, SATURATIONS, Vectorscope, VectorscopeSummary, hue_saturation};

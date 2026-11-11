//! The histogram: a plot of bins in the tone or RGB look, with a selection, markers and a stale mark.
//!
//! - [`Histogram`]: the plot, a leaf over a [`HistogramData`].
//! - [`paint_bins`], [`HistogramInks`], [`PlotFrame`] and [`paint_frame`]: the one paint path of the bins and the one
//!   plot frame, which a curve graph and the scope plots paint too.
//! - [`paint`]: the geometry the paint path is made of (heights, meshes, the selection band, the markers).

mod data;
pub mod paint;
mod plot;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod words;

pub use data::{BinCount, Blend, ChannelColor, HistogramData, HistogramLook, HistogramScale, LevelRange};
pub use paint::{HistogramInks, PlotFrame, level_at, level_x, paint_bins, paint_edge, paint_frame};
pub use plot::{Histogram, HistogramOutput, HistogramStyle, PlotHeight, StaleMark};
pub use words::HistogramWords;

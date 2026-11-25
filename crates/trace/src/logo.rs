//! Logo mode: flat-colour logos traced to clean, sub-pixel-accurate vector shapes.
//!
//! The tracer in the other modes follows hard pixel edges, so an outline can only be as good as
//! the pixel grid. Here each pixel is read as a *blend* of the logo's flat colours, the blend is
//! interpolated smoothly, and every outline is placed where two colours are equally present, to a
//! fraction of a pixel. The outlines are then cleaned the way a person would: corners the blur
//! rounded are put back, straight runs and circular arcs become exact lines and arcs, and the
//! rest is fitted with as few Bézier curves as the tolerance allows. Areas of transparency
//! fully enclosed by artwork (letters knocked out of a badge, the counters of an "o") are painted
//! white, so the logo reads the same on any background; with Ignore White they stay real holes.
//! Flat-colour logos only: gradients and photos are refused ([`TraceError::NotFlat`](crate::TraceError)).

mod bezier;
mod field;
mod geom;
mod march;
mod outline;
mod palette;
mod pipeline;
mod restore;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;
mod unmix;

pub(crate) use bezier::{fit_cubics, tangent};
pub(crate) use geom::{Cubic, all_within, resample_closed, resample_open, sample_cubics, smooth_closed, smooth_pinned};
pub(crate) use outline::{Stats, Style, fit_open, fit_outline};
pub(crate) use palette::parse_hex;
pub(crate) use pipeline::trace_logo;
pub(crate) use restore::RestoreParams;

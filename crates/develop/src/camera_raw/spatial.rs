#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Dehaze, Clarity and Texture resolved for one frame: the sizes, radii and constants their
//! filters use, so another implementation of them (the GPU preview,
//! `photocraft_gpu::CameraRawDevelop`) runs the same filters. [`develop`](super::develop) builds
//! its own from the same constructors.
//!
//! - **Dehaze** follows the dark channel prior (K. He, J. Sun, X. Tang, *Single Image Haze Removal
//!   Using Dark Channel Prior*, CVPR 2009) on a box-averaged copy at most [`DEHAZE_SIDE`] pixels
//!   across, with the transmission refined by a guided filter (K. He, J. Sun, X. Tang, *Guided
//!   Image Filtering*, ECCV 2010) and brought back to full size bilinearly.
//! - **Clarity** raises the luminance's difference from a fast guided filter of itself (K. He,
//!   J. Sun, *Fast Guided Filter*, arXiv 2015), weighted toward the midtones.
//! - **Texture** raises its difference from a Gaussian blur (three box passes), limited to
//!   ±[`TEXTURE_LIMIT`].

use super::{CameraRaw, DevelopInputs};

/// Dehaze measures the haze on a copy at most this many pixels across.
pub const DEHAZE_SIDE: usize = 768;
/// The dark channel's minimum filter has a radius of the copy's long side over this, at least
/// [`DEHAZE_MIN_RADIUS`].
pub const DEHAZE_RADIUS_DIVISOR: usize = 60;
/// See [`DEHAZE_RADIUS_DIVISOR`].
pub const DEHAZE_MIN_RADIUS: usize = 2;
/// The atmospheric light is the mean colour of the brightest 1 / this of the dark channel (at
/// least one cell).
pub const DEHAZE_AIR_FRACTION: usize = 1000;
/// Each channel of the atmospheric light is at least this.
pub const DEHAZE_AIR_FLOOR: f32 = 0.2;
/// The share of the haze the raw transmission removes (the paper's ω).
pub const DEHAZE_OMEGA: f32 = 0.95;
/// The guided filter's regularisation for the transmission.
pub const DEHAZE_EPS: f32 = 1e-3;
/// The transmission is at least this.
pub const DEHAZE_T_FLOOR: f32 = 0.1;
/// Clarity's guided filter regularisation.
pub const CLARITY_EPS: f32 = 0.005;
/// Texture moves the luminance by at most this.
pub const TEXTURE_LIMIT: f32 = 0.1;

/// Dehaze on a `w × h` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Dehaze {
    /// The slider / 100 (−1 … 1).
    pub amount: f32,
    /// The copy averages `factor × factor` blocks.
    pub factor: usize,
    /// The copy's size.
    pub size: [usize; 2],
    /// The minimum filter's radius.
    pub radius: usize,
    /// The guided filter's radius.
    pub guide_radius: usize,
    /// How many of the copy's cells the atmospheric light averages.
    pub top: usize,
}

impl Dehaze {
    /// Dehaze for `p` on a `w × h` frame; `None` when it is off.
    pub fn new(p: &CameraRaw, w: usize, h: usize) -> Option<Self> {
        if p.dehaze == 0.0 {
            return None;
        }
        let factor = w.max(h).div_ceil(DEHAZE_SIDE).max(1);
        let size = [w.div_ceil(factor), h.div_ceil(factor)];
        let radius = (size[0].max(size[1]) / DEHAZE_RADIUS_DIVISOR).max(DEHAZE_MIN_RADIUS);
        Some(Dehaze {
            amount: p.dehaze / 100.0,
            factor,
            size,
            radius,
            guide_radius: radius * 2,
            top: (size[0] * size[1] / DEHAZE_AIR_FRACTION).max(1),
        })
    }

    /// The copy the haze is measured on, for `px` (`w × h`, display-encoded) before white balance
    /// and exposure (`gains`, as in [`PerPixel`](super::PerPixel)): `size` cells in rows, each the
    /// mean light of its block, clipped to 0 … 1. These are exactly the cells
    /// [`develop`](super::develop) uses: the brightest of them set the atmospheric light, and a
    /// tie between them decides which, so another implementation that computes stage 1 to a few
    /// ulps should start from these. Empty when `px` isn't `w × h`.
    pub fn cells(&self, px: &[[f32; 4]], w: usize, h: usize, gains: Option<[f32; 3]>) -> Vec<[f32; 3]> {
        if px.len() != w.saturating_mul(h) || w.div_ceil(self.factor) != self.size[0] || h.div_ceil(self.factor) != self.size[1] {
            return Vec::new();
        }
        super::haze_cells(px, w, h, self, |q| gains.map_or(q, |g| super::balance(q, g)))
    }
}

/// Clarity on a `w × h` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Clarity {
    /// The strength: the slider / 100 × 0.8.
    pub amount: f32,
    /// The guided filter's radius at full size.
    pub radius: usize,
    /// The fast guided filter works on `subsample × subsample` block means…
    pub subsample: usize,
    /// …with this radius…
    pub sub_radius: usize,
    /// …at this size.
    pub sub_size: [usize; 2],
}

impl Clarity {
    /// Clarity for `p` on a `w × h` frame; `None` when it is off.
    pub fn new(p: &CameraRaw, w: usize, h: usize) -> Option<Self> {
        if p.clarity == 0.0 {
            return None;
        }
        let radius = ((w.max(h) as f32 * 0.02) as usize).max(3);
        let (subsample, sub_radius) = fast_guided_steps(radius);
        Some(Clarity { amount: p.clarity / 100.0 * 0.8, radius, subsample, sub_radius, sub_size: [w.div_ceil(subsample), h.div_ceil(subsample)] })
    }
}

/// The fast guided filter's subsampling and its radius there, for radius `r` at full size. A
/// subsampling of 1 is the plain guided filter at radius `r`.
pub fn fast_guided_steps(r: usize) -> (usize, usize) {
    let s = (r / 8).clamp(1, 8);
    (s, if s == 1 { r } else { (r / s).max(1) })
}

/// Texture on a `w × h` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    /// The strength: the slider / 100 × 0.9.
    pub amount: f32,
    /// The blur's box radii, in order; empty when the blur is too small to change anything.
    pub radii: Vec<usize>,
}

impl Texture {
    /// Texture for `p` on a `w × h` frame; `None` when it is off.
    pub fn new(p: &CameraRaw, inputs: &DevelopInputs, w: usize, h: usize) -> Option<Self> {
        if p.texture == 0.0 {
            return None;
        }
        let ps = inputs.pixel_scale.clamp(0.01, 1.0);
        let sigma = (w.max(h) as f32 * 0.002).max(1.5 * ps);
        Some(Texture { amount: p.texture / 100.0 * 0.9, radii: gauss_radii(sigma) })
    }
}

/// The box radii of develop's Gaussian of `sigma`: none below 0.3.
pub(crate) fn gauss_radii(sigma: f32) -> Vec<usize> {
    if sigma < 0.3 {
        return Vec::new();
    }
    super::util::gauss_box_radii(sigma).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CameraRaw` with the named fields set (the type is `#[non_exhaustive]`, so no struct expression), or a copy of a base with them set.
    macro_rules! cr {
        ($($f:ident : $v:expr),* $(,)?) => {{
            let mut s = CameraRaw::default();
            $(s.$f = $v;)*
            s
        }};
        ($base:expr => $($f:ident : $v:expr),* $(,)?) => {{
            let mut s: CameraRaw = $base;
            $(s.$f = $v;)*
            s
        }};
    }

    #[test]
    fn stages_resolve_for_the_dialogs_proxy_and_are_off_at_zero() {
        let p = cr! { dehaze: 40.0, clarity: -50.0, texture: 50.0};
        let scale = |pixel_scale| DevelopInputs { pixel_scale, ..Default::default() };
        let d = Dehaze::new(&p, 858, 572).unwrap();
        assert_eq!((d.factor, d.size, d.radius, d.guide_radius, d.top), (2, [429, 286], 7, 14, 122));
        let c = Clarity::new(&p, 858, 572).unwrap();
        assert_eq!((c.radius, c.subsample, c.sub_radius, c.sub_size), (17, 2, 8, [429, 286]));
        assert!((c.amount + 0.4).abs() < 1e-6);
        assert_eq!(Texture::new(&p, &scale(0.143), 858, 572).unwrap().radii, [1, 1, 2]);
        // Small frames: the plain guided filter, a blur too narrow to change anything.
        assert_eq!(fast_guided_steps(3), (1, 3));
        assert!(Texture::new(&cr! { texture: 10.0}, &scale(0.1), 64, 48).unwrap().radii.is_empty());
        let off = CameraRaw::default();
        assert!(
            Dehaze::new(&off, 858, 572).is_none() && Clarity::new(&off, 858, 572).is_none() && Texture::new(&off, &scale(1.0), 858, 572).is_none()
        );
    }

    #[test]
    fn dehaze_cells_average_blocks_after_white_balance_and_refuse_a_wrong_size() {
        let d = Dehaze::new(&cr! { dehaze: 10.0}, 3, 1).unwrap();
        let px = [[0.2, 0.4, 1.5, 1.0], [0.4, 0.0, 0.5, 1.0], [0.9, 0.9, 0.9, 1.0]];
        assert_eq!(d.cells(&px, 3, 1, None), vec![[0.2, 0.4, 1.0], [0.4, 0.0, 0.5], [0.9, 0.9, 0.9]]);
        let brighter = d.cells(&px, 3, 1, Some([2.0, 2.0, 2.0]));
        assert!(brighter[0][0] > 0.2 && brighter[2] == [1.0; 3]);
        assert!(d.cells(&px[..2], 3, 1, None).is_empty());
        assert!(d.cells(&px, 1, 3, None).is_empty());
    }
}

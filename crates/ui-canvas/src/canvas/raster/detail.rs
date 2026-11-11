//! `DetailPlan`: the detail texture a zoomed-in view of a reduced document needs, and the texel-grid maths of
//! region updates.

use crate::canvas::PixelRect;
use crate::primitives::texture_factor;
use craft_behavior::{CanvasView, PixelSize};

/// The detail region snaps outward to multiples of this many texels.
const DETAIL_QUANTUM: i32 = 256;

/// The coarsest texel, in document pixels, a detail texture may use.
const COARSEST_PROXY: u32 = 64;

/// The visible region grows by this many pixels on every side so filtering at its edges reads real pixels.
const FILTER_MARGIN: i32 = 2;

/// The detail texture a zoomed-in view of a reduced document needs: the part in view (`region`, snapped outward to
/// 256-texel steps so a small pan reuses it) at one texel per `factor` pixels, a power of two no coarser than the
/// screen and capped by the texture side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct DetailPlan {
    /// Pixels of the document per texel; a power of two.
    pub factor: u32,
    /// The part of the document in view, in document pixels.
    pub region: PixelRect,
}

/// The largest power of two, up to `COARSEST_PROXY`, whose texels are still no coarser than a device pixel at
/// `device_zoom`.
fn proxy_factor(device_zoom: f64) -> u32 {
    let mut k = 1u32;
    while k < COARSEST_PROXY && f64::from(k * 2) * device_zoom <= 1.0 {
        k *= 2;
    }
    k
}

impl DetailPlan {
    /// The plan for a view at `device_zoom` (device pixels per document pixel) over `visible`, or `None` when the
    /// base texture is already as fine as the screen or nothing is in view. `size` is the document and `side` the
    /// device's texture side.
    pub fn of(size: PixelSize, side: u32, device_zoom: f64, visible: PixelRect) -> Option<DetailPlan> {
        if !device_zoom.is_finite() || device_zoom <= 0.0 {
            return None;
        }
        let base = texture_factor(size, side);
        let bounds = PixelRect::of_size(size);
        let seen = visible.intersect(&bounds);
        if seen.is_empty() {
            return None;
        }
        let mut factor = proxy_factor(device_zoom);
        while factor < base {
            let quantum = DETAIL_QUANTUM.saturating_mul(i32::try_from(factor).unwrap_or(i32::MAX));
            let down = |x: i32| x.div_euclid(quantum).saturating_mul(quantum);
            let up = |x: i32| x.div_euclid(quantum).saturating_add(i32::from(x.rem_euclid(quantum) != 0)).saturating_mul(quantum);
            let snapped = PixelRect::new(down(seen.x0), down(seen.y0), up(seen.x1), up(seen.y1)).intersect(&bounds);
            // A reduced region's texel count can round one up.
            let fits = |n: i32| n.unsigned_abs().div_ceil(factor).saturating_add(1) <= side;
            // Snapped when that fits, else just what is in view (a window near the device's limit).
            for region in [snapped, seen] {
                if !region.is_empty() && fits(region.width()) && fits(region.height()) {
                    return Some(DetailPlan { factor, region });
                }
            }
            factor = factor.saturating_mul(2);
        }
        None
    }

    /// The plan for `view` over a document of `size`: the document pixels it shows, and the device zoom of its
    /// finer axis at `pixels_per_point`.
    pub fn in_view(view: &CanvasView, size: PixelSize, side: u32, pixels_per_point: f32) -> Option<DetailPlan> {
        let device_zoom = view.zoom.max(view.zoom_x()) * f64::from(pixels_per_point);
        Self::of(size, side, device_zoom, visible_pixels(view, size))
    }
}

/// The document pixels `view` shows (its bounding box when turned), rounded outward with a margin for filtering and
/// clipped to a document of `size`.
fn visible_pixels(view: &CanvasView, size: PixelSize) -> PixelRect {
    let shown = view.visible();
    let clamp = |n: f64| n.clamp(-1e9, 1e9) as i32;
    let seen = PixelRect::new(
        clamp(shown.x0.floor()) - FILTER_MARGIN,
        clamp(shown.y0.floor()) - FILTER_MARGIN,
        clamp(shown.x1.ceil()) + FILTER_MARGIN,
        clamp(shown.y1.ceil()) + FILTER_MARGIN,
    );
    seen.intersect(&PixelRect::of_size(size))
}

/// `region` grown outward to whole texels of `factor` pixels counted from `origin`, the document pixel where the
/// texture starts. A base texture starts at `[0, 0]`; a detail region that fell back to the visible rect does not
/// start at a multiple of `factor`, and a region update must land on whole texels of its own grid.
pub fn snap_to_texels(region: PixelRect, origin: [i32; 2], factor: u32) -> PixelRect {
    let f = i32::try_from(factor.max(1)).unwrap_or(i32::MAX);
    let [dx, dy] = origin;
    let down = |n: i32| n.div_euclid(f).saturating_mul(f);
    let up = |n: i32| n.div_euclid(f).saturating_add(i32::from(n.rem_euclid(f) != 0)).saturating_mul(f);
    PixelRect::new(
        down(region.x0 - dx).saturating_add(dx),
        down(region.y0 - dy).saturating_add(dy),
        up(region.x1 - dx).saturating_add(dx),
        up(region.y1 - dy).saturating_add(dy),
    )
}

/// The size of `region` in texels at one texel per `factor` pixels, rounded up.
pub(super) fn texel_extent(region: PixelRect, factor: u32) -> [usize; 2] {
    let f = factor.max(1) as usize;
    [region.width() as usize, region.height() as usize].map(|n| n.div_ceil(f))
}

/// Where `region` starts, in texels, in a texture that holds `origin` at one texel per `factor` pixels.
pub(super) fn texel_origin(region: PixelRect, origin: [i32; 2], factor: u32) -> [u32; 2] {
    let f = factor.max(1);
    [(region.x0 - origin[0]).max(0) as u32 / f, (region.y0 - origin[1]).max(0) as u32 / f]
}

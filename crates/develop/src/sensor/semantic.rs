//! A DNG semantic mask ([`craft_raw::SemanticMask`]) over the developed picture.

use crate::raster::Image;
use craft_raw::semantic::MAX_MASK_SAMPLES;
use craft_raw::{Rect, SemanticMask};

/// The mask over the developed image ([`super::Develop::develop`]: the default `crop` of the `active_area`,
/// before orientation), at the mask's own resolution, as 0..255 alpha. `None` for an empty crop.
pub fn developed(mask: &SemanticMask, active_area: Rect, crop: Rect) -> Option<Image<u8>> {
    let c = crop.clipped(active_area.width, active_area.height);
    if c.width == 0 || c.height == 0 || mask.width == 0 || mask.height == 0 {
        return None;
    }
    let (fw, fh) = mask.sub_area.map_or((mask.width, mask.height), |a| (a.full_width as usize, a.full_height as usize));
    // uncropped-mask px per active-area px
    let (sx, sy) = (fw as f64 / active_area.width as f64, fh as f64 / active_area.height as f64);
    let ow = ((c.width as f64 * sx).round() as usize).clamp(1, mask.width.max(fw));
    let oh = ((c.height as f64 * sy).round() as usize).clamp(1, mask.height.max(fh));
    if ow.checked_mul(oh).is_none_or(|n| n > MAX_MASK_SAMPLES) {
        return None;
    }
    let (kx, ky) = (c.width as f64 / ow as f64, c.height as f64 / oh as f64);
    let mut out = Image::new(ow, oh);
    crate::raster::par_rows(&mut out.data, ow, |y, row| {
        let ay = c.y as f64 + (y as f64 + 0.5) * ky;
        for (x, v) in row.iter_mut().enumerate() {
            let ax = c.x as f64 + (x as f64 + 0.5) * kx;
            *v = (mask.value_at(ax * sx, ay * sy) * 255.0).round() as u8;
        }
    });
    Some(out)
}

//! The alternate record's radial map, which applies the crop zoom before looking up the table. The 11-setting spacing
//! (128/13 intervals per crop half-diagonal) is empirical, not a published Sony specification.
use crate::lens_metadata::SonyLensGeometry;
use crate::{Opcode, Rect};

const FIT_ERROR_PX: f64 = 0.35;
/// The table's unit: a word of 32768 doubles the radius.
const UNIT: f64 = 32768.0;
/// Largest word accepted, ±25 % of the radius.
const MAX_WORD: i16 = 8192;

struct Map<'a> {
    table: &'a [i16; 19],
    intervals: f64,
    zoom: f64,
}

impl<'a> Map<'a> {
    fn new(table: &'a [i16; 19], count: u16, aspect: f64) -> Option<Self> {
        if table[0] != 0 {
            return None;
        }
        let intervals = match count {
            16 => 15.0,
            11 => 128.0 / 13.0,
            _ => return None,
        };
        let mut map = Self { table, intervals, zoom: 1.0 };
        // Sony's localized controls reject a maximum-over-all-border-radii crop. The measured
        // endpoint rule leaves small residual scale differences; it is not an exact Sony crop claim.
        let edge = if map.value(intervals) < 0.0 { 1.0 / aspect.hypot(1.0) } else { 1.0 };
        let (mut lo, mut hi) = (0.8, 1.2);
        let ratio = |z| (1.0 + map.value(edge / z * intervals) / UNIT) / z;
        if ratio(lo) <= 1.0 || ratio(hi) >= 1.0 {
            return None;
        }
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if ratio(mid) > 1.0 { lo = mid } else { hi = mid }
        }
        map.zoom = (lo + hi) / 2.0;
        // Bound and reject folds independently of the later polynomial approximation.
        let mut previous = -1.0;
        for i in 0..=8192 {
            let mapped = map.source(i as f64 / 8192.0);
            if !mapped.is_finite() || mapped <= previous {
                return None;
            }
            previous = mapped;
        }
        Some(map)
    }

    fn value(&self, at: f64) -> f64 {
        let last = self.table.len() - 1;
        let at = at.clamp(0.0, last as f64);
        let i = (at as usize).min(last - 1);
        let a = f64::from(self.table[i]);
        a + (f64::from(self.table[i + 1]) - a) * (at - i as f64)
    }

    /// Corrected output radius and returned source radius, both in crop half-diagonal units.
    fn source(&self, r: f64) -> f64 {
        let u = r / self.zoom;
        u * (1.0 + self.value(u * self.intervals) / UNIT)
    }
}

/// The record's warp on any frame with an optical reference ([`crate::arw_lens::optical_reference`]): `count` (11 or
/// 16) picks the knot spacing, and the words stay within ±25 %. The four-term fit keeps its positional bound; a curve
/// it cannot follow (a strong wide-angle barrel) takes the higher-order one under the same bound.
pub(super) fn general_opcode(table: &[i16; 19], count: u16, g: &SonyLensGeometry) -> Option<Opcode> {
    let crop = crate::arw_lens::optical_reference(g)?;
    if table.iter().all(|&v| v == 0) || table.iter().any(|v| !(-MAX_WORD..=MAX_WORD).contains(v)) {
        return None;
    }
    let map = Map::new(table, count, crop.width as f64 / crop.height as f64)?;
    fit_opcode(&map, g, crop).or_else(|| {
        let knots = (0..table.len()).map(|i| i as f64 * map.zoom / map.intervals).filter(|&r| r <= 1.0);
        crate::arw_lens::high_order::fit_map(Rect::new(0, 0, g.storage_width, g.storage_height), crop, knots, |r| map.source(r), FIT_ERROR_PX)
    })
}

fn fit_opcode(map: &Map<'_>, g: &SonyLensGeometry, crop: Rect) -> Option<Opcode> {
    // a one-pixel extent has no span to normalise the centre by (0 / 0)
    if g.storage_width < 2 || g.storage_height < 2 || crop.width < 2 || crop.height < 2 {
        return None;
    }
    let (w, h) = (crop.width as f64, crop.height as f64);
    let radius = w.hypot(h) / 2.0;
    let (cx, cy) = (crop.x as f64 + (w - 1.0) / 2.0, crop.y as f64 + (h - 1.0) / 2.0);
    let (aw, ah) = (g.storage_width as f64 - 1.0, g.storage_height as f64 - 1.0);
    let m = crate::opcodes::max_radius(cx, cy, aw, ah);
    // Padding is not part of the displayed image. Fitting over its radius biases strong barrel
    // curves differently in M/S and native buffers; fit the actual crop, then change radial units.
    let error = FIT_ERROR_PX * w / 1440.0 / radius;
    let k = crate::shared::fit_radial_warp(&|u| map.source(u), error, 1.0)?;
    // The fit samples are not its validation samples. Test densely throughout the displayed
    // crop, plus every interpolation knot, to reject local bumps hidden between fit samples.
    for u in (0..=32768).map(|i| i as f64 / 32768.0).chain((0..map.table.len()).map(|i| i as f64 / map.intervals * map.zoom).filter(|&u| u <= 1.0)) {
        let u2 = u * u;
        let fit = u * (k[0] + u2 * (k[1] + u2 * (k[2] + u2 * k[3])));
        let derivative = k[0] + u2 * (3.0 * k[1] + u2 * (5.0 * k[2] + u2 * 7.0 * k[3]));
        if (fit - map.source(u)).abs() > error || derivative <= 0.0 {
            return None;
        }
    }
    let scale = m / radius;
    let k: [f64; 4] = std::array::from_fn(|i| k[i] * scale.powi(2 * i as i32));
    // The opcode can also evaluate padded pixels before the final crop. Keep that extension
    // finite and non-folding, without claiming a measured table fit outside the photograph.
    for i in 0..=8192 {
        let u2 = (i as f64 / 8192.0).powi(2);
        let derivative = k[0] + u2 * (3.0 * k[1] + u2 * (5.0 * k[2] + u2 * 7.0 * k[3]));
        if !derivative.is_finite() || derivative <= 0.0 {
            return None;
        }
    }
    Some(Opcode::WarpRectilinear { planes: vec![[k[0], k[1], k[2], k[3], 0.0, 0.0]], center: [cx / aw, cy / ah] })
}

#[cfg(test)]
mod tests;

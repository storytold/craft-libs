//! Ink coverage read from plates: per ink at a point and over a page, total area coverage
//! (TAC) per pixel, and the pixels over a TAC limit.
//!
//! Inks are chosen with an `enabled` list indexed by [`InkId::index`], as a Separations
//! Preview's toggles hold them: inks past its end are enabled, so `&[]` enables every ink.
//! Percentages are of one ink's 100%, so a TAC runs to 100 × the number of inks.

use crate::{InkId, Plates};

/// The most regions [`Above::regions`] lists.
pub const MAX_REGIONS: usize = 256;

/// Coverage at one pixel, in percent.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    /// Each ink's tint, in [`InkSet::iter`](crate::InkSet::iter) order, enabled or not.
    pub inks: Vec<f32>,
    /// The total area coverage: the sum of the enabled inks' tints.
    pub total: f32,
}

/// One ink's coverage over a page, in percent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InkStats {
    /// The highest tint.
    pub max: f32,
    /// The mean tint over every pixel of the page.
    pub mean: f32,
    /// The share of the page the ink prints on (any tint above 0).
    pub area: f32,
}

/// A page's coverage figures, in percent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Each ink's figures, in [`InkSet::iter`](crate::InkSet::iter) order, enabled or not.
    pub inks: Vec<InkStats>,
    /// The highest TAC of the enabled inks.
    pub max_total: f32,
    /// The first pixel (in row order) with [`max_total`](Self::max_total); `None` for an empty
    /// page.
    pub max_total_at: Option<(u32, u32)>,
    /// The mean TAC of the enabled inks.
    pub mean_total: f32,
}

/// The pixels whose TAC is over a limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Above {
    pub width: u32,
    pub height: u32,
    /// One entry per pixel, row-major: `true` where the TAC is over the limit.
    pub mask: Vec<bool>,
    /// The number of pixels over the limit.
    pub count: usize,
}

/// A connected group of pixels over the limit (4-connected), with its bounding box in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// The pixels over the limit in the region.
    pub pixels: u32,
}

/// `len` copies of `value`; `None` when the memory can't be had.
pub(crate) fn filled<T: Clone>(len: usize, value: T) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, value);
    Some(v)
}

fn on(enabled: &[bool], index: usize) -> bool {
    enabled.get(index).copied().unwrap_or(true)
}

fn percent(tint: u64) -> f32 {
    (tint as f64 * 100.0 / 255.0) as f32
}

impl Plates {
    /// `width × height`; `None` when it overflows `usize`, as it can for hand-made plates.
    pub(crate) fn pixels(&self) -> Option<usize> {
        usize::try_from(self.width).ok()?.checked_mul(usize::try_from(self.height).ok()?)
    }

    pub(crate) fn index_of(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        usize::try_from(y).ok()?.checked_mul(usize::try_from(self.width).ok()?)?.checked_add(usize::try_from(x).ok()?)
    }

    fn enabled_planes<'a>(&'a self, enabled: &'a [bool]) -> impl Iterator<Item = &'a Vec<u8>> + 'a {
        self.planes.iter().enumerate().filter(|(i, _)| on(enabled, *i)).map(|(_, p)| p)
    }

    /// The pixel under a point given as fractions `0.0..1.0` of the page's width and height,
    /// as a pointer over a preview gives it.
    pub fn pixel_at(&self, u: f32, v: f32) -> Option<(u32, u32)> {
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return None;
        }
        let x = ((u * self.width as f32) as u32).min(self.width.checked_sub(1)?);
        let y = ((v * self.height as f32) as u32).min(self.height.checked_sub(1)?);
        Some((x, y))
    }

    /// Each ink's tint and the TAC of the enabled inks at pixel (`x`, `y`).
    pub fn coverage_at(&self, x: u32, y: u32, enabled: &[bool]) -> Option<Coverage> {
        let at = self.index_of(x, y)?;
        let tints: Vec<u64> = self.planes.iter().map(|p| p.get(at).copied().map_or(0, u64::from)).collect();
        let total = tints.iter().enumerate().filter(|(i, _)| on(enabled, *i)).map(|(_, t)| t).sum();
        Some(Coverage { inks: tints.into_iter().map(percent).collect(), total: percent(total) })
    }

    /// Ink `id`'s tint at pixel (`x`, `y`) in percent.
    pub fn percent_at(&self, id: InkId, x: u32, y: u32) -> Option<f32> {
        self.tint(id, x, y).map(|t| percent(u64::from(t)))
    }

    /// The TAC of the enabled inks at every pixel, row-major, as a sum of tint bytes (255 per
    /// ink at 100%). Empty when `width × height` overflows or can't be allocated.
    fn totals(&self, enabled: &[bool]) -> Vec<u32> {
        let Some(mut totals) = self.pixels().and_then(|n| filled(n, 0_u32)) else { return Vec::new() };
        for plane in self.enabled_planes(enabled) {
            for (sum, tint) in totals.iter_mut().zip(plane) {
                *sum = sum.saturating_add(u32::from(*tint));
            }
        }
        totals
    }

    /// The TAC of the enabled inks at every pixel, row-major, in percent.
    pub fn tac(&self, enabled: &[bool]) -> Vec<f32> {
        self.totals(enabled).into_iter().map(|t| percent(u64::from(t))).collect()
    }

    /// Each ink's figures over the page, and the TAC of the enabled inks.
    pub fn stats(&self, enabled: &[bool]) -> Stats {
        let n = self.pixels().unwrap_or(0);
        let mean = |sum: u64| if n == 0 { 0.0 } else { percent(sum) / n as f32 };
        let inks = self
            .planes
            .iter()
            .map(|plane| {
                let plane = plane.get(..n).unwrap_or(plane);
                let max = plane.iter().copied().max().unwrap_or(0);
                let sum: u64 = plane.iter().map(|t| u64::from(*t)).sum();
                let inked = plane.iter().filter(|t| **t > 0).count();
                let area = if n == 0 { 0.0 } else { inked as f32 * 100.0 / n as f32 };
                InkStats { max: percent(u64::from(max)), mean: mean(sum), area }
            })
            .collect();
        let totals = self.totals(enabled);
        // The first pixel with the highest total: `max_by_key` keeps the last of equals.
        let top = totals.iter().enumerate().rev().max_by_key(|(_, t)| **t);
        let width = self.width as usize;
        let max_total_at = top.and_then(|(at, _)| Some((u32::try_from(at % width).ok()?, u32::try_from(at / width).ok()?)));
        Stats {
            inks,
            max_total: top.map_or(0.0, |(_, t)| percent(u64::from(*t))),
            max_total_at,
            mean_total: mean(totals.iter().map(|t| u64::from(*t)).sum()),
        }
    }

    /// The pixels whose TAC of the enabled inks is over `limit` percent (such as 300 for
    /// coated paper). A limit that isn't finite marks nothing.
    pub fn above(&self, limit: f32, enabled: &[bool]) -> Above {
        let limit = if limit.is_finite() { limit.max(0.0) } else { f32::MAX };
        let mask: Vec<bool> = self.totals(enabled).into_iter().map(|t| percent(u64::from(t)) > limit).collect();
        let count = mask.iter().filter(|m| **m).count();
        Above { width: self.width, height: self.height, mask, count }
    }
}

impl Above {
    /// The connected regions of marked pixels, largest first (ties in row order of their first
    /// pixel), at most [`MAX_REGIONS`] of them.
    pub fn regions(&self) -> Vec<Region> {
        let (w, h) = (self.width as usize, self.height as usize);
        if self.mask.len() != w.saturating_mul(h) {
            return Vec::new();
        }
        let mut seen = vec![false; self.mask.len()];
        let mut regions = Vec::new();
        let mut stack = Vec::new();
        for start in 0..self.mask.len() {
            if !self.mask.get(start).copied().unwrap_or(false) || seen.get(start).copied().unwrap_or(true) {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1, mut pixels) = (start % w, start / w, start % w, start / w, 0_u32);
            if let Some(s) = seen.get_mut(start) {
                *s = true;
            }
            stack.push(start);
            while let Some(at) = stack.pop() {
                let (x, y) = (at % w, at / w);
                pixels = pixels.saturating_add(1);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                let next = [(x > 0).then(|| at - 1), (x + 1 < w).then(|| at + 1), (y > 0).then(|| at - w), (y + 1 < h).then(|| at + w)];
                for n in next.into_iter().flatten() {
                    if self.mask.get(n).copied().unwrap_or(false)
                        && let Some(s) = seen.get_mut(n).filter(|s| !**s)
                    {
                        *s = true;
                        stack.push(n);
                    }
                }
            }
            // The mask is at most `width × height` with both `u32`, so coordinates fit.
            regions.push(Region { x: x0 as u32, y: y0 as u32, width: (x1 - x0 + 1) as u32, height: (y1 - y0 + 1) as u32, pixels });
        }
        regions.sort_by_key(|r| std::cmp::Reverse(r.pixels));
        regions.truncate(MAX_REGIONS);
        regions
    }

    /// Paints the marked pixels of an sRGB8 image of the same size (three bytes per pixel,
    /// such as [`Plates::composite`]'s) in `color`, as a TAC warning.
    pub fn paint(&self, rgb: &mut [u8], color: [u8; 3]) {
        for (px, _) in rgb.as_chunks_mut::<3>().0.iter_mut().zip(&self.mask).filter(|(_, m)| **m) {
            *px = color;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Alternate, Conversion, Ink, InkSet, Spot};

    /// Plates of `width × height` pixels for the process inks and a Gold spot (Lab alternate
    /// `gold`), from planes given in `C M Y K Gold` order.
    pub(crate) fn synthetic(width: u32, height: u32, gold: [f32; 3], planes: [Vec<u8>; 5]) -> Plates {
        let mut inks = InkSet::new();
        inks.add(Ink::Spot(Spot { name: "Gold".into(), alternate: Alternate::Lab(gold), solidity: None }));
        Plates { inks, unseparated: Vec::new(), width, height, dpi: 72.0, conversion: Conversion::DeviceFormulas, planes: planes.to_vec() }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn coverage_tac_and_regions_over_a_limit() {
        // 3 × 2 pixels: rich black with Gold at (0,0), (1,0) (Gold at 50%) and (2,1); black
        // alone at (2,0).
        let p = synthetic(
            3,
            2,
            [70.0, 5.0, 60.0],
            [vec![0; 6], vec![255, 255, 0, 0, 0, 255], vec![255, 255, 0, 0, 0, 255], vec![255, 255, 255, 0, 0, 255], vec![255, 128, 0, 0, 0, 255]],
        );
        let at = p.coverage_at(1, 0, &[]).unwrap();
        assert!(close(at.inks[4], 50.196) && close(at.total, 350.196), "{at:?}");
        assert!(close(p.coverage_at(1, 0, &[true, true, true, true, false]).unwrap().total, 300.0), "Gold hidden");
        assert_eq!(p.coverage_at(3, 0, &[]), None);
        assert_eq!(p.pixel_at(0.99, 0.6), Some((2, 1)));
        assert_eq!(p.pixel_at(1.0, 0.0), None);
        assert!(close(p.tac(&[])[2], 100.0));

        let s = p.stats(&[]);
        assert!(close(s.max_total, 400.0) && s.max_total_at == Some((0, 0)), "{s:?}");
        assert!(close(s.mean_total, 3188.0 / 1530.0 * 100.0), "{s:?}");
        let k = s.inks[3];
        assert!(close(k.max, 100.0) && close(k.mean, 66.667) && close(k.area, 66.667), "{k:?}");
        let gold = s.inks[4];
        assert!(close(gold.mean, 638.0 / 1530.0 * 100.0) && close(gold.area, 50.0), "{gold:?}");
        assert_eq!(s.inks[0], InkStats::default());

        let over = p.above(300.0, &[]);
        assert_eq!(over.count, 3);
        assert_eq!(over.mask, [true, true, false, false, false, true]);
        let regions = over.regions();
        assert_eq!(regions, [Region { x: 0, y: 0, width: 2, height: 1, pixels: 2 }, Region { x: 2, y: 1, width: 1, height: 1, pixels: 1 }]);
        assert_eq!(p.above(300.0, &[true, true, true, true, false]).count, 0, "exactly at the limit is not over it");
        assert_eq!(p.above(f32::NAN, &[]).count, 0);

        let mut rgb = vec![0_u8; 18];
        over.paint(&mut rgb, [255, 0, 255]);
        assert_eq!(&rgb[..6], [255, 0, 255, 255, 0, 255]);
        assert_eq!(&rgb[6..9], [0, 0, 0]);
    }

    #[test]
    fn regions_join_four_connected_pixels_and_keep_the_largest() {
        // A ring around an unmarked center is one region; the diagonal neighbor is another.
        #[rustfmt::skip]
        let mask = vec![
            true,  true,  true,  false,
            true,  false, true,  false,
            true,  true,  true,  false,
            false, false, false, true,
        ];
        let above = Above { width: 4, height: 4, count: 9, mask };
        assert_eq!(above.regions(), [Region { x: 0, y: 0, width: 3, height: 3, pixels: 8 }, Region { x: 3, y: 3, width: 1, height: 1, pixels: 1 }]);
        let empty = Plates { width: 0, height: 0, planes: Vec::new(), ..synthetic(0, 0, [0.0; 3], Default::default()) };
        assert_eq!(empty.stats(&[]).max_total_at, None);
        assert!(empty.above(10.0, &[]).regions().is_empty());
    }
}

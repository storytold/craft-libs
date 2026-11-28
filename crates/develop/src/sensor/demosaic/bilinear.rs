//! Bilinear demosaicing for any CFA pattern: each missing colour is the mean of the same-colour samples in the
//! 3×3 neighbourhood (5×5 when the 3×3 has none, as happens in some non-Bayer layouts).

use super::Mosaic;
use crate::raster::par_rows;
use crate::sensor::highlight::{CLIPPED_BITS, FROM_CLIPPED_BITS};
use crate::sensor::{Rect, Rgb32f};

type PhaseTaps = [Vec<(isize, isize)>; 3];

/// Offsets (dx, dy) of same-colour neighbours for each pattern position and colour.
fn neighbour_table(m: &Mosaic) -> Vec<PhaseTaps> {
    let cfa = m.cfa;
    let mut table = Vec::with_capacity(cfa.width * cfa.height);
    for py in 0..cfa.height {
        for px in 0..cfa.width {
            let mut per: [Vec<(isize, isize)>; 3] = Default::default();
            for (c, list) in per.iter_mut().enumerate() {
                for r in [1isize, 2] {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let (x, y) = (px as isize + dx + 6 * cfa.width as isize, py as isize + dy + 6 * cfa.height as isize);
                            if cfa.color_at(x as usize, y as usize) as usize == c {
                                list.push((dx, dy));
                            }
                        }
                    }
                    if !list.is_empty() {
                        break;
                    }
                }
            }
            table.push(per);
        }
    }
    table
}

/// Visit the samples that contribute to a missing channel, in interpolation order.
#[inline]
fn for_each_tap(m: &Mosaic, table: &[PhaseTaps], x: usize, y: usize, c: usize, mut visit: impl FnMut(f32)) {
    let cfa = m.cfa;
    if x >= 2 && y >= 2 && x + 2 < m.w && y + 2 < m.h {
        let entry = &table[(y % cfa.height) * cfa.width + (x % cfa.width)];
        for &(dx, dy) in &entry[c] {
            visit(m.data[(y as isize + dy) as usize * m.w + (x as isize + dx) as usize]);
        }
        return;
    }
    for r in [1isize, 2] {
        let mut found = false;
        for dy in -r..=r {
            for dx in -r..=r {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if m.color(nx, ny) as usize == c {
                    visit(m.get(nx, ny));
                    found = true;
                }
            }
        }
        if found {
            break;
        }
    }
}

pub(crate) fn bilinear(m: &Mosaic) -> Rgb32f {
    let (w, h) = (m.w, m.h);
    let table = neighbour_table(m);
    let cfa = m.cfa;
    let mut out = Rgb32f::new(w, h);
    par_rows(&mut out.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let own = cfa.color_at(x, y) as usize;
            for (c, channel) in px.iter_mut().enumerate() {
                if c == own {
                    *channel = m.data[y * w + x];
                    continue;
                }
                let (mut s, mut n) = (0.0f32, 0u32);
                for_each_tap(m, &table, x, y, c, |v| {
                    s += v;
                    n += 1;
                });
                *channel = if n > 0 { s / n as f32 } else { m.data[y * w + x] };
            }
        }
    });
    out
}

pub(in crate::sensor) fn clip_mask(m: &Mosaic, crop: Rect, clip: f32) -> Vec<u8> {
    let table = neighbour_table(m);
    let mut mask = vec![0; crop.width * crop.height];
    par_rows(&mut mask, crop.width, |oy, row| {
        let y = crop.y + oy;
        for (ox, bits) in row.iter_mut().enumerate() {
            let x = crop.x + ox;
            let own = m.cfa.color_at(x, y) as usize;
            if m.data[y * m.w + x] >= clip {
                *bits = CLIPPED_BITS[own];
                continue;
            }
            // Each channel uses only its own taps, so leave the measured-colour field unset.
            for (c, bit) in FROM_CLIPPED_BITS.into_iter().enumerate().filter(|&(c, _)| c != own) {
                for_each_tap(m, &table, x, y, c, |v| {
                    if v >= clip {
                        *bits |= bit;
                    }
                });
            }
        }
    });
    mask
}

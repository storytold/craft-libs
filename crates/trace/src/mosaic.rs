//! Object → Create Object Mosaic: average colours of a grid of tiles over a raster.

use crate::Raster;

/// The most tiles along a side.
const MAX_SIDE: u32 = 4096;

/// Average colour (straight RGBA) of each of `cols` × `rows` tiles covering `img`, row by row.
/// Transparent pixels count by their alpha, so a tile over transparency fades out. Tiles over the
/// same pixels (more tiles than pixels) are averaged once, and an empty raster or one whose bytes
/// are not `width * height * 4` gives transparent tiles.
pub fn mosaic(img: &Raster, cols: u32, rows: u32) -> Vec<[u8; 4]> {
    let (cols, rows) = (cols.clamp(1, MAX_SIDE), rows.clamp(1, MAX_SIDE));
    let tiles = cols as usize * rows as usize;
    let (w, h) = (img.width as usize, img.height as usize);
    // An empty raster has nothing to average (its rows would still be walked, up to 2^32 of them).
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(img.rgba.len()) {
        return vec![[0; 4]; tiles];
    }
    let mut out = Vec::with_capacity(tiles);
    // The pixel rows of the last row of tiles, and where its tiles start in `out`.
    let mut last_row: Option<((u32, u32), usize)> = None;
    for r in 0..rows {
        let y0 = (r as u64 * img.height as u64 / rows as u64) as u32;
        let y1 = (((r + 1) as u64 * img.height as u64 / rows as u64) as u32).max(y0 + 1).min(img.height);
        if let Some((range, start)) = last_row
            && range == (y0, y1)
        {
            out.extend_from_within(start..start + cols as usize);
            continue;
        }
        let start = out.len();
        let mut last: Option<((u32, u32), [u8; 4])> = None;
        for c in 0..cols {
            let x0 = (c as u64 * img.width as u64 / cols as u64) as u32;
            let x1 = (((c + 1) as u64 * img.width as u64 / cols as u64) as u32).max(x0 + 1).min(img.width);
            if let Some((range, tile)) = last
                && range == (x0, x1)
            {
                out.push(tile);
                continue;
            }
            let tile = average(img, (x0, x1), (y0, y1));
            out.push(tile);
            last = Some(((x0, x1), tile));
        }
        last_row = Some(((y0, y1), start));
    }
    out
}

/// The average colour of pixels `xs` × `ys` (half-open).
fn average(img: &Raster, (x0, x1): (u32, u32), (y0, y1): (u32, u32)) -> [u8; 4] {
    let (mut sr, mut sg, mut sb, mut sa, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for y in y0..y1.max(y0 + 1).min(img.height) {
        for x in x0..x1.max(x0 + 1).min(img.width) {
            let [pr, pg, pb, pa] = img.pixel(x, y);
            let a = pa as u64;
            sr += pr as u64 * a;
            sg += pg as u64 * a;
            sb += pb as u64 * a;
            sa += a;
            n += 1;
        }
    }
    match (sr.checked_div(sa), sg.checked_div(sa), sb.checked_div(sa)) {
        (Some(r), Some(g), Some(b)) => [r as u8, g as u8, b as u8, (sa / n.max(1)) as u8],
        _ => [0, 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_each_tile() {
        // Left half red, right half blue; a 2 × 1 mosaic gives one tile of each.
        let img = Raster::from_fn(10, 4, |x, _| if x < 5 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        assert_eq!(mosaic(&img, 2, 1), vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        // One tile: the average, purple.
        let m = mosaic(&img, 1, 1);
        assert_eq!(m[0][3], 255);
        assert!((m[0][0] as i32 - 127).abs() <= 1 && (m[0][2] as i32 - 127).abs() <= 1);
        // More tiles than pixels still works.
        assert_eq!(mosaic(&img, 20, 8).len(), 160);
        // Tiles over the same pixels average the same.
        let m = mosaic(&img, 20, 8);
        assert!(m.chunks(20).all(|row| row == &m[..20]));
        // Bytes that are not the raster's size: transparent.
        let short = Raster { width: 4, height: 4, rgba: vec![] };
        assert_eq!(mosaic(&short, 2, 2), vec![[0; 4]; 4]);
        // No pixels at all, however tall: transparent, with no rows walked.
        let empty = Raster { width: 0, height: u32::MAX, rgba: vec![] };
        assert_eq!(mosaic(&empty, 2, 2), vec![[0; 4]; 4]);
        // Transparency.
        let clear = Raster::from_fn(2, 2, |_, _| [9, 9, 9, 0]);
        assert_eq!(mosaic(&clear, 1, 1), vec![[0, 0, 0, 0]]);
    }
}

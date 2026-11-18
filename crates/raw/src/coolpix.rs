//! Two-field Nikon Coolpix raws: E5700 (CYGM four-colour filter), E5400, E8400 and E8800 (RGB Bayer).
//!
//! The layout was measured clean-room by black-box analysis of the CC0 raw.pixls.us files (E5700 2576 x 1924,
//! E8400 and E8800 3280 x 2454), not taken from any other raw converter:
//!
//! * one strip of `w * h * 1.5` bytes: 12-bit samples packed MSB-first, two per three bytes, no row padding (the
//!   ordinary uncompressed NEF reader unpacks it);
//! * the strip holds two interlaced fields of `h / 2` full-width rows, field A first and field B second. Interleaving
//!   them line by line gives the sensor image. The E5700 puts B on the even lines (`out[2k] = B[k]`, `out[2k+1] = A[k]`),
//!   the other bodies put A on the even lines. The order is a property of the model, not something the files state: the
//!   `0x828e` colour-filter tag is anchored at line 0 of the interleaved image, so the wrong parity swaps the two rows
//!   of every filter cell (a green cast on the CYGM file);
//! * black level 0, white level 4095, nothing masked;
//! * E5400, E8400 and E8800: `0x828e = [2, 1, 1, 0]`, an ordinary BGGR Bayer. E5700: `0x828e = [5, 3, 1, 4]`, i.e. Y C / G M.
//!
//! Field storage, like field order, is a property of the body: the Nikon reader rearranges rows only for the
//! models in [`field_order`], and only when the file has their measured legacy layout (binary maker-note version
//! 0.2.0.0, one tight 12-bit strip). Of 370 NEFs from 86 bodies checked, exactly these four bodies' files use that
//! maker-note version, and all four store fields. Other files, including any unmeasured body, keep their stored rows.
//!
//! CYGM to RGB is deliberately simple. Each of the four colour planes is bilinearly interpolated to full resolution,
//! scaled by fixed per-plane gains ([`CYGM_GAINS`]) and combined with the ideal complementary-colour relations
//! `Y = R + G, C = G + B, M = R + B`: `R = (Y + M - C) / 2`, `B = (C + M - Y) / 2` and `G` as the mean of the green
//! plane and `(Y + C - M) / 2`. A fitted 3 x 4 matrix was rejected: it was ill-conditioned on a nearly neutral scene,
//! and a free fit of the four gains against the camera JPEG turned the green gain negative.
//! Limits: no per-camera colour matrix (colours are approximate), the files' own white balance is not read (the gains
//! give a fixed daylight balance; the white-balance controls adjust from there), bilinear (soft, zippered at edges)
//! interpolation, negative results clamp to zero.

/// The measured two-field bodies and whether the first stored field holds the even lines of the sensor image.
const FIELD_BODIES: [(&str, bool); 4] = [("E5400", true), ("E5700", false), ("E8400", true), ("E8800", true)];

/// For a measured two-field body, whether the first field of its strip (rows `0..h/2`) holds the even lines;
/// `None` for every other model. (The `0x828e` colour-filter tag is anchored at line 0 of the interleaved image,
/// so the wrong parity swaps the two rows of every filter cell.)
pub(crate) fn field_order(model: &str) -> Option<bool> {
    let model = model.trim();
    FIELD_BODIES.iter().find(|(name, _)| *name == model).map(|&(_, first_even)| first_even)
}

/// Interleave the two fields of `samples` (`h / 2` rows each, field A first) into a `w x h` image.
pub(crate) fn deinterlace(samples: &[u16], w: usize, h: usize, first_field_even: bool) -> Option<Vec<u16>> {
    let n = w.checked_mul(h).filter(|n| *n <= crate::MAX_SAMPLES)?;
    if w == 0 || h < 2 || !h.is_multiple_of(2) || samples.len() != n {
        return None;
    }
    let (a, b) = samples.split_at_checked(n / 2)?;
    let (even, odd) = if first_field_even { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(n);
    for (even_row, odd_row) in even.chunks_exact(w).zip(odd.chunks_exact(w)) {
        out.extend_from_slice(even_row);
        out.extend_from_slice(odd_row);
    }
    (out.len() == n).then_some(out)
}

/// The colour-filter tag of the four-colour (CYGM) sensor: Y C / G M.
pub(crate) const CYGM_PATTERN: [u8; 4] = [5, 3, 1, 4];

const CODE_G: u8 = 1;
const CODE_C: u8 = 3;
const CODE_M: u8 = 4;
const CODE_Y: u8 = 5;

/// Fixed E5700 plane gains, in the order Y, C, G, M. Measured on the CC0 raw.pixls.us E5700 file (the only E5700
/// raw available) against its own camera JPEG (640 x 480, linearised): on the bins the camera rendered neutral
/// (channel spread under 8 %, mid-tone, unclipped; 215 of 19,200 at 160 x 120), the median per-bin gains that make
/// `Y = C = M = 2 G`, normalised to the weakest of Y, C, M. Over bin grids of 160 x 120 to 640 x 480 and neutrality
/// thresholds of 4 to 12 % the gains stay within 1.043-1.048, 0.613-0.616 and 1.265-1.271 (bootstrap 95 % intervals
/// within 0.006), and gains fitted on one half of the frame keep the other half's neutrals near neutral.
pub(crate) const CYGM_GAINS: [f32; 4] = [1.046, 1.0, 0.614, 1.268];

/// Output scale of the converted RGB.
const CYGM_SCALE: f32 = 4.0;

/// White level of the converted RGB: a clipped neutral patch reaches 4095 in the weakest-gained of Y, C, M
/// (gain 1.0, C) and gives half of it in R, G, B. Fixed, so the header probe and the full decode report the same value.
pub(crate) const CYGM_WHITE: f32 = 4095.0 * 0.5 * CYGM_SCALE;

/// Bilinear tap along one axis of a sub-sampled plane: (low index, high index, weight of the high one) for output
/// coordinate `c` of a plane of `n` samples starting at `phase`.
fn tap(n: usize, phase: usize, c: usize) -> (usize, usize, f32) {
    let last = n.saturating_sub(1);
    match c.checked_sub(phase) {
        None => (0, 0, 0.0),
        Some(d) => {
            let i = d / 2;
            (i.min(last), (i + 1).min(last), if d % 2 == 1 { 0.5 } else { 0.0 })
        }
    }
}

/// Linear-light RGB (interleaved, `u16`, white level [`CYGM_WHITE`]) from a CYGM mosaic.
///
/// `pattern` is the `0x828e` tag, row-major over the 2 x 2 cell at (0, 0); it must hold each of Y, C, G, M once.
pub(crate) fn cygm_to_rgb(mosaic: &[u16], w: usize, h: usize, pattern: [u8; 4]) -> Option<Vec<u16>> {
    use rayon::prelude::*;
    let pixels = w.checked_mul(h)?;
    let output_samples = pixels.checked_mul(3).filter(|n| *n <= crate::MAX_SAMPLES)?;
    if w < 4 || h < 4 || !w.is_multiple_of(2) || !h.is_multiple_of(2) || mosaic.len() != pixels {
        return None;
    }
    // phase (py * 2 + px) of each colour: [Y, C, G, M]. Four different codes found means four different phases.
    let phase_of = |code: u8| pattern.iter().position(|&c| c == code);
    let phases = [phase_of(CODE_Y)?, phase_of(CODE_C)?, phase_of(CODE_G)?, phase_of(CODE_M)?];
    let (pw, prows) = (w / 2, h / 2);
    // the four sub-sampled planes, in colour order Y, C, G, M
    let planes = phases.map(|p| {
        let (py, px) = (p / 2, p % 2);
        mosaic.chunks_exact(w).skip(py).step_by(2).flat_map(|row| row.iter().skip(px).step_by(2).map(|&v| f32::from(v))).collect::<Vec<f32>>()
    });
    if planes.iter().any(|p| p.len() != pw * prows) {
        return None;
    }

    let mut out = vec![0u16; output_samples];
    out.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        for (x, px) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let mut v = [0f32; 4];
            for (((slot, plane), &p), &gain) in v.iter_mut().zip(&planes).zip(&phases).zip(&CYGM_GAINS) {
                let (y0, y1, fy) = tap(prows, p / 2, y);
                let (x0, x1, fx) = tap(pw, p % 2, x);
                let at = |i: usize, j: usize| plane.get(i * pw + j).copied().unwrap_or(0.0);
                let top = at(y0, x0) * (1.0 - fx) + at(y0, x1) * fx;
                let bot = at(y1, x0) * (1.0 - fx) + at(y1, x1) * fx;
                *slot = (top * (1.0 - fy) + bot * fy) * gain;
            }
            let [yy, cc, gg, mm] = v;
            let r = (yy + mm - cc) * 0.5;
            let g = (gg + (yy + cc - mm) * 0.5) * 0.5;
            let b = (cc + mm - yy) * 0.5;
            for (o, c) in px.iter_mut().zip([r, g, b]) {
                *o = (c * CYGM_SCALE).round().clamp(0.0, 65535.0) as u16;
            }
        }
    });
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_dimensions_are_refused_before_work_or_allocation() {
        for (w, h) in [(usize::MAX, 2), (usize::MAX - 1, 64), (crate::MAX_SAMPLES, 4)] {
            assert!(deinterlace(&[], w, h, true).is_none());
            assert!(cygm_to_rgb(&[], w, h, CYGM_PATTERN).is_none());
        }
    }

    #[test]
    fn two_field_round_trip_for_both_parities() {
        let (w, h) = (6usize, 8usize);
        // sensor image whose every line is distinct
        let img: Vec<u16> = (0..w * h).map(|i| (i * 7 % 4096) as u16).collect();
        for first_even in [false, true] {
            let lines: Vec<&[u16]> = img.chunks_exact(w).collect();
            // fields as stored: A = first h/2 rows, B = the rest
            let evens = lines.iter().step_by(2);
            let odds = lines.iter().skip(1).step_by(2);
            let stored: Vec<u16> = if first_even {
                evens.chain(odds).flat_map(|l| l.iter().copied()).collect()
            } else {
                odds.chain(evens).flat_map(|l| l.iter().copied()).collect()
            };
            assert_eq!(deinterlace(&stored, w, h, first_even).unwrap(), img, "first_even = {first_even}");
        }
        // the two parities differ
        let s: Vec<u16> = (0..w * h).map(|i| i as u16).collect();
        assert_ne!(deinterlace(&s, w, h, true), deinterlace(&s, w, h, false));
    }

    #[test]
    fn deinterlace_refuses_bad_shapes() {
        assert!(deinterlace(&[0; 12], 3, 5, true).is_none());
        assert!(deinterlace(&[0; 11], 3, 4, true).is_none());
        assert!(deinterlace(&[], 0, 0, true).is_none());
    }

    #[test]
    fn field_order_is_per_model_and_only_for_measured_bodies() {
        assert_eq!(field_order("E5700"), Some(false));
        assert_eq!(field_order("E5700 "), Some(false));
        for model in ["E5400", "E8400", "E8800"] {
            assert_eq!(field_order(model), Some(true), "{model}");
        }
        for model in ["E5000", "E8700", "D1X", "", "E57000"] {
            assert_eq!(field_order(model), None, "{model}");
        }
    }

    #[test]
    fn white_level_follows_the_weakest_gain() {
        let weakest = CYGM_GAINS[0].min(CYGM_GAINS[1]).min(CYGM_GAINS[3]);
        assert!((CYGM_WHITE - 4095.0 * weakest * 0.5 * CYGM_SCALE).abs() < 1e-3);
    }

    #[test]
    fn cygm_neutral_surface_stays_neutral() {
        let (w, h) = (16usize, 12usize);
        // a neutral surface of level v on this sensor: each plane reads v / gain (G at half level)
        let [gy, gc, gg, gm] = CYGM_GAINS;
        for v in [400.0f32, 2000.0, 3500.0] {
            let at = |i: usize| match ((i / w) % 2, i % 2) {
                (0, 0) => v / gy,
                (0, _) => v / gc,
                (_, 0) => v / 2.0 / gg,
                _ => v / gm,
            };
            let m: Vec<u16> = (0..w * h).map(|i| at(i).round() as u16).collect();
            let rgb = cygm_to_rgb(&m, w, h, CYGM_PATTERN).unwrap();
            for px in rgb.as_chunks::<3>().0 {
                let (lo, hi) = (px.iter().min().copied().unwrap_or(0), px.iter().max().copied().unwrap_or(0));
                assert!(lo > 0 && f32::from(hi - lo) <= f32::from(hi) * 0.01, "{px:?} at {v}");
            }
        }
    }

    #[test]
    fn cygm_refuses_bad_input_without_panicking() {
        assert!(cygm_to_rgb(&[0; 10], 4, 4, [5, 3, 1, 4]).is_none());
        assert!(cygm_to_rgb(&[], 0, 0, [5, 3, 1, 4]).is_none());
        assert!(cygm_to_rgb(&[0; 16], 4, 4, [5, 3, 1, 1]).is_none());
        assert!(cygm_to_rgb(&[0; 16], 4, 4, [2, 1, 1, 0]).is_none());
        assert!(cygm_to_rgb(&[0; 15], 5, 3, [5, 3, 1, 4]).is_none());
        // all-black input is fine
        assert!(cygm_to_rgb(&[0; 16], 4, 4, [5, 3, 1, 4]).is_some());
    }
}

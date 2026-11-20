//! HP-GL/2 export helpers: region clipping ([`clip`]), inside-first nesting ([`nest`]), and
//! base-64 `PE` polyline encoding.

pub mod clip;
pub mod nest;

use std::io::Write as _;

/// The largest coordinate written, plotter units (2²³ − 1).
pub const MAX_COORD: i64 = (1 << 23) - 1;

/// The most pens a palette holds.
pub const MAX_PENS: usize = 255;

/// A pen in an HP-GL/2 palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pen {
    pub number: usize,
    pub rgb: [u8; 3],
}

/// Encodes a standalone HP-GL/2 `PE` command as bytes, not UTF-8 text.
/// The first point is absolute; each path starts with a pen-up move.
pub fn encode_pe(paths: &[&[(i64, i64)]]) -> Vec<u8> {
    let mut out = b"PE".to_vec();
    append_paths(&mut out, paths.iter().copied(), &mut None);
    out.push(b';');
    out
}

fn append_paths<'a>(out: &mut Vec<u8>, paths: impl Iterator<Item = &'a [(i64, i64)]>, at: &mut Option<(i64, i64)>) {
    for path in paths {
        for (index, &(x, y)) in path.iter().enumerate() {
            if index == 0 {
                // '<' lifts the pen for this move; '=' alone only selects absolute coordinates.
                out.push(b'<');
            }
            let relative = at.and_then(|(px, py)| {
                let dx = x.checked_sub(px)?;
                let dy = y.checked_sub(py)?;
                (dx.unsigned_abs() <= MAX_COORD as u64 && dy.unsigned_abs() <= MAX_COORD as u64).then_some((dx, dy))
            });
            let (dx, dy) = match relative {
                Some(delta) => delta,
                None => {
                    out.push(b'=');
                    (x, y)
                }
            };
            push_pe_num(out, dx);
            push_pe_num(out, dy);
            *at = Some((x, y));
        }
    }
}

fn push_pe_num(out: &mut Vec<u8>, value: i64) {
    // Widen before shifting so even i64::MIN is encoded without overflow.
    let mut number = (u128::from(value.unsigned_abs()) << 1) | u128::from(value < 0);
    while number >= 64 {
        out.push(63 + (number & 63) as u8);
        number >>= 6;
    }
    out.push(191 + number as u8);
}

/// Writes an HP-GL/2 byte stream for up to 255 pens and corresponding paths.
/// Pen numbers outside 1..=255 are ignored. Extra palettes/paths are ignored together.
/// The caller owns coordinate admission and output-size budgets.
pub fn write_hpgl2(pens: &[Pen], paths_per_pen: &[Vec<Vec<(i64, i64)>>]) -> Vec<u8> {
    let pens: Vec<_> = pens.iter().take(MAX_PENS).enumerate().filter(|(_, pen)| (1..=MAX_PENS).contains(&pen.number)).collect();
    let mut out = b"IN;".to_vec();
    if let Some(maximum) = pens.iter().map(|(_, pen)| pen.number).max() {
        // Raster devices number pens from zero, so reserve the selected pen's slot too.
        let count = (maximum + 1).next_power_of_two().max(2);
        let _ = write!(out, "NP{count};");
        for (_, pen) in &pens {
            let [r, g, b] = pen.rgb;
            let _ = write!(out, "PC{},{r},{g},{b};", pen.number);
        }
    }
    let mut at = None;
    for (index, pen) in pens {
        let Some(paths) = paths_per_pen.get(index) else { continue };
        if paths.is_empty() {
            continue;
        }
        let _ = write!(out, "PU;SP{};PE", pen.number);
        // Position is shared across pen changes, not reset to the origin for each PE command.
        append_paths(&mut out, paths.iter().map(Vec::as_slice), &mut at);
        out.push(b';');
    }
    out.extend_from_slice(b"PU;SP0;");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pe_numbers_are_raw_bytes_and_paths_lift_the_pen() {
        let first: &[(i64, i64)] = &[(0, 0), (10, -10)];
        let second: &[(i64, i64)] = &[(20, 0), (20, 10)];
        assert_eq!(encode_pe(&[first, second]), b"PE<=\xbf\xbf\xd3\xd4<\xd3\xd3\xbf\xd3;");
    }

    #[test]
    fn stream_preserves_position_across_pen_changes() {
        let pens = [Pen { number: 1, rgb: [1, 2, 3] }, Pen { number: 2, rgb: [4, 5, 6] }];
        let paths = vec![vec![vec![(0, 0), (10, 0)]], vec![vec![(20, 0), (20, 10)]]];
        assert_eq!(write_hpgl2(&pens, &paths), b"IN;NP4;PC1,1,2,3;PC2,4,5,6;PU;SP1;PE<=\xbf\xbf\xd3\xbf;PU;SP2;PE<\xd3\xbf\xbf\xd3;PU;SP0;");
    }

    #[test]
    fn large_relative_move_uses_absolute_coordinates() {
        let path = [(-MAX_COORD, 0), (MAX_COORD, 0)];
        let result = encode_pe(&[&path]);
        assert_eq!(result.iter().filter(|&&b| b == b'=').count(), 2);
    }

    #[test]
    fn empty_plot_and_sparse_pen_number_are_preserved() {
        assert_eq!(write_hpgl2(&[], &[]), b"IN;PU;SP0;");
        assert_eq!(write_hpgl2(&[Pen { number: 255, rgb: [0, 0, 0] }], &[]), b"IN;NP256;PC255,0,0,0;PU;SP0;");
    }

    #[test]
    fn multibyte_pe_numbers_are_not_utf8() {
        assert_eq!(encode_pe(&[&[(32, -32)]]), b"PE<=?\xc0@\xc0;");
    }

    #[test]
    fn invalid_pens_keep_the_original_path_indices() {
        let pens = [Pen { number: 0, rgb: [1, 2, 3] }, Pen { number: 2, rgb: [4, 5, 6] }];
        let paths = vec![vec![vec![(100, 100)]], vec![vec![(0, 0), (10, 0)]]];
        assert_eq!(write_hpgl2(&pens, &paths), b"IN;NP4;PC2,4,5,6;PU;SP2;PE<=\xbf\xbf\xd3\xbf;PU;SP0;");
        assert_eq!(write_hpgl2(&[Pen { number: 256, rgb: [0, 0, 0] }], &paths), b"IN;PU;SP0;");
    }

    #[test]
    fn palette_cap_also_caps_drawn_paths() {
        let mut pens = vec![Pen { number: 0, rgb: [0, 0, 0] }; MAX_PENS + 1];
        pens[MAX_PENS - 1] = Pen { number: 255, rgb: [1, 2, 3] };
        pens[MAX_PENS] = Pen { number: 1, rgb: [4, 5, 6] };
        let paths = vec![vec![vec![(0, 0), (10, 0)]]; MAX_PENS + 1];
        assert_eq!(write_hpgl2(&pens, &paths), b"IN;NP256;PC255,1,2,3;PU;SP255;PE<=\xbf\xbf\xd3\xbf;PU;SP0;");
    }

    #[test]
    fn extreme_coordinates_do_not_overflow() {
        let path = [(i64::MIN, i64::MAX), (i64::MAX, i64::MIN)];
        let output = encode_pe(&[&path]);
        assert!(output.starts_with(b"PE<="));
        assert!(output.ends_with(b";"));
    }
}

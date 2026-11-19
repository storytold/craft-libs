//! HP-GL/2 export helpers: region clipping ([`clip`]), inside-first nesting ([`nest`]), and
//! base-64 `PE` polyline encoding.

pub mod clip;
pub mod nest;

use std::fmt::Write as _;

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

/// Encodes `paths` (each a slice of `(x, y)` in plotter units) as an HP-GL/2 `PE` command.
///
/// Uses relative coordinates in base-64 (6 bits per byte, offset by 63, high bit set on the
/// terminating byte of each number, `=` before pen-up moves).
pub fn encode_pe(paths: &[&[(i64, i64)]]) -> String {
    let mut out = String::from("PE");
    let mut cx = 0_i64;
    let mut cy = 0_i64;
    for path in paths {
        let Some((&(x0, y0), rest)) = path.split_first() else {
            continue;
        };
        out.push('=');
        push_pe_num(&mut out, x0 - cx);
        push_pe_num(&mut out, y0 - cy);
        cx = x0;
        cy = y0;
        for &(x, y) in rest {
            push_pe_num(&mut out, x - cx);
            push_pe_num(&mut out, y - cy);
            cx = x;
            cy = y;
        }
    }
    out.push(';');
    out
}

fn push_pe_num(out: &mut String, v: i64) {
    let mut n = if v < 0 { ((-v) << 1) | 1 } else { v << 1 } as u64;
    loop {
        let byte = (n & 0x3f) as u8;
        n >>= 6;
        if n == 0 {
            let _ = out.write_char((byte + 191) as char);
            break;
        }
        let _ = out.write_char((byte + 63) as char);
    }
}

/// Writes an HP-GL/2 stream for `pens` and their corresponding `paths_per_pen` (in plotter units).
pub fn write_hpgl2(pens: &[Pen], paths_per_pen: &[Vec<Vec<(i64, i64)>>]) -> String {
    let mut out = String::from("IN;\n");
    if !pens.is_empty() {
        let _ = writeln!(out, "NP{};", pens.len().min(MAX_PENS));
        for pen in pens.iter().take(MAX_PENS) {
            let [r, g, b] = pen.rgb;
            let _ = writeln!(out, "PC{},{r},{g},{b};", pen.number);
        }
    }
    for (pen, paths) in pens.iter().zip(paths_per_pen) {
        if paths.is_empty() {
            continue;
        }
        let _ = writeln!(out, "PU;SP{};", pen.number);
        let slices: Vec<&[(i64, i64)]> = paths.iter().map(Vec::as_slice).collect();
        out.push_str(&encode_pe(&slices));
        out.push('\n');
    }
    out.push_str("PU;SP0;\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_pe_basic() {
        let path: &[(i64, i64)] = &[(0, 0), (10, 0), (10, 10)];
        let pe = encode_pe(&[path]);
        assert!(pe.starts_with("PE="));
        assert!(pe.ends_with(';'));
    }

    #[test]
    fn write_hpgl2_basic() {
        let pens = [Pen { number: 1, rgb: [0, 0, 0] }];
        let paths = vec![vec![vec![(0, 0), (40, 0), (40, 40)]]];
        let s = write_hpgl2(&pens, &paths);
        assert!(s.starts_with("IN;\nNP1;\nPC1,0,0,0;\nPU;SP1;\nPE="));
        assert!(s.ends_with("PU;SP0;\n"));
    }
}

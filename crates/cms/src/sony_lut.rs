//! Sony Imageworks SPI3D and SPIMTX text readers, extracted from EffectCraft.
//!
//! These preserve the source reader's format tolerance. SPI3D sample values are not
//! clamped or sanitized; rendering policy belongs to the caller. Duplicate coordinates
//! retain their last value, and the total row count must match the lattice volume.

/// Maximum lattice side accepted by the source reader.
pub const MAX_SIDE: usize = 129;

/// Row-major homogeneous transform with a separate normalized code-value offset.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    pub matrix: [f64; 16],
    pub offset: [f64; 4],
}

/// A cubic RGB table, red coordinate varying fastest.
#[derive(Clone, Debug, PartialEq)]
pub struct Lattice {
    pub size: usize,
    pub values: Vec<[f32; 3]>,
}

/// Read exactly twelve finite SPIMTX numbers, dividing the offset column by 65535.
/// Parsing uses a fixed array, not storage proportional to an oversized input.
pub fn parse_spimtx(text: &str) -> Option<Matrix> {
    let mut words = text.split_whitespace();
    let mut v = [0.0f64; 12];
    for value in &mut v {
        *value = words.next()?.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
    }
    if words.next().is_some() {
        return None;
    }
    let mut matrix = [0.0; 16];
    let mut offset = [0.0; 4];
    for (r, row) in v.as_chunks::<4>().0.iter().enumerate() {
        matrix[r * 4..r * 4 + 3].copy_from_slice(&row[..3]);
        offset[r] = row[3] / 65535.0;
    }
    matrix[15] = 1.0;
    Some(Matrix { matrix, offset })
}

/// Read a SPI3D table with a bounded 2..=129 cubic lattice and exactly six fields per row.
/// A `SPILUT` prefix is accepted and the second header line is ignored, as in EffectCraft.
pub fn parse_spi3d(text: &str) -> Option<Lattice> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if !lines.next()?.starts_with("SPILUT") {
        return None;
    }
    lines.next()?;
    let mut sizes = lines.next()?.split_whitespace();
    let n: usize = sizes.next()?.parse().ok()?;
    let g: usize = sizes.next()?.parse().ok()?;
    let b: usize = sizes.next()?.parse().ok()?;
    if sizes.next().is_some() || g != n || b != n || !(2..=MAX_SIDE).contains(&n) {
        return None;
    }
    let count = n * n * n;
    // Validate every row and the row count before reserving the lattice, so a short or
    // malformed file never allocates the table its header claims. The rows are then read
    // again in file order, which keeps the last value of a duplicate coordinate.
    let mut seen = 0usize;
    for line in lines.clone() {
        if seen == count {
            return None;
        }
        spi3d_row(line, n)?;
        seen += 1;
    }
    if seen != count {
        return None;
    }
    #[cfg(test)]
    tests::LATTICES_RESERVED.with(|r| r.set(r.get() + 1));
    let mut values = vec![[0.0f32; 3]; count];
    for line in lines {
        let (index, color) = spi3d_row(line, n)?;
        *values.get_mut(index)? = color;
    }
    Some(Lattice { size: n, values })
}

/// One `r g b  R G B` row of a side-`n` lattice: its red-fastest index and its colour.
fn spi3d_row(line: &str, n: usize) -> Option<(usize, [f32; 3])> {
    let mut fields = line.split_whitespace();
    let r: usize = fields.next()?.parse().ok()?;
    let g: usize = fields.next()?.parse().ok()?;
    let b: usize = fields.next()?.parse().ok()?;
    let color = [fields.next()?.parse().ok()?, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?];
    if fields.next().is_some() || r >= n || g >= n || b >= n {
        return None;
    }
    Some((r + g * n + b * n * n, color))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    thread_local! {
        /// How many lattices [`parse_spi3d`] has reserved on this thread.
        pub(super) static LATTICES_RESERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn reserved() -> usize {
        LATTICES_RESERVED.with(std::cell::Cell::get)
    }

    #[test]
    fn spi3d_reserves_the_lattice_only_once_its_rows_are_valid() {
        let mut ok = "SPILUT 1.0\n3 3\n2 2 2\n".to_owned();
        for i in 0..8 {
            ok += &format!("{} {} {} 0 0 0\n", i & 1, (i >> 1) & 1, i >> 2);
        }
        // A header-only maximal claim, a short table, a full count of malformed rows, a table
        // whose last row is bad and one with a valid row too many.
        let max = format!("SPILUT 1.0\n3 3\n{MAX_SIDE} {MAX_SIDE} {MAX_SIDE}\n");
        let short = "SPILUT 1.0\n3 3\n2 2 2\n0 0 0 1 1 1\n".to_owned();
        let junk = "SPILUT 1.0\n3 3\n2 2 2\n".to_owned() + &"x\n".repeat(8);
        let last_bad = ok.replace("1 1 1 0 0 0", "1 1 2 0 0 0");
        let extra = ok.clone() + "0 0 0 0 0 0\n";
        for bad in [max, short, junk, last_bad, extra] {
            let before = reserved();
            assert!(parse_spi3d(&bad).is_none());
            assert_eq!(reserved(), before, "{bad}");
        }
        let before = reserved();
        assert!(parse_spi3d(&ok).is_some());
        assert_eq!(reserved(), before + 1);
    }
}

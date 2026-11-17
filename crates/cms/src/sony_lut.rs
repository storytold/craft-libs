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
    let mut values = vec![[0.0f32; 3]; count];
    let mut seen = 0usize;
    for line in lines {
        if seen == count {
            return None;
        }
        let mut fields = line.split_whitespace();
        let r: usize = fields.next()?.parse().ok()?;
        let g: usize = fields.next()?.parse().ok()?;
        let b: usize = fields.next()?.parse().ok()?;
        let color = [fields.next()?.parse().ok()?, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?];
        if fields.next().is_some() || r >= n || g >= n || b >= n {
            return None;
        }
        *values.get_mut(r + g * n + b * n * n)? = color;
        seen += 1;
    }
    (seen == count).then_some(Lattice { size: n, values })
}

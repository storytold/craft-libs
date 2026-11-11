//! Separable RGB display transforms without a coarse 3D interpolation near black.

/// RGB input curves, a 3×3 matrix with offsets, and RGB output curves for a GPU display
/// transform. Values are floats independent of the document's pixel depth.
///
/// The input table is sampled linearly at `clamp(rgb, 0, 1) * INPUT_INTERVALS`. Apply
/// [`Self::matrix_rows`] to that result, then sample the output table at
/// `clamp(linear_rgb, 0, 1)^(1/4) * (OUTPUT_ENTRIES - 1)`. Interpolate each channel
/// separately between its two adjacent table entries. These nonuniform output samples
/// retain shadow detail where a pure gamma curve's inverse has an infinite slope.
///
/// Tables contain RGB values and a padding alpha of 1, suitable for RGBA32Float texels.
/// Use unfiltered texture loads and manual interpolation if float filtering is unavailable.
/// This representation covers SDR display output; input and output are clamped to `[0, 1]`.
/// Construction validates all sizes and values; arbitrary CLUT/proofing chains retain the
/// [`crate::Lut3d`] fallback.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuMatrixShaper {
    matrix: [[f32; 4]; 3],
    input: Vec<[f32; 4]>,
    output: Vec<[f32; 4]>,
}

impl GpuMatrixShaper {
    /// The input table has one more entry than this number of equal intervals.
    pub const INPUT_INTERVALS: usize = 4096;
    /// Output entries are spaced uniformly in the fourth root of the linear value.
    pub const OUTPUT_ENTRIES: usize = 4096;

    /// Row-major matrix: each row contains three RGB coefficients followed by its offset.
    pub fn matrix_rows(&self) -> [[f32; 4]; 3] {
        self.matrix
    }

    /// RGB input curves at `i / INPUT_INTERVALS`, with padding alpha 1.
    pub fn input_table(&self) -> &[[f32; 4]] {
        &self.input
    }

    /// RGB output curves at `(i / (OUTPUT_ENTRIES - 1))^4`, with padding alpha 1.
    pub fn output_table(&self) -> &[[f32; 4]] {
        &self.output
    }

    pub(crate) fn from_parts(matrix: [[f32; 4]; 3], input: &[Vec<f32>], output: Option<&[Vec<f32>]>) -> Option<Self> {
        if !matrix.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        let input = pack_table(if input.is_empty() { None } else { Some(input) }, Self::INPUT_INTERVALS + 1, |u| u)?;
        let output = pack_table(output, Self::OUTPUT_ENTRIES, |u| u * u * u * u)?;
        let maxima: [f64; 3] = std::array::from_fn(|channel| input.iter().map(|entry| f64::from(entry[channel]).abs()).fold(0.0, f64::max));
        for row in matrix {
            let magnitude = row[..3].iter().zip(maxima).map(|(coefficient, maximum)| f64::from(*coefficient).abs() * maximum).sum::<f64>()
                + f64::from(row[3]).abs();
            if magnitude > f64::from(f32::MAX) {
                return None;
            }
        }
        Some(Self { matrix, input, output })
    }
}

fn pack_table(tables: Option<&[Vec<f32>]>, entries: usize, identity: impl Fn(f32) -> f32) -> Option<Vec<[f32; 4]>> {
    if let Some(tables) = tables
        && (tables.len() != 3 || tables.iter().any(|t| t.len() != entries))
    {
        return None;
    }
    let mut packed: Vec<[f32; 4]> = Vec::new();
    // Both lengths are fixed constants, but a failed allocation still means the caller
    // should retain its existing fallback rather than abort while opening a document.
    packed.try_reserve_exact(entries).ok()?;
    for i in 0..entries {
        let rgb = match tables {
            Some(tables) => [*tables.first()?.get(i)?, *tables.get(1)?.get(i)?, *tables.get(2)?.get(i)?],
            None => [identity(i as f32 / (entries - 1) as f32); 3],
        };
        if !rgb.iter().all(|v| v.is_finite()) {
            return None;
        }
        if let Some(previous) = packed.last()
            && rgb.iter().zip(previous).any(|(next, before)| !(*next - *before).is_finite())
        {
            return None;
        }
        packed.push([rgb[0], rgb[1], rgb[2], 1.0]);
    }
    Some(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

    #[test]
    fn rejects_invalid_tables_and_nonfinite_values() {
        let inputs = vec![vec![0.0; GpuMatrixShaper::INPUT_INTERVALS + 1]; 3];
        let outputs = vec![vec![0.0; GpuMatrixShaper::OUTPUT_ENTRIES]; 3];
        assert!(GpuMatrixShaper::from_parts(IDENTITY, &inputs, Some(&outputs)).is_some());
        assert!(GpuMatrixShaper::from_parts(IDENTITY, &inputs[..2], Some(&outputs)).is_none());
        assert!(GpuMatrixShaper::from_parts(IDENTITY, &inputs, Some(&outputs[..2])).is_none());
        assert!(GpuMatrixShaper::from_parts(IDENTITY, &inputs, Some(&[])).is_none());
        let mut invalid = inputs.clone();
        invalid[0].pop();
        assert!(GpuMatrixShaper::from_parts(IDENTITY, &invalid, Some(&outputs)).is_none());
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = inputs.clone();
            invalid[1][123] = v;
            assert!(GpuMatrixShaper::from_parts(IDENTITY, &invalid, Some(&outputs)).is_none());
            let mut invalid = outputs.clone();
            invalid[2][321] = v;
            assert!(GpuMatrixShaper::from_parts(IDENTITY, &inputs, Some(&invalid)).is_none());
            let mut invalid = IDENTITY;
            invalid[1][3] = v;
            assert!(GpuMatrixShaper::from_parts(invalid, &inputs, Some(&outputs)).is_none());
        }
    }

    #[test]
    fn rejects_finite_data_that_overflows_shader_arithmetic() {
        let mut matrix = IDENTITY;
        matrix[0] = [f32::MAX; 4];
        assert!(GpuMatrixShaper::from_parts(matrix, &[], None).is_none());
        let mut inputs = vec![vec![0.0; GpuMatrixShaper::INPUT_INTERVALS + 1]; 3];
        inputs[0][100] = -f32::MAX;
        inputs[0][101] = f32::MAX;
        assert!(GpuMatrixShaper::from_parts(IDENTITY, &inputs, None).is_none());
    }
}

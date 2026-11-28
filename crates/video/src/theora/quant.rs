//! Quantization parameters and dequantization matrix generation (Section 6.4.3).

use super::Error;

/// Storage for decoded quantization parameters from the setup header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantParams {
    /// 64-element AC scale values.
    pub ac_scale: [u16; 64],
    /// 64-element DC scale values.
    pub dc_scale: [u16; 64],
    /// Base matrices (up to 384, each 64 bytes).
    pub base_matrices: Vec<[u8; 64]>,
    /// Number of quant ranges for `[qti (0..2)][pli (0..3)]`.
    pub nqrs: [[usize; 3]; 2],
    /// Sizes of quant ranges `[qti][pli][qri]`.
    pub qr_sizes: [[[usize; 63]; 3]; 2],
    /// Base matrix indices `[qti][pli][qri]`.
    pub qr_bmis: [[[usize; 64]; 3]; 2],
}

impl QuantParams {
    /// Compute a single 64-element quantization matrix for `(qti, pli, qi)` (Section 6.4.3).
    pub fn compute_qmat(&self, qti: usize, pli: usize, qi: usize) -> Result<[u16; 64], Error> {
        let nqr = self.nqrs[qti][pli];
        if nqr == 0 || nqr > 63 {
            return Err(Error::Corrupt("invalid quant range count"));
        }

        // 1. Locate the quant range qri containing qi
        let mut qi_start = 0;
        let mut selected_qri = None;

        for qri in 0..nqr {
            let size = self.qr_sizes[qti][pli][qri];
            let qi_end = qi_start + size;
            if qi >= qi_start && qi <= qi_end {
                selected_qri = Some((qri, qi_start, qi_end, size));
                break;
            }
            qi_start = qi_end;
        }

        let (qri, qi_start, qi_end, range_size) = selected_qri.ok_or(Error::Corrupt("qi not found in any quant range"))?;

        if range_size == 0 {
            return Err(Error::Corrupt("zero-sized quant range"));
        }

        let bmi = self.qr_bmis[qti][pli][qri];
        let bmj = self.qr_bmis[qti][pli][qri + 1];

        if bmi >= self.base_matrices.len() || bmj >= self.base_matrices.len() {
            return Err(Error::Corrupt("base matrix index out of bounds"));
        }

        let bm_a = &self.base_matrices[bmi];
        let bm_b = &self.base_matrices[bmj];

        let mut qmat = [0u16; 64];

        for ci in 0..64 {
            let num = 2 * (qi_end as i32 - qi as i32) * bm_a[ci] as i32 + 2 * (qi as i32 - qi_start as i32) * bm_b[ci] as i32 + range_size as i32;
            let den = 2 * range_size as i32;
            let bm_ci = num / den;

            let qscale = if ci == 0 { self.dc_scale[qi] as i32 } else { self.ac_scale[qi] as i32 };

            let val = (qscale * bm_ci / 100) * 4;
            let qmin = match (qti, ci == 0) {
                (0, true) => 16,
                (0, false) => 8,
                (1, true) => 32,
                (1, false) => 16,
                _ => 16,
            };

            qmat[ci] = val.clamp(qmin, 4096) as u16;
        }

        Ok(qmat)
    }

    /// Precompute all quantization matrices `[qti][pli][qi]` for fast decoding.
    pub fn precompute_all_qmat(&self) -> Result<[[[[u16; 64]; 64]; 3]; 2], Error> {
        let mut tables = [[[[0u16; 64]; 64]; 3]; 2];
        for qti in 0..2 {
            for pli in 0..3 {
                for qi in 0..64 {
                    tables[qti][pli][qi] = self.compute_qmat(qti, pli, qi)?;
                }
            }
        }
        Ok(tables)
    }
}

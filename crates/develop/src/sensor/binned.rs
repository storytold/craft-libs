//! Reduced-size development straight from the mosaic ("superpixel" binning).
//!
//! When the caller needs an image at most half (or a third, a quarter…) of the sensor size — a
//! 2560 px loupe preview or a grid thumbnail of a 24 MP raw — demosaicing the full mosaic and then
//! downscaling throws most of the work away. Instead every `k × k` block of CFA samples becomes one
//! output pixel whose channels are the means of the block's samples of that colour. Every block must
//! contain all three colours (Bayer: `k` even; X-Trans: `k` a multiple of 3). There is no
//! interpolation, so there are no demosaicing artefacts; the colour planes are offset by less than
//! one output pixel, which is invisible at these scales.
//!
//! Clipping is decided per sample, before averaging. [`RawImage::develop_binned_masked`] keeps every
//! channel the mean of its samples and says, per output pixel, which colours had a sample at or above
//! `clip` (for `crate::highlight::reconstruct_masked`, which rebuilds them with the mean as a lower
//! bound). A specular point that clips one green sample of the 32 in an 8 × 8 block thus counts as
//! partly clipped without the block's green jumping to the clip level, which would read as a green
//! speckle once reconstructed. [`RawImage::develop_binned`] instead gives such a channel the block's
//! maximum sample of the colour, so that plain value thresholds see it as clipped.

use super::{Normalized, RawData, RawDevelopExt, RawImage, Rect, Result, Rgb32f, cfa_valid, deferred_linearization_table};

// Cap the precomputed colour bytes, not image size or the supported block size.
const PHASE_COLOUR_BYTES: usize = 4 * 1024 * 1024;
type PhaseLayout = (Vec<u8>, [f32; 3]);

/// Does at least one `k × k` block fit the active area, with single-plane CFA data and R, G
/// and B present in every block phase? Bayer also requires an even `k`.
pub fn can_bin(raw: &RawImage, k: usize) -> bool {
    if k < 2 || k > raw.active_area.width || k > raw.active_area.height {
        return false;
    }
    let Some(cfa) = &raw.cfa else { return false };
    if raw.cpp != 1 || (raw.pixel_aspect - 1.0).abs() >= 0.01 || !cfa_valid(cfa) || (cfa.is_bayer() && !k.is_multiple_of(2)) {
        return false;
    }
    // Repeating a CFA period cannot add a colour to the phase's coverage.
    (0..cfa.height).all(|j| {
        (0..cfa.width).all(|i| {
            let mut colours = 0u8;
            for dy in 0..k.min(cfa.height) {
                for dx in 0..k.min(cfa.width) {
                    colours |= 1 << cfa.color_at(i + dx, j + dy).min(2);
                }
            }
            colours == 0b111
        })
    })
}

/// Like [`RawDevelopExt::develop`] at `1/k` of the size (camera RGB, white = 1.0, default crop applied,
/// not oriented).
pub fn develop_binned(raw: &RawImage, k: usize, clip: f32) -> Result<Option<Rgb32f>> {
    Ok(bin(raw, k, clip, true)?.map(|(img, _)| img))
}

/// Like [`develop_binned`], with every channel the mean of its block's samples, and the clip mask.
pub fn develop_binned_masked(raw: &RawImage, k: usize, clip: f32) -> Result<Option<(Rgb32f, Vec<u8>)>> {
    bin(raw, k, clip, false)
}

/// The binned image and its clip mask; `keep_max`: a channel with a clipped sample takes the
/// block's maximum sample of the colour instead of the mean.
fn bin(raw: &RawImage, k: usize, clip: f32, keep_max: bool) -> Result<Option<(Rgb32f, Vec<u8>)>> {
    bin_with_phase_budget(raw, k, clip, keep_max, PHASE_COLOUR_BYTES)
}

fn bin_with_phase_budget(raw: &RawImage, k: usize, clip: f32, keep_max: bool, phase_budget: usize) -> Result<Option<(Rgb32f, Vec<u8>)>> {
    raw.validate()?;
    if !can_bin(raw, k) || !raw.opcodes.list3.is_empty() {
        return Ok(None);
    }
    let Some(cfa) = &raw.cfa else { return Ok(None) };
    let a = raw.active_area;
    // bin the default crop (relative to the active area) only
    let c = raw.crop.clipped(a.width, a.height);
    let c = if c.width < k || c.height < k { Rect::new(0, 0, a.width, a.height) } else { c };
    let (bw, bh) = (c.width / k, c.height / k);
    if bw == 0 || bh == 0 {
        return Ok(None);
    }
    // Opcode lists and a deferred table need the full normalization path. Plain files are
    // normalized row by row so a preview does not allocate another full-resolution buffer.
    let norm: Option<Normalized> = if raw.opcodes.list1.is_empty() && raw.opcodes.list2.is_empty() && deferred_linearization_table(raw).is_none() {
        None
    } else {
        Some(raw.normalized()?)
    };
    let cfa = cfa.shifted(a.x, a.y);
    let range = raw.white_at(0) - raw.black.mean();
    let scale = if range > 0.0 { 1.0 / range } else { 1.0 };
    let uniform_black = raw.black.delta_h.is_empty() && raw.black.delta_v.is_empty() && raw.black.values.iter().all(|&v| v == raw.black.values[0]);
    let black0 = raw.black.at(0, 0, 0, 1);
    let (pw, ph) = (cfa.width, cfa.height);
    let layouts = phase_layouts(&cfa, k, phase_budget);
    let wlen = bw * k;
    let mut out = Rgb32f::new(bw, bh);
    let mut mask = vec![0u8; bw * bh];
    let process_row = |by: usize, row: &mut [[f32; 3]], mrow: &mut [u8]| {
        // the block row's k sample rows, normalised
        let mut rows = vec![0f32; k * wlen];
        for dy in 0..k {
            let y = c.y + by * k + dy;
            let dst = &mut rows[dy * wlen..(dy + 1) * wlen];
            match (&norm, &raw.data) {
                (Some(nm), _) => dst.copy_from_slice(&nm.data[y * nm.width + c.x..][..wlen]),
                (None, RawData::U16(d)) if uniform_black => {
                    let src = &d[(a.y + y) * raw.width + a.x + c.x..][..wlen];
                    for (o, &v) in dst.iter_mut().zip(src) {
                        *o = (v as f32 - black0) * scale;
                    }
                }
                (None, data) => {
                    let base = (a.y + y) * raw.width + a.x + c.x;
                    for (x, o) in dst.iter_mut().enumerate() {
                        *o = (data.get(base + x) - raw.black.at(c.x + x, y, 0, 1)) * scale;
                    }
                }
            }
        }
        let py = (c.y + by * k) % ph;
        for (bx, (px, m)) in row.iter_mut().zip(mrow.iter_mut()).enumerate() {
            let phase_x = (c.x + bx * k) % pw;
            let (sum, max, inv) = if let Some(layouts) = &layouts {
                let (layout, inv) = &layouts[py * pw + phase_x];
                let (mut sum, mut max) = ([0f32; 3], [f32::MIN; 3]);
                for dy in 0..k {
                    let src = &rows[dy * wlen + bx * k..][..k];
                    for (&v, &col) in src.iter().zip(&layout[dy * k..(dy + 1) * k]) {
                        let col = col as usize;
                        sum[col] += v;
                        max[col] = max[col].max(v);
                    }
                }
                (sum, max, *inv)
            } else {
                periodic_block(&rows[bx * k..], wlen, k, &cfa, (phase_x, py))
            };
            for ch in 0..3 {
                let clipped = max[ch] >= clip;
                px[ch] = if clipped && keep_max { max[ch] } else { sum[ch] * inv[ch] };
                *m |= u8::from(clipped) << ch;
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.data.par_chunks_mut(bw).zip(mask.par_chunks_mut(bw)).enumerate().for_each(|(by, (row, mrow))| process_row(by, row, mrow));
    }
    #[cfg(target_arch = "wasm32")]
    {
        for (by, (row, mrow)) in out.data.chunks_mut(bw).zip(mask.chunks_mut(bw)).enumerate() {
            process_row(by, row, mrow);
        }
    }
    Ok(Some((out, mask)))
}

fn phase_layouts(cfa: &super::Cfa, k: usize, budget: usize) -> Option<Vec<PhaseLayout>> {
    let (pw, ph) = (cfa.width, cfa.height);
    // There are at most 16 × 16 phase descriptors; the variable payload is one byte per sample.
    let bytes = k.checked_mul(k)?.checked_mul(pw)?.checked_mul(ph)?;
    if bytes > budget {
        return None;
    }
    Some(
        (0..ph)
            .flat_map(|j| (0..pw).map(move |i| (i, j)))
            .map(|(i, j)| {
                let (layout, count) = block_layout(cfa, i, j, k);
                (layout, count.map(|n| 1.0 / n.max(1) as f32))
            })
            .collect(),
    )
}

/// Accumulate in the same row-major order as the precomputed layout, using one CFA row at a time.
fn periodic_block(rows: &[f32], stride: usize, k: usize, cfa: &super::Cfa, phase: (usize, usize)) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (phase_x, phase_y) = phase;
    let (pw, ph) = (cfa.width, cfa.height);
    let (mut sum, mut max, mut count) = ([0f32; 3], [f32::MIN; 3], [0u32; 3]);
    for dy in 0..k {
        let src = &rows[dy * stride..][..k];
        // The CFA was shifted to the active area's origin; both stored origin offsets are zero.
        let start = ((phase_y + dy) % ph) * pw;
        let colours = cfa.pattern[start..start + pw].iter().cycle().skip(phase_x);
        for (&v, &col) in src.iter().zip(colours) {
            let col = col.min(2) as usize;
            sum[col] += v;
            max[col] = max[col].max(v);
            count[col] += 1;
        }
    }
    (sum, max, count.map(|n| 1.0 / n.max(1) as f32))
}

/// Colours of the `k × k` samples of a block whose corner sits at pattern position `(i, j)`
/// (row-major) and the count of each colour.
fn block_layout(cfa: &super::Cfa, i: usize, j: usize, k: usize) -> (Vec<u8>, [u32; 3]) {
    let mut n = [0u32; 3];
    let l: Vec<u8> = (0..k * k)
        .map(|t| {
            let c = cfa.color_at(i + t % k, j + t / k).min(2);
            n[c as usize] += 1;
            c
        })
        .collect();
    (l, n)
}

#[cfg(test)]
mod tests {
    use super::super::demosaic::mosaic_from_rgb;
    use super::super::*;
    use super::*;

    fn raw_from(n: &Normalized, black: f32, white: f32) -> RawImage {
        let data = n.data.iter().map(|&v| (v * (white - black) + black).round() as u16).collect();
        RawImage {
            format: RawFormat::Dng,
            width: n.width,
            height: n.height,
            cpp: 1,
            data: RawData::U16(data),
            cfa: n.cfa.clone(),
            bits: 14,
            black: BlackLevel::uniform(black),
            white: vec![white],
            active_area: Rect::new(0, 0, n.width, n.height),
            crop: Rect::new(0, 0, n.width, n.height),
            orientation: craft_raw::Orientation::Normal,
            color: ColorData::default(),
            wb_multipliers: None,
            linearized: false,
            linearization_table: None,
            pixel_aspect: 1.0,
            opcodes: OpcodeLists::default(),
            camera: Default::default(),
            cinema: Default::default(),
            lens_metadata: Default::default(),
            level_sources: Default::default(),
            notices: Default::default(),
        }
    }

    /// Omit the writer's WhiteLevel entry from this generated little-endian TIFF without moving
    /// offsets or payloads. This forces the actual decoder to infer white from the table maximum.
    fn omit_white_level(dng: &mut [u8]) {
        assert_eq!(&dng[..4], b"II*\0");
        let ifd = u32::from_le_bytes(dng[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(dng[ifd..ifd + 2].try_into().unwrap());
        let entries = ifd + 2;
        let end = entries + 12 * usize::from(count);
        let white = (entries..end)
            .step_by(12)
            .find(|&i| u16::from_le_bytes(dng[i..i + 2].try_into().unwrap()) == 50_717)
            .expect("the generated DNG has a WhiteLevel entry");
        dng.copy_within(white + 12..end + 4, white);
        dng[ifd..ifd + 2].copy_from_slice(&(count - 1).to_le_bytes());
    }

    #[test]
    fn deferred_linearization_runs_after_opcode_list_one_before_normalizing() {
        use craft_raw::opcodes::Area;
        use craft_raw::{DngCompression, DngWriteOptions};

        let mosaic = Normalized { width: 4, height: 4, cpp: 1, data: vec![1.0; 16], cfa: Cfa::bayer("RGGB") };
        let mut stored = raw_from(&mosaic, 0.0, 1.0);
        stored.white = vec![65535.0]; // deliberately different from the table maximum
        stored.linearization_table = Some(vec![0, 20, 100, 200]);
        let area = Area { top: 0, left: 0, bottom: 4, right: 4, plane: 0, planes: 1, row_pitch: 1, col_pitch: 1 };
        stored.opcodes.list1.push(Opcode::MapTable { area, table: vec![0, 2, 2, 3] });
        let options = DngWriteOptions { compression: DngCompression::Uncompressed, ..Default::default() };
        let mut dng = craft_raw::write_dng(&stored, &options).unwrap();
        omit_white_level(&mut dng);
        let raw = craft_raw::decode(&dng).unwrap();
        assert_eq!(raw.data, RawData::U16(vec![1; 16]));
        assert!(!raw.linearized);
        assert_eq!(raw.opcodes.list1, stored.opcodes.list1);
        assert_eq!(raw.linearization_table.as_deref(), Some([0, 20, 100, 200].as_slice()));
        assert_eq!(raw.white, vec![200.0]);

        // Stored 1 → OpcodeList1 code 2 → table value 100 → 100/200 = 0.5.
        // No LUT gives 0.01; applying it before List1 instead gives 3/200 = 0.015.
        let normalized = raw.normalized().unwrap();
        assert_eq!(normalized.data, vec![0.5; 16]);
        let (preview, clipped) = raw.develop_binned_masked(2, 0.25).unwrap().unwrap();
        assert_eq!(preview.data, vec![[0.5; 3]; 4]);
        assert_eq!(clipped, vec![0b111; 4]);

        let mut no_table = raw.clone();
        no_table.linearization_table = None;
        assert!(no_table.normalized().unwrap().data.iter().all(|&v| (v - 0.01).abs() < 1e-6));

        // A List1 opcode can produce a fractional code; 2.75 rounds to 3, not 2.
        let mut fractional = raw.clone();
        fractional.opcodes.list1 = vec![Opcode::MapPolynomial { area, coefficients: vec![0.0, 2.75] }];
        assert_eq!(fractional.normalized().unwrap().data, vec![1.0; 16]);

        // A stale table on a float input must not turn linear samples into integer codes.
        let mut floats = raw.clone();
        floats.opcodes.list1.clear();
        floats.data = RawData::F32(vec![50.0; 16]);
        assert_eq!(floats.normalized().unwrap().data, vec![0.25; 16]);

        // Public callers can also construct a pending integer table without List1.
        let mut pending = raw;
        pending.opcodes.list1.clear();
        let (preview, _) = pending.develop_binned_masked(2, 0.25).unwrap().unwrap();
        assert!(preview.data.iter().flatten().all(|&v| (v - 0.1).abs() < 1e-6));

        // Without List1, decode already applied the table. Develop must not apply it again.
        stored.opcodes.list1.clear();
        let mut dng = craft_raw::write_dng(&stored, &options).unwrap();
        omit_white_level(&mut dng);
        let decoded = craft_raw::decode(&dng).unwrap();
        assert!(decoded.linearized);
        assert_eq!(decoded.data, RawData::U16(vec![20; 16]));
        assert!(decoded.normalized().unwrap().data.iter().all(|&v| (v - 0.1).abs() < 1e-6));
    }

    #[test]
    fn binning_matches_block_means_and_keeps_clipping() {
        let img = Rgb32f::from_fn(24, 18, |x, y| [0.1 + x as f32 * 0.01, 0.2 + y as f32 * 0.01, 0.3]);
        let mut raw = raw_from(&mosaic_from_rgb(&img, &Cfa::bayer("GRBG").unwrap()), 512.0, 16383.0);
        assert!(raw.can_bin(2) && raw.can_bin(4) && !raw.can_bin(3));
        let b = raw.develop_binned(2, 0.99).unwrap().unwrap();
        assert_eq!((b.width, b.height), (12, 9));
        let p = b.get(3, 2);
        // red sample of block (3,2) under GRBG is at (7,4); blue at (6,5); greens average (6,4)+(7,5)
        assert!((p[0] - img.get(7, 4)[0]).abs() < 1e-3, "{p:?}");
        assert!((p[2] - 0.3).abs() < 1e-3);
        assert!((p[1] - (img.get(6, 4)[1] + img.get(7, 5)[1]) / 2.0).abs() < 1e-3);
        // a single clipped green sample keeps the block's green clipped
        if let RawData::U16(d) = &mut raw.data {
            d[4 * 24 + 6] = 16383;
        }
        assert!(raw.develop_binned(2, 0.99).unwrap().unwrap().get(3, 2)[1] >= 0.99);
        // default crop is honoured at the binned scale
        raw.crop = Rect::new(2, 2, 20, 12);
        let c = raw.develop_binned(2, 0.99).unwrap().unwrap();
        assert_eq!((c.width, c.height), (10, 6));
        // X-Trans: 3×3 blocks hold all colours, 2×2 don't
        let x = raw_from(&mosaic_from_rgb(&img, &Cfa::xtrans()), 0.0, 1000.0);
        assert!(x.can_bin(3) && x.can_bin(6) && !x.can_bin(2));
        let xb = x.develop_binned(3, 0.99).unwrap().unwrap();
        assert_eq!((xb.width, xb.height), (8, 6));
        assert!((xb.get(4, 3)[2] - 0.3).abs() < 2e-3);
    }

    #[test]
    fn binning_requires_a_complete_block_in_the_active_area() {
        let img = Rgb32f::filled(12, 10, [0.25, 0.5, 0.75]);
        let mut raw = raw_from(&mosaic_from_rgb(&img, &Cfa::bayer("RGGB").unwrap()), 0.0, 4096.0);
        raw.active_area = Rect::new(2, 2, 8, 6);
        raw.crop = Rect::new(0, 0, 8, 6);
        assert!(raw.can_bin(6));
        let binned = raw.develop_binned(6, 0.99).unwrap().unwrap();
        assert_eq!((binned.width, binned.height), (1, 1));
        assert_eq!(binned.data, vec![[0.25, 0.5, 0.75]]);
        assert!(!can_bin(&raw, 8));
        assert!(!raw.can_bin(8));
        assert!(raw.develop_binned(8, 0.99).unwrap().is_none());
        assert!(raw.develop_binned_masked(8, 0.99).unwrap().is_none());
    }

    #[test]
    fn binned_phase_budget_preserves_xtrans_pixels_and_clip_masks() {
        let img = Rgb32f::from_fn(23, 21, |x, y| if (x, y) == (7, 4) { [1.0; 3] } else { [x as f32 / 64.0, y as f32 / 64.0, (x + y) as f32 / 64.0] });
        let mut raw = raw_from(&mosaic_from_rgb(&img, &Cfa::xtrans()), 0.0, 4096.0);
        raw.active_area = Rect::new(1, 1, 21, 19);
        raw.crop = Rect::new(1, 2, 17, 15);
        let k = 5;
        let cfa = raw.cfa.as_ref().unwrap().shifted(raw.active_area.x, raw.active_area.y);
        assert!(phase_layouts(&cfa, k, PHASE_COLOUR_BYTES).is_some());
        assert!(phase_layouts(&cfa, k, 0).is_none());
        let compare = |keep_max| {
            let (fast, fast_mask) = bin_with_phase_budget(&raw, k, 0.875, keep_max, PHASE_COLOUR_BYTES).unwrap().unwrap();
            let (bounded, bounded_mask) = bin_with_phase_budget(&raw, k, 0.875, keep_max, 0).unwrap().unwrap();
            assert_eq!((fast.width, fast.height), (3, 3));
            assert_eq!((fast.width, fast.height), (bounded.width, bounded.height));
            let bits = |img: &Rgb32f| img.data.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>();
            assert_eq!(bits(&fast), bits(&bounded));
            assert_eq!(fast_mask, bounded_mask);
            assert_eq!(fast_mask.iter().filter(|&&m| m != 0).count(), 1);
            fast.get(1, 0)
        };
        assert_ne!(compare(false), compare(true));
    }

    /// Issue #548: specular points that clip one green sample in some 8 × 8 blocks of a grey surface.
    /// Taking the block's maximum makes those blocks' green the clip level, which reconstruction then
    /// keeps (a green speckle); the masked binning keeps the mean and marks the green as clipped.
    #[test]
    fn clipping_is_decided_before_averaging() {
        let img = Rgb32f::filled(64, 64, [0.3, 0.3, 0.3]);
        let mut raw = raw_from(&mosaic_from_rgb(&img, &Cfa::bayer("RGGB").unwrap()), 0.0, 4095.0);
        if let RawData::U16(d) = &mut raw.data {
            // one green sample (RGGB: odd column of an even row) in every other block
            for by in 0..8 {
                for bx in (by % 2..8).step_by(2) {
                    d[(by * 8 + 2) * 64 + bx * 8 + 3] = 4095;
                }
            }
        }
        let wb = [1.0f32; 3];
        let (mut masked, mask) = raw.develop_binned_masked(8, 0.99).unwrap().unwrap();
        assert_eq!(mask.len(), 64);
        assert_eq!(mask.iter().filter(|&&m| m == highlight::CLIPPED_G).count(), 32);
        assert!(mask.iter().all(|&m| m & (highlight::CLIPPED_R | highlight::CLIPPED_B) == 0));
        // the clipped block's green is its mean: (31 · 0.3 + 1) / 32
        assert!((masked.get(0, 0)[1] - (31.0 * 0.3 + 1.0) / 32.0).abs() < 1e-3, "{:?}", masked.get(0, 0));
        assert_eq!(highlight::reconstruct_masked(&mut masked, wb, 0.99, [1.0; 3], &mask), 32);
        let green = |img: &Rgb32f| img.data.iter().map(|p| p[1] / p[0]).fold(0.0f32, f32::max);
        // (no more than the mean itself: the clipped sample counts at the clip level)
        assert!(green(&masked) < 1.08, "green excess {}", green(&masked));
        // the maximum rule: a green speckle at the clip level
        let mut max = raw.develop_binned(8, 0.99).unwrap().unwrap();
        assert!(max.get(0, 0)[1] >= 0.99);
        highlight::reconstruct(&mut max, wb, 0.99);
        assert!(green(&max) > 3.0, "{}", green(&max));
        // without clipped samples both are the same plain means, and the mask is empty
        let clean = raw_from(&mosaic_from_rgb(&img, &Cfa::bayer("RGGB").unwrap()), 0.0, 4095.0);
        let (a, m) = clean.develop_binned_masked(8, 0.99).unwrap().unwrap();
        assert!(m.iter().all(|&v| v == 0));
        assert!(a.data == clean.develop_binned(8, 0.99).unwrap().unwrap().data);
    }

    #[test]
    fn binning_matches_demosaic_downscaled_on_smooth_images() {
        let img = Rgb32f::from_fn(64, 48, |x, y| [0.2 + 0.3 * (x as f32 / 64.0), 0.4 - 0.2 * (y as f32 / 48.0), 0.25]);
        let raw = raw_from(&mosaic_from_rgb(&img, &Cfa::bayer("RGGB").unwrap()), 0.0, 4095.0);
        let b = raw.develop_binned(2, 0.99).unwrap().unwrap();
        let full = crate::raster::resample::half(&raw.develop(Method::Ahd).unwrap());
        for y in 2..b.height - 2 {
            for x in 2..b.width - 2 {
                for c in 0..3 {
                    assert!((b.get(x, y)[c] - full.get(x, y)[c]).abs() < 0.01, "({x},{y})");
                }
            }
        }
    }
}

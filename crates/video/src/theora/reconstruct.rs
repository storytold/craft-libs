//! Frame geometry, DC prediction undoing, block reconstruction, and loop filtering (Sections 2, 7.8, 7.9, 7.10).

use super::idct::idct_2d;
use super::loopfilter::{filter_horizontal_edge, filter_vertical_edge};
use super::tables::{DC_PRED_CONFIG, MB_HILBERT, SB_HILBERT, ZIGZAG};
use super::{Error, PixelFormat};

fn alloc_vec<T: Clone>(val: T, n: usize) -> Result<Vec<T>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::OutOfMemory)?;
    v.resize(n, val);
    Ok(v)
}

fn alloc_capacity<T>(n: usize) -> Result<Vec<T>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::OutOfMemory)?;
    Ok(v)
}

/// Information about a single 8x8 block in the coded frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockMeta {
    /// Color plane index (0: Y, 1: Cb, 2: Cr).
    pub pli: usize,
    /// Block column coordinate in its plane (0..w_blocks).
    pub bx: usize,
    /// Block row coordinate in its plane (0..h_blocks).
    pub by: usize,
    /// Pixel coordinate of bottom-left corner in plane.
    pub pixel_x: usize,
    pub pixel_y: usize,
    /// Global superblock index this block belongs to (0..nsbs).
    pub sbi: usize,
    /// Global macroblock index this block belongs to (0..nmbs).
    pub mbi: usize,
    /// Neighbor block indices in coded order: `[Left, Down-Left, Down, Down-Right, Right, Up]`.
    pub neighbors: [Option<usize>; 6],
}

/// Precomputed layout and block mappings for a frame size and pixel format.
#[derive(Clone, Debug)]
pub struct FrameGeometry {
    pub fmbw: usize,
    pub fmbh: usize,
    pub pixel_format: PixelFormat,
    pub frame_width: usize,
    pub frame_height: usize,
    pub nmbs: usize,
    pub nsbs: usize,
    pub nbs: usize,

    /// Plane dimensions: `(width, height, w_blocks, h_blocks, sb_w, sb_h, num_blocks, num_sbs)`.
    pub planes: [(usize, usize, usize, usize, usize, usize, usize, usize); 3],

    /// Metadata for each block in coded order `0..nbs`.
    pub blocks: Vec<BlockMeta>,

    /// Coded-order block indices in raster order for each plane: `blocks_raster[pli]`.
    pub blocks_raster: [Vec<usize>; 3],

    /// For each macroblock `mbi in 0..nmbs`: `(luma_A, luma_B, luma_C, luma_D)`.
    pub mb_luma_blocks: Vec<[usize; 4]>,

    /// For each macroblock `mbi in 0..nmbs`: chroma block indices in coded order.
    pub mb_chroma_cb_blocks: Vec<Vec<usize>>,
    pub mb_chroma_cr_blocks: Vec<Vec<usize>>,

    /// Number of blocks inside each superblock `sbi in 0..nsbs`.
    pub sb_block_counts: Vec<usize>,
}

impl FrameGeometry {
    pub fn new(fmbw: usize, fmbh: usize, pixel_format: PixelFormat) -> Result<Self, Error> {
        let frame_width = fmbw * 16;
        let frame_height = fmbh * 16;
        let nmbs = fmbw * fmbh;

        // Plane dimensions
        // Y:
        let y_pw = frame_width;
        let y_ph = frame_height;
        let y_wb = fmbw * 2;
        let y_hb = fmbh * 2;
        let y_sb_w = y_wb.div_ceil(4);
        let y_sb_h = y_hb.div_ceil(4);
        let y_blocks = y_wb * y_hb;
        let y_sbs = y_sb_w * y_sb_h;

        // Cb & Cr:
        let (c_pw, c_ph, c_wb, c_hb) = match pixel_format {
            PixelFormat::Yuv420 => (fmbw * 8, fmbh * 8, fmbw, fmbh),
            PixelFormat::Yuv422 => (fmbw * 8, fmbh * 16, fmbw, fmbh * 2),
            PixelFormat::Yuv444 => (fmbw * 16, fmbh * 16, fmbw * 2, fmbh * 2),
        };
        let c_sb_w = c_wb.div_ceil(4);
        let c_sb_h = c_hb.div_ceil(4);
        let c_blocks = c_wb * c_hb;
        let c_sbs = c_sb_w * c_sb_h;

        let planes = [
            (y_pw, y_ph, y_wb, y_hb, y_sb_w, y_sb_h, y_blocks, y_sbs),
            (c_pw, c_ph, c_wb, c_hb, c_sb_w, c_sb_h, c_blocks, c_sbs),
            (c_pw, c_ph, c_wb, c_hb, c_sb_w, c_sb_h, c_blocks, c_sbs),
        ];

        let nbs = y_blocks + 2 * c_blocks;
        let nsbs = y_sbs + 2 * c_sbs;

        // Map macroblocks in Hilbert coded order within luma superblocks
        // `mb_grid[my][mx] = mbi`
        let mut mb_grid = Vec::new();
        mb_grid.try_reserve_exact(fmbh).map_err(|_| Error::OutOfMemory)?;
        for _ in 0..fmbh {
            mb_grid.push(alloc_vec(0usize, fmbw)?);
        }
        let mut mb_coords = alloc_capacity(nmbs)?;
        let mut cur_mbi = 0;

        let luma_sb_w = fmbw.div_ceil(2);
        let luma_sb_h = fmbh.div_ceil(2);

        for sb_y in 0..luma_sb_h {
            for sb_x in 0..luma_sb_w {
                for &(dx, dy) in &MB_HILBERT {
                    let mx = sb_x * 2 + dx;
                    let my = sb_y * 2 + dy;
                    if mx < fmbw && my < fmbh {
                        mb_grid[my][mx] = cur_mbi;
                        mb_coords.push((mx, my));
                        cur_mbi += 1;
                    }
                }
            }
        }

        // Build coded blocks for all 3 planes
        let mut blocks = alloc_capacity(nbs)?;
        let mut plane_block_grid = [
            {
                let mut g = Vec::new();
                g.try_reserve_exact(y_hb).map_err(|_| Error::OutOfMemory)?;
                for _ in 0..y_hb {
                    g.push(alloc_vec(0usize, y_wb)?);
                }
                g
            },
            {
                let mut g = Vec::new();
                g.try_reserve_exact(c_hb).map_err(|_| Error::OutOfMemory)?;
                for _ in 0..c_hb {
                    g.push(alloc_vec(0usize, c_wb)?);
                }
                g
            },
            {
                let mut g = Vec::new();
                g.try_reserve_exact(c_hb).map_err(|_| Error::OutOfMemory)?;
                for _ in 0..c_hb {
                    g.push(alloc_vec(0usize, c_wb)?);
                }
                g
            },
        ];

        let mut sb_block_counts = alloc_vec(0usize, nsbs)?;
        let mut global_sbi = 0;
        let mut cur_bi = 0;

        for (pli, plane) in planes.iter().enumerate() {
            let (_, _, wb, hb, sb_w, sb_h, _, _) = *plane;
            let sb_start = global_sbi;

            for sb_y in 0..sb_h {
                for sb_x in 0..sb_w {
                    let sbi = sb_start + sb_y * sb_w + sb_x;
                    let mut count_in_sb = 0;

                    for &(dx, dy) in &SB_HILBERT {
                        let bx = sb_x * 4 + dx;
                        let by = sb_y * 4 + dy;
                        if bx < wb && by < hb {
                            // Find macroblock index mbi
                            let (mx, my) = match (pli, pixel_format) {
                                (0, _) => (bx / 2, by / 2),
                                (1 | 2, PixelFormat::Yuv420) => (bx, by),
                                (1 | 2, PixelFormat::Yuv422) => (bx, by / 2),
                                _ => (bx / 2, by / 2),
                            };
                            let mbi = mb_grid[my][mx];

                            plane_block_grid[pli][by][bx] = cur_bi;
                            blocks.push(BlockMeta { pli, bx, by, pixel_x: bx * 8, pixel_y: by * 8, sbi, mbi, neighbors: [None; 6] });

                            count_in_sb += 1;
                            cur_bi += 1;
                        }
                    }
                    sb_block_counts[sbi] = count_in_sb;
                }
            }
            global_sbi += sb_w * sb_h;
        }

        // Connect neighbors within each plane
        // 0: Left, 1: Down-Left, 2: Down, 3: Down-Right, 4: Right, 5: Up
        for bi in 0..nbs {
            let pli = blocks[bi].pli;
            let bx = blocks[bi].bx;
            let by = blocks[bi].by;
            let (_, _, wb, hb, _, _, _, _) = planes[pli];

            let left = if bx > 0 { Some(plane_block_grid[pli][by][bx - 1]) } else { None };
            let down_left = if bx > 0 && by > 0 { Some(plane_block_grid[pli][by - 1][bx - 1]) } else { None };
            let down = if by > 0 { Some(plane_block_grid[pli][by - 1][bx]) } else { None };
            let down_right = if bx + 1 < wb && by > 0 { Some(plane_block_grid[pli][by - 1][bx + 1]) } else { None };
            let right = if bx + 1 < wb { Some(plane_block_grid[pli][by][bx + 1]) } else { None };
            let up = if by + 1 < hb { Some(plane_block_grid[pli][by + 1][bx]) } else { None };

            blocks[bi].neighbors = [left, down_left, down, down_right, right, up];
        }

        // Build raster order lists for each plane
        let mut blocks_raster = [Vec::with_capacity(y_blocks), Vec::with_capacity(c_blocks), Vec::with_capacity(c_blocks)];

        for (pli, plane) in planes.iter().enumerate() {
            let (_, _, wb, hb, _, _, _, _) = *plane;
            for by in 0..hb {
                for bx in 0..wb {
                    blocks_raster[pli].push(plane_block_grid[pli][by][bx]);
                }
            }
        }

        // Build macroblock -> block mappings
        let mut mb_luma_blocks = alloc_vec([0usize; 4], nmbs)?;
        let mut mb_chroma_cb_blocks = Vec::new();
        mb_chroma_cb_blocks.try_reserve_exact(nmbs).map_err(|_| Error::OutOfMemory)?;
        for _ in 0..nmbs {
            mb_chroma_cb_blocks.push(Vec::new());
        }
        let mut mb_chroma_cr_blocks = Vec::new();
        mb_chroma_cr_blocks.try_reserve_exact(nmbs).map_err(|_| Error::OutOfMemory)?;
        for _ in 0..nmbs {
            mb_chroma_cr_blocks.push(Vec::new());
        }

        for mbi in 0..nmbs {
            let (mx, my) = mb_coords[mbi];
            // Luma raster: A(lower-left), B(lower-right), C(upper-left), D(upper-right)
            let a = plane_block_grid[0][my * 2][mx * 2];
            let b = plane_block_grid[0][my * 2][mx * 2 + 1];
            let c = plane_block_grid[0][my * 2 + 1][mx * 2];
            let d = plane_block_grid[0][my * 2 + 1][mx * 2 + 1];
            mb_luma_blocks[mbi] = [a, b, c, d];

            match pixel_format {
                PixelFormat::Yuv420 => {
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my][mx]);
                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my][mx]);
                }
                PixelFormat::Yuv422 => {
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my * 2][mx]);
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my * 2 + 1][mx]);
                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my * 2][mx]);
                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my * 2 + 1][mx]);
                }
                PixelFormat::Yuv444 => {
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my * 2][mx * 2]);
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my * 2][mx * 2 + 1]);
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my * 2 + 1][mx * 2]);
                    mb_chroma_cb_blocks[mbi].push(plane_block_grid[1][my * 2 + 1][mx * 2 + 1]);

                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my * 2][mx * 2]);
                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my * 2][mx * 2 + 1]);
                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my * 2 + 1][mx * 2]);
                    mb_chroma_cr_blocks[mbi].push(plane_block_grid[2][my * 2 + 1][mx * 2 + 1]);
                }
            }
        }

        Ok(Self {
            fmbw,
            fmbh,
            pixel_format,
            frame_width,
            frame_height,
            nmbs,
            nsbs,
            nbs,
            planes,
            blocks,
            blocks_raster,
            mb_luma_blocks,
            mb_chroma_cb_blocks,
            mb_chroma_cr_blocks,
            sb_block_counts,
        })
    }
}

/// Map macroblock coding mode (0..=7) to reference frame index (Table 7.46).
/// 0: INTRA, 1: Previous, 2: Golden.
#[inline]
pub fn mb_mode_to_ref_frame(mode: u8) -> usize {
    match mode {
        1 => 0,     // INTRA
        5 | 6 => 2, // GOLDEN NOMV, GOLDEN MV
        _ => 1,     // INTER NOMV, INTER MV, INTER MV LAST, INTER MV LAST2, INTER MV FOUR
    }
}

/// Undo DC prediction across all planes in raster order (Section 7.8).
pub fn undo_dc_prediction(geom: &FrameGeometry, bcoded: &[bool], mb_modes: &[u8], coeffs: &mut [[i16; 64]]) {
    for pli in 0..3 {
        let mut last_dc = [0i32; 3];

        for &bi in &geom.blocks_raster[pli] {
            if !bcoded[bi] {
                continue;
            }

            let meta = &geom.blocks[bi];
            let rfi = mb_mode_to_ref_frame(mb_modes[meta.mbi]);

            // Neighbors: 0: L, 1: DL, 2: D, 3: DR
            let mut p = [0u8; 4];
            let mut pbi = [0usize; 4];

            for (idx, &neigh_opt) in meta.neighbors[0..4].iter().enumerate() {
                if let Some(bj) = neigh_opt
                    && bcoded[bj]
                {
                    let mbj = geom.blocks[bj].mbi;
                    if mb_mode_to_ref_frame(mb_modes[mbj]) == rfi {
                        p[idx] = 1;
                        pbi[idx] = bj;
                    }
                }
            }

            let dc_pred = if p[0] == 0 && p[1] == 0 && p[2] == 0 && p[3] == 0 {
                last_dc[rfi]
            } else {
                let p_key = (p[0] as usize) | ((p[1] as usize) << 1) | ((p[2] as usize) << 2) | ((p[3] as usize) << 3);
                let (w, pdiv) = DC_PRED_CONFIG[p_key];

                let mut sum = 0i32;
                if p[0] != 0 {
                    sum += w[0] * coeffs[pbi[0]][0] as i32;
                }
                if p[1] != 0 {
                    sum += w[1] * coeffs[pbi[1]][0] as i32;
                }
                if p[2] != 0 {
                    sum += w[2] * coeffs[pbi[2]][0] as i32;
                }
                if p[3] != 0 {
                    sum += w[3] * coeffs[pbi[3]][0] as i32;
                }

                let mut pred = sum / pdiv;

                if p[0] != 0 && p[1] != 0 && p[2] != 0 {
                    let d0 = (pred - coeffs[pbi[2]][0] as i32).abs();
                    let d1 = (pred - coeffs[pbi[0]][0] as i32).abs();
                    let d2 = (pred - coeffs[pbi[1]][0] as i32).abs();

                    if d0 > 128 {
                        pred = coeffs[pbi[2]][0] as i32;
                    } else if d1 > 128 {
                        pred = coeffs[pbi[0]][0] as i32;
                    } else if d2 > 128 {
                        pred = coeffs[pbi[1]][0] as i32;
                    }
                }
                pred
            };

            let dc = (coeffs[bi][0] as i32 + dc_pred) as i16;
            coeffs[bi][0] = dc;
            last_dc[rfi] = dc as i32;
        }
    }
}

/// Compute block predictor (Intra, whole-pixel, or half-pixel; Section 7.9.1).
pub fn compute_block_predictor(
    pli: usize,
    bx: usize,
    by: usize,
    rfi: usize,
    mv: (i8, i8),
    pixel_format: PixelFormat,
    ref_y: &[u8],
    ref_cb: &[u8],
    ref_cr: &[u8],
    plane_w: usize,
    plane_h: usize,
    pred: &mut [[u8; 8]; 8],
) {
    if rfi == 0 {
        // Intra predictor is constant 128
        for row in pred.iter_mut() {
            row.fill(128);
        }
        return;
    }

    let ref_plane = match pli {
        0 => ref_y,
        1 => ref_cb,
        2 => ref_cr,
        _ => ref_y,
    };

    let is_chroma_subsampled_x = pli > 0 && pixel_format != PixelFormat::Yuv444;
    let is_chroma_subsampled_y = pli > 0 && pixel_format == PixelFormat::Yuv420;

    let (mvx1, mvx2) = split_mv_component(mv.0 as i32, is_chroma_subsampled_x);
    let (mvy1, mvy2) = split_mv_component(mv.1 as i32, is_chroma_subsampled_y);

    let is_whole_pixel = mvx1 == mvx2 && mvy1 == mvy2;

    for row_idx in 0..8 {
        let py = by + row_idx;
        let ry1 = (py as i32 + mvy1).clamp(0, plane_h as i32 - 1) as usize;
        let ry2 = (py as i32 + mvy2).clamp(0, plane_h as i32 - 1) as usize;

        for col_idx in 0..8 {
            let px = bx + col_idx;
            let rx1 = (px as i32 + mvx1).clamp(0, plane_w as i32 - 1) as usize;

            if is_whole_pixel {
                pred[row_idx][col_idx] = ref_plane[ry1 * plane_w + rx1];
            } else {
                let rx2 = (px as i32 + mvx2).clamp(0, plane_w as i32 - 1) as usize;
                let s1 = ref_plane[ry1 * plane_w + rx1] as u16;
                let s2 = ref_plane[ry2 * plane_w + rx2] as u16;
                pred[row_idx][col_idx] = ((s1 + s2) >> 1) as u8;
            }
        }
    }
}

/// Split motion vector component into whole-pixel bounds `(mv1, mv2)`.
/// mv1: truncated towards zero.
/// mv2: truncated away from zero.
#[inline]
fn split_mv_component(mv: i32, is_subsampled: bool) -> (i32, i32) {
    if !is_subsampled {
        // Half-pixel accuracy: 1 pixel = 2 units
        if mv >= 0 { (mv / 2, (mv + 1) / 2) } else { (-((-mv) / 2), -((-mv + 1) / 2)) }
    } else {
        // Quarter-pixel accuracy: 1 pixel = 4 units
        if mv >= 0 { (mv / 4, (mv + 3) / 4) } else { (-((-mv) / 4), -((-mv + 3) / 4)) }
    }
}

/// Reconstruct a single 8x8 block into the destination plane (Section 7.9).
pub fn reconstruct_block(
    meta: &BlockMeta,
    coded: bool,
    ncoeffs: usize,
    pred: &[[u8; 8]; 8],
    coeffs_bi: &[i16; 64],
    dc_qmat: &[u16; 64],
    ac_qmat: &[u16; 64],
    dest_plane: &mut [u8],
    stride: usize,
) {
    let mut res = [[0i16; 8]; 8];

    if coded {
        if ncoeffs < 2 {
            // Special DC-only fast path (Section 7.9.4 step 2(d)vii)
            let dc = ((coeffs_bi[0] as i32 * dc_qmat[0] as i32 + 15) >> 5) as i16;
            for row in res.iter_mut() {
                row.fill(dc);
            }
        } else {
            // Full dequantization and inverse DCT (Section 7.9.2 & 7.9.3)
            let mut dqc = [0i16; 64];
            dqc[0] = (coeffs_bi[0] as i32 * dc_qmat[0] as i32) as i16;
            for ci in 1..64 {
                let zzi = ZIGZAG[ci];
                dqc[ci] = (coeffs_bi[zzi] as i32 * ac_qmat[ci] as i32) as i16;
            }
            res = idct_2d(&dqc);
        }
    }

    // Add prediction and residual, clamp to 0..=255, and write to destination
    for by in 0..8 {
        let row_offset = (meta.pixel_y + by) * stride + meta.pixel_x;
        for bx in 0..8 {
            let p = pred[by][bx] as i32 + res[by][bx] as i32;
            dest_plane[row_offset + bx] = p.clamp(0, 255) as u8;
        }
    }
}

/// In-loop deblocking filter pass across all block boundaries (Section 7.10).
pub fn apply_loop_filter(geom: &FrameGeometry, bcoded: &[bool], lflim_l: u8, rec_y: &mut [u8], rec_cb: &mut [u8], rec_cr: &mut [u8]) {
    if lflim_l == 0 {
        return;
    }

    let l = lflim_l as i32;

    for pli in 0..3 {
        let (pw, ph, _, _, _, _, _, _) = geom.planes[pli];
        let rec_plane = match pli {
            0 => &mut *rec_y,
            1 => &mut *rec_cb,
            2 => &mut *rec_cr,
            _ => continue,
        };

        for &bi in &geom.blocks_raster[pli] {
            if !bcoded[bi] {
                continue;
            }

            let meta = &geom.blocks[bi];
            let bx = meta.pixel_x;
            let by = meta.pixel_y;

            // Left edge
            if bx > 0 {
                filter_horizontal_edge(rec_plane, pw, bx - 2, by, l);
            }

            // Bottom edge
            if by > 0 {
                filter_vertical_edge(rec_plane, pw, bx, by - 2, l);
            }

            // Right edge (if neighbor is uncoded)
            if bx + 8 < pw
                && let Some(bi_right) = meta.neighbors[4]
                && !bcoded[bi_right]
            {
                filter_horizontal_edge(rec_plane, pw, bx + 6, by, l);
            }

            // Top edge (if neighbor is uncoded)
            if by + 8 < ph
                && let Some(bi_up) = meta.neighbors[5]
                && !bcoded[bi_up]
            {
                filter_vertical_edge(rec_plane, pw, bx, by + 6, l);
            }
        }
    }
}

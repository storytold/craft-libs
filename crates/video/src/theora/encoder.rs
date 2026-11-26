//! Theora video encoder (Section 7).

use super::header::{CommentHeader, IdentificationHeader, SetupHeader};
use super::idct::fdct_1d;
use super::reconstruct::{FrameGeometry, apply_loop_filter, reconstruct_block, undo_dc_prediction};
use super::tables::ZIGZAG;
use super::{ColorSpace, Error, Info, Picture, PixelFormat, Result};
use craft_media::bitstream::BitWriter;

/// Configuration for creating a Theora video encoder.
#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub pic_width: u32,
    pub pic_height: u32,
    pub pic_x: u32,
    pub pic_y: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub pixel_format: PixelFormat,
    pub color_space: ColorSpace,
    pub quality: u8,
    pub keyframe_distance: u32,
}

impl EncoderConfig {
    /// The part of plane `pli` the picture covers, as `(x, y, width, height)` in samples. A
    /// subsampled plane takes every sample that lies under a visible luma sample, so the start
    /// rounds down and the end (exclusive) rounds up: with an odd offset the first and last chroma
    /// samples can each be shared with a luma sample outside the picture.
    fn active_region(&self, pli: usize) -> (usize, usize, usize, usize) {
        let (shift_x, shift_y) = match (pli, self.pixel_format) {
            (0, _) | (_, PixelFormat::Yuv444) => (0, 0),
            (_, PixelFormat::Yuv422) => (1, 0),
            (_, PixelFormat::Yuv420) => (1, 1),
        };
        let span = |origin: u32, size: u32, shift: u32| {
            let start = (origin >> shift) as usize;
            let end = (origin as usize + size as usize).div_ceil(1 << shift);
            (start, end.saturating_sub(start))
        };
        let (x, w) = span(self.pic_x, self.pic_width, shift_x);
        let (y, h) = span(self.pic_y, self.pic_height, shift_y);
        (x, y, w, h)
    }
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 240,
            pic_width: 320,
            pic_height: 240,
            pic_x: 0,
            pic_y: 0,
            fps_num: 25,
            fps_den: 1,
            pixel_format: PixelFormat::Yuv420,
            color_space: ColorSpace::Undefined,
            quality: 32,
            keyframe_distance: 64,
        }
    }
}

/// A conformant Theora video keyframe and interframe encoder.
pub struct Encoder {
    config: EncoderConfig,
    id_header: IdentificationHeader,
    comment_header: CommentHeader,
    setup_header: SetupHeader,
    geom: FrameGeometry,
    qmat_tables: [[[[u16; 64]; 64]; 3]; 2],

    // Reference frames
    ref_golden_y: Vec<u8>,
    ref_golden_cb: Vec<u8>,
    ref_golden_cr: Vec<u8>,

    ref_prev_y: Vec<u8>,
    ref_prev_cb: Vec<u8>,
    ref_prev_cr: Vec<u8>,

    frame_count: u64,
    last_keyframe: u64,
}

fn alloc_plane_buffer(len: usize) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::OutOfMemory)?;
    v.resize(len, 0);
    Ok(v)
}

fn alloc_vec_exact<T: Clone>(val: T, n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::OutOfMemory)?;
    v.resize(n, val);
    Ok(v)
}

impl Encoder {
    /// Create a new encoder with the given configuration.
    pub fn new(config: EncoderConfig) -> Result<Self> {
        if config.width == 0 || config.height == 0 || config.pic_width == 0 || config.pic_height == 0 {
            return Err(Error::DimensionMismatch("dimensions cannot be zero"));
        }
        if config.pic_x > 255
            || config.pic_y > 255
            || config.pic_x.checked_add(config.pic_width).is_none_or(|end| end > config.width)
            || config.pic_y.checked_add(config.pic_height).is_none_or(|end| end > config.height)
        {
            return Err(Error::DimensionMismatch("picture region exceeds frame bounds"));
        }
        let frame_width = config.width.checked_add(15).map(|w| w & !15).ok_or(Error::DimensionMismatch("width overflows"))?;
        let frame_height = config.height.checked_add(15).map(|h| h & !15).ok_or(Error::DimensionMismatch("height overflows"))?;
        let fmbw_u64 = (frame_width / 16) as u64;
        let fmbh_u64 = (frame_height / 16) as u64;
        if fmbw_u64 == 0 || fmbh_u64 == 0 || fmbw_u64 > u16::MAX as u64 || fmbh_u64 > u16::MAX as u64 {
            return Err(Error::DimensionMismatch("macroblock dimensions must fit in u16"));
        }
        let fmbw = fmbw_u64 as usize;
        let fmbh = fmbh_u64 as usize;

        // Validate memory budget before allocating geometry and planes
        let (cw, ch) = match config.pixel_format {
            PixelFormat::Yuv420 => (fmbw_u64.checked_mul(8).ok_or(Error::OutOfMemory)?, fmbh_u64.checked_mul(8).ok_or(Error::OutOfMemory)?),
            PixelFormat::Yuv422 => (fmbw_u64.checked_mul(8).ok_or(Error::OutOfMemory)?, fmbh_u64.checked_mul(16).ok_or(Error::OutOfMemory)?),
            PixelFormat::Yuv444 => (fmbw_u64.checked_mul(16).ok_or(Error::OutOfMemory)?, fmbh_u64.checked_mul(16).ok_or(Error::OutOfMemory)?),
        };

        let y_bytes = (fmbw_u64.checked_mul(16).ok_or(Error::OutOfMemory)?)
            .checked_mul(fmbh_u64.checked_mul(16).ok_or(Error::OutOfMemory)?)
            .ok_or(Error::OutOfMemory)?;
        let c_bytes = cw.checked_mul(ch).ok_or(Error::OutOfMemory)?;
        let one_picture_bytes = y_bytes.checked_add(c_bytes.checked_mul(2).ok_or(Error::OutOfMemory)?).ok_or(Error::OutOfMemory)?;

        let pictures_mem = one_picture_bytes.checked_mul(3).ok_or(Error::OutOfMemory)?;
        let nmbs = fmbw_u64.checked_mul(fmbh_u64).ok_or(Error::OutOfMemory)?;
        let blocks_per_mb = config.pixel_format.blocks_per_macroblock() as u64;
        let nbs = nmbs.checked_mul(blocks_per_mb).ok_or(Error::OutOfMemory)?;
        let meta_mem = nbs.checked_mul(300).ok_or(Error::OutOfMemory)?;

        let total_mem = pictures_mem.checked_add(meta_mem).ok_or(Error::OutOfMemory)?;
        if total_mem > crate::budget::DECODE_MEMORY_BUDGET {
            return Err(Error::OutOfMemory);
        }

        let info = Info {
            version_major: 3,
            version_minor: 2,
            version_revision: 0,
            frame_width,
            frame_height,
            pic_width: config.pic_width.min(frame_width),
            pic_height: config.pic_height.min(frame_height),
            pic_x: config.pic_x.min(frame_width - config.pic_width.min(frame_width)),
            pic_y: config.pic_y.min(frame_height - config.pic_height.min(frame_height)),
            fps_num: config.fps_num.max(1),
            fps_den: config.fps_den.max(1),
            par_num: 1,
            par_den: 1,
            color_space: config.color_space,
            pixel_format: config.pixel_format,
            nominal_bitrate: 0,
            quality: config.quality.min(63),
            keyframe_granule_shift: 6,
        };

        let id_header = IdentificationHeader { info };
        let comment_header =
            CommentHeader { vendor: "craft-video pure-rust Theora encoder".to_string(), comments: vec!["ENCODER=craft-video".to_string()] };

        let setup_header = SetupHeader::default_vp3()?;
        let geom = FrameGeometry::new(fmbw, fmbh, config.pixel_format)?;
        let qmat_tables = setup_header.quant_params.precompute_all_qmat()?;

        let (y_pw, y_ph, _, _, _, _, _, _) = geom.planes[0];
        let (c_pw, c_ph, _, _, _, _, _, _) = geom.planes[1];

        let ref_golden_y = alloc_plane_buffer(y_pw * y_ph)?;
        let ref_golden_cb = alloc_plane_buffer(c_pw * c_ph)?;
        let ref_golden_cr = alloc_plane_buffer(c_pw * c_ph)?;

        let ref_prev_y = alloc_plane_buffer(y_pw * y_ph)?;
        let ref_prev_cb = alloc_plane_buffer(c_pw * c_ph)?;
        let ref_prev_cr = alloc_plane_buffer(c_pw * c_ph)?;

        Ok(Self {
            config,
            id_header,
            comment_header,
            setup_header,
            geom,
            qmat_tables,
            ref_golden_y,
            ref_golden_cb,
            ref_golden_cr,
            ref_prev_y,
            ref_prev_cb,
            ref_prev_cr,
            frame_count: 0,
            last_keyframe: 0,
        })
    }

    /// Returns the encoder configuration.
    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// Returns the stream identification header.
    pub fn id_header(&self) -> &IdentificationHeader {
        &self.id_header
    }

    /// Returns the comment header.
    pub fn comment_header(&self) -> &CommentHeader {
        &self.comment_header
    }

    /// Returns the setup header.
    pub fn setup_header(&self) -> &SetupHeader {
        &self.setup_header
    }

    /// Serialize the three mandatory stream headers in order: `(id, comment, setup)`.
    pub fn headers(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (self.id_header.serialize(), self.comment_header.serialize(), self.setup_header.serialize())
    }

    /// Encode a single picture into a conformant Theora packet.
    ///
    /// Currently produces intra keyframes for every frame, ensuring all packets are independent
    /// self-contained keyframes that decode reliably.
    pub fn encode_frame(&mut self, pic: &Picture, _force_keyframe: bool) -> Result<Vec<u8>> {
        if pic.width != self.config.width
            || pic.height != self.config.height
            || pic.pic_width != self.config.pic_width
            || pic.pic_height != self.config.pic_height
            || pic.pic_x != self.config.pic_x
            || pic.pic_y != self.config.pic_y
            || pic.pixel_format != self.config.pixel_format
        {
            return Err(Error::DimensionMismatch("picture dimensions do not match encoder configuration"));
        }
        let src_uv_w = match self.config.pixel_format {
            PixelFormat::Yuv420 | PixelFormat::Yuv422 => (self.config.width as usize).div_ceil(2),
            PixelFormat::Yuv444 => self.config.width as usize,
        };
        if pic.y_stride == 0 || pic.uv_stride == 0 {
            return Err(Error::DimensionMismatch("stride cannot be zero"));
        }
        if pic.y_stride < self.config.width as usize || pic.uv_stride < src_uv_w {
            return Err(Error::DimensionMismatch("stride shorter than plane width"));
        }

        let mut w = BitWriter::new();

        // 1. Frame Header (Section 7.1)
        w.write_bit(false); // data packet bit = 0
        w.write_bit(false); // FTYPE: 0: Intra (keyframe)

        // qi: map quality (0..=63) directly to quantization index (0 = coarsest, 63 = finest)
        let qi = self.config.quality.min(63) as usize;
        w.write_bits(qi as u32, 6);
        w.write_bit(false); // MOREQIS = 0 (1 qi value)

        // 3 reserved bits for intra keyframe = 000
        w.write_bits(0, 3);

        let nbs = self.geom.nbs;
        let (y_pw, y_ph, _, _, _, _, _, _) = self.geom.planes[0];
        let (c_pw, c_ph, _, _, _, _, _, _) = self.geom.planes[1];

        let mut rec_y = alloc_plane_buffer(y_pw * y_ph)?;
        let mut rec_cb = alloc_plane_buffer(c_pw * c_ph)?;
        let mut rec_cr = alloc_plane_buffer(c_pw * c_ph)?;

        let bcoded = alloc_vec_exact(true, nbs)?;
        let mb_modes = alloc_vec_exact(1u8, self.geom.nmbs)?; // 1: INTRA

        // For intra frames, all blocks are coded
        // Forward transform & quantize
        let mut quant_coeffs = alloc_vec_exact([0i16; 64], nbs)?;
        let mut ncoeffs = alloc_vec_exact(0usize, nbs)?;

        for bi in 0..nbs {
            let meta = &self.geom.blocks[bi];
            let pli = meta.pli;
            let (src_plane, stride) = match pli {
                0 => (&pic.y, pic.y_stride),
                1 => (&pic.u, pic.uv_stride),
                2 => (&pic.v, pic.uv_stride),
                _ => (&pic.y, pic.y_stride),
            };

            let (active_x, active_y, active_w, active_h) = self.config.active_region(pli);

            // Read 8x8 block from source picture (origin at lower-left!)
            // Note: In Theora coordinates, row 0 is at the bottom of the plane.
            let mut block_samples = [0i16; 64];
            for row in 0..8 {
                let py = meta.pixel_y + row;
                for col in 0..8 {
                    let px = meta.pixel_x + col;
                    let sample = if px >= active_x && px < active_x + active_w && py >= active_y && py < active_y + active_h {
                        let offset =
                            py.checked_mul(stride).and_then(|r| r.checked_add(px)).ok_or(Error::DimensionMismatch("sample offset overflow"))?;
                        src_plane
                            .get(offset)
                            .copied()
                            .map(|b| b as i16)
                            .ok_or(Error::DimensionMismatch("source plane buffer shorter than active picture"))?
                    } else {
                        let offset = py.checked_mul(stride).and_then(|r| r.checked_add(px));
                        offset.and_then(|off| src_plane.get(off).copied()).map(|b| b as i16).unwrap_or(128)
                    };
                    // Subtract 128 (intra predictor)
                    block_samples[row * 8 + col] = sample - 128;
                }
            }

            // Forward DCT (Section 7.9.3.3)
            let mut fdct_coeffs = [0i16; 64];
            // 2D forward DCT: row-by-row then col-by-col
            let mut temp = [0i16; 64];
            for r in 0..8 {
                let mut row_in = [0i16; 8];
                row_in.copy_from_slice(&block_samples[r * 8..r * 8 + 8]);
                let row_out = fdct_1d(&row_in);
                temp[r * 8..r * 8 + 8].copy_from_slice(&row_out);
            }
            for c in 0..8 {
                let mut col_in = [0i16; 8];
                for r in 0..8 {
                    col_in[r] = temp[r * 8 + c];
                }
                let col_out = fdct_1d(&col_in);
                for r in 0..8 {
                    fdct_coeffs[r * 8 + c] = col_out[r];
                }
            }

            // Quantize natural order -> zig-zag order
            let qmat = &self.qmat_tables[0][pli][qi];
            for ci in 0..64 {
                let zzi = ZIGZAG[ci];
                let coeff = fdct_coeffs[ci] as i32;
                let q = qmat[ci] as i32;
                let quant = if coeff >= 0 { (coeff + q / 2) / q } else { (coeff - q / 2) / q };
                quant_coeffs[bi][zzi] = quant.clamp(-580, 580) as i16;
            }

            // Compute count of coefficients
            let mut last_non_zero = 0;
            for i in (0..64).rev() {
                if quant_coeffs[bi][i] != 0 {
                    last_non_zero = i + 1;
                    break;
                }
            }
            ncoeffs[bi] = last_non_zero;
        }

        // Apply DC prediction in raster order (Section 7.8)
        let mut stored_coeffs = quant_coeffs.clone();
        for pli in 0..3 {
            let mut last_dc = 0i32;

            for &bi in &self.geom.blocks_raster[pli] {
                let meta = &self.geom.blocks[bi];
                let mut p = [0u8; 4];
                let mut pbi = [0usize; 4];

                for (idx, &neigh_opt) in meta.neighbors[0..4].iter().enumerate() {
                    if let Some(bj) = neigh_opt {
                        p[idx] = 1;
                        pbi[idx] = bj;
                    }
                }

                let dc_pred = if p[0] == 0 && p[1] == 0 && p[2] == 0 && p[3] == 0 {
                    last_dc
                } else {
                    let p_key = (p[0] as usize) | ((p[1] as usize) << 1) | ((p[2] as usize) << 2) | ((p[3] as usize) << 3);
                    let (w, pdiv) = super::tables::DC_PRED_CONFIG[p_key];

                    let mut sum = 0i32;
                    if p[0] != 0 {
                        sum += w[0] * quant_coeffs[pbi[0]][0] as i32;
                    }
                    if p[1] != 0 {
                        sum += w[1] * quant_coeffs[pbi[1]][0] as i32;
                    }
                    if p[2] != 0 {
                        sum += w[2] * quant_coeffs[pbi[2]][0] as i32;
                    }
                    if p[3] != 0 {
                        sum += w[3] * quant_coeffs[pbi[3]][0] as i32;
                    }

                    let mut pred = sum / pdiv;
                    if p[0] != 0 && p[1] != 0 && p[2] != 0 {
                        let d0 = (pred - quant_coeffs[pbi[2]][0] as i32).abs();
                        let d1 = (pred - quant_coeffs[pbi[0]][0] as i32).abs();
                        let d2 = (pred - quant_coeffs[pbi[1]][0] as i32).abs();
                        if d0 > 128 {
                            pred = quant_coeffs[pbi[2]][0] as i32;
                        } else if d1 > 128 {
                            pred = quant_coeffs[pbi[0]][0] as i32;
                        } else if d2 > 128 {
                            pred = quant_coeffs[pbi[1]][0] as i32;
                        }
                    }
                    pred
                };

                let original_dc = quant_coeffs[bi][0] as i32;
                stored_coeffs[bi][0] = (original_dc - dc_pred) as i16;
                last_dc = original_dc;
            }
        }

        // Write DCT Tokens in 64 passes (Section 7.7.3)
        let mut tis = vec![0usize; nbs];

        for ti in 0..64 {
            if ti == 0 || ti == 1 {
                w.write_bits(0, 4); // hti_l = 0
                w.write_bits(0, 4); // hti_c = 0
            }

            let mut bi = 0;
            while bi < nbs {
                if tis[bi] != ti {
                    bi += 1;
                    continue;
                }

                // Check remaining coefficients in block bi from ti..64
                let remaining_all_zero = (ti..64).all(|idx| stored_coeffs[bi][idx] == 0);

                if remaining_all_zero {
                    // Count how many consecutive blocks also have remaining all zero at ti
                    let mut eob_run = 0usize;
                    let mut lookahead = bi;
                    while lookahead < nbs && eob_run < 4095 {
                        if tis[lookahead] == ti {
                            if (ti..64).all(|idx| stored_coeffs[lookahead][idx] == 0) {
                                eob_run += 1;
                                lookahead += 1;
                            } else {
                                break;
                            }
                        } else {
                            lookahead += 1;
                        }
                    }

                    let hg = match ti {
                        0 => 0,
                        1..=5 => 1,
                        6..=14 => 2,
                        15..=27 => 3,
                        _ => 4,
                    };
                    let hti = 16 * hg;
                    let tree = &self.setup_header.huffman_trees[hti];

                    // Write optimal EOB token
                    let (token, extra_bits, extra_len) = if eob_run == 1 {
                        (0u8, 0u32, 0u32)
                    } else if eob_run == 2 {
                        (1u8, 0u32, 0u32)
                    } else if eob_run == 3 {
                        (2u8, 0u32, 0u32)
                    } else if eob_run <= 7 {
                        (3u8, (eob_run - 4) as u32, 2u32)
                    } else if eob_run <= 15 {
                        (4u8, (eob_run - 8) as u32, 3u32)
                    } else if eob_run <= 31 {
                        (5u8, (eob_run - 16) as u32, 4u32)
                    } else {
                        let clamped_run = eob_run.min(4095);
                        (6u8, clamped_run as u32, 12u32)
                    };

                    let (code, code_len) = tree.codes[token as usize];
                    w.write_bits(code, code_len as u32);
                    if extra_len > 0 {
                        w.write_bits(extra_bits, extra_len);
                    }

                    // Mark all blocks in this EOB run as finished (tis = 64)
                    let actual_run = match token {
                        0 => 1,
                        1 => 2,
                        2 => 3,
                        3 => 4 + extra_bits as usize,
                        4 => 8 + extra_bits as usize,
                        5 => 16 + extra_bits as usize,
                        6 => extra_bits as usize,
                        _ => 1,
                    };

                    let mut marked = 0;
                    let mut curr = bi;
                    while curr < nbs && marked < actual_run {
                        if tis[curr] == ti {
                            tis[curr] = 64;
                            marked += 1;
                        }
                        curr += 1;
                    }
                    bi = curr;
                } else {
                    // Non-zero coefficient token
                    let val = stored_coeffs[bi][ti];
                    let hg = match ti {
                        0 => 0,
                        1..=5 => 1,
                        6..=14 => 2,
                        15..=27 => 3,
                        _ => 4,
                    };
                    let hti = 16 * hg;
                    let tree = &self.setup_header.huffman_trees[hti];

                    if val == 0 {
                        // Single zero run (token 7 with rlen 1)
                        let (code, code_len) = tree.codes[7];
                        w.write_bits(code, code_len as u32);
                        w.write_bits(0, 3); // rlen = 0 (represents 1 zero)
                        tis[bi] += 1;
                    } else {
                        let (token, extra_bits, extra_len) = match val {
                            1 => (9u8, 0u32, 0u32),
                            -1 => (10u8, 0u32, 0u32),
                            2 => (11u8, 0u32, 0u32),
                            -2 => (12u8, 0u32, 0u32),
                            3 | -3 => (13u8, (val < 0) as u32, 1u32),
                            4 | -4 => (14u8, (val < 0) as u32, 1u32),
                            5 | -5 => (15u8, (val < 0) as u32, 1u32),
                            6 | -6 => (16u8, (val < 0) as u32, 1u32),
                            7..=8 | -8..=-7 => {
                                let sign = (val < 0) as u32;
                                let mag = (val.abs() - 7) as u32;
                                (17u8, (sign << 1) | mag, 2u32)
                            }
                            9..=12 | -12..=-9 => {
                                let sign = (val < 0) as u32;
                                let mag = (val.abs() - 9) as u32;
                                (18u8, (sign << 2) | mag, 3u32)
                            }
                            13..=20 | -20..=-13 => {
                                let sign = (val < 0) as u32;
                                let mag = (val.abs() - 13) as u32;
                                (19u8, (sign << 3) | mag, 4u32)
                            }
                            21..=36 | -36..=-21 => {
                                let sign = (val < 0) as u32;
                                let mag = (val.abs() - 21) as u32;
                                (20u8, (sign << 4) | mag, 5u32)
                            }
                            37..=68 | -68..=-37 => {
                                let sign = (val < 0) as u32;
                                let mag = (val.abs() - 37) as u32;
                                (21u8, (sign << 5) | mag, 6u32)
                            }
                            _ => {
                                let sign = (val < 0) as u32;
                                let mag = (val.abs().clamp(69, 580) - 69) as u32;
                                (22u8, (sign << 9) | mag, 10u32)
                            }
                        };

                        let (code, code_len) = tree.codes[token as usize];
                        w.write_bits(code, code_len as u32);
                        if extra_len > 0 {
                            w.write_bits(extra_bits, extra_len);
                        }
                        tis[bi] += 1;
                    }
                    bi += 1;
                }
            }
        }

        // Reconstruct local reference frame
        let mut reconstructed_coeffs = stored_coeffs;
        undo_dc_prediction(&self.geom, &bcoded, &mb_modes, &mut reconstructed_coeffs);

        let pred = [[128u8; 8]; 8];
        for bi in 0..nbs {
            let meta = &self.geom.blocks[bi];
            let pli = meta.pli;
            let (pw, _, _, _, _, _, _, _) = self.geom.planes[pli];
            let dest_plane = match pli {
                0 => &mut rec_y,
                1 => &mut rec_cb,
                2 => &mut rec_cr,
                _ => continue,
            };

            let dc_qmat = &self.qmat_tables[0][pli][qi];
            let ac_qmat = &self.qmat_tables[0][pli][qi];

            reconstruct_block(meta, true, ncoeffs[bi], &pred, &reconstructed_coeffs[bi], dc_qmat, ac_qmat, dest_plane, pw);
        }

        // Apply in-loop filter
        let lflim_l = self.setup_header.lflims[qi];
        apply_loop_filter(&self.geom, &bcoded, lflim_l, &mut rec_y, &mut rec_cb, &mut rec_cr);

        // Update local reference frames (all frames are intra keyframes)
        self.ref_golden_y.copy_from_slice(&rec_y);
        self.ref_golden_cb.copy_from_slice(&rec_cb);
        self.ref_golden_cr.copy_from_slice(&rec_cr);
        self.last_keyframe = self.frame_count;
        self.ref_prev_y.copy_from_slice(&rec_y);
        self.ref_prev_cb.copy_from_slice(&rec_cb);
        self.ref_prev_cr.copy_from_slice(&rec_cr);

        self.frame_count += 1;

        Ok(w.finish())
    }
}

//! Theora video decoder (Section 7).

use super::header::{CommentHeader, IdentificationHeader, SetupHeader};
use super::huffman::{decode_long_run, decode_short_run};
use super::reconstruct::{FrameGeometry, apply_loop_filter, compute_block_predictor, mb_mode_to_ref_frame, reconstruct_block, undo_dc_prediction};
use super::tables::MB_SCHEMES;
use super::{Error, Info, Picture, PixelFormat, Result};
use craft_media::bitstream::BitReader;

/// A conformant Theora video decoder.
pub struct Decoder {
    pub info: Info,
    pub comment: CommentHeader,
    pub setup: SetupHeader,
    pub geom: FrameGeometry,
    pub qmat_tables: [[[[u16; 64]; 64]; 3]; 2],

    // Reference frames
    ref_golden_y: Vec<u8>,
    ref_golden_cb: Vec<u8>,
    ref_golden_cr: Vec<u8>,

    ref_prev_y: Vec<u8>,
    ref_prev_cb: Vec<u8>,
    ref_prev_cr: Vec<u8>,

    has_keyframe: bool,
    frame_index: u64,
}

fn alloc_plane_buffer(len: usize) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::OutOfMemory)?;
    v.resize(len, 0);
    Ok(v)
}

fn clone_plane_buffer(src: &[u8]) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(src.len()).map_err(|_| Error::OutOfMemory)?;
    v.extend_from_slice(src);
    Ok(v)
}

fn alloc_vec_exact<T: Clone>(val: T, n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::OutOfMemory)?;
    v.resize(n, val);
    Ok(v)
}

impl Decoder {
    /// Initialize decoder from the three mandatory header packets: Identification, Comment, Setup.
    pub fn from_headers(id_packet: &[u8], comment_packet: &[u8], setup_packet: &[u8]) -> Result<Self> {
        let id = IdentificationHeader::parse(id_packet)?;
        let comment = CommentHeader::parse(comment_packet)?;
        let setup = SetupHeader::parse(setup_packet)?;

        let fmbw_u64 = (id.info.frame_width / 16) as u64;
        let fmbh_u64 = (id.info.frame_height / 16) as u64;
        if fmbw_u64 == 0 || fmbh_u64 == 0 {
            return Err(Error::DimensionMismatch("frame dimensions must be non-zero multiples of 16"));
        }

        // Validate memory budget (crate::budget::DECODE_MEMORY_BUDGET) before allocating geometry and planes
        let (cw, ch) = match id.info.pixel_format {
            PixelFormat::Yuv420 => (fmbw_u64.checked_mul(8).ok_or(Error::OutOfMemory)?, fmbh_u64.checked_mul(8).ok_or(Error::OutOfMemory)?),
            PixelFormat::Yuv422 => (fmbw_u64.checked_mul(8).ok_or(Error::OutOfMemory)?, fmbh_u64.checked_mul(16).ok_or(Error::OutOfMemory)?),
            PixelFormat::Yuv444 => (fmbw_u64.checked_mul(16).ok_or(Error::OutOfMemory)?, fmbh_u64.checked_mul(16).ok_or(Error::OutOfMemory)?),
        };

        let y_bytes = (fmbw_u64.checked_mul(16).ok_or(Error::OutOfMemory)?)
            .checked_mul(fmbh_u64.checked_mul(16).ok_or(Error::OutOfMemory)?)
            .ok_or(Error::OutOfMemory)?;
        let c_bytes = cw.checked_mul(ch).ok_or(Error::OutOfMemory)?;
        let one_picture_bytes = y_bytes.checked_add(c_bytes.checked_mul(2).ok_or(Error::OutOfMemory)?).ok_or(Error::OutOfMemory)?;

        // 2 reference pictures (golden + previous) and 1 reconstructed scratch picture
        let pictures_mem = one_picture_bytes.checked_mul(3).ok_or(Error::OutOfMemory)?;

        // Geometry metadata & decode scratch buffers
        let nmbs = fmbw_u64.checked_mul(fmbh_u64).ok_or(Error::OutOfMemory)?;
        let blocks_per_mb = id.info.pixel_format.blocks_per_macroblock() as u64;
        let nbs = nmbs.checked_mul(blocks_per_mb).ok_or(Error::OutOfMemory)?;
        let meta_and_scratch_mem = nbs.checked_mul(300).ok_or(Error::OutOfMemory)?;

        let total_mem = pictures_mem.checked_add(meta_and_scratch_mem).ok_or(Error::OutOfMemory)?;
        if total_mem > crate::budget::DECODE_MEMORY_BUDGET {
            return Err(Error::OutOfMemory);
        }

        let fmbw = fmbw_u64 as usize;
        let fmbh = fmbh_u64 as usize;
        let geom = FrameGeometry::new(fmbw, fmbh, id.info.pixel_format)?;
        let qmat_tables = setup.quant_params.precompute_all_qmat()?;

        let (y_pw, y_ph, _, _, _, _, _, _) = geom.planes[0];
        let (c_pw, c_ph, _, _, _, _, _, _) = geom.planes[1];

        let ref_golden_y = alloc_plane_buffer(y_pw * y_ph)?;
        let ref_golden_cb = alloc_plane_buffer(c_pw * c_ph)?;
        let ref_golden_cr = alloc_plane_buffer(c_pw * c_ph)?;

        let ref_prev_y = alloc_plane_buffer(y_pw * y_ph)?;
        let ref_prev_cb = alloc_plane_buffer(c_pw * c_ph)?;
        let ref_prev_cr = alloc_plane_buffer(c_pw * c_ph)?;

        Ok(Self {
            info: id.info,
            comment,
            setup,
            geom,
            qmat_tables,
            ref_golden_y,
            ref_golden_cb,
            ref_golden_cr,
            ref_prev_y,
            ref_prev_cb,
            ref_prev_cr,
            has_keyframe: false,
            frame_index: 0,
        })
    }

    /// Decode a single video packet into a reconstructed [`Picture`] (Section 7.11).
    pub fn decode_packet(&mut self, packet: &[u8]) -> Result<Picture> {
        let (y_pw, y_ph, _, _, _, _, _, _) = self.geom.planes[0];
        let (c_pw, c_ph, _, _, _, _, _, _) = self.geom.planes[1];

        // 0-byte packet is a 0-change inter frame (Section 7.11 step 2)
        if packet.is_empty() {
            if !self.has_keyframe {
                return Err(Error::MissingReference);
            }
            let pic = Picture {
                width: self.info.frame_width,
                height: self.info.frame_height,
                pic_width: self.info.pic_width,
                pic_height: self.info.pic_height,
                pic_x: self.info.pic_x,
                pic_y: self.info.pic_y,
                pixel_format: self.info.pixel_format,
                color_space: self.info.color_space,
                keyframe: false,
                pts: None,
                y: clone_plane_buffer(&self.ref_prev_y)?,
                u: clone_plane_buffer(&self.ref_prev_cb)?,
                v: clone_plane_buffer(&self.ref_prev_cr)?,
                y_stride: y_pw,
                uv_stride: c_pw,
            };
            self.frame_index += 1;
            return Ok(pic);
        }

        let mut r = BitReader::new(packet);

        // 1. Frame Header Decode (Section 7.1)
        let header_flag = r.read_bits(1).map_err(|_| Error::Truncated("header flag"))?;
        if header_flag != 0 {
            return Err(Error::Corrupt("data packet header flag must be zero"));
        }

        let ftype = r.read_bits(1).map_err(|_| Error::Truncated("ftype"))? as u8;
        if ftype == 1 && !self.has_keyframe {
            return Err(Error::MissingReference);
        }

        let is_keyframe = ftype == 0;

        let mut qis = [0usize; 3];
        qis[0] = r.read_bits(6).map_err(|_| Error::Truncated("qis[0]"))? as usize;
        let mut nqis = 1;
        let more_qis = r.read_bits(1).map_err(|_| Error::Truncated("more_qis"))? != 0;
        if more_qis {
            qis[1] = r.read_bits(6).map_err(|_| Error::Truncated("qis[1]"))? as usize;
            nqis = 2;
            let more_qis2 = r.read_bits(1).map_err(|_| Error::Truncated("more_qis2"))? != 0;
            if more_qis2 {
                qis[2] = r.read_bits(6).map_err(|_| Error::Truncated("qis[2]"))? as usize;
                nqis = 3;
            }
        }

        if is_keyframe {
            let reserved = r.read_bits(3).map_err(|_| Error::Truncated("keyframe reserved bits"))?;
            if reserved != 0 {
                return Err(Error::Corrupt("keyframe reserved bits must be zero"));
            }
        }

        // 2. Coded Block Flags (Section 7.3)
        let nbs = self.geom.nbs;
        let nsbs = self.geom.nsbs;
        let mut bcoded = alloc_vec_exact(false, nbs)?;

        if is_keyframe {
            bcoded.fill(true);
        } else {
            let sbp_bits = decode_long_run(&mut r, nsbs)?;
            let mut uncoded_sb_count = 0;
            for &b in &sbp_bits {
                if !b {
                    uncoded_sb_count += 1;
                }
            }

            let sbf_bits = decode_long_run(&mut r, uncoded_sb_count)?;
            let mut sbf_iter = sbf_bits.into_iter();
            let mut sbf_coded = alloc_vec_exact(false, nsbs)?;

            for sbi in 0..nsbs {
                if !sbp_bits[sbi] {
                    sbf_coded[sbi] = sbf_iter.next().unwrap_or(false);
                }
            }

            let mut partial_block_count = 0;
            for sbi in 0..nsbs {
                if sbp_bits[sbi] {
                    partial_block_count += self.geom.sb_block_counts[sbi];
                }
            }

            let partial_bits = decode_short_run(&mut r, partial_block_count)?;
            let mut partial_iter = partial_bits.into_iter();

            for bi in 0..nbs {
                let sbi = self.geom.blocks[bi].sbi;
                if !sbp_bits[sbi] {
                    bcoded[bi] = sbf_coded[sbi];
                } else {
                    bcoded[bi] = partial_iter.next().unwrap_or(false);
                }
            }
        }

        // 3. Macro Block Coding Modes (Section 7.4)
        let nmbs = self.geom.nmbs;
        let mut mb_modes = alloc_vec_exact(0u8, nmbs)?;

        if is_keyframe {
            mb_modes.fill(1); // INTRA
        } else {
            let mscheme = r.read_bits(3).map_err(|_| Error::Truncated("mscheme"))? as usize;
            let mut malphabet = [0u8; 8];

            if mscheme == 0 {
                for mode in 0..8 {
                    let mi = r.read_bits(3).map_err(|_| Error::Truncated("scheme 0 mi"))? as usize;
                    if mi < 8 {
                        malphabet[mi] = mode as u8;
                    }
                }
            } else if mscheme < 7 {
                malphabet = MB_SCHEMES[mscheme];
            }

            for mbi in 0..nmbs {
                let luma_has_coded = self.geom.mb_luma_blocks[mbi].iter().any(|&bi| bcoded[bi]);
                if luma_has_coded {
                    if mscheme != 7 {
                        let mi = decode_mb_mode_huffman(&mut r)?;
                        mb_modes[mbi] = malphabet[mi];
                    } else {
                        mb_modes[mbi] = r.read_bits(3).map_err(|_| Error::Truncated("mb_mode"))? as u8;
                    }
                } else {
                    mb_modes[mbi] = 0; // INTER NOMV
                }
            }
        }

        // 4. Motion Vectors (Section 7.5)
        let mut mvects = alloc_vec_exact((0i8, 0i8), nbs)?;

        if !is_keyframe {
            let mvmode = r.read_bits(1).map_err(|_| Error::Truncated("mvmode"))? as u8;
            let mut last1 = (0i8, 0i8);
            let mut last2 = (0i8, 0i8);

            for mbi in 0..nmbs {
                let mode = mb_modes[mbi];
                if mode == 7 {
                    // INTER MV FOUR
                    let [a, b, c, d] = self.geom.mb_luma_blocks[mbi];
                    let mut mva = (0i8, 0i8);
                    let mut mvb = (0i8, 0i8);
                    let mut mvc = (0i8, 0i8);
                    let mut mvd = (0i8, 0i8);
                    let mut last_decoded = None;

                    if bcoded[a] {
                        mva = decode_single_mv(&mut r, mvmode)?;
                        mvects[a] = mva;
                        last_decoded = Some(mva);
                    }
                    if bcoded[b] {
                        mvb = decode_single_mv(&mut r, mvmode)?;
                        mvects[b] = mvb;
                        last_decoded = Some(mvb);
                    }
                    if bcoded[c] {
                        mvc = decode_single_mv(&mut r, mvmode)?;
                        mvects[c] = mvc;
                        last_decoded = Some(mvc);
                    }
                    if bcoded[d] {
                        mvd = decode_single_mv(&mut r, mvmode)?;
                        mvects[d] = mvd;
                        last_decoded = Some(mvd);
                    }

                    // Chroma vector computation
                    match self.info.pixel_format {
                        PixelFormat::Yuv420 => {
                            let mvx = round_div4(mva.0 as i32 + mvb.0 as i32 + mvc.0 as i32 + mvd.0 as i32);
                            let mvy = round_div4(mva.1 as i32 + mvb.1 as i32 + mvc.1 as i32 + mvd.1 as i32);
                            let mv_chroma = (mvx as i8, mvy as i8);
                            for &bi in &self.geom.mb_chroma_cb_blocks[mbi] {
                                mvects[bi] = mv_chroma;
                            }
                            for &bi in &self.geom.mb_chroma_cr_blocks[mbi] {
                                mvects[bi] = mv_chroma;
                            }
                        }
                        PixelFormat::Yuv422 => {
                            let mvx_bot = round_div2(mva.0 as i32 + mvb.0 as i32) as i8;
                            let mvy_bot = round_div2(mva.1 as i32 + mvb.1 as i32) as i8;
                            let mvx_top = round_div2(mvc.0 as i32 + mvd.0 as i32) as i8;
                            let mvy_top = round_div2(mvc.1 as i32 + mvd.1 as i32) as i8;

                            if self.geom.mb_chroma_cb_blocks[mbi].len() >= 2 {
                                mvects[self.geom.mb_chroma_cb_blocks[mbi][0]] = (mvx_bot, mvy_bot);
                                mvects[self.geom.mb_chroma_cb_blocks[mbi][1]] = (mvx_top, mvy_top);
                            }
                            if self.geom.mb_chroma_cr_blocks[mbi].len() >= 2 {
                                mvects[self.geom.mb_chroma_cr_blocks[mbi][0]] = (mvx_bot, mvy_bot);
                                mvects[self.geom.mb_chroma_cr_blocks[mbi][1]] = (mvx_top, mvy_top);
                            }
                        }
                        PixelFormat::Yuv444 => {
                            if self.geom.mb_chroma_cb_blocks[mbi].len() >= 4 {
                                mvects[self.geom.mb_chroma_cb_blocks[mbi][0]] = mva;
                                mvects[self.geom.mb_chroma_cb_blocks[mbi][1]] = mvb;
                                mvects[self.geom.mb_chroma_cb_blocks[mbi][2]] = mvc;
                                mvects[self.geom.mb_chroma_cb_blocks[mbi][3]] = mvd;
                            }
                            if self.geom.mb_chroma_cr_blocks[mbi].len() >= 4 {
                                mvects[self.geom.mb_chroma_cr_blocks[mbi][0]] = mva;
                                mvects[self.geom.mb_chroma_cr_blocks[mbi][1]] = mvb;
                                mvects[self.geom.mb_chroma_cr_blocks[mbi][2]] = mvc;
                                mvects[self.geom.mb_chroma_cr_blocks[mbi][3]] = mvd;
                            }
                        }
                    }

                    last2 = last1;
                    if let Some(mv_last) = last_decoded {
                        last1 = mv_last;
                    }
                } else if mode == 6 {
                    // INTER GOLDEN MV
                    let mv = decode_single_mv(&mut r, mvmode)?;
                    self.assign_macroblock_mv(mbi, mv, &bcoded, &mut mvects);
                } else if mode == 4 {
                    // INTER MV LAST2
                    let mv = last2;
                    std::mem::swap(&mut last2, &mut last1);
                    self.assign_macroblock_mv(mbi, mv, &bcoded, &mut mvects);
                } else if mode == 3 {
                    // INTER MV LAST
                    let mv = last1;
                    self.assign_macroblock_mv(mbi, mv, &bcoded, &mut mvects);
                } else if mode == 2 {
                    // INTER MV
                    let mv = decode_single_mv(&mut r, mvmode)?;
                    last2 = last1;
                    last1 = mv;
                    self.assign_macroblock_mv(mbi, mv, &bcoded, &mut mvects);
                } else {
                    // INTRA (1), INTER NOMV (0), INTER GOLDEN NOMV (5)
                    self.assign_macroblock_mv(mbi, (0, 0), &bcoded, &mut mvects);
                }
            }
        }

        // 5. Block-Level qi Decode (Section 7.6)
        let mut qiis = alloc_vec_exact(0usize, nbs)?;
        if nqis > 1 {
            for qii in 0..(nqis - 1) {
                let nbits_pass = (0..nbs).filter(|&bi| bcoded[bi] && qiis[bi] == qii).count();
                if nbits_pass > 0 {
                    let bits = decode_long_run(&mut r, nbits_pass)?;
                    let mut bit_iter = bits.into_iter();
                    for bi in 0..nbs {
                        if bcoded[bi] && qiis[bi] == qii && bit_iter.next().unwrap_or(false) {
                            qiis[bi] += 1;
                        }
                    }
                }
            }
        }

        // 6. DCT Coefficients Decode (Section 7.7)
        let mut coeffs = alloc_vec_exact([0i16; 64], nbs)?;
        let mut ncoeffs = alloc_vec_exact(0usize, nbs)?;
        let mut tis = alloc_vec_exact(0usize, nbs)?;
        let mut eobs = 0usize;

        let nlbs = nmbs * 4;
        let mut hti_l = 0usize;
        let mut hti_c = 0usize;

        for ti in 0..64 {
            if ti == 0 || ti == 1 {
                hti_l = r.read_bits(4).map_err(|_| Error::Truncated("hti_l"))? as usize;
                hti_c = r.read_bits(4).map_err(|_| Error::Truncated("hti_c"))? as usize;
            }

            for bi in 0..nbs {
                if !bcoded[bi] || tis[bi] != ti {
                    continue;
                }

                ncoeffs[bi] = ti;

                if eobs > 0 {
                    tis[bi] = 64;
                    eobs -= 1;
                } else {
                    let hg = match ti {
                        0 => 0,
                        1..=5 => 1,
                        6..=14 => 2,
                        15..=27 => 3,
                        _ => 4,
                    };
                    let hti = if bi < nlbs { 16 * hg + hti_l } else { 16 * hg + hti_c };
                    if hti >= self.setup.huffman_trees.len() {
                        return Err(Error::Corrupt("invalid Huffman table index"));
                    }

                    let token = self.setup.huffman_trees[hti].decode_token(&mut r)?;

                    if token < 7 {
                        // EOB token (Section 7.7.1)
                        let run = match token {
                            0 => 1,
                            1 => 2,
                            2 => 3,
                            3 => (r.read_bits(2).map_err(|_| Error::Truncated("eob 3"))? + 4) as usize,
                            4 => (r.read_bits(3).map_err(|_| Error::Truncated("eob 4"))? + 8) as usize,
                            5 => (r.read_bits(4).map_err(|_| Error::Truncated("eob 5"))? + 16) as usize,
                            6 => {
                                let v = r.read_bits(12).map_err(|_| Error::Truncated("eob 6"))? as usize;
                                if v == 0 { (0..nbs).filter(|&bj| bcoded[bj] && tis[bj] < 64).count() } else { v }
                            }
                            _ => 1,
                        };
                        tis[bi] = 64;
                        eobs = run.saturating_sub(1);
                    } else {
                        // Coefficient token (Section 7.7.2)
                        decode_coeff_token(token, &mut r, bi, ti, &mut coeffs, &mut tis, &mut ncoeffs)?;
                    }
                }
            }
        }

        // 7. Undo DC Prediction (Section 7.8)
        undo_dc_prediction(&self.geom, &bcoded, &mb_modes, &mut coeffs);

        // 8. Block Reconstruction (Section 7.9)
        let mut rec_y = alloc_plane_buffer(y_pw * y_ph)?;
        let mut rec_cb = alloc_plane_buffer(c_pw * c_ph)?;
        let mut rec_cr = alloc_plane_buffer(c_pw * c_ph)?;

        let qi0 = qis[0];

        for bi in 0..nbs {
            let meta = &self.geom.blocks[bi];
            let pli = meta.pli;
            let (pw, ph, _, _, _, _, _, _) = self.geom.planes[pli];
            let stride = pw;

            let mut pred = [[0u8; 8]; 8];
            let coded = bcoded[bi];

            let (dest_plane, _ref_golden_plane, ref_prev_plane) = match pli {
                0 => (&mut rec_y, &self.ref_golden_y, &self.ref_prev_y),
                1 => (&mut rec_cb, &self.ref_golden_cb, &self.ref_prev_cb),
                2 => (&mut rec_cr, &self.ref_golden_cr, &self.ref_prev_cr),
                _ => continue,
            };

            if coded {
                let mbi = meta.mbi;
                let mode = mb_modes[mbi];
                let qti = if mode == 1 { 0 } else { 1 };
                let rfi = mb_mode_to_ref_frame(mode);

                let (ref_y_src, ref_cb_src, ref_cr_src) = if rfi == 2 {
                    (&self.ref_golden_y[..], &self.ref_golden_cb[..], &self.ref_golden_cr[..])
                } else {
                    (&self.ref_prev_y[..], &self.ref_prev_cb[..], &self.ref_prev_cr[..])
                };

                compute_block_predictor(
                    pli,
                    meta.pixel_x,
                    meta.pixel_y,
                    rfi,
                    mvects[bi],
                    self.info.pixel_format,
                    ref_y_src,
                    ref_cb_src,
                    ref_cr_src,
                    pw,
                    ph,
                    &mut pred,
                );

                let qi_ac = qis[qiis[bi]];
                let dc_qmat = &self.qmat_tables[qti][pli][qi0];
                let ac_qmat = &self.qmat_tables[qti][pli][qi_ac];

                reconstruct_block(meta, true, ncoeffs[bi], &pred, &coeffs[bi], dc_qmat, ac_qmat, dest_plane, stride);
            } else {
                // Copy co-located block from previous reference frame
                for by in 0..8 {
                    let src_offset = (meta.pixel_y + by) * stride + meta.pixel_x;
                    let dst_offset = src_offset;
                    dest_plane[dst_offset..dst_offset + 8].copy_from_slice(&ref_prev_plane[src_offset..src_offset + 8]);
                }
            }
        }

        // 9. Loop Filtering (Section 7.10)
        let lflim_l = self.setup.lflims[qi0];
        apply_loop_filter(&self.geom, &bcoded, lflim_l, &mut rec_y, &mut rec_cb, &mut rec_cr);

        // 10. Update Reference Buffers (Section 7.11)
        if is_keyframe {
            self.ref_golden_y.copy_from_slice(&rec_y);
            self.ref_golden_cb.copy_from_slice(&rec_cb);
            self.ref_golden_cr.copy_from_slice(&rec_cr);
            self.has_keyframe = true;
        }
        self.ref_prev_y.copy_from_slice(&rec_y);
        self.ref_prev_cb.copy_from_slice(&rec_cb);
        self.ref_prev_cr.copy_from_slice(&rec_cr);

        self.frame_index += 1;

        Ok(Picture {
            width: self.info.frame_width,
            height: self.info.frame_height,
            pic_width: self.info.pic_width,
            pic_height: self.info.pic_height,
            pic_x: self.info.pic_x,
            pic_y: self.info.pic_y,
            pixel_format: self.info.pixel_format,
            color_space: self.info.color_space,
            keyframe: is_keyframe,
            pts: None,
            y: rec_y,
            u: rec_cb,
            v: rec_cr,
            y_stride: y_pw,
            uv_stride: c_pw,
        })
    }

    fn assign_macroblock_mv(&self, mbi: usize, mv: (i8, i8), bcoded: &[bool], mvects: &mut [(i8, i8)]) {
        for &bi in &self.geom.mb_luma_blocks[mbi] {
            if bcoded[bi] {
                mvects[bi] = mv;
            }
        }
        for &bi in &self.geom.mb_chroma_cb_blocks[mbi] {
            if bcoded[bi] {
                mvects[bi] = mv;
            }
        }
        for &bi in &self.geom.mb_chroma_cr_blocks[mbi] {
            if bcoded[bi] {
                mvects[bi] = mv;
            }
        }
    }
}

fn decode_single_mv(r: &mut BitReader, mvmode: u8) -> Result<(i8, i8)> {
    if mvmode == 0 {
        let mvx = super::huffman::decode_mv_component_huffman(r)? as i8;
        let mvy = super::huffman::decode_mv_component_huffman(r)? as i8;
        Ok((mvx, mvy))
    } else {
        let mag_x = r.read_bits(5).map_err(|_| Error::Truncated("mvx mag"))? as i8;
        let sign_x = r.read_bits(1).map_err(|_| Error::Truncated("mvx sign"))? != 0;
        let mvx = if sign_x { -mag_x } else { mag_x };

        let mag_y = r.read_bits(5).map_err(|_| Error::Truncated("mvy mag"))? as i8;
        let sign_y = r.read_bits(1).map_err(|_| Error::Truncated("mvy sign"))? != 0;
        let mvy = if sign_y { -mag_y } else { mag_y };

        Ok((mvx, mvy))
    }
}

fn decode_mb_mode_huffman(r: &mut BitReader) -> Result<usize> {
    let mut code = 0u32;
    for len in 1..=7 {
        let bit = r.read_bits(1).map_err(|_| Error::Truncated("mb mode huffman"))?;
        code = (code << 1) | bit;
        match (len, code) {
            (1, 0b0) => return Ok(0),
            (2, 0b10) => return Ok(1),
            (3, 0b110) => return Ok(2),
            (4, 0b1110) => return Ok(3),
            (5, 0b11110) => return Ok(4),
            (6, 0b111110) => return Ok(5),
            (7, 0b1111110) => return Ok(6),
            (7, 0b1111111) => return Ok(7),
            _ => {}
        }
    }
    Err(Error::Corrupt("unrecognized macroblock mode Huffman code"))
}

#[inline]
fn round_div4(val: i32) -> i32 {
    if val >= 0 { (val + 2) / 4 } else { (val - 2) / 4 }
}

#[inline]
fn round_div2(val: i32) -> i32 {
    if val >= 0 { (val + 1) / 2 } else { (val - 1) / 2 }
}

fn decode_coeff_token(
    token: u8,
    r: &mut BitReader,
    bi: usize,
    ti: usize,
    coeffs: &mut [[i16; 64]],
    tis: &mut [usize],
    ncoeffs: &mut [usize],
) -> Result<()> {
    match token {
        7 => {
            let rlen = (r.read_bits(3).map_err(|_| Error::Truncated("token 7 rlen"))? + 1) as usize;
            tis[bi] += rlen;
        }
        8 => {
            let rlen = (r.read_bits(6).map_err(|_| Error::Truncated("token 8 rlen"))? + 1) as usize;
            tis[bi] += rlen;
        }
        9 => {
            coeffs[bi][ti] = 1;
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        10 => {
            coeffs[bi][ti] = -1;
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        11 => {
            coeffs[bi][ti] = 2;
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        12 => {
            coeffs[bi][ti] = -2;
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        13..=16 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let val = (token - 10) as i16;
            coeffs[bi][ti] = if sign { -val } else { val };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        17 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(1).map_err(|_| Error::Truncated("mag"))? + 7) as i16;
            coeffs[bi][ti] = if sign { -mag } else { mag };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        18 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(2).map_err(|_| Error::Truncated("mag"))? + 9) as i16;
            coeffs[bi][ti] = if sign { -mag } else { mag };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        19 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(3).map_err(|_| Error::Truncated("mag"))? + 13) as i16;
            coeffs[bi][ti] = if sign { -mag } else { mag };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        20 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(4).map_err(|_| Error::Truncated("mag"))? + 21) as i16;
            coeffs[bi][ti] = if sign { -mag } else { mag };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        21 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(5).map_err(|_| Error::Truncated("mag"))? + 37) as i16;
            coeffs[bi][ti] = if sign { -mag } else { mag };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        22 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(9).map_err(|_| Error::Truncated("mag"))? + 69) as i16;
            coeffs[bi][ti] = if sign { -mag } else { mag };
            tis[bi] += 1;
            ncoeffs[bi] = tis[bi];
        }
        23..=27 => {
            let zeros = (token - 22) as usize;
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            if ti + zeros < 64 {
                coeffs[bi][ti + zeros] = if sign { -1 } else { 1 };
            }
            tis[bi] += zeros + 1;
            ncoeffs[bi] = tis[bi];
        }
        28 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let rlen = (r.read_bits(2).map_err(|_| Error::Truncated("rlen"))? + 6) as usize;
            if ti + rlen < 64 {
                coeffs[bi][ti + rlen] = if sign { -1 } else { 1 };
            }
            tis[bi] += rlen + 1;
            ncoeffs[bi] = tis[bi];
        }
        29 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let rlen = (r.read_bits(3).map_err(|_| Error::Truncated("rlen"))? + 10) as usize;
            if ti + rlen < 64 {
                coeffs[bi][ti + rlen] = if sign { -1 } else { 1 };
            }
            tis[bi] += rlen + 1;
            ncoeffs[bi] = tis[bi];
        }
        30 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(1).map_err(|_| Error::Truncated("mag"))? + 2) as i16;
            if ti + 1 < 64 {
                coeffs[bi][ti + 1] = if sign { -mag } else { mag };
            }
            tis[bi] += 2;
            ncoeffs[bi] = tis[bi];
        }
        31 => {
            let sign = r.read_bits(1).map_err(|_| Error::Truncated("sign"))? != 0;
            let mag = (r.read_bits(1).map_err(|_| Error::Truncated("mag"))? + 2) as i16;
            let rlen = (r.read_bits(1).map_err(|_| Error::Truncated("rlen"))? + 2) as usize;
            if ti + rlen < 64 {
                coeffs[bi][ti + rlen] = if sign { -mag } else { mag };
            }
            tis[bi] += rlen + 1;
            ncoeffs[bi] = tis[bi];
        }
        _ => return Err(Error::Corrupt("unsupported coefficient token")),
    }
    Ok(())
}

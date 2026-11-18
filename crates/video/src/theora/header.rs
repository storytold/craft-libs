//! Theora bitstream headers (Identification, Comment, Setup; Section 6).

use super::huffman::HuffmanTree;
use super::quant::QuantParams;
use super::tables::{DEFAULT_ACSCALE, DEFAULT_BMS, DEFAULT_DCSCALE, DEFAULT_HUFFMAN_SETUP_BYTES, DEFAULT_LFLIMS};
use super::{ColorSpace, Error, Info, PixelFormat, Result};
use craft_media::bitstream::{BitReader, BitWriter};

/// Identification header packet (`0x80theora`, 42 bytes, Section 6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentificationHeader {
    pub info: Info,
}

impl IdentificationHeader {
    /// Parse identification header packet bytes (Section 6.2).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 42 {
            return Err(Error::Truncated("identification header"));
        }
        if data[0] != 0x80 || &data[1..7] != b"theora" {
            return Err(Error::InvalidHeader("invalid identification header magic"));
        }

        let vmaj = data[7];
        let vmin = data[8];
        let vrev = data[9];
        if vmaj != 3 || vmin != 2 {
            return Err(Error::UnsupportedVersion(vmaj, vmin, vrev));
        }

        let fmbw = u16::from_be_bytes([data[10], data[11]]) as u32;
        let fmbh = u16::from_be_bytes([data[12], data[13]]) as u32;
        if fmbw == 0 || fmbh == 0 {
            return Err(Error::InvalidHeader("frame macroblock dimensions cannot be zero"));
        }

        let frame_width = fmbw * 16;
        let frame_height = fmbh * 16;

        let picw = u32::from_be_bytes([0, data[14], data[15], data[16]]);
        let pich = u32::from_be_bytes([0, data[17], data[18], data[19]]);
        let picx = data[20] as u32;
        let picy = data[21] as u32;

        if picw > frame_width || pich > frame_height {
            return Err(Error::InvalidHeader("picture dimensions exceed frame dimensions"));
        }
        if picx > frame_width.saturating_sub(picw) || picy > frame_height.saturating_sub(pich) {
            return Err(Error::InvalidHeader("picture offset out of frame bounds"));
        }

        let frn = u32::from_be_bytes([data[22], data[23], data[24], data[25]]);
        let frd = u32::from_be_bytes([data[26], data[27], data[28], data[29]]);
        if frn == 0 || frd == 0 {
            return Err(Error::InvalidHeader("framerate cannot be zero"));
        }

        let parn = u32::from_be_bytes([0, data[30], data[31], data[32]]);
        let pard = u32::from_be_bytes([0, data[33], data[34], data[35]]);

        let cs_byte = data[36];
        let color_space = match cs_byte {
            0 => ColorSpace::Undefined,
            1 => ColorSpace::Rec470M,
            2 => ColorSpace::Rec470Bg,
            _ => ColorSpace::Undefined,
        };

        let nombr = u32::from_be_bytes([0, data[37], data[38], data[39]]);

        let last_two = u16::from_be_bytes([data[40], data[41]]);
        let quality = ((last_two >> 10) & 0x3F) as u8;
        let kfgshift = ((last_two >> 5) & 0x1F) as u8;
        let pf_bits = ((last_two >> 3) & 0x03) as u8;
        let reserved = last_two & 0x07;

        if reserved != 0 {
            return Err(Error::InvalidHeader("reserved bits in identification header are non-zero"));
        }

        let pixel_format = match pf_bits {
            0 => PixelFormat::Yuv420,
            2 => PixelFormat::Yuv422,
            3 => PixelFormat::Yuv444,
            _ => return Err(Error::UnsupportedPixelFormat(pf_bits)),
        };

        Ok(Self {
            info: Info {
                version_major: vmaj,
                version_minor: vmin,
                version_revision: vrev,
                frame_width,
                frame_height,
                pic_width: picw,
                pic_height: pich,
                pic_x: picx,
                pic_y: picy,
                fps_num: frn,
                fps_den: frd,
                par_num: parn,
                par_den: pard,
                color_space,
                pixel_format,
                nominal_bitrate: nombr,
                quality,
                keyframe_granule_shift: kfgshift,
            },
        })
    }

    /// Serialize identification header into exactly 42 packet bytes (Section 6.2).
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(42);
        buf.push(0x80);
        buf.extend_from_slice(b"theora");

        buf.push(self.info.version_major);
        buf.push(self.info.version_minor);
        buf.push(self.info.version_revision);

        let fmbw = (self.info.frame_width / 16) as u16;
        let fmbh = (self.info.frame_height / 16) as u16;
        buf.extend_from_slice(&fmbw.to_be_bytes());
        buf.extend_from_slice(&fmbh.to_be_bytes());

        let picw_bytes = self.info.pic_width.to_be_bytes();
        buf.extend_from_slice(&picw_bytes[1..4]);
        let pich_bytes = self.info.pic_height.to_be_bytes();
        buf.extend_from_slice(&pich_bytes[1..4]);

        buf.push(self.info.pic_x as u8);
        buf.push(self.info.pic_y as u8);

        buf.extend_from_slice(&self.info.fps_num.to_be_bytes());
        buf.extend_from_slice(&self.info.fps_den.to_be_bytes());

        let parn_bytes = self.info.par_num.to_be_bytes();
        buf.extend_from_slice(&parn_bytes[1..4]);
        let pard_bytes = self.info.par_den.to_be_bytes();
        buf.extend_from_slice(&pard_bytes[1..4]);

        buf.push(self.info.color_space as u8);

        let nombr_bytes = self.info.nominal_bitrate.to_be_bytes();
        buf.extend_from_slice(&nombr_bytes[1..4]);

        let pf_bits = match self.info.pixel_format {
            PixelFormat::Yuv420 => 0u16,
            PixelFormat::Yuv422 => 2u16,
            PixelFormat::Yuv444 => 3u16,
        };

        let last_two: u16 = ((self.info.quality as u16 & 0x3F) << 10) | ((self.info.keyframe_granule_shift as u16 & 0x1F) << 5) | (pf_bits << 3);
        buf.extend_from_slice(&last_two.to_be_bytes());

        buf
    }
}

/// Comment header packet (`0x81theora`, Section 6.3).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CommentHeader {
    /// Vendor identification string.
    pub vendor: String,
    /// List of user comments (typically `KEY=VALUE`).
    pub comments: Vec<String>,
}

impl CommentHeader {
    /// Parse comment header packet bytes (Section 6.3).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 7 {
            return Err(Error::Truncated("comment header"));
        }
        if data[0] != 0x81 || &data[1..7] != b"theora" {
            return Err(Error::InvalidHeader("invalid comment header magic"));
        }

        let mut pos = 7;
        if data.len() < pos + 4 {
            return Err(Error::Truncated("vendor string length"));
        }
        let vendor_len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let vendor_end = pos.checked_add(vendor_len).ok_or(Error::Truncated("vendor string"))?;
        if data.len() < vendor_end {
            return Err(Error::Truncated("vendor string"));
        }
        let vendor = String::from_utf8_lossy(&data[pos..vendor_end]).into_owned();
        pos = vendor_end;

        if data.len() < pos + 4 {
            return Err(Error::Truncated("comment count"));
        }
        let ncomments = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        // Bounded allocation protection
        if ncomments > 10_000 {
            return Err(Error::Corrupt("excessive comment count in header"));
        }

        let mut comments = Vec::with_capacity(ncomments);
        for _ in 0..ncomments {
            if data.len() < pos + 4 {
                return Err(Error::Truncated("user comment length"));
            }
            let len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
            pos += 4;
            let comment_end = pos.checked_add(len).ok_or(Error::Truncated("user comment string"))?;
            if data.len() < comment_end {
                return Err(Error::Truncated("user comment string"));
            }
            let s = String::from_utf8_lossy(&data[pos..comment_end]).into_owned();
            pos = comment_end;
            comments.push(s);
        }

        Ok(Self { vendor, comments })
    }

    /// Serialize comment header packet bytes (Section 6.3).
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x81);
        buf.extend_from_slice(b"theora");

        let vendor_bytes = self.vendor.as_bytes();
        buf.extend_from_slice(&(vendor_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(vendor_bytes);

        buf.extend_from_slice(&(self.comments.len() as u32).to_le_bytes());
        for c in &self.comments {
            let c_bytes = c.as_bytes();
            buf.extend_from_slice(&(c_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(c_bytes);
        }

        buf
    }
}

/// Setup header packet (`0x82theora`, Section 6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupHeader {
    /// 64 loop filter limit values.
    pub lflims: [u8; 64],
    /// Quantization parameters.
    pub quant_params: QuantParams,
    /// 80 Huffman trees for DCT tokens.
    pub huffman_trees: Vec<HuffmanTree>,
}

impl SetupHeader {
    /// Create a setup header initialized with the default VP3.1 tables (Appendix B).
    pub fn default_vp3() -> Result<Self> {
        let mut bit_r = BitReader::new(DEFAULT_HUFFMAN_SETUP_BYTES);
        let mut trees = Vec::with_capacity(80);
        for _ in 0..80 {
            trees.push(HuffmanTree::parse(&mut bit_r)?);
        }

        let mut nqrs = [[0usize; 3]; 2];
        let mut qr_sizes = [[[0usize; 63]; 3]; 2];
        let mut qr_bmis = [[[0usize; 64]; 3]; 2];

        // VP3 default ranges
        nqrs[0][0] = 1;
        qr_sizes[0][0][0] = 63;
        qr_bmis[0][0][0] = 0;
        qr_bmis[0][0][1] = 0;
        nqrs[0][1] = 1;
        qr_sizes[0][1][0] = 63;
        qr_bmis[0][1][0] = 1;
        qr_bmis[0][1][1] = 1;
        nqrs[0][2] = 1;
        qr_sizes[0][2][0] = 63;
        qr_bmis[0][2][0] = 1;
        qr_bmis[0][2][1] = 1;
        nqrs[1][0] = 1;
        qr_sizes[1][0][0] = 63;
        qr_bmis[1][0][0] = 2;
        qr_bmis[1][0][1] = 2;
        nqrs[1][1] = 1;
        qr_sizes[1][1][0] = 63;
        qr_bmis[1][1][0] = 2;
        qr_bmis[1][1][1] = 2;
        nqrs[1][2] = 1;
        qr_sizes[1][2][0] = 63;
        qr_bmis[1][2][0] = 2;
        qr_bmis[1][2][1] = 2;

        Ok(Self {
            lflims: DEFAULT_LFLIMS,
            quant_params: QuantParams {
                ac_scale: DEFAULT_ACSCALE,
                dc_scale: DEFAULT_DCSCALE,
                base_matrices: DEFAULT_BMS.to_vec(),
                nqrs,
                qr_sizes,
                qr_bmis,
            },
            huffman_trees: trees,
        })
    }

    /// Parse setup header packet bytes (Section 6.4).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 7 {
            return Err(Error::Truncated("setup header"));
        }
        if data[0] != 0x82 || &data[1..7] != b"theora" {
            return Err(Error::InvalidHeader("invalid setup header magic"));
        }

        let mut r = BitReader::new(&data[7..]);

        // 1. Loop Filter Limit Table (Section 6.4.1)
        let nbits = r.read_bits(3).map_err(|_| Error::Truncated("lflim nbits"))? as usize;
        let mut lflims = [0u8; 64];
        if nbits > 0 {
            for qi in 0..64 {
                lflims[qi] = r.read_bits(nbits as u32).map_err(|_| Error::Truncated("lflim table"))? as u8;
            }
        }

        // 2. Quantization Parameters (Section 6.4.2)
        let ac_nbits = (r.read_bits(4).map_err(|_| Error::Truncated("acscale nbits"))? + 1) as usize;
        let mut ac_scale = [0u16; 64];
        for qi in 0..64 {
            ac_scale[qi] = r.read_bits(ac_nbits as u32).map_err(|_| Error::Truncated("acscale table"))? as u16;
        }

        let dc_nbits = (r.read_bits(4).map_err(|_| Error::Truncated("dcscale nbits"))? + 1) as usize;
        let mut dc_scale = [0u16; 64];
        for qi in 0..64 {
            dc_scale[qi] = r.read_bits(dc_nbits as u32).map_err(|_| Error::Truncated("dcscale table"))? as u16;
        }

        let nbms = (r.read_bits(9).map_err(|_| Error::Truncated("nbms"))? + 1) as usize;
        if nbms > 384 {
            return Err(Error::Corrupt("nbms exceeds 384"));
        }

        let mut base_matrices = Vec::with_capacity(nbms);
        for _ in 0..nbms {
            let mut bm = [0u8; 64];
            for ci in 0..64 {
                bm[ci] = r.read_bits(8).map_err(|_| Error::Truncated("base matrix"))? as u8;
            }
            base_matrices.push(bm);
        }

        let mut nqrs = [[0usize; 3]; 2];
        let mut qr_sizes = [[[0usize; 63]; 3]; 2];
        let mut qr_bmis = [[[0usize; 64]; 3]; 2];

        for qti in 0..2 {
            for pli in 0..3 {
                let newqr = if qti > 0 || pli > 0 { r.read_bits(1).map_err(|_| Error::Truncated("newqr flag"))? != 0 } else { true };

                if !newqr {
                    let rpqr = if qti > 0 { r.read_bits(1).map_err(|_| Error::Truncated("rpqr flag"))? != 0 } else { false };

                    let (qtj, plj) = if rpqr { (qti - 1, pli) } else { ((3 * qti + pli - 1) / 3, (pli + 2) % 3) };

                    nqrs[qti][pli] = nqrs[qtj][plj];
                    qr_sizes[qti][pli] = qr_sizes[qtj][plj];
                    qr_bmis[qti][pli] = qr_bmis[qtj][plj];
                } else {
                    let mut qri = 0;
                    let mut qi = 0;
                    let nbms_ilog = ilog(nbms.saturating_sub(1) as u32) as u32;

                    let bmi = r.read_bits(nbms_ilog).map_err(|_| Error::Truncated("qrbmi initial"))? as usize;
                    if bmi >= nbms {
                        return Err(Error::Corrupt("initial bmi >= nbms"));
                    }
                    qr_bmis[qti][pli][0] = bmi;

                    while qi < 63 {
                        let size_ilog = ilog((62 - qi) as u32) as u32;
                        let size = (r.read_bits(size_ilog).map_err(|_| Error::Truncated("qrsize"))? + 1) as usize;
                        qr_sizes[qti][pli][qri] = size;
                        qi += size;
                        qri += 1;

                        let next_bmi = r.read_bits(nbms_ilog).map_err(|_| Error::Truncated("qrbmi loop"))? as usize;
                        if next_bmi >= nbms {
                            return Err(Error::Corrupt("next bmi >= nbms"));
                        }
                        qr_bmis[qti][pli][qri] = next_bmi;
                    }

                    if qi > 63 {
                        return Err(Error::Corrupt("quant range sum exceeded 63"));
                    }

                    nqrs[qti][pli] = qri;
                }
            }
        }

        // 3. DCT Token Huffman Tables (Section 6.4.4)
        let mut huffman_trees = Vec::with_capacity(80);
        for _ in 0..80 {
            huffman_trees.push(HuffmanTree::parse(&mut r)?);
        }

        Ok(Self { lflims, quant_params: QuantParams { ac_scale, dc_scale, base_matrices, nqrs, qr_sizes, qr_bmis }, huffman_trees })
    }

    /// Serialize setup header into packet bytes (Section 6.4).
    pub fn serialize(&self) -> Vec<u8> {
        let mut w = BitWriter::new();

        // 1. Loop Filter Limit Table (Section 6.4.1)
        w.write_bits(7, 3); // 7 bits per entry
        for &lim in &self.lflims {
            w.write_bits(lim as u32, 7);
        }

        // 2. Quantization Parameters (Section 6.4.2)
        // Find max bits required for ac_scale and dc_scale
        let ac_max = self.quant_params.ac_scale.iter().copied().max().unwrap_or(0);
        let ac_nbits = ilog(ac_max as u32).clamp(1, 16);
        w.write_bits((ac_nbits - 1) as u32, 4);
        for &s in &self.quant_params.ac_scale {
            w.write_bits(s as u32, ac_nbits as u32);
        }

        let dc_max = self.quant_params.dc_scale.iter().copied().max().unwrap_or(0);
        let dc_nbits = ilog(dc_max as u32).clamp(1, 16);
        w.write_bits((dc_nbits - 1) as u32, 4);
        for &s in &self.quant_params.dc_scale {
            w.write_bits(s as u32, dc_nbits as u32);
        }

        let nbms = self.quant_params.base_matrices.len();
        w.write_bits((nbms - 1) as u32, 9);
        for bm in &self.quant_params.base_matrices {
            for &b in bm {
                w.write_bits(b as u32, 8);
            }
        }

        // Quant ranges
        let nbms_ilog = ilog(nbms.saturating_sub(1) as u32);
        for qti in 0..2 {
            for pli in 0..3 {
                if qti > 0 || pli > 0 {
                    w.write_bit(true); // Always write NEWQR = 1 for simplicity and robustness
                }
                let nqr = self.quant_params.nqrs[qti][pli];
                let mut qi = 0;
                w.write_bits(self.quant_params.qr_bmis[qti][pli][0] as u32, nbms_ilog as u32);
                for qri in 0..nqr {
                    let size = self.quant_params.qr_sizes[qti][pli][qri];
                    let size_ilog = ilog((62 - qi) as u32);
                    w.write_bits((size - 1) as u32, size_ilog as u32);
                    qi += size;
                    w.write_bits(self.quant_params.qr_bmis[qti][pli][qri + 1] as u32, nbms_ilog as u32);
                }
            }
        }

        // 3. DCT Token Huffman Tables (Section 6.4.4)
        for tree in &self.huffman_trees {
            Self::write_tree_node(&tree.root, &mut w);
        }

        let payload = w.finish();
        let mut buf = Vec::with_capacity(7 + payload.len());
        buf.push(0x82);
        buf.extend_from_slice(b"theora");
        buf.extend_from_slice(&payload);
        buf
    }

    fn write_tree_node(node: &super::huffman::HuffmanNode, w: &mut BitWriter) {
        match node {
            super::huffman::HuffmanNode::Leaf(token) => {
                w.write_bit(true);
                w.write_bits(*token as u32, 5);
            }
            super::huffman::HuffmanNode::Branch(left, right) => {
                w.write_bit(false);
                Self::write_tree_node(left, w);
                Self::write_tree_node(right, w);
            }
        }
    }
}

/// Compute `ilog(v)` as defined in the Theora specification notation:
/// ilog(v) is the minimum number of bits required to store a positive integer v in two's complement,
/// or 0 for non-positive integers.
#[inline]
pub fn ilog(v: u32) -> usize {
    if v == 0 { 0 } else { (32 - v.leading_zeros()) as usize }
}

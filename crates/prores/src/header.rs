//! Frame and picture headers (RDD 36 §5).

use crate::{AlphaType, ChromaFormat, ColorInfo, Error, Interlace, Result};
use filmcraft_bitstream::BitReader;

/// `'icpf'`: the frame identifier following the 32-bit frame size.
pub const FRAME_ID: [u8; 4] = *b"icpf";

/// A parsed ProRes frame header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    /// Size of the frame header in bytes (from its own first field).
    pub header_size: u16,
    pub bitstream_version: u8,
    /// Four-character encoder identifier (e.g. `apl0`, `Lavc`, `fmcr`).
    pub encoder_id: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub chroma: ChromaFormat,
    pub interlace: Interlace,
    /// `aspect_ratio_information` (0 = unknown/square, 1 = 1:1, 2 = 4:3, 3 = 16:9).
    pub aspect_ratio: u8,
    /// `frame_rate_code` (0 = unknown).
    pub frame_rate_code: u8,
    pub color: ColorInfo,
    pub alpha: AlphaType,
    /// Luma quantisation matrix in raster order (all 4 when not transmitted).
    pub qmat_luma: [u8; 64],
    /// Chroma quantisation matrix in raster order (the luma matrix when not transmitted).
    pub qmat_chroma: [u8; 64],
    pub luma_matrix_present: bool,
    pub chroma_matrix_present: bool,
}

impl FrameHeader {
    /// Parse the 8-byte frame prefix and frame header. Returns the header, the byte offset of the
    /// first picture, and the frame size (bytes of `data` that belong to this frame).
    pub fn parse(data: &[u8]) -> Result<(FrameHeader, usize, usize)> {
        if data.len() < 8 + 20 {
            return Err(Error::Truncated);
        }
        let frame_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data[4..8] != FRAME_ID {
            return Err(Error::Invalid("missing 'icpf' frame identifier"));
        }
        if frame_size < 28 || frame_size > data.len() {
            return Err(Error::Truncated);
        }
        let data = &data[..frame_size];
        let h = &data[8..];
        let mut r = BitReader::new(h);
        let rd = |r: &mut BitReader, n: u32| r.read_bits(n).map_err(|_| Error::Truncated);
        let header_size = rd(&mut r, 16)? as u16;
        if (header_size as usize) < 20 || 8 + header_size as usize > data.len() {
            return Err(Error::Invalid("bad frame header size"));
        }
        let _reserved = rd(&mut r, 8)?;
        let bitstream_version = rd(&mut r, 8)? as u8;
        let encoder_id = [h[4], h[5], h[6], h[7]];
        r.skip(32).map_err(|_| Error::Truncated)?;
        let width = rd(&mut r, 16)? as u16;
        let height = rd(&mut r, 16)? as u16;
        let chroma = match rd(&mut r, 2)? {
            2 => ChromaFormat::Yuv422,
            3 => ChromaFormat::Yuv444,
            _ => return Err(Error::Unsupported("chroma format")),
        };
        let _ = rd(&mut r, 2)?;
        let interlace = match rd(&mut r, 2)? {
            0 => Interlace::Progressive,
            1 => Interlace::TopFieldFirst,
            2 => Interlace::BottomFieldFirst,
            _ => return Err(Error::Invalid("interlace mode")),
        };
        let _ = rd(&mut r, 2)?;
        let aspect_ratio = rd(&mut r, 4)? as u8;
        let frame_rate_code = rd(&mut r, 4)? as u8;
        let color = ColorInfo { primaries: rd(&mut r, 8)? as u8, transfer: rd(&mut r, 8)? as u8, matrix: rd(&mut r, 8)? as u8 };
        let _ = rd(&mut r, 4)?;
        let alpha = match rd(&mut r, 4)? {
            0 => AlphaType::None,
            1 => AlphaType::Bits8,
            2 => AlphaType::Bits16,
            _ => return Err(Error::Unsupported("alpha channel type")),
        };
        let _ = rd(&mut r, 14)?;
        let luma_matrix_present = rd(&mut r, 1)? == 1;
        let chroma_matrix_present = rd(&mut r, 1)? == 1;
        let mut pos = 20usize;
        let mut qmat_luma = [4u8; 64];
        if luma_matrix_present {
            let m = h.get(pos..pos + 64).ok_or(Error::Truncated)?;
            qmat_luma.copy_from_slice(m);
            pos += 64;
        }
        let mut qmat_chroma = qmat_luma;
        if chroma_matrix_present {
            let m = h.get(pos..pos + 64).ok_or(Error::Truncated)?;
            qmat_chroma.copy_from_slice(m);
            pos += 64;
        }
        if pos > header_size as usize {
            return Err(Error::Invalid("quantisation matrices exceed frame header"));
        }
        if qmat_luma.contains(&0) || qmat_chroma.contains(&0) {
            return Err(Error::Invalid("zero quantisation matrix entry"));
        }
        if width == 0 || height == 0 {
            return Err(Error::Invalid("zero frame dimension"));
        }
        let hdr = FrameHeader {
            header_size,
            bitstream_version,
            encoder_id,
            width,
            height,
            chroma,
            interlace,
            aspect_ratio,
            frame_rate_code,
            color,
            alpha,
            qmat_luma,
            qmat_chroma,
            luma_matrix_present,
            chroma_matrix_present,
        };
        Ok((hdr, 8 + header_size as usize, frame_size))
    }

    /// Serialise the frame header (without the 8-byte size/identifier prefix).
    pub fn write(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(148);
        let size = 20 + 64 * (self.luma_matrix_present as usize + self.chroma_matrix_present as usize);
        v.extend_from_slice(&(size as u16).to_be_bytes());
        v.push(0);
        v.push(self.bitstream_version);
        v.extend_from_slice(&self.encoder_id);
        v.extend_from_slice(&self.width.to_be_bytes());
        v.extend_from_slice(&self.height.to_be_bytes());
        let cf = match self.chroma {
            ChromaFormat::Yuv422 => 2u8,
            ChromaFormat::Yuv444 => 3,
        };
        let il = match self.interlace {
            Interlace::Progressive => 0u8,
            Interlace::TopFieldFirst => 1,
            Interlace::BottomFieldFirst => 2,
        };
        v.push((cf << 6) | (il << 2));
        v.push((self.aspect_ratio << 4) | (self.frame_rate_code & 15));
        v.push(self.color.primaries);
        v.push(self.color.transfer);
        v.push(self.color.matrix);
        v.push(match self.alpha {
            AlphaType::None => 0,
            AlphaType::Bits8 => 1,
            AlphaType::Bits16 => 2,
        });
        v.push(0);
        v.push(((self.luma_matrix_present as u8) << 1) | self.chroma_matrix_present as u8);
        if self.luma_matrix_present {
            v.extend_from_slice(&self.qmat_luma);
        }
        if self.chroma_matrix_present {
            v.extend_from_slice(&self.qmat_chroma);
        }
        v
    }
}

/// A parsed picture header (one per progressive frame, two per interlaced frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureHeader {
    pub header_size: usize,
    /// Total picture size in bytes including this header, the slice table and the slices.
    pub picture_size: usize,
    pub num_slices: usize,
    pub log2_slice_mbs: u8,
}

impl PictureHeader {
    pub fn parse(data: &[u8]) -> Result<PictureHeader> {
        if data.len() < 8 {
            return Err(Error::Truncated);
        }
        let header_size = (data[0] >> 3) as usize;
        if header_size < 8 || header_size > data.len() {
            return Err(Error::Invalid("bad picture header size"));
        }
        let picture_size = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
        let num_slices = u16::from_be_bytes([data[5], data[6]]) as usize;
        let log2_slice_mbs = (data[7] >> 4) & 3;
        Ok(PictureHeader { header_size, picture_size, num_slices, log2_slice_mbs })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(8 << 3);
        out.extend_from_slice(&(self.picture_size as u32).to_be_bytes());
        out.extend_from_slice(&(self.num_slices as u16).to_be_bytes());
        out.push((self.log2_slice_mbs & 3) << 4);
    }
}

/// Slice widths (in macroblocks) of one macroblock row: as many `1 << log2` slices as fit, then
/// the remainder split into decreasing powers of two.
pub fn row_slice_widths(mb_width: usize, log2_slice_mbs: u8) -> Vec<usize> {
    let size = 1usize << log2_slice_mbs;
    let mut v = vec![size; mb_width / size];
    let mut rem = mb_width % size;
    let mut s = size >> 1;
    while rem > 0 {
        if rem >= s {
            v.push(s);
            rem -= s;
        }
        s >>= 1;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_widths() {
        assert_eq!(row_slice_widths(120, 3), vec![8; 15]);
        assert_eq!(row_slice_widths(45, 3), vec![8, 8, 8, 8, 8, 4, 1]);
        assert_eq!(row_slice_widths(7, 3), vec![4, 2, 1]);
        assert_eq!(row_slice_widths(2, 3), vec![2]);
        assert_eq!(row_slice_widths(5, 0), vec![1; 5]);
    }
}

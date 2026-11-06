//! Coding-unit header (SMPTE ST 2019-1 §7.2): parse and write.

use crate::tables::{Kind, cid_info, ri_frame_size};
use crate::{ChromaFormat, ColorVolume, Error, Result};

/// Offset of the macroblock scan indices payload.
pub const SCAN_INDEX_OFFSET: usize = 0x170;
/// EOF signature when the CRC flag is clear (§7.4, Figure 39).
pub const EOF_SIGNATURE: [u8; 4] = [0x60, 0x0D, 0xC0, 0xDE];

/// Field/frame code of a coding unit (`FFC`, §7.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldCode {
    /// A progressive frame, or an interlaced frame coded as one unit.
    Frame,
    /// Field 1 (the top field) of a field-coded interlaced frame.
    Field1,
    /// Field 2 (the bottom field).
    Field2,
}

/// A parsed coding-unit header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    /// Header size `HS` in bytes (the compressed payload starts here).
    pub header_size: u32,
    /// Header version number `HVN` (1, 2 = HD profile; 3 = RI profile; 4 = HD with alpha).
    pub version: u8,
    pub cid: u32,
    /// Variable bitrate (no payload padding).
    pub vbr: bool,
    pub field: FieldCode,
    /// Macroblock-adaptive field/frame coding (CID 1260).
    pub macf: bool,
    /// The EOF signature holds a CRC-32 instead of `0x600DC0DE`.
    pub crc: bool,
    pub alpha: bool,
    /// Alpha is coded losslessly (differential RLE) instead of with the DCT.
    pub lossless_alpha: bool,
    pub premultiplied_alpha: bool,
    /// Samples per line (`SPL`).
    pub width: u32,
    /// Active lines (`ALPF`) of this coding unit: lines per frame, or per field for a field
    /// coding unit.
    pub lines: u32,
    /// Pixel aspect ratio `PARC / PARN` (0/0 = not signalled).
    pub par: (u16, u16),
    pub bit_depth: u8,
    /// Source scan type is interlaced (`SST`).
    pub interlaced: bool,
    /// Frame encoding (`FFE`); false = each field is its own coding unit.
    pub frame_encoding: bool,
    pub chroma: ChromaFormat,
    /// The bitstream codes RGB (`CLF`).
    pub rgb: bool,
    pub color_volume: ColorVolume,
    /// SMPTE ST 12-1 time code bytes, if present.
    pub timecode: Option<[u8; 8]>,
    /// Number of macroblock scan lines `NS`.
    pub mb_rows: u32,
}

impl FrameHeader {
    /// Whether the 4:4:4 planes carry R′G′B′ (macroblocks with ACF = 1 carry Y′CbCr).
    ///
    /// The standard ties RGB to `CLF = 1`. ffmpeg's encoder, the most common source of DNxHR 444
    /// files, writes RGB with `CLF = 0` (and Y′CbCr with `CLF = 1`, `ACF = 1` everywhere), and
    /// its decoder reads them that way; we follow that interpretation: every 4:4:4 stream is RGB
    /// except macroblocks flagged ACF = 1.
    pub fn rgb_planes(&self) -> bool {
        self.rgb || self.chroma == ChromaFormat::Yuv444
    }

    /// Macroblocks per scan line `NW`.
    pub fn mb_width(&self) -> u32 {
        self.width.div_ceil(16)
    }

    /// Size of this coding unit for CBR streams (`None` for VBR streams), from Table C.1 or
    /// equation 7.1.
    pub fn coding_unit_size(&self) -> Option<u32> {
        if self.vbr {
            return None;
        }
        let info = cid_info(self.cid)?;
        Some(match info.kind {
            Kind::Hd { frame_size, interlaced, .. } => {
                let f = if self.alpha { frame_size + frame_size / 2 } else { frame_size };
                if interlaced && !self.frame_encoding { f / 2 } else { f }
            }
            Kind::Ri { c0 } => ri_frame_size(self.width, self.lines, c0, self.alpha),
        })
    }
}

fn be16(d: &[u8], o: usize) -> u32 {
    u32::from(d[o]) << 8 | u32::from(d[o + 1])
}

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// Parse a coding-unit header at the start of `d`.
pub fn parse(d: &[u8]) -> Result<FrameHeader> {
    if d.len() < 0x280 {
        return Err(Error::Truncated);
    }
    let header_size = be32(d, 0);
    let version = d[4];
    if !(1..=4).contains(&version) {
        return Err(Error::Invalid("unknown header version"));
    }
    if header_size < 0x280 || header_size as usize > d.len() {
        return Err(if header_size as usize > d.len() { Error::Truncated } else { Error::Invalid("header size") });
    }
    let cid = be32(d, 0x28);
    let info = cid_info(cid).ok_or(Error::UnknownCid(cid))?;
    let vbr = d[5] & 0x10 != 0;
    let field = match d[5] & 3 {
        2 => FieldCode::Field1,
        3 => FieldCode::Field2,
        _ => FieldCode::Frame,
    };
    let macf = d[6] & 0x20 != 0;
    let crc = d[6] & 0x10 != 0;
    let alpha = d[7] & 1 != 0;
    let lossless_alpha = d[7] & 2 != 0;
    let premultiplied_alpha = d[7] & 4 != 0;
    let lines = be16(d, 0x18);
    let width = be16(d, 0x1A);
    let parc = (u32::from(d[0x1C] >> 2 & 3) << 8 | u32::from(d[0x1F])) as u16;
    let parn = (u32::from(d[0x1C] & 3) << 8 | u32::from(d[0x20])) as u16;
    let bit_depth = match d[0x21] >> 5 {
        1 => 8,
        2 => 10,
        3 => 12,
        _ => return Err(Error::Invalid("sample bit depth")),
    };
    let interlaced = d[0x22] & 4 != 0;
    let frame_encoding = d[0x2C] & 0x80 != 0;
    let chroma = match d[0x2C] >> 5 & 3 {
        0 => ChromaFormat::Yuv422,
        1 => ChromaFormat::Yuv420,
        2 => ChromaFormat::Yuv444,
        _ => return Err(Error::Invalid("sub sampling")),
    };
    let rgb = d[0x2C] & 1 != 0;
    let color_volume = match d[0x2C] >> 1 & 3 {
        0 => ColorVolume::Bt709,
        1 => ColorVolume::Bt2020Ncl,
        2 => ColorVolume::Bt2020Cl,
        _ => ColorVolume::OutOfBand,
    };
    let timecode = if d[0x30] & 0x80 != 0 { d.get(0x31..).and_then(|t| t.first_chunk::<8>()).copied() } else { None };
    let mb_rows = be16(d, 0x16C);
    if width == 0 || lines == 0 || width > 16384 || lines > 16384 {
        return Err(Error::Invalid("raster size"));
    }
    if let Kind::Hd { width: w, depth, .. } = info.kind
        && (w as u32 != width || depth != bit_depth)
    {
        return Err(Error::Invalid("raster does not match the compression ID"));
    }
    if rgb && chroma != ChromaFormat::Yuv444 {
        return Err(Error::Invalid("RGB requires 4:4:4"));
    }
    if chroma == ChromaFormat::Yuv444 && !matches!(cid, 1256 | 1270) {
        return Err(Error::Invalid("4:4:4 requires CID 1256 or 1270"));
    }
    // macroblock scan lines must cover the coded raster, and their indices must fit the header
    if mb_rows == 0 || SCAN_INDEX_OFFSET + 4 * mb_rows as usize > header_size as usize {
        return Err(Error::Invalid("macroblock scan line count"));
    }
    if mb_rows != lines.div_ceil(16) {
        return Err(Error::Invalid("macroblock scan lines do not match the raster"));
    }
    Ok(FrameHeader {
        header_size,
        version,
        cid,
        vbr,
        field,
        macf,
        crc,
        alpha,
        lossless_alpha,
        premultiplied_alpha,
        width,
        lines,
        par: (parc, parn),
        bit_depth,
        interlaced,
        frame_encoding,
        chroma,
        rgb,
        color_volume,
        timecode,
        mb_rows,
    })
}

/// Header size for a raster of `lines` lines (§7.2: 640 bytes up to 1088 lines, plus 4 bytes
/// per further 16 lines).
pub fn header_size_for(lines: u32) -> u32 {
    if lines <= 1088 { 0x280 } else { 0x280 + 4 * (lines - 1088).div_ceil(16) }
}

/// Write a header (without scan indices) for `h` into a zeroed buffer of `h.header_size` bytes.
pub fn write(h: &FrameHeader) -> Vec<u8> {
    let mut d = vec![0u8; h.header_size as usize];
    d[0..4].copy_from_slice(&h.header_size.to_be_bytes());
    d[4] = h.version;
    d[5] = (h.vbr as u8) << 4
        | match h.field {
            FieldCode::Frame => 1,
            FieldCode::Field1 => 2,
            FieldCode::Field2 => 3,
        };
    d[6] = 0x80 | (h.macf as u8) << 5 | (h.crc as u8) << 4;
    d[7] = 0xA0 | (h.premultiplied_alpha as u8) << 2 | (h.lossless_alpha as u8) << 1 | h.alpha as u8;
    d[0x18..0x1A].copy_from_slice(&(h.lines as u16).to_be_bytes());
    d[0x1A..0x1C].copy_from_slice(&(h.width as u16).to_be_bytes());
    d[0x1C] = ((h.par.0 >> 8) as u8 & 3) << 2 | ((h.par.1 >> 8) as u8 & 3);
    d[0x1D..0x1F].copy_from_slice(&(h.lines as u16).to_be_bytes());
    d[0x1F] = h.par.0 as u8;
    d[0x20] = h.par.1 as u8;
    let sbd = match h.bit_depth {
        8 => 1,
        10 => 2,
        _ => 3,
    };
    d[0x21] = sbd << 5 | 0x18;
    d[0x22] = 0x88 | (h.interlaced as u8) << 2;
    d[0x28..0x2C].copy_from_slice(&h.cid.to_be_bytes());
    let ssc = match h.chroma {
        ChromaFormat::Yuv422 => 0,
        ChromaFormat::Yuv420 => 1,
        ChromaFormat::Yuv444 => 2,
    };
    let clv = match h.color_volume {
        ColorVolume::Bt709 => 0,
        ColorVolume::Bt2020Ncl => 1,
        ColorVolume::Bt2020Cl => 2,
        ColorVolume::OutOfBand => 3,
    };
    d[0x2C] = (h.frame_encoding as u8) << 7 | ssc << 5 | clv << 1 | h.rgb as u8;
    if let Some(tc) = h.timecode {
        d[0x30] = 0x80;
        d[0x31..0x39].copy_from_slice(&tc);
    }
    d[0x5F] = 0x01;
    d[0x167] = 0x02;
    let msips = (4 * h.mb_rows + 4) as u16;
    d[0x16A..0x16C].copy_from_slice(&msips.to_be_bytes());
    d[0x16C..0x16E].copy_from_slice(&(h.mb_rows as u16).to_be_bytes());
    d[0x16F] = 0x10;
    d
}

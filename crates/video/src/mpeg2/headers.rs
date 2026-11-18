//! Start codes and headers (H.262 §6.2 / §6.3; ISO/IEC 11172-2 §2.4.2).

use crate::mpeg2::bits::Bits;
use crate::mpeg2::{Error, Result};

pub const PICTURE_START: u8 = 0x00;
pub const SLICE_MIN: u8 = 0x01;
pub const SLICE_MAX: u8 = 0xAF;
pub const USER_DATA: u8 = 0xB2;
pub const SEQUENCE_HEADER: u8 = 0xB3;
pub const SEQUENCE_ERROR: u8 = 0xB4;
pub const EXTENSION: u8 = 0xB5;
pub const SEQUENCE_END: u8 = 0xB7;
pub const GROUP_START: u8 = 0xB8;

/// Start codes in `data`: (offset of the `00 00 01` prefix, start code value). A unit's payload
/// runs from `offset + 4` to the next start code.
pub fn start_codes(data: &[u8]) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = data.len();
    while i + 3 < n {
        // skip quickly to a zero byte at i + 2
        if data[i + 2] > 1 {
            i += 3;
            continue;
        }
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            out.push((i, data[i + 3]));
            i += 4;
        } else {
            i += 1;
        }
    }
    out
}

/// Default intra quantiser matrix (H.262 §6.3.11), raster order.
pub const DEFAULT_INTRA: [u8; 64] = [
    8, 16, 19, 22, 26, 27, 29, 34, //
    16, 16, 22, 24, 27, 29, 34, 37, //
    19, 22, 26, 27, 29, 34, 34, 38, //
    22, 22, 26, 27, 29, 34, 37, 40, //
    22, 26, 27, 29, 32, 35, 40, 48, //
    26, 27, 29, 32, 35, 40, 48, 58, //
    26, 27, 29, 34, 38, 46, 56, 69, //
    27, 29, 35, 38, 46, 56, 69, 83,
];

/// Scan index → raster position: zigzag (scan 0) and alternate (scan 1), H.262 Figure 7-2/7-3.
pub const SCAN: [[u8; 64]; 2] = [
    [
        0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50,
        43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    ],
    [
        0, 8, 16, 24, 1, 9, 2, 10, 17, 25, 32, 40, 48, 56, 57, 49, 41, 33, 26, 18, 3, 11, 4, 12, 19, 27, 34, 42, 50, 58, 35, 43, 51, 59, 20, 28, 5,
        13, 6, 14, 21, 29, 36, 44, 52, 60, 37, 45, 53, 61, 22, 30, 7, 15, 23, 31, 38, 46, 54, 62, 39, 47, 55, 63,
    ],
];

/// A quantiser matrix as transmitted (zigzag order) → raster order.
fn read_matrix(b: &mut Bits) -> [u8; 64] {
    let mut m = [0u8; 64];
    for i in 0..64 {
        m[SCAN[0][i] as usize] = b.read(8) as u8;
    }
    m
}

/// sequence_header() (§6.2.2.1).
#[derive(Clone, Debug, PartialEq)]
pub struct SequenceHeader {
    pub horizontal_size: u32,
    pub vertical_size: u32,
    pub aspect_ratio_information: u8,
    pub frame_rate_code: u8,
    pub bit_rate_value: u32,
    pub vbv_buffer_size_value: u32,
    pub constrained_parameters: bool,
    /// Raster order; `None`: default.
    pub intra_matrix: Option<[u8; 64]>,
    pub non_intra_matrix: Option<[u8; 64]>,
}

impl SequenceHeader {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 8 {
            return Err(Error::Invalid("short sequence header".into()));
        }
        let mut b = Bits::new(payload);
        let horizontal_size = b.read(12);
        let vertical_size = b.read(12);
        let aspect_ratio_information = b.read(4) as u8;
        let frame_rate_code = b.read(4) as u8;
        let bit_rate_value = b.read(18);
        b.skip(1);
        let vbv_buffer_size_value = b.read(10);
        let constrained_parameters = b.bit();
        let intra_matrix = b.bit().then(|| read_matrix(&mut b));
        let non_intra_matrix = b.bit().then(|| read_matrix(&mut b));
        if b.overrun() {
            return Err(Error::Invalid("truncated sequence header".into()));
        }
        if horizontal_size == 0 || vertical_size == 0 {
            return Err(Error::Invalid("zero picture size".into()));
        }
        Ok(Self {
            horizontal_size,
            vertical_size,
            aspect_ratio_information,
            frame_rate_code,
            bit_rate_value,
            vbv_buffer_size_value,
            constrained_parameters,
            intra_matrix,
            non_intra_matrix,
        })
    }
}

/// sequence_extension() (§6.2.2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceExtension {
    pub profile_and_level_indication: u8,
    pub progressive_sequence: bool,
    /// 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4.
    pub chroma_format: u8,
    pub horizontal_size_extension: u8,
    pub vertical_size_extension: u8,
    pub bit_rate_extension: u16,
    pub vbv_buffer_size_extension: u8,
    pub low_delay: bool,
    pub frame_rate_extension_n: u8,
    pub frame_rate_extension_d: u8,
}

/// sequence_display_extension() (§6.2.2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DisplayExtension {
    pub video_format: u8,
    /// (colour_primaries, transfer_characteristics, matrix_coefficients), H.262 Tables 6-7..6-9.
    pub colour: Option<(u8, u8, u8)>,
    pub display_horizontal_size: u16,
    pub display_vertical_size: u16,
}

/// quant_matrix_extension() (§6.2.3.2), raster order.
#[derive(Clone, Copy, Debug, Default)]
pub struct QuantMatrixExtension {
    pub intra: Option<[u8; 64]>,
    pub non_intra: Option<[u8; 64]>,
    pub chroma_intra: Option<[u8; 64]>,
    pub chroma_non_intra: Option<[u8; 64]>,
}

/// picture_coding_extension() (§6.2.3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureCodingExtension {
    /// `f_code[s][t]`: s 0 forward / 1 backward, t 0 horizontal / 1 vertical.
    pub f_code: [[u8; 2]; 2],
    pub intra_dc_precision: u8,
    /// 1 top field, 2 bottom field, 3 frame picture.
    pub picture_structure: u8,
    pub top_field_first: bool,
    pub frame_pred_frame_dct: bool,
    pub concealment_motion_vectors: bool,
    pub q_scale_type: bool,
    pub intra_vlc_format: bool,
    pub alternate_scan: bool,
    pub repeat_first_field: bool,
    pub chroma_420_type: bool,
    pub progressive_frame: bool,
}

impl PictureCodingExtension {
    /// The values an MPEG-1 picture behaves as (progressive frame picture, zigzag, linear q).
    pub fn mpeg1(ph: &PictureHeader) -> Self {
        Self {
            f_code: [[ph.forward_f_code; 2], [ph.backward_f_code; 2]],
            intra_dc_precision: 0,
            picture_structure: 3,
            top_field_first: false,
            frame_pred_frame_dct: true,
            concealment_motion_vectors: false,
            q_scale_type: false,
            intra_vlc_format: false,
            alternate_scan: false,
            repeat_first_field: false,
            chroma_420_type: true,
            progressive_frame: true,
        }
    }
}

/// An extension unit (§6.2.2.2 / §6.2.3).
#[derive(Clone, Debug)]
pub enum Extension {
    Sequence(SequenceExtension),
    Display(DisplayExtension),
    QuantMatrix(Box<QuantMatrixExtension>),
    PictureCoding(PictureCodingExtension),
    /// Scalability, copyright, picture display… (not needed to decode the base layer).
    Other(u8),
}

impl Extension {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut b = Bits::new(payload);
        let id = b.read(4) as u8;
        let e = match id {
            1 => {
                let profile_and_level_indication = b.read(8) as u8;
                let progressive_sequence = b.bit();
                let chroma_format = b.read(2) as u8;
                let horizontal_size_extension = b.read(2) as u8;
                let vertical_size_extension = b.read(2) as u8;
                let bit_rate_extension = b.read(12) as u16;
                b.skip(1);
                let vbv_buffer_size_extension = b.read(8) as u8;
                let low_delay = b.bit();
                let frame_rate_extension_n = b.read(2) as u8;
                let frame_rate_extension_d = b.read(5) as u8;
                Extension::Sequence(SequenceExtension {
                    profile_and_level_indication,
                    progressive_sequence,
                    chroma_format,
                    horizontal_size_extension,
                    vertical_size_extension,
                    bit_rate_extension,
                    vbv_buffer_size_extension,
                    low_delay,
                    frame_rate_extension_n,
                    frame_rate_extension_d,
                })
            }
            2 => {
                let video_format = b.read(3) as u8;
                let colour = b.bit().then(|| (b.read(8) as u8, b.read(8) as u8, b.read(8) as u8));
                let display_horizontal_size = b.read(14) as u16;
                b.skip(1);
                let display_vertical_size = b.read(14) as u16;
                Extension::Display(DisplayExtension { video_format, colour, display_horizontal_size, display_vertical_size })
            }
            3 => {
                let intra = b.bit().then(|| read_matrix(&mut b));
                let non_intra = b.bit().then(|| read_matrix(&mut b));
                let chroma_intra = b.bit().then(|| read_matrix(&mut b));
                let chroma_non_intra = b.bit().then(|| read_matrix(&mut b));
                Extension::QuantMatrix(Box::new(QuantMatrixExtension { intra, non_intra, chroma_intra, chroma_non_intra }))
            }
            8 => {
                let f_code = [[b.read(4) as u8, b.read(4) as u8], [b.read(4) as u8, b.read(4) as u8]];
                let intra_dc_precision = b.read(2) as u8;
                let picture_structure = b.read(2) as u8;
                let top_field_first = b.bit();
                let frame_pred_frame_dct = b.bit();
                let concealment_motion_vectors = b.bit();
                let q_scale_type = b.bit();
                let intra_vlc_format = b.bit();
                let alternate_scan = b.bit();
                let repeat_first_field = b.bit();
                let chroma_420_type = b.bit();
                let progressive_frame = b.bit();
                if picture_structure == 0 {
                    return Err(Error::Invalid("reserved picture_structure".into()));
                }
                Extension::PictureCoding(PictureCodingExtension {
                    f_code,
                    intra_dc_precision,
                    picture_structure,
                    top_field_first,
                    frame_pred_frame_dct,
                    concealment_motion_vectors,
                    q_scale_type,
                    intra_vlc_format,
                    alternate_scan,
                    repeat_first_field,
                    chroma_420_type,
                    progressive_frame,
                })
            }
            other => Extension::Other(other),
        };
        if b.overrun() {
            return Err(Error::Invalid(format!("truncated extension {id}")));
        }
        Ok(e)
    }
}

/// group_of_pictures_header() (§6.2.2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GopHeader {
    pub drop_frame: bool,
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub pictures: u8,
    pub closed_gop: bool,
    pub broken_link: bool,
}

impl GopHeader {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 4 {
            return Err(Error::Invalid("short GOP header".into()));
        }
        let mut b = Bits::new(payload);
        let drop_frame = b.bit();
        let hours = b.read(5) as u8;
        let minutes = b.read(6) as u8;
        b.skip(1);
        let seconds = b.read(6) as u8;
        let pictures = b.read(6) as u8;
        let closed_gop = b.bit();
        let broken_link = b.bit();
        Ok(Self { drop_frame, hours, minutes, seconds, pictures, closed_gop, broken_link })
    }
}

/// picture_coding_type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureType {
    I,
    P,
    B,
    /// ISO/IEC 11172-2 DC-coded picture.
    D,
}

impl PictureType {
    pub fn is_anchor(self) -> bool {
        matches!(self, PictureType::I | PictureType::P | PictureType::D)
    }
}

/// picture_header() (§6.2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureHeader {
    pub temporal_reference: u16,
    pub coding_type: PictureType,
    pub vbv_delay: u16,
    pub full_pel_forward_vector: bool,
    pub forward_f_code: u8,
    pub full_pel_backward_vector: bool,
    pub backward_f_code: u8,
}

impl PictureHeader {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 4 {
            return Err(Error::Invalid("short picture header".into()));
        }
        let mut b = Bits::new(payload);
        let temporal_reference = b.read(10) as u16;
        let coding_type = match b.read(3) {
            1 => PictureType::I,
            2 => PictureType::P,
            3 => PictureType::B,
            4 => PictureType::D,
            t => return Err(Error::Invalid(format!("picture_coding_type {t}"))),
        };
        let vbv_delay = b.read(16) as u16;
        let (mut full_pel_forward_vector, mut forward_f_code, mut full_pel_backward_vector, mut backward_f_code) = (false, 7, false, 7);
        if matches!(coding_type, PictureType::P | PictureType::B) {
            full_pel_forward_vector = b.bit();
            forward_f_code = b.read(3) as u8;
        }
        if coding_type == PictureType::B {
            full_pel_backward_vector = b.bit();
            backward_f_code = b.read(3) as u8;
        }
        Ok(Self { temporal_reference, coding_type, vbv_delay, full_pel_forward_vector, forward_f_code, full_pel_backward_vector, backward_f_code })
    }
}

/// Frame rate from frame_rate_code (Table 6-4) and the sequence extension's n/d (MPEG-2).
pub fn frame_rate(code: u8, ext_n: u8, ext_d: u8) -> Option<(u32, u32)> {
    let (n, d) = match code {
        1 => (24000, 1001),
        2 => (24, 1),
        3 => (25, 1),
        4 => (30000, 1001),
        5 => (30, 1),
        6 => (50, 1),
        7 => (60000, 1001),
        8 => (60, 1),
        _ => return None,
    };
    let (n, d) = (n * (ext_n as u32 + 1), d * (ext_d as u32 + 1));
    let g = gcd(n, d);
    Some((n / g, d / g))
}

pub(crate) fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// Profile and level names (Tables 8-2, 8-3, 8-7).
pub fn profile_level_name(pli: u8) -> String {
    if pli & 0x80 != 0 {
        return match pli {
            0x82 => "4:2:2@High".into(),
            0x85 => "4:2:2@Main".into(),
            0x8A => "Multi-view@High".into(),
            0x8B => "Multi-view@High-1440".into(),
            0x8D => "Multi-view@Main".into(),
            0x8E => "Multi-view@Low".into(),
            other => format!("profile/level 0x{other:02x}"),
        };
    }
    let profile = match (pli >> 4) & 7 {
        1 => "High",
        2 => "Spatially Scalable",
        3 => "SNR Scalable",
        4 => "Main",
        5 => "Simple",
        _ => "Reserved",
    };
    let level = match pli & 15 {
        4 => "High",
        6 => "High-1440",
        8 => "Main",
        10 => "Low",
        _ => "Reserved",
    };
    format!("{profile}@{level}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_are_permutations_and_inverse_of_the_figures() {
        for s in SCAN {
            let mut seen = [false; 64];
            for &p in &s {
                assert!(!seen[p as usize]);
                seen[p as usize] = true;
            }
        }
        // Figure 7-3 row 0 of the alternate scan: 0 4 6 20 22 36 38 52
        let inv: Vec<usize> = (0..8).map(|u| SCAN[1].iter().position(|&p| p as usize == u).unwrap()).collect();
        assert_eq!(inv, vec![0, 4, 6, 20, 22, 36, 38, 52]);
        // Figure 7-2 row 0 of the zigzag scan: 0 1 5 6 14 15 27 28
        let inv: Vec<usize> = (0..8).map(|u| SCAN[0].iter().position(|&p| p as usize == u).unwrap()).collect();
        assert_eq!(inv, vec![0, 1, 5, 6, 14, 15, 27, 28]);
    }

    #[test]
    fn start_codes_found_at_every_alignment() {
        let d = [0, 0, 1, 0xB3, 9, 0, 0, 0, 1, 0x00, 0, 0, 1, 0x01, 0, 0];
        assert_eq!(start_codes(&d), vec![(0, 0xB3), (6, 0x00), (10, 0x01)]);
    }

    #[test]
    fn frame_rates() {
        assert_eq!(frame_rate(4, 0, 0), Some((30000, 1001)));
        assert_eq!(frame_rate(3, 1, 0), Some((50, 1)));
        assert_eq!(frame_rate(0, 0, 0), None);
        assert_eq!(profile_level_name(0x44), "Main@High");
        assert_eq!(profile_level_name(0x85), "4:2:2@Main");
    }
}

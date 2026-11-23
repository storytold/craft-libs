//! Start codes and header parsing (ISO/IEC 14496-2 §6.2): visual object sequence, visual object,
//! video object layer (VOL), group of VOP, VOP, video packet resync markers, and H.263 short
//! video headers.

use craft_media::bitstream::BitReader;

use super::DecodeError;
use super::vlc::{DEFAULT_INTRA_MAT, DEFAULT_NONINTRA_MAT, ZIGZAG};
use crate::mpeg4::MAX_SIDE;

pub(super) const VISUAL_OBJECT_SEQUENCE_START: u8 = 0xB0;
pub(super) const VISUAL_OBJECT_SEQUENCE_END: u8 = 0xB1;
pub(super) const USER_DATA_START: u8 = 0xB2;
pub(super) const GROUP_OF_VOP_START: u8 = 0xB3;
pub(super) const VISUAL_OBJECT_START: u8 = 0xB5;
pub(super) const VOP_START: u8 = 0xB6;

/// Picture coding type of a VOP (`vop_coding_type`, Table 6-20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VopType {
    /// Intra-coded (`I`).
    I,
    /// Predictive-coded (`P`).
    P,
    /// Bidirectionally predictive-coded (`B`).
    B,
    /// Sprite / GMC predictive-coded (`S(GMC)`).
    S,
    /// Not-coded VOP (`vop_coded == 0`), repeating the prior reference picture.
    N,
}

/// Colour description from `VisualObject` (`video_signal_type`, ITU-T H.273 code points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorDescription {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

/// Global Motion Compensation (GMC) configuration from `VideoObjectLayer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GmcConfig {
    pub warping_points: u8,
    /// `0` = 1/2 pel, `1` = 1/4 pel, `2` = 1/8 pel, `3` = 1/16 pel.
    pub warping_accuracy: u8,
}

/// Parsed `VideoObjectLayer` (VOL) configuration (ISO/IEC 14496-2 §6.2.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VolConfig {
    pub width: u32,
    pub height: u32,
    pub sar: (u8, u8),
    pub time_increment_resolution: u32,
    pub fixed_time_increment: Option<u32>,
    pub low_delay: bool,
    pub interlaced: bool,
    pub quant_type: bool,
    pub intra_quant_mat: [u8; 64],
    pub nonintra_quant_mat: [u8; 64],
    pub quarter_sample: bool,
    pub resync_marker_disable: bool,
    pub gmc: Option<GmcConfig>,
    pub short_header: bool,
}

impl VolConfig {
    /// Number of bits used to represent `vop_time_increment` (1..=16).
    pub fn time_increment_bits(&self) -> u32 {
        (32 - self.time_increment_resolution.saturating_sub(1).leading_zeros()).max(1)
    }

    /// Macroblock width (`(width + 15) / 16`).
    pub fn mb_width(&self) -> usize {
        (self.width as usize).div_ceil(16)
    }

    /// Macroblock height (`(height + 15) / 16`).
    pub fn mb_height(&self) -> usize {
        (self.height as usize).div_ceil(16)
    }

    /// Bits of `macroblock_number` in a video packet header (Table 6-22).
    pub fn mb_number_bits(&self) -> u32 {
        let total = (self.mb_width() * self.mb_height()).saturating_sub(1) as u32;
        (32 - total.leading_zeros()).max(1)
    }
}

/// Parsed `VideoObjectPlane` (VOP) header (ISO/IEC 14496-2 §6.2.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VopHeader {
    pub coding_type: VopType,
    pub modulo_time_base: u32,
    pub time_increment: u32,
    pub vop_coded: bool,
    pub rounding: bool,
    pub top_field_first: bool,
    pub alternate_vertical_scan: bool,
    /// Quantiser threshold at or above which intra DC is coded with the AC VLC (Table 6-21).
    pub intra_dc_vlc_thr_qp: i32,
    pub quant: u8,
    pub fcode_forward: u8,
    pub fcode_backward: u8,
    /// Translational GMC vector in half-pel units (for 0- or 1-warping-point `S(GMC)` VOPs).
    pub gmc_mv: (i32, i32),
}

/// Find all `0x000001xx` start-code units in `data`, returning `(start_code_byte, payload_after_4_byte_prefix)`.
pub(super) fn scan_start_codes(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut positions = Vec::new();
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            positions.push((i, data[i + 3]));
            i += 4;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(positions.len());
    for (idx, &(pos, code)) in positions.iter().enumerate() {
        let start = pos + 4;
        let end = positions.get(idx + 1).map_or(data.len(), |&(next_pos, _)| next_pos);
        units.push((code, &data[start..end]));
    }
    units
}

/// Check if a `user_data` payload carries a DivX/Xvid packed-bitstream marker (e.g. `DivX503b1393p`).
pub(super) fn is_packed_userdata(payload: &[u8]) -> bool {
    let trimmed = payload.iter().copied().take_while(|&b| b != 0).collect::<Vec<u8>>();
    if let Some(rest) = trimmed.strip_prefix(b"DivX")
        && rest.last() == Some(&b'p')
        && rest.len() >= 2
        && rest[rest.len() - 2].is_ascii_digit()
    {
        return true;
    }
    false
}

/// Parse `VisualObject()` after the `0x000001B5` start code (§6.2.2).
/// Returns optional colour description when `video_signal_type` is present.
pub(super) fn parse_visual_object(payload: &[u8]) -> Result<Option<ColorDescription>, DecodeError> {
    let mut r = BitReader::new(payload);
    if r.read_bit()? {
        let _verid = r.read_bits(4)?;
        let _priority = r.read_bits(3)?;
    }
    let visual_object_type = r.read_bits(4)?;
    if (visual_object_type == 1 || visual_object_type == 2) && r.read_bit()? {
        let _video_format = r.read_bits(3)?;
        let full_range = r.read_bit()?;
        if r.read_bit()? {
            let primaries = r.read_bits(8)? as u8;
            let transfer = r.read_bits(8)? as u8;
            let matrix = r.read_bits(8)? as u8;
            return Ok(Some(ColorDescription { primaries, transfer, matrix, full_range }));
        }
        return Ok(Some(ColorDescription { primaries: 2, transfer: 2, matrix: 2, full_range }));
    }
    Ok(None)
}

/// Read an 8×8 quantisation matrix in zigzag order (§6.3.3): a `0` entry repeats the last
/// non-zero value for the remainder of the matrix.
fn read_quant_matrix(r: &mut BitReader<'_>) -> Result<[u8; 64], DecodeError> {
    let mut mat = [0u8; 64];
    let mut last = 0u8;
    for i in 0..64 {
        let v = r.read_bits(8)? as u8;
        if v == 0 {
            if i == 0 {
                return Err(DecodeError::Invalid("quantisation matrix starts with zero".into()));
            }
            for &z in &ZIGZAG[i..64] {
                mat[z] = last;
            }
            return Ok(mat);
        }
        last = v;
        mat[ZIGZAG[i]] = v;
    }
    Ok(mat)
}

/// Parse `VideoObjectLayer()` after the `0x0000012x` start code (§6.2.3).
pub(super) fn parse_vol(payload: &[u8]) -> Result<VolConfig, DecodeError> {
    let mut r = BitReader::new(payload);
    let _random_accessible_vol = r.read_bit()?;
    let _video_object_type_indication = r.read_bits(8)?;
    let vol_verid = if r.read_bit()? {
        let verid = r.read_bits(4)? as u8;
        let _priority = r.read_bits(3)?;
        verid
    } else {
        1
    };

    let aspect_code = r.read_bits(4)? as u8;
    let sar = match aspect_code {
        1 => (1, 1),
        2 => (12, 11),
        3 => (10, 11),
        4 => (16, 11),
        5 => (40, 33),
        0xF => {
            let pw = r.read_bits(8)? as u8;
            let ph = r.read_bits(8)? as u8;
            if pw > 0 && ph > 0 { (pw, ph) } else { (1, 1) }
        }
        _ => (1, 1),
    };

    let mut low_delay = false;
    if r.read_bit()? {
        let chroma_format = r.read_bits(2)?;
        if chroma_format != 1 {
            return Err(DecodeError::Unsupported(format!("VOL chroma_format {chroma_format} (only 4:2:0 is supported)")));
        }
        low_delay = r.read_bit()?;
        if r.read_bit()? {
            // vbv_parameters (79 bits total)
            let _first_half_bit_rate = r.read_bits(15)?;
            let _ = r.read_bit()?;
            let _latter_half_bit_rate = r.read_bits(15)?;
            let _ = r.read_bit()?;
            let _first_half_vbv_buffer_size = r.read_bits(15)?;
            let _ = r.read_bit()?;
            let _latter_half_vbv_buffer_size = r.read_bits(3)?;
            let _first_half_vbv_occupancy = r.read_bits(11)?;
            let _ = r.read_bit()?;
            let _latter_half_vbv_occupancy = r.read_bits(15)?;
            let _ = r.read_bit()?;
        }
    }

    let shape = r.read_bits(2)?;
    if shape != 0 {
        return Err(DecodeError::Unsupported(format!("non-rectangular VOL shape {shape}")));
    }

    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit before vop_time_increment_resolution".into()));
    }
    let time_increment_resolution = r.read_bits(16)?;
    if time_increment_resolution == 0 {
        return Err(DecodeError::Invalid("vop_time_increment_resolution cannot be 0".into()));
    }
    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit after vop_time_increment_resolution".into()));
    }

    let time_bits = (32 - (time_increment_resolution - 1).leading_zeros()).max(1);
    let fixed_time_increment = if r.read_bit()? { Some(r.read_bits(time_bits)?) } else { None };

    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit before VOL width".into()));
    }
    let width = r.read_bits(13)?;
    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit between VOL width and height".into()));
    }
    let height = r.read_bits(13)?;
    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit after VOL height".into()));
    }
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(DecodeError::Invalid(format!("invalid VOL dimensions {width}x{height}")));
    }

    let interlaced = r.read_bit()?;
    let obmc_disable = r.read_bit()?;
    if !obmc_disable {
        return Err(DecodeError::Unsupported("overlapped block motion compensation (OBMC)".into()));
    }

    let sprite_enable = if vol_verid == 1 { u8::from(r.read_bit()?) } else { r.read_bits(2)? as u8 };
    let gmc = match sprite_enable {
        0 => None,
        2 => {
            let warping_points = r.read_bits(6)? as u8;
            if warping_points > 3 {
                return Err(DecodeError::Invalid(format!("invalid GMC warping points {warping_points}")));
            }
            let warping_accuracy = r.read_bits(2)? as u8;
            let brightness_change = r.read_bit()?;
            if brightness_change {
                return Err(DecodeError::Unsupported("GMC sprite brightness change".into()));
            }
            Some(GmcConfig { warping_points, warping_accuracy })
        }
        other => return Err(DecodeError::Unsupported(format!("sprite mode {other}"))),
    };

    if r.read_bit()? {
        let _quant_precision = r.read_bits(4)?;
        let bits_per_pixel = r.read_bits(4)?;
        if bits_per_pixel != 8 {
            return Err(DecodeError::Unsupported(format!("{bits_per_pixel}-bit sample depth")));
        }
    }

    let quant_type = r.read_bit()?;
    let mut intra_quant_mat = DEFAULT_INTRA_MAT;
    let mut nonintra_quant_mat = DEFAULT_NONINTRA_MAT;
    if quant_type {
        if r.read_bit()? {
            intra_quant_mat = read_quant_matrix(&mut r)?;
        }
        if r.read_bit()? {
            nonintra_quant_mat = read_quant_matrix(&mut r)?;
        }
    }

    let quarter_sample = if vol_verid != 1 { r.read_bit()? } else { false };
    let complexity_estimation_disable = r.read_bit()?;
    if !complexity_estimation_disable {
        return Err(DecodeError::Unsupported("VOP complexity estimation header".into()));
    }

    let resync_marker_disable = r.read_bit()?;
    let data_partitioned = r.read_bit()?;
    if data_partitioned {
        return Err(DecodeError::Unsupported("data-partitioned VOL".into()));
    }

    if vol_verid != 1 {
        if r.read_bit()? {
            return Err(DecodeError::Unsupported("NEWPRED VOL".into()));
        }
        if r.read_bit()? {
            return Err(DecodeError::Unsupported("reduced-resolution VOP".into()));
        }
    }

    let scalability = r.read_bit()?;
    if scalability {
        return Err(DecodeError::Unsupported("scalable VOL".into()));
    }

    Ok(VolConfig {
        width,
        height,
        sar,
        time_increment_resolution,
        fixed_time_increment,
        low_delay,
        interlaced,
        quant_type,
        intra_quant_mat,
        nonintra_quant_mat,
        quarter_sample,
        resync_marker_disable,
        gmc,
        short_header: false,
    })
}

/// Table 6-21: `intra_dc_vlc_thr` code (0..=7) to minimum `qp` where intra DC uses the AC VLC.
pub(super) fn intra_dc_threshold_qp(code: u8) -> i32 {
    match code {
        0 => 99,
        7 => 1,
        c => 11 + 2 * i32::from(c),
    }
}

/// Read one `duv` trajectory delta component (§6.2.5.1, Table B-15).
fn read_gmc_duv(r: &mut BitReader<'_>) -> Result<i32, DecodeError> {
    // Prefix length code: 00->0, 010->1, 011->2, 100->3, 101->4, 110->5, 1110->6, ..., up to 111111111110->13, 1111111111110->14
    let size = if !r.read_bit()? {
        if !r.read_bit()? { 0 } else { 1 + r.read_bits(1)? }
    } else if !r.read_bit()? {
        3 + r.read_bits(1)?
    } else if !r.read_bit()? {
        5
    } else {
        let mut ones = 3u32;
        while r.read_bit()? {
            ones += 1;
            if ones > 14 {
                return Err(DecodeError::Invalid("invalid GMC duv length".into()));
            }
        }
        ones + 3
    };
    if size == 0 {
        return Ok(0);
    }
    let raw = r.read_bits(size)? as i32;
    let v = if (raw >> (size - 1)) == 0 { raw - ((1 << size) - 1) } else { raw };
    Ok(v)
}

/// Parse `VideoObjectPlane()` header after the `0x000001B6` start code (§6.2.5), leaving `r`
/// positioned at the first macroblock (when `vop_coded` is true).
pub(super) fn parse_vop_header(r: &mut BitReader<'_>, vol: &VolConfig) -> Result<VopHeader, DecodeError> {
    let coding_type = match r.read_bits(2)? {
        0 => VopType::I,
        1 => VopType::P,
        2 => VopType::B,
        _ => VopType::S,
    };

    let mut modulo_time_base = 0u32;
    while r.read_bit()? {
        modulo_time_base += 1;
        if modulo_time_base > 64 {
            return Err(DecodeError::Invalid("excessive modulo_time_base".into()));
        }
    }
    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit after modulo_time_base".into()));
    }
    let time_increment = r.read_bits(vol.time_increment_bits())?;
    if !r.read_bit()? {
        return Err(DecodeError::Invalid("missing marker bit after vop_time_increment".into()));
    }

    let vop_coded = r.read_bit()?;
    if !vop_coded {
        return Ok(VopHeader {
            coding_type: VopType::N,
            modulo_time_base,
            time_increment,
            vop_coded: false,
            rounding: false,
            top_field_first: false,
            alternate_vertical_scan: false,
            intra_dc_vlc_thr_qp: 99,
            quant: 1,
            fcode_forward: 1,
            fcode_backward: 1,
            gmc_mv: (0, 0),
        });
    }

    let rounding = if coding_type == VopType::P || coding_type == VopType::S { r.read_bit()? } else { false };

    let mut gmc_mv = (0, 0);
    if coding_type == VopType::S {
        let Some(gmc) = vol.gmc else {
            return Err(DecodeError::Invalid("S-VOP without GMC enabled in VOL".into()));
        };
        if gmc.warping_points > 1 {
            return Err(DecodeError::Unsupported(format!("S(GMC) VOP with {} warping points", gmc.warping_points)));
        }
        if gmc.warping_points == 1 {
            let dx = read_gmc_duv(r)?;
            let dy = read_gmc_duv(r)?;
            let _ = r.read_bit()?; // marker_bit
            // Convert from accuracy (0 = 1/2, 1 = 1/4, 2 = 1/8, 3 = 1/16) to half-pel (or qpel if quarter_sample)
            let target_shift = if vol.quarter_sample { 1u8 } else { 0u8 };
            let shift = gmc.warping_accuracy.saturating_sub(target_shift);
            let up = target_shift.saturating_sub(gmc.warping_accuracy);
            gmc_mv = ((dx << up) >> shift, (dy << up) >> shift);
        }
    }

    let (top_field_first, alternate_vertical_scan) = if vol.interlaced { (r.read_bit()?, r.read_bit()?) } else { (false, false) };

    let thr_code = r.read_bits(3)? as u8;
    let intra_dc_vlc_thr_qp = intra_dc_threshold_qp(thr_code);

    let quant = r.read_bits(5)? as u8;
    if quant == 0 {
        return Err(DecodeError::Invalid("vop_quant cannot be 0".into()));
    }

    let fcode_forward = if coding_type != VopType::I {
        let f = r.read_bits(3)? as u8;
        if f == 0 {
            return Err(DecodeError::Invalid("vop_fcode_forward cannot be 0".into()));
        }
        f
    } else {
        1
    };

    let fcode_backward = if coding_type == VopType::B {
        let f = r.read_bits(3)? as u8;
        if f == 0 {
            return Err(DecodeError::Invalid("vop_fcode_backward cannot be 0".into()));
        }
        f
    } else {
        1
    };

    Ok(VopHeader {
        coding_type,
        modulo_time_base,
        time_increment,
        vop_coded: true,
        rounding,
        top_field_first,
        alternate_vertical_scan,
        intra_dc_vlc_thr_qp,
        quant,
        fcode_forward,
        fcode_backward,
        gmc_mv,
    })
}

/// Check if the bitstream begins with an ITU-T H.263 short video header (`0000 0000 0000 0000 1000 00`, §6.2.5.2)
/// and parse both the implied [`VolConfig`] and [`VopHeader`].
pub(super) fn try_parse_short_video_header(r: &mut BitReader<'_>) -> Result<Option<(VolConfig, VopHeader)>, DecodeError> {
    if r.bits_left() < 48 || r.peek(22) != 0x20 {
        return Ok(None);
    }
    let _ = r.skip(22);
    let temporal_reference = r.read_bits(8)?;
    if !r.read_bit()? {
        return Err(DecodeError::Invalid("short video header missing marker bit".into()));
    }
    if r.read_bit()? {
        return Err(DecodeError::Invalid("short video header zero_bit is 1".into()));
    }
    let _split_screen = r.read_bit()?;
    let _doc_camera = r.read_bit()?;
    let _freeze_release = r.read_bit()?;
    let source_format = r.read_bits(3)?;
    let (width, height) = match source_format {
        1 => (128, 96),
        2 => (176, 144),
        3 => (352, 288),
        4 => (704, 576),
        5 => (1408, 1152),
        _ => return Err(DecodeError::Unsupported(format!("H.263 short video header source_format {source_format}"))),
    };
    let coding_type = if r.read_bit()? { VopType::P } else { VopType::I };
    let reserved = r.read_bits(4)?;
    if reserved != 0 {
        return Err(DecodeError::Unsupported(format!("H.263 optional modes ({reserved:#x}) in short video header")));
    }
    let quant = r.read_bits(5)? as u8;
    if quant == 0 {
        return Err(DecodeError::Invalid("short video header vop_quant cannot be 0".into()));
    }
    if r.read_bit()? {
        return Err(DecodeError::Invalid("short video header CPM bit is 1".into()));
    }
    while r.read_bit()? {
        let _psupp = r.read_bits(8)?;
    }

    let vol = VolConfig {
        width,
        height,
        sar: (12, 11),
        time_increment_resolution: 30000,
        fixed_time_increment: Some(1001),
        low_delay: true,
        interlaced: false,
        quant_type: false,
        intra_quant_mat: DEFAULT_INTRA_MAT,
        nonintra_quant_mat: DEFAULT_NONINTRA_MAT,
        quarter_sample: false,
        resync_marker_disable: false,
        gmc: None,
        short_header: true,
    };
    let vop = VopHeader {
        coding_type,
        modulo_time_base: 0,
        time_increment: temporal_reference,
        vop_coded: true,
        rounding: false,
        top_field_first: false,
        alternate_vertical_scan: false,
        intra_dc_vlc_thr_qp: 99,
        quant,
        fcode_forward: 1,
        fcode_backward: 1,
        gmc_mv: (0, 0),
    };
    Ok(Some((vol, vop)))
}

/// Check if a video packet resync marker (or H.263 GOB resync marker) follows at the next
/// byte boundary after valid stuffing bits (`0` followed by zero or more `1`s, §6.2.5.2).
/// If present, consumes the stuffing, marker, and video packet header, returning
/// `Some((macroblock_number, quant_scale))`.
pub(super) fn try_read_video_packet_header(r: &mut BitReader<'_>, vol: &VolConfig, vop: &mut VopHeader) -> Result<Option<(usize, u8)>, DecodeError> {
    if vol.resync_marker_disable {
        return Ok(None);
    }
    if vol.short_header {
        // H.263 GOB header: 17-bit resync marker `0000 0000 0000 0000 1`
        if r.bits_left() >= 17 + 12 && r.peek(17) == 1 {
            let _ = r.skip(17);
            let gob_num = r.read_bits(5)? as usize;
            let _gfid = r.read_bits(2)?;
            let quant = r.read_bits(5)? as u8;
            if quant == 0 {
                return Err(DecodeError::Invalid("GOB quant_scale cannot be 0".into()));
            }
            let mb_row = gob_num * if vol.height > 400 { 2 } else { 1 };
            return Ok(Some((mb_row * vol.mb_width(), quant)));
        }
        return Ok(None);
    }

    let marker_zeros = match vop.coding_type {
        VopType::I => 16u32,
        VopType::P | VopType::S => 15 + u32::from(vop.fcode_forward),
        VopType::B => (15 + u32::from(vop.fcode_forward.max(vop.fcode_backward))).max(17),
        VopType::N => return Ok(None),
    };
    let marker_len = marker_zeros + 1;

    // Stuffing to byte boundary: either already byte-aligned (if a resync marker starts directly)
    // or 1..=8 bits of `0` followed by `1`s to the next byte boundary.
    let pos = r.position();
    let rem = (8 - (pos % 8)) % 8;
    let mut trial = r.clone();
    if rem > 0 {
        let stuff = trial.read_bits(rem as u32)?;
        let expected = (1u32 << (rem - 1)) - 1;
        if stuff != expected {
            return Ok(None);
        }
    } else if trial.bits_left() >= 8 && trial.peek(8) == 0x7F {
        // Full 8-bit stuffing byte `0111 1111` at a byte boundary before the resync marker.
        let _ = trial.skip(8);
    }

    if trial.bits_left() < marker_len as usize || trial.peek(marker_len) != 1 {
        return Ok(None);
    }
    let _ = trial.skip(marker_len as usize);

    let mb_num = trial.read_bits(vol.mb_number_bits())? as usize;
    let total_mbs = vol.mb_width() * vol.mb_height();
    if mb_num >= total_mbs {
        return Err(DecodeError::Invalid(format!("video packet macroblock_number {mb_num} >= {total_mbs}")));
    }
    let quant = trial.read_bits(5)? as u8;
    if quant == 0 {
        return Err(DecodeError::Invalid("video packet quant_scale cannot be 0".into()));
    }
    let hec = trial.read_bit()?;
    if hec {
        while trial.read_bit()? {}
        if !trial.read_bit()? {
            return Err(DecodeError::Invalid("HEC missing marker bit after modulo_time_base".into()));
        }
        let _ti = trial.read_bits(vol.time_increment_bits())?;
        if !trial.read_bit()? {
            return Err(DecodeError::Invalid("HEC missing marker bit after time_increment".into()));
        }
        let _vop_type = trial.read_bits(2)?;
        let thr = trial.read_bits(3)? as u8;
        vop.intra_dc_vlc_thr_qp = intra_dc_threshold_qp(thr);
        if vop.coding_type != VopType::I {
            let f = trial.read_bits(3)? as u8;
            if f > 0 {
                vop.fcode_forward = f;
            }
        }
        if vop.coding_type == VopType::B {
            let f = trial.read_bits(3)? as u8;
            if f > 0 {
                vop.fcode_backward = f;
            }
        }
    }

    *r = trial;
    Ok(Some((mb_num, quant)))
}

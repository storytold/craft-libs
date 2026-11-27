//! Start codes and headers (ISO/IEC 14496-2 §6.2): visual object sequence, visual object, video
//! object, video object layer, video object plane, and video packet resync markers.

use craft_media::bitstream::BitWriter;

use super::ColorConfig;
use super::tables::ZIGZAG;

const VISUAL_OBJECT_SEQUENCE: u32 = 0x1B0;
const VISUAL_OBJECT: u32 = 0x1B5;
const VIDEO_OBJECT: u32 = 0x100;
const VIDEO_OBJECT_LAYER: u32 = 0x120;
const USER_DATA: u32 = 0x1B2;
const VOP: u32 = 0x1B6;

/// `visual_object_type`: video.
const VIDEO_ID: u32 = 1;
/// `video_object_type_indication`: Simple Object Type.
const SIMPLE_OBJECT: u32 = 1;
/// `video_object_type_indication`: Advanced Simple Object Type (Table 6-11).
const ADVANCED_SIMPLE_OBJECT: u32 = 17;

/// Picture coding type of a VOP (`vop_coding_type`, Table 6-20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VopCodingType {
    /// Intra-coded (`00`).
    I,
    /// Predictive-coded (`01`).
    P,
    /// Bidirectionally predictive-coded (`10`).
    B,
    /// Not-coded P-VOP placeholder (`01` with `vop_coded = 0`).
    N,
}

fn start_code(w: &mut BitWriter, code: u32) {
    w.write_bits(code, 32);
}

/// `next_start_code()` / `next_resync_marker()`: a zero bit, then ones to the byte boundary.
pub(super) fn stuffing(w: &mut BitWriter) {
    w.write_bit(false);
    while !w.is_byte_aligned() {
        w.write_bit(true);
    }
}

fn marker(w: &mut BitWriter) {
    w.write_bit(true);
}

/// The stream's time base.
#[derive(Clone, Copy, Debug)]
pub(super) struct Timing {
    /// `vop_time_increment_resolution`: ticks per second.
    pub resolution: u32,
    /// Ticks per frame.
    pub increment: u32,
}

impl Timing {
    /// Bits of `vop_time_increment` (enough for `resolution - 1`, at least 1).
    pub fn bits(self) -> u32 {
        (32 - self.resolution.saturating_sub(1).leading_zeros()).max(1)
    }
}

pub(super) struct Sequence<'a> {
    pub width: u32,
    pub height: u32,
    pub timing: Timing,
    pub profile_level: u8,
    pub asp: bool,
    pub sar: (u8, u8),
    pub color: Option<ColorConfig>,
    pub low_delay: bool,
    pub mpeg_quant: bool,
    pub intra_matrix: Option<&'a [u8; 64]>,
    pub non_intra_matrix: Option<&'a [u8; 64]>,
    pub quarter_sample: bool,
    pub resync_markers: bool,
    pub packed_userdata: bool,
}

/// Table 6-12 `aspect_ratio_info` codes with a fixed pixel aspect ratio.
const ASPECT: [(u8, (u8, u8)); 5] = [(1, (1, 1)), (2, (12, 11)), (3, (10, 11)), (4, (16, 11)), (5, (40, 33))];

/// Write an 8×8 quantisation matrix in zigzag scan order (§6.2.3).
fn write_quant_matrix(w: &mut BitWriter, mat: &[u8; 64]) {
    for &z in &ZIGZAG {
        w.write_bits(u32::from(mat[z]), 8);
    }
}

/// `VisualObjectSequence()` through `VideoObjectLayer()`: what a decoder needs before the first
/// VOP, and the `DecoderSpecificInfo` of an MP4 `esds`.
pub(super) fn sequence_headers(s: &Sequence<'_>) -> Vec<u8> {
    let mut w = BitWriter::new();
    start_code(&mut w, VISUAL_OBJECT_SEQUENCE);
    w.write_bits(u32::from(s.profile_level), 8);

    start_code(&mut w, VISUAL_OBJECT);
    let need_verid = s.asp || s.quarter_sample;
    w.write_bit(need_verid); // is_visual_object_identifier
    if need_verid {
        w.write_bits(5, 4); // visual_object_verid: ISO/IEC 14496-2 AMD 1 (ASP)
        w.write_bits(1, 3); // visual_object_priority
    }
    w.write_bits(VIDEO_ID, 4);
    w.write_bit(s.color.is_some()); // video_signal_type
    if let Some(c) = s.color {
        w.write_bits(5, 3); // video_format: unspecified
        w.write_bit(c.full_range);
        w.write_bit(true); // colour_description
        w.write_bits(u32::from(c.primaries), 8);
        w.write_bits(u32::from(c.transfer), 8);
        w.write_bits(u32::from(c.matrix), 8);
    }
    stuffing(&mut w);

    start_code(&mut w, VIDEO_OBJECT);

    start_code(&mut w, VIDEO_OBJECT_LAYER);
    w.write_bit(false); // random_accessible_vol
    w.write_bits(if s.asp { ADVANCED_SIMPLE_OBJECT } else { SIMPLE_OBJECT }, 8);
    w.write_bit(need_verid); // is_object_layer_identifier
    if need_verid {
        w.write_bits(5, 4); // video_object_layer_verid
        w.write_bits(1, 3); // video_object_layer_priority
    }
    match ASPECT.iter().find(|(_, r)| *r == s.sar) {
        Some(&(code, _)) => w.write_bits(u32::from(code), 4),
        None => {
            w.write_bits(0xF, 4); // extended PAR
            w.write_bits(u32::from(s.sar.0), 8);
            w.write_bits(u32::from(s.sar.1), 8);
        }
    }
    w.write_bit(true); // vol_control_parameters
    w.write_bits(1, 2); // chroma_format 4:2:0
    w.write_bit(s.low_delay); // low_delay: true when no B-VOPs
    w.write_bit(false); // vbv_parameters
    w.write_bits(0, 2); // video_object_layer_shape: rectangular
    marker(&mut w);
    w.write_bits(s.timing.resolution, 16);
    marker(&mut w);
    let fixed = s.timing.increment < s.timing.resolution;
    w.write_bit(fixed); // fixed_vop_rate
    if fixed {
        w.write_bits(s.timing.increment, s.timing.bits());
    }
    marker(&mut w);
    w.write_bits(s.width, 13);
    marker(&mut w);
    w.write_bits(s.height, 13);
    marker(&mut w);
    w.write_bit(false); // interlaced
    w.write_bit(true); // obmc_disable
    if need_verid {
        w.write_bits(0, 2); // sprite_enable (2 bits when verid != 1)
    } else {
        w.write_bit(false); // sprite_enable (1 bit when verid == 1)
    }
    w.write_bit(false); // not_8_bit
    w.write_bit(s.mpeg_quant); // quant_type: 0 = H.263, 1 = MPEG matrices
    if s.mpeg_quant {
        w.write_bit(s.intra_matrix.is_some()); // load_intra_quant_mat
        if let Some(m) = s.intra_matrix {
            write_quant_matrix(&mut w, m);
        }
        w.write_bit(s.non_intra_matrix.is_some()); // load_nonintra_quant_mat
        if let Some(m) = s.non_intra_matrix {
            write_quant_matrix(&mut w, m);
        }
    }
    if need_verid {
        w.write_bit(s.quarter_sample); // quarter_sample
    }
    w.write_bit(true); // complexity_estimation_disable
    w.write_bit(!s.resync_markers); // resync_marker_disable
    w.write_bit(false); // data_partitioned
    if need_verid {
        w.write_bit(false); // newpred_enable
        w.write_bit(false); // reduced_resolution_vop_enable
    }
    w.write_bit(false); // scalability
    stuffing(&mut w);

    if s.packed_userdata {
        start_code(&mut w, USER_DATA);
        w.write_bytes(b"DivX503b1393p\0");
    }

    w.finish()
}

/// `VideoObjectPlane()` up to the first macroblock.
pub(super) struct VopHeader {
    pub coding_type: VopCodingType,
    /// Whole seconds since the previous VOP time reference (`modulo_time_base`).
    pub seconds: u32,
    pub time_increment: u32,
    pub rounding: bool,
    pub quant: u8,
    pub fcode_forward: u8,
    pub fcode_backward: u8,
}

pub(super) fn vop_header(w: &mut BitWriter, h: &VopHeader, timing: Timing) {
    start_code(w, VOP);
    let type_bits = match h.coding_type {
        VopCodingType::I => 0,
        VopCodingType::P | VopCodingType::N => 1,
        VopCodingType::B => 2,
    };
    w.write_bits(type_bits, 2); // vop_coding_type
    for _ in 0..h.seconds {
        w.write_bit(true);
    }
    w.write_bit(false);
    marker(w);
    w.write_bits(h.time_increment, timing.bits());
    marker(w);
    if h.coding_type == VopCodingType::N {
        w.write_bit(false); // vop_coded = 0
        return;
    }
    w.write_bit(true); // vop_coded = 1
    if h.coding_type == VopCodingType::P {
        w.write_bit(h.rounding);
    }
    w.write_bits(0, 3); // intra_dc_vlc_thr: DC always with its own VLC
    w.write_bits(u32::from(h.quant), 5);
    if h.coding_type != VopCodingType::I {
        w.write_bits(u32::from(h.fcode_forward), 3);
    }
    if h.coding_type == VopCodingType::B {
        w.write_bits(u32::from(h.fcode_backward), 3);
    }
}

/// Bits of `macroblock_number` in `video_packet_header()` (Table 6-23: `ceil(log2(total_mbs))`, at least 1).
pub(super) fn mb_number_bits(total_mbs: usize) -> u32 {
    let max_idx = total_mbs.saturating_sub(1) as u32;
    (32 - max_idx.leading_zeros()).max(1)
}

/// Write `next_resync_marker()` and `video_packet_header()` (§6.2.5.2) before macroblock `mb_num`.
pub(super) fn write_video_packet_header(
    w: &mut BitWriter,
    coding_type: VopCodingType,
    fcode_forward: u8,
    fcode_backward: u8,
    mb_num: usize,
    total_mbs: usize,
    quant: u8,
) {
    stuffing(w);
    let zeros = match coding_type {
        VopCodingType::I => 16u32,
        VopCodingType::P | VopCodingType::N => 15 + u32::from(fcode_forward),
        VopCodingType::B => (15 + u32::from(fcode_forward.max(fcode_backward))).max(17),
    };
    w.write_bits(0, zeros);
    w.write_bit(true);
    w.write_bits(mb_num as u32, mb_number_bits(total_mbs));
    w.write_bits(u32::from(quant), 5);
    w.write_bit(false); // header_extension_code = 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_increment_bits() {
        let t = |resolution| Timing { resolution, increment: 1 }.bits();
        assert_eq!(t(1), 1);
        assert_eq!(t(2), 1);
        assert_eq!(t(25), 5);
        assert_eq!(t(30000), 15);
        assert_eq!(t(65535), 16);
    }

    #[test]
    fn headers_are_byte_aligned_start_codes() {
        let s = Sequence {
            width: 352,
            height: 288,
            timing: Timing { resolution: 25, increment: 1 },
            profile_level: 3,
            asp: false,
            sar: (1, 1),
            color: None,
            low_delay: true,
            mpeg_quant: false,
            intra_matrix: None,
            non_intra_matrix: None,
            quarter_sample: false,
            resync_markers: false,
            packed_userdata: false,
        };
        let h = sequence_headers(&s);
        assert_eq!(&h[..5], &[0, 0, 1, 0xB0, 3]);
        assert_eq!(&h[5..9], &[0, 0, 1, 0xB5]);
        assert!(h.windows(4).any(|w| w == [0, 0, 1, 0x20]));
    }
}

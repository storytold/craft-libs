//! Start codes and headers (ISO/IEC 14496-2 §6.2): visual object sequence, visual object, video
//! object, video object layer and video object plane.

use craft_media::bitstream::BitWriter;

use super::ColorConfig;

const VISUAL_OBJECT_SEQUENCE: u32 = 0x1B0;
const VISUAL_OBJECT: u32 = 0x1B5;
const VIDEO_OBJECT: u32 = 0x100;
const VIDEO_OBJECT_LAYER: u32 = 0x120;
const VOP: u32 = 0x1B6;

/// `visual_object_type`: video.
const VIDEO_ID: u32 = 1;
/// `video_object_type_indication`: Simple Object Type.
const SIMPLE_OBJECT: u32 = 1;

fn start_code(w: &mut BitWriter, code: u32) {
    w.write_bits(code, 32);
}

/// `next_start_code()`: a zero bit, then ones to the byte boundary.
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

pub(super) struct Sequence {
    pub width: u32,
    pub height: u32,
    pub timing: Timing,
    pub profile_level: u8,
    pub sar: (u8, u8),
    pub color: Option<ColorConfig>,
}

/// Table 6-12 `aspect_ratio_info` codes with a fixed pixel aspect ratio.
const ASPECT: [(u8, (u8, u8)); 5] = [(1, (1, 1)), (2, (12, 11)), (3, (10, 11)), (4, (16, 11)), (5, (40, 33))];

/// `VisualObjectSequence()` through `VideoObjectLayer()`: what a decoder needs before the first
/// VOP, and the `DecoderSpecificInfo` of an MP4 `esds`.
pub(super) fn sequence_headers(s: &Sequence) -> Vec<u8> {
    let mut w = BitWriter::new();
    start_code(&mut w, VISUAL_OBJECT_SEQUENCE);
    w.write_bits(u32::from(s.profile_level), 8);

    start_code(&mut w, VISUAL_OBJECT);
    w.write_bit(false); // is_visual_object_identifier
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
    w.write_bits(SIMPLE_OBJECT, 8);
    w.write_bit(false); // is_object_layer_identifier
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
    w.write_bit(true); // low_delay: no B-VOPs
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
    w.write_bit(false); // sprite_enable
    w.write_bit(false); // not_8_bit
    w.write_bit(false); // quant_type: H.263 quantisation
    w.write_bit(true); // complexity_estimation_disable
    w.write_bit(true); // resync_marker_disable
    w.write_bit(false); // data_partitioned
    w.write_bit(false); // scalability
    stuffing(&mut w);
    w.finish()
}

/// `VideoObjectPlane()` up to the first macroblock.
pub(super) struct VopHeader {
    pub intra: bool,
    /// Whole seconds since the previous VOP (`modulo_time_base`).
    pub seconds: u32,
    pub time_increment: u32,
    pub rounding: bool,
    pub quant: u8,
    pub fcode: u8,
}

pub(super) fn vop_header(w: &mut BitWriter, h: &VopHeader, timing: Timing) {
    start_code(w, VOP);
    w.write_bits(if h.intra { 0 } else { 1 }, 2); // vop_coding_type
    for _ in 0..h.seconds {
        w.write_bit(true);
    }
    w.write_bit(false);
    marker(w);
    w.write_bits(h.time_increment, timing.bits());
    marker(w);
    w.write_bit(true); // vop_coded
    if !h.intra {
        w.write_bit(h.rounding);
    }
    w.write_bits(0, 3); // intra_dc_vlc_thr: DC always with its own VLC
    w.write_bits(u32::from(h.quant), 5);
    if !h.intra {
        w.write_bits(u32::from(h.fcode), 3);
    }
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
        let s = Sequence { width: 352, height: 288, timing: Timing { resolution: 25, increment: 1 }, profile_level: 3, sar: (1, 1), color: None };
        let h = sequence_headers(&s);
        assert_eq!(&h[..5], &[0, 0, 1, 0xB0, 3]);
        assert_eq!(&h[5..9], &[0, 0, 1, 0xB5]);
        assert!(h.windows(4).any(|w| w == [0, 0, 1, 0x20]));
    }
}

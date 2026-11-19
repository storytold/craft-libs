//! Parameter sets (7.3.2.1 VPS, 7.3.2.2 SPS with E.2.1 VUI, 7.3.2.3 PPS), profile_tier_level (7.3.3),
//! level selection (Annex A, Table A.8) and the `hvcC` record (ISO/IEC 14496-15 §8.3.3.1).

use craft_media::bitstream::{BitWriter, escape_rbsp};
use craft_media::isobmff::{HevcConfig, HevcNalArray};

pub const NAL_TRAIL_R: u8 = 1;
pub const NAL_IDR_N_LP: u8 = 20;
pub const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;

/// Build a NAL unit (2-byte header, `nuh_layer_id` 0, `TemporalId` 0) from an RBSP, inserting
/// emulation prevention bytes via [`escape_rbsp`].
pub fn nal_unit(nal_type: u8, rbsp: &[u8]) -> Vec<u8> {
    let escaped = escape_rbsp(rbsp);
    let mut out = Vec::with_capacity(2 + escaped.len());
    out.push(nal_type << 1);
    out.push(1);
    out.extend_from_slice(&escaped);
    out
}

/// Fixed coding structure of the encoder.
pub const LOG2_CTB: u32 = 6;
pub const LOG2_MIN_CB: u32 = 3;
pub const LOG2_MIN_TB: u32 = 2;
pub const LOG2_MAX_TB: u32 = 5;
pub const LOG2_MAX_POC_LSB: u32 = 8;
/// MaxNumMergeCand.
pub const MAX_MERGE_CAND: usize = 5;

/// Sequence-level settings shared by the parameter-set writers.
#[derive(Clone, Debug)]
pub struct SeqParams {
    /// Coded (padded) size, multiples of the minimum CB size.
    pub coded_width: u32,
    pub coded_height: u32,
    /// Output (cropped) size.
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub profile_idc: u8,
    pub level_idc: u8,
    pub full_range: bool,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coeffs: u8,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// (level_idc, MaxLumaPs, MaxLumaSr, MaxBR main tier in kbit/s) of Table A.8 (and A.9 for luma rates).
const LEVELS: [(u8, u64, u64, u64); 13] = [
    (30, 36_864, 552_960, 128),
    (60, 122_880, 3_686_400, 1_500),
    (63, 245_760, 7_372_800, 3_000),
    (90, 552_960, 16_588_800, 6_000),
    (93, 983_040, 33_177_600, 10_000),
    (120, 2_228_224, 66_846_720, 12_000),
    (123, 2_228_224, 133_693_440, 20_000),
    (150, 8_912_896, 267_386_880, 25_000),
    (153, 8_912_896, 534_773_760, 40_000),
    (156, 8_912_896, 1_069_547_520, 60_000),
    (180, 35_651_584, 1_069_547_520, 60_000),
    (183, 35_651_584, 2_139_095_040, 120_000),
    (186, 35_651_584, 4_278_190_080, 240_000),
];

/// Lowest level whose picture size, sample rate and (when known) bitrate limits fit, or `None` when
/// even level 6.2 is too small: the stream could not claim to conform to any level.
pub fn choose_level(width: u32, height: u32, fps_num: u32, fps_den: u32, kbps: Option<u32>) -> Option<u8> {
    let ps = width as u64 * height as u64;
    let sr = (ps as f64 * fps_num as f64 / fps_den.max(1) as f64).ceil() as u64;
    for &(idc, max_ps, max_sr, max_br) in &LEVELS {
        let max_dim = ((max_ps * 8) as f64).sqrt() as u64;
        if ps <= max_ps && (width as u64) <= max_dim && (height as u64) <= max_dim && sr <= max_sr && kbps.is_none_or(|k| k as u64 <= max_br) {
            return Some(idc);
        }
    }
    None
}

fn profile_tier_level(w: &mut BitWriter, p: &SeqParams) {
    w.write_bits(0, 2); // general_profile_space
    w.write_bits(0, 1); // general_tier_flag (Main tier)
    w.write_bits(p.profile_idc as u32, 5);
    w.write_bits(compat_flags(p.profile_idc), 32);
    w.write_bits(1, 1); // general_progressive_source_flag
    w.write_bits(0, 1); // general_interlaced_source_flag
    w.write_bits(0, 1); // general_non_packed_constraint_flag
    w.write_bits(1, 1); // general_frame_only_constraint_flag
    w.write_bits(0, 32); // general_reserved_zero_43bits ...
    w.write_bits(0, 11);
    w.write_bits(0, 1); // general_inbld_flag / reserved
    w.write_bits(p.level_idc as u32, 8);
    // sps_max_sub_layers_minus1 == 0: no sub-layer flags
}

/// general_profile_compatibility_flag[j] as a 32-bit word (flag 0 is the MSB). A Main stream is also
/// Main 10 compatible.
fn compat_flags(profile_idc: u8) -> u32 {
    let mut f = 1u32 << (31 - profile_idc as u32);
    if profile_idc == 1 {
        f |= 1 << (31 - 2);
    }
    f
}

pub fn vps(p: &SeqParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // vps_video_parameter_set_id
    w.write_bits(1, 1); // vps_base_layer_internal_flag
    w.write_bits(1, 1); // vps_base_layer_available_flag
    w.write_bits(0, 6); // vps_max_layers_minus1
    w.write_bits(0, 3); // vps_max_sub_layers_minus1
    w.write_bits(1, 1); // vps_temporal_id_nesting_flag
    w.write_bits(0xffff, 16);
    profile_tier_level(&mut w, p);
    w.write_bits(1, 1); // vps_sub_layer_ordering_info_present_flag
    w.write_ue(1); // vps_max_dec_pic_buffering_minus1
    w.write_ue(0); // vps_max_num_reorder_pics
    w.write_ue(0); // vps_max_latency_increase_plus1
    w.write_bits(0, 6); // vps_max_layer_id
    w.write_ue(0); // vps_num_layer_sets_minus1
    w.write_bits(0, 1); // vps_timing_info_present_flag
    w.write_bits(0, 1); // vps_extension_flag
    w.rbsp_trailing();
    nal_unit(NAL_VPS, &w.finish())
}

pub fn sps(p: &SeqParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // sps_video_parameter_set_id
    w.write_bits(0, 3); // sps_max_sub_layers_minus1
    w.write_bits(1, 1); // sps_temporal_id_nesting_flag
    profile_tier_level(&mut w, p);
    w.write_ue(0); // sps_seq_parameter_set_id
    w.write_ue(1); // chroma_format_idc 4:2:0
    w.write_ue(p.coded_width);
    w.write_ue(p.coded_height);
    let crop = p.coded_width != p.width || p.coded_height != p.height;
    w.write_bit(crop); // conformance_window_flag
    if crop {
        // offsets in chroma sample units (SubWidthC = SubHeightC = 2)
        w.write_ue(0);
        w.write_ue((p.coded_width - p.width) / 2);
        w.write_ue(0);
        w.write_ue((p.coded_height - p.height) / 2);
    }
    w.write_ue(p.bit_depth - 8); // bit_depth_luma_minus8
    w.write_ue(p.bit_depth - 8); // bit_depth_chroma_minus8
    w.write_ue(LOG2_MAX_POC_LSB - 4);
    w.write_bits(1, 1); // sps_sub_layer_ordering_info_present_flag
    w.write_ue(1); // sps_max_dec_pic_buffering_minus1
    w.write_ue(0); // sps_max_num_reorder_pics
    w.write_ue(0); // sps_max_latency_increase_plus1
    w.write_ue(LOG2_MIN_CB - 3);
    w.write_ue(LOG2_CTB - LOG2_MIN_CB);
    w.write_ue(LOG2_MIN_TB - 2);
    w.write_ue(LOG2_MAX_TB - LOG2_MIN_TB);
    w.write_ue(0); // max_transform_hierarchy_depth_inter
    w.write_ue(0); // max_transform_hierarchy_depth_intra
    w.write_bits(0, 1); // scaling_list_enabled_flag
    w.write_bits(0, 1); // amp_enabled_flag
    w.write_bits(0, 1); // sample_adaptive_offset_enabled_flag
    w.write_bits(0, 1); // pcm_enabled_flag
    w.write_ue(1); // num_short_term_ref_pic_sets
    // st_ref_pic_set(0): one negative picture at delta POC -1, used by the current picture
    w.write_ue(1); // num_negative_pics
    w.write_ue(0); // num_positive_pics
    w.write_ue(0); // delta_poc_s0_minus1
    w.write_bits(1, 1); // used_by_curr_pic_s0_flag
    w.write_bits(0, 1); // long_term_ref_pics_present_flag
    w.write_bits(0, 1); // sps_temporal_mvp_enabled_flag
    w.write_bits(1, 1); // strong_intra_smoothing_enabled_flag
    w.write_bits(1, 1); // vui_parameters_present_flag
    vui(&mut w, p);
    w.write_bits(0, 1); // sps_extension_present_flag
    w.rbsp_trailing();
    nal_unit(NAL_SPS, &w.finish())
}

fn vui(w: &mut BitWriter, p: &SeqParams) {
    w.write_bits(0, 1); // aspect_ratio_info_present_flag
    w.write_bits(0, 1); // overscan_info_present_flag
    w.write_bits(1, 1); // video_signal_type_present_flag
    w.write_bits(5, 3); // video_format: unspecified
    w.write_bit(p.full_range);
    w.write_bits(1, 1); // colour_description_present_flag
    w.write_bits(p.colour_primaries as u32, 8); // colour_primaries
    w.write_bits(p.transfer_characteristics as u32, 8); // transfer_characteristics
    w.write_bits(p.matrix_coeffs as u32, 8); // matrix_coeffs
    w.write_bits(0, 1); // chroma_loc_info_present_flag
    w.write_bits(0, 1); // neutral_chroma_indication_flag
    w.write_bits(0, 1); // field_seq_flag
    w.write_bits(0, 1); // frame_field_info_present_flag
    w.write_bits(0, 1); // default_display_window_flag
    w.write_bits(1, 1); // vui_timing_info_present_flag
    w.write_bits(p.fps_den, 32); // vui_num_units_in_tick
    w.write_bits(p.fps_num, 32); // vui_time_scale
    w.write_bits(0, 1); // vui_poc_proportional_to_timing_flag
    w.write_bits(0, 1); // vui_hrd_parameters_present_flag
    w.write_bits(0, 1); // bitstream_restriction_flag
}

pub fn pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // pps_pic_parameter_set_id
    w.write_ue(0); // pps_seq_parameter_set_id
    w.write_bits(0, 1); // dependent_slice_segments_enabled_flag
    w.write_bits(0, 1); // output_flag_present_flag
    w.write_bits(0, 3); // num_extra_slice_header_bits
    w.write_bits(1, 1); // sign_data_hiding_enabled_flag
    w.write_bits(0, 1); // cabac_init_present_flag
    w.write_ue(0); // num_ref_idx_l0_default_active_minus1
    w.write_ue(0); // num_ref_idx_l1_default_active_minus1
    w.write_se(0); // init_qp_minus26
    w.write_bits(0, 1); // constrained_intra_pred_flag
    w.write_bits(0, 1); // transform_skip_enabled_flag
    w.write_bits(0, 1); // cu_qp_delta_enabled_flag
    w.write_se(0); // pps_cb_qp_offset
    w.write_se(0); // pps_cr_qp_offset
    w.write_bits(0, 1); // pps_slice_chroma_qp_offsets_present_flag
    w.write_bits(0, 1); // weighted_pred_flag
    w.write_bits(0, 1); // weighted_bipred_flag
    w.write_bits(0, 1); // transquant_bypass_enabled_flag
    w.write_bits(0, 1); // tiles_enabled_flag
    w.write_bits(0, 1); // entropy_coding_sync_enabled_flag
    w.write_bits(0, 1); // pps_loop_filter_across_slices_enabled_flag
    w.write_bits(1, 1); // deblocking_filter_control_present_flag
    w.write_bits(0, 1); // deblocking_filter_override_enabled_flag
    w.write_bits(0, 1); // pps_deblocking_filter_disabled_flag
    w.write_se(0); // pps_beta_offset_div2
    w.write_se(0); // pps_tc_offset_div2
    w.write_bits(0, 1); // pps_scaling_list_data_present_flag
    w.write_bits(0, 1); // lists_modification_present_flag
    w.write_ue(0); // log2_parallel_merge_level_minus2
    w.write_bits(0, 1); // slice_segment_header_extension_present_flag
    w.write_bits(0, 1); // pps_extension_present_flag
    w.rbsp_trailing();
    nal_unit(NAL_PPS, &w.finish())
}

/// HEVCDecoderConfigurationRecord with one VPS, SPS and PPS (lengthSizeMinusOne = 3).
pub fn hvcc(p: &SeqParams, sets: &[Vec<u8>; 3]) -> Vec<u8> {
    HevcConfig {
        general_profile_space: 0,
        general_tier_flag: false,
        general_profile_idc: p.profile_idc,
        general_profile_compatibility_flags: compat_flags(p.profile_idc),
        general_constraint_indicator_flags: 0x9000_0000_0000,
        general_level_idc: p.level_idc,
        min_spatial_segmentation_idc: 0,
        parallelism_type: 0,
        chroma_format_idc: 1,
        bit_depth_luma: p.bit_depth as u8,
        bit_depth_chroma: p.bit_depth as u8,
        avg_frame_rate: 0,
        constant_frame_rate: 0,
        num_temporal_layers: 1,
        temporal_id_nested: true,
        length_size: 4,
        arrays: sets
            .iter()
            .zip([NAL_VPS, NAL_SPS, NAL_PPS])
            .map(|(set, nal_type)| HevcNalArray { completeness: true, nal_type, nalus: vec![set.clone()] })
            .collect(),
    }
    .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(choose_level(1920, 1080, 30, 1, None), Some(120));
        assert_eq!(choose_level(1920, 1080, 60, 1, None), Some(123));
        assert_eq!(choose_level(3840, 2160, 30, 1, None), Some(150));
        assert_eq!(choose_level(1280, 720, 30, 1, None), Some(93));
        assert_eq!(choose_level(64, 64, 30, 1, None), Some(30));
        // 8192x8192 is 67 Mi luma samples, over the 35 Mi of level 6.x; so is a rate beyond level 6.2.
        assert_eq!(choose_level(8192, 8192, 30, 1, None), None);
        assert_eq!(choose_level(8192, 4320, 300, 1, None), None);
        assert_eq!(choose_level(1920, 1080, 30, 1, Some(240_001)), None);
        assert_eq!(choose_level(8192, 4320, 30, 1, None), Some(180));
        assert_eq!(choose_level(1920, 1080, 30, 1, Some(15_000)), Some(123));
    }
}

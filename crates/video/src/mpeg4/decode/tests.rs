//! Unit tests for the clean-room MPEG-4 Part 2 decoder.

use craft_media::bitstream::{BitReader, BitWriter};

use super::headers::{GmcConfig, VolConfig};
use super::headers::{
    VopType, intra_dc_threshold_qp, is_packed_userdata, parse_visual_object, parse_vol, parse_vop_header, scan_start_codes,
    try_parse_short_video_header,
};
use super::mc::{MbMeta, Plane, chroma_mv_1mv, chroma_mv_4mv, mv_predictor, predict_qpel};
use super::picture::{dc_scaler, dequant_intra, idct};
use super::vlc::{
    ALT_HORIZONTAL_SCAN, ALT_VERTICAL_SCAN, BMbType, CBPY, DC_SIZE_CHROMA, DC_SIZE_LUMA, ESCAPE, MCBPC_INTER, MCBPC_INTRA, MVD_TABLE, ZIGZAG,
    inter_tcoef, intra_tcoef, read_b_mb_type, read_cbpy, read_dc_diff, read_mcbpc_inter, read_mcbpc_intra, read_mv_component,
};
use super::{DecodeError, Decoder};

#[test]
fn scan_tables_are_permutations_and_transpose() {
    for (name, table) in [("zigzag", &ZIGZAG), ("alt_horizontal", &ALT_HORIZONTAL_SCAN), ("alt_vertical", &ALT_VERTICAL_SCAN)] {
        let mut seen = [false; 64];
        for &pos in table {
            assert!(pos < 64, "{name}: out of range index {pos}");
            seen[pos] = true;
        }
        assert!(seen.iter().all(|&s| s), "{name}: not a complete permutation of 0..64");
    }
    for i in 0..64 {
        let h = ALT_HORIZONTAL_SCAN[i];
        let v = ALT_VERTICAL_SCAN[i];
        let (hx, hy) = (h % 8, h / 8);
        let (vx, vy) = (v % 8, v / 8);
        assert_eq!((vx, vy), (hy, hx), "alt_vertical is the transpose of alt_horizontal at index {i}");
    }
}

#[test]
fn mcbpc_and_cbpy_vlc_roundtrip_with_stuffing() {
    for (idx, &(code, len)) in MCBPC_INTRA.iter().enumerate() {
        let mut w = BitWriter::new();
        w.write_bits(1, 9); // stuffing code 0000 0000 1
        w.write_bits(u32::from(code), u32::from(len));
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        let (mb_type, cbpc) = read_mcbpc_intra(&mut r).expect("decode intra mcbpc");
        assert_eq!((mb_type, cbpc), (3 + (idx as u8 / 4), idx as u8 % 4));
    }

    for (idx, &(code, len)) in MCBPC_INTER.iter().enumerate() {
        let mut w = BitWriter::new();
        w.write_bits(1, 9);
        w.write_bits(u32::from(code), u32::from(len));
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(read_mcbpc_inter(&mut r).expect("stuffing"), None, "stuffing hands control back to the macroblock header");
        let (mb_type, cbpc) = read_mcbpc_inter(&mut r).expect("decode inter mcbpc").expect("not a stuffing code");
        assert_eq!((mb_type, cbpc), (idx as u8 / 4, idx as u8 % 4));
    }

    for (pat, &(code, len)) in CBPY.iter().enumerate() {
        let mut w = BitWriter::new();
        w.write_bits(u32::from(code), u32::from(len));
        let bytes = w.finish();
        let mut r_intra = BitReader::new(&bytes);
        assert_eq!(read_cbpy(&mut r_intra, true).expect("intra cbpy"), pat as u8);
        let mut r_inter = BitReader::new(&bytes);
        assert_eq!(read_cbpy(&mut r_inter, false).expect("inter cbpy"), (pat as u8) ^ 0xF);
    }
}

#[test]
fn dc_differential_and_mvd_roundtrip() {
    for luma in [true, false] {
        let table = if luma { &DC_SIZE_LUMA } else { &DC_SIZE_CHROMA };
        for diff in [-2047i32, -512, -255, -7, -1, 0, 1, 7, 255, 512, 2047] {
            let mut w = BitWriter::new();
            let size = 32 - diff.unsigned_abs().leading_zeros();
            let (code, len) = table[size as usize];
            w.write_bits(u32::from(code), u32::from(len));
            if size > 0 {
                let bits = if diff > 0 { diff } else { diff + (1 << size) - 1 };
                w.write_bits(bits as u32, size);
                if size > 8 {
                    w.write_bit(true);
                }
            }
            let bytes = w.finish();
            let mut r = BitReader::new(&bytes);
            assert_eq!(read_dc_diff(&mut r, luma).expect("read dc diff"), diff);
        }
    }

    // MVD with fcode = 2: range [-64, 63]
    let mut w = BitWriter::new();
    // code = 3, sign = 0 (positive), residual = 1 (1 bit for fcode 2) -> mag = ((3-1)<<1) + 1 + 1 = 6
    let (code, len) = MVD_TABLE[3];
    w.write_bits(u32::from(code), u32::from(len));
    w.write_bit(false);
    w.write_bits(1, 1);
    let bytes = w.finish();
    let mut r = BitReader::new(&bytes);
    assert_eq!(read_mv_component(&mut r, 10, 2).expect("read mv"), 16);
}

#[test]
fn b_mb_type_and_tcoef_escape_modes_decode() {
    let mut w = BitWriter::new();
    w.write_bit(true); // Direct
    w.write_bits(0b01, 2); // Interpolate
    w.write_bits(0b001, 3); // Backward
    w.write_bits(0b0001, 4); // Forward
    let bytes = w.finish();
    let mut r = BitReader::new(&bytes);
    assert_eq!(read_b_mb_type(&mut r).expect("direct"), BMbType::Direct);
    assert_eq!(read_b_mb_type(&mut r).expect("interp"), BMbType::Interpolate);
    assert_eq!(read_b_mb_type(&mut r).expect("bwd"), BMbType::Backward);
    assert_eq!(read_b_mb_type(&mut r).expect("fwd"), BMbType::Forward);

    // Escape mode 3: `0000 011` + `11` + last(1) + run(6 = 5) + marker(1) + level(12 = -300) + marker(1)
    let mut w = BitWriter::new();
    w.write_bits(u32::from(ESCAPE.0), u32::from(ESCAPE.1));
    w.write_bits(0b11, 2);
    w.write_bit(true);
    w.write_bits(5, 6);
    w.write_bit(true);
    w.write_bits((-300i32 as u32) & 0xFFF, 12);
    w.write_bit(true);
    let bytes = w.finish();
    let mut r = BitReader::new(&bytes);
    assert_eq!(intra_tcoef().read_event(&mut r, false).expect("escape 3"), (true, 5, -300));
    let mut r2 = BitReader::new(&bytes);
    assert_eq!(inter_tcoef().read_event(&mut r2, false).expect("inter escape 3"), (true, 5, -300));
}

#[test]
fn chroma_4mv_and_spatial_predictor_respect_slices() {
    // 4 luma vectors summing to 16 in x (+2 half-pel) and -15 in y (-2 half-pel per Table 7-6)
    let mvs = [(4, -4), (4, -4), (4, -4), (4, -3)];
    assert_eq!(chroma_mv_4mv(&mvs, false), (2, -2));
    assert_eq!(chroma_mv_1mv((4, -2), true), (1, -1));

    let mut mbs = vec![MbMeta::default(); 4];
    mbs[0] = MbMeta { not_coded: false, intra: false, mvs: [(6, -4); 4], slice_id: 0 };
    mbs[1] = MbMeta { not_coded: false, intra: false, mvs: [(10, 2); 4], slice_id: 1 };
    // Macroblock 1 is in slice_id 1, so left neighbour (mb 0 in slice_id 0) is excluded -> (0, 0).
    assert_eq!(mv_predictor(&mbs, (2, 2), (1, 0), 0, 1), (0, 0));
    // If in slice_id 0, only left neighbour (mb 0) is inside VOP -> predictor is single valid neighbour (6, -4).
    assert_eq!(mv_predictor(&mbs, (2, 2), (1, 0), 0, 0), (6, -4));
}

#[test]
fn qpel_prediction_preserves_flat_plane() {
    let plane = Plane::new(32, 32, 77);
    let mut out = [0u8; 256];
    for qx in 0..4 {
        for qy in 0..4 {
            predict_qpel(&plane, 4, 4, (qx, qy), 16, false, &mut out);
            assert!(out.iter().all(|&v| v == 77), "flat plane at qpel ({qx}, {qy})");
        }
    }
}

#[test]
fn short_video_header_and_packed_userdata_parse() {
    assert!(is_packed_userdata(b"DivX503b1393p\0"));
    assert!(!is_packed_userdata(b"DivX503b1393n\0"));
    assert_eq!(intra_dc_threshold_qp(0), 99);
    assert_eq!(intra_dc_threshold_qp(1), 13);
    assert_eq!(intra_dc_threshold_qp(7), 1);

    // Build an H.263 short video header for sub-QCIF (128x96, format 1), I-picture, quant 5.
    let mut w = BitWriter::new();
    w.write_bits(0x20, 22); // short_video_start_marker
    w.write_bits(7, 8); // temporal_reference
    w.write_bit(true); // marker
    w.write_bit(false); // zero
    w.write_bit(false); // split_screen
    w.write_bit(false); // doc_camera
    w.write_bit(false); // freeze_release
    w.write_bits(1, 3); // source_format = 1 (128x96)
    w.write_bit(false); // I-picture
    w.write_bits(0, 4); // reserved zeros
    w.write_bits(5, 5); // quant = 5
    w.write_bit(false); // cpm = 0
    w.write_bit(false); // pei = 0
    w.write_bits(0, 16); // padding
    let bytes = w.finish();
    let mut r = BitReader::new(&bytes);
    let (vol, vop) = try_parse_short_video_header(&mut r).expect("parse").expect("short header");
    assert_eq!((vol.width, vol.height), (128, 96));
    assert_eq!(vop.coding_type, VopType::I);
    assert_eq!(vop.quant, 5);
}

#[test]
fn hostile_and_malformed_inputs_return_errors_never_panic() {
    let mut dec = Decoder::new();
    assert!(dec.decode(&[], 0).expect("empty packet is ok").is_empty());
    assert!(dec.decode(&[0xde, 0xad, 0xbe, 0xef], 0).is_err());
    // VOP before VOL
    assert!(dec.decode(&[0, 0, 1, 0xB6, 0, 0], 0).is_err());
    // Configuration holding a VOP start code must be rejected
    assert!(dec.configure(&[0, 0, 1, 0xB6, 0]).is_err());

    let _ =
        (dc_scaler(8, true), idct(&mut [0; 64]), scan_start_codes(&[0, 0, 1, 0xB0, 3]), parse_visual_object(&[0]), parse_vol(&[0]), parse_vop_header);
    let _ = DecodeError::Invalid("test".into());
}

fn plain_vol(interlaced: bool) -> VolConfig {
    VolConfig {
        width: 64,
        height: 48,
        sar: (1, 1),
        time_increment_resolution: 30,
        fixed_time_increment: None,
        low_delay: true,
        interlaced,
        quant_type: true,
        intra_quant_mat: [255; 64],
        nonintra_quant_mat: [255; 64],
        quarter_sample: false,
        resync_marker_disable: false,
        gmc: None::<GmcConfig>,
        short_header: false,
    }
}

#[test]
fn matrix_dequantisation_of_extreme_levels_saturates() {
    let vol = plain_vol(false);
    let mut levels = [0i32; 64];
    levels[1..].fill(i32::MAX / 4);
    let mut deq = [0i32; 64];
    dequant_intra(&levels, 31, 8, &vol, &mut deq);
    assert!(deq[1..].iter().all(|&c| c == 2047));
}

#[test]
fn intra_dc_threshold_precedes_the_interlace_flags() {
    let vol = plain_vol(true);
    let mut w = BitWriter::new();
    w.write_bits(0, 2); // I-VOP
    w.write_bit(false); // modulo_time_base end
    w.write_bit(true); // marker
    w.write_bits(3, 5); // vop_time_increment
    w.write_bit(true); // marker
    w.write_bit(true); // vop_coded
    w.write_bits(0, 3); // intra_dc_vlc_thr: DC VLC for the whole VOP
    w.write_bit(true); // top_field_first
    w.write_bit(false); // alternate_vertical_scan
    w.write_bits(5, 5); // vop_quant
    w.write_bits(0, 16);
    let bytes = w.finish();
    let mut r = BitReader::new(&bytes);
    let vop = parse_vop_header(&mut r, &vol).expect("header");
    assert_eq!(vop.intra_dc_vlc_thr_qp, intra_dc_threshold_qp(0));
    assert!(vop.top_field_first);
    assert!(!vop.alternate_vertical_scan);
    assert_eq!(vop.quant, 5);
}

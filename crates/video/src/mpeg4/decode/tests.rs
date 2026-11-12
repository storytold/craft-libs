//! Robustness tests that need no oracle.

use super::*;

#[test]
fn garbage_never_panics() {
    let mut x = 0x9E37_79B9u32;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for len in [0usize, 1, 3, 4, 5, 17, 100, 1000] {
        for _ in 0..50 {
            let mut data: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            if len > 8 {
                data[..4].copy_from_slice(&[0, 0, 1, 0xB6]);
            }
            let mut d = Decoder::new();
            let _ = d.decode(&data, 0);
            let _ = d.flush();
        }
    }
}

#[test]
fn start_codes_are_found() {
    let data = [0, 0, 1, 0xB0, 3, 0, 0, 0, 1, 0xB6, 0, 0, 1];
    assert_eq!(start_codes(&data), vec![(0, 0xB0), (6, 0xB6)]);
}

#[test]
fn vop_before_vol_is_an_error() {
    let mut d = Decoder::new();
    assert!(d.decode(&[0, 0, 1, 0xB6, 0x10, 0x40], 0).is_err());
    assert_eq!(d.errors(), 1);
}

/// Direct mode with hostile time stamps: the temporal distances reach ±2^20 and a vector ±2048,
/// whose products exceed `i32` (an overflow panic in debug builds before they were widened). The
/// vectors that come out are bounded, so the chrominance vector of four of them cannot overflow
/// either.
#[test]
fn direct_mode_scaling_of_extreme_distances_does_not_overflow() {
    use super::picture::direct_scale;
    for (v, trb, trd) in [(-2048, 1 << 20, -1), (2048, 1 << 20, 1), (-2048, -(1 << 20), 1), (2047, 1 << 20, -(1 << 20))] {
        for d in [0, 5] {
            let [f, b] = direct_scale(v, d, trb, trd);
            for q in [false, true] {
                let c = super::mc::chroma_mv_4(&[[f, b]; 4], q);
                assert!(c.iter().all(|x| x.unsigned_abs() <= 1 << 17), "{c:?}");
            }
            let sat = |x: i64| x.clamp(-(1 << 16), 1 << 16);
            let (v64, trb64, trd64) = (i64::from(v), i64::from(trb), i64::from(trd));
            let exact_f = v64 * trb64 / trd64 + i64::from(d);
            let exact_b = if d == 0 { v64 * (trb64 - trd64) / trd64 } else { exact_f - v64 };
            assert_eq!(i64::from(f), sat(exact_f), "{v} {d} {trb} {trd}");
            assert_eq!(i64::from(b), sat(exact_b), "{v} {d} {trb} {trd}");
        }
    }
    // Ordinary distances keep the i32 results.
    assert_eq!(direct_scale(7, 0, 1, 3), [7 / 3, 7 * (1 - 3) / 3]);
    assert_eq!(direct_scale(-9, 2, 2, 4), [-9 * 2 / 4 + 2, -9 * 2 / 4 + 2 + 9]);
}

/// A version 1 VOL of 32×32 with B-VOPs (`low_delay` 0), no fixed VOP rate and
/// `vop_time_increment_resolution` 25, data partitioned (which is unsupported) or not.
fn variable_rate_vol(data_partitioned: bool) -> Vec<u8> {
    use craft_media::bitstream::BitWriter;
    let mut w = BitWriter::new();
    w.write_bits(0x120, 32);
    w.write_bit(false); // random_accessible_vol
    w.write_bits(17, 8); // Advanced Simple
    w.write_bit(false); // is_object_layer_identifier
    w.write_bits(1, 4); // square pixels
    w.write_bit(true); // vol_control_parameters
    w.write_bits(1, 2); // 4:2:0
    w.write_bit(false); // low_delay
    w.write_bit(false); // vbv_parameters
    w.write_bits(0, 2); // rectangular
    w.write_bit(true);
    w.write_bits(25, 16);
    w.write_bit(true);
    w.write_bit(false); // fixed_vop_rate
    w.write_bit(true);
    w.write_bits(32, 13);
    w.write_bit(true);
    w.write_bits(32, 13);
    w.write_bit(true);
    w.write_bit(false); // interlaced
    w.write_bit(true); // obmc_disable
    w.write_bit(false); // sprite_enable
    w.write_bit(false); // not_8_bit
    w.write_bit(false); // quant_type
    w.write_bit(true); // complexity_estimation_disable
    w.write_bit(true); // resync_marker_disable
    w.write_bit(data_partitioned);
    if data_partitioned {
        w.write_bit(false); // reversible_vlc
    }
    w.write_bit(false); // scalability
    w.write_bit(false);
    while !w.is_byte_aligned() {
        w.write_bit(true);
    }
    w.finish()
}

/// A B-VOP at time increment 3 of [`variable_rate_vol`], with no macroblock data.
fn b_vop() -> Vec<u8> {
    use craft_media::bitstream::BitWriter;
    let mut w = BitWriter::new();
    w.write_bits(0x1B6, 32);
    w.write_bits(2, 2); // B
    w.write_bit(false); // modulo_time_base
    w.write_bit(true);
    w.write_bits(3, 5); // vop_time_increment
    w.write_bit(true);
    w.write_bit(true); // vop_coded
    w.write_bits(0, 3); // intra_dc_vlc_thr
    w.write_bits(4, 5); // vop_quant
    w.write_bits(1, 3); // vop_fcode_forward
    w.write_bits(1, 3); // vop_fcode_backward
    w.write_bits(0, 16);
    w.finish()
}

/// A B-VOP without references (an open GOP's at the start of decoding) is dropped and must not set
/// the frame period estimate that field direct mode uses later.
#[test]
fn an_orphan_b_vop_does_not_set_the_frame_period() {
    let mut d = Decoder::new();
    let pics = d.decode(&[variable_rate_vol(false), b_vop()].concat(), 0).expect("headers and a dropped B-VOP");
    assert!(pics.is_empty());
    assert!(d.vol.as_ref().is_some_and(|v| v.fixed_increment.is_none()));
    assert_eq!(d.t_frame, 0);
}

/// A layer header the decoder cannot decode ends the old layer: the VOPs that follow it are not
/// decoded with the old configuration, and fail with the reason the layer was refused.
#[test]
fn an_unsupported_layer_replaces_the_old_one() {
    let mut d = Decoder::new();
    d.decode(&[variable_rate_vol(false), b_vop()].concat(), 0).expect("a supported layer");
    assert!(d.info().is_some());
    assert!(matches!(d.decode(&variable_rate_vol(true), 1), Err(DecodeError::Unsupported(_))));
    assert!(d.info().is_none());
    assert!(matches!(d.decode(&b_vop(), 2), Err(DecodeError::Unsupported(_))), "a VOP after the unsupported layer");
    // A decodable layer ends the refusal.
    d.decode(&[variable_rate_vol(false), b_vop()].concat(), 3).expect("a supported layer again");
    assert!(d.info().is_some());
}

/// A refused layer and its VOPs in one chunk: `last_error` names the refusal, not the VOP.
#[test]
fn the_refusal_of_a_layer_stays_the_last_error() {
    let mut d = Decoder::new();
    assert!(d.decode(&[variable_rate_vol(true), b_vop(), b_vop()].concat(), 0).is_err());
    assert!(matches!(d.last_error(), Some(DecodeError::Unsupported(_))), "{:?}", d.last_error());
    assert_eq!(d.errors(), 3);
}

/// The chrominance vector of four extreme luminance vectors (more than direct mode now gives)
/// overflowed the `i32` sum.
#[test]
fn chroma_vector_of_four_extreme_vectors_does_not_overflow() {
    for q in [false, true] {
        let c = super::mc::chroma_mv_4(&[[i32::MAX, i32::MIN]; 4], q);
        assert!(c[0] > 0 && c[1] < 0, "{c:?}");
    }
}

/// Direct mode's chrominance vector comes from the exact luminance vectors: bounding each first
/// would let vectors of opposite signs cancel differently. Forward vectors 2^20, 2^20, -2^19 and
/// -2^19 (co-located 1024, 1024, -512, -512 with TRB 1024 and TRD 1) sum to 2^20, a chrominance
/// vector far past the picture (bounded to 2^16 for sampling); bounded first, they sum to 0.
#[test]
fn direct_chroma_vector_of_mixed_signs_uses_the_exact_vectors() {
    use super::picture::direct_four;
    let mvs = [[1024, 0], [1024, 0], [-512, 0], [-512, 0]];
    let [(fwd, chroma), _] = direct_four(&mvs, [0, 0], 1024, 1, false);
    assert_eq!(fwd, [[1 << 16, 0], [1 << 16, 0], [-(1 << 16), 0], [-(1 << 16), 0]]);
    assert_eq!(chroma, [1 << 16, 0]);
    // Ordinary vectors give what the four-vector rule gives.
    let mvs = [[6, -3], [5, 2], [-4, 7], [3, 1]];
    for q in [false, true] {
        let [(fwd, chroma), (bwd, back)] = direct_four(&mvs, [1, -1], 1, 2, q);
        assert_eq!(chroma, super::mc::chroma_mv_4(&fwd, q));
        assert_eq!(back, super::mc::chroma_mv_4(&bwd, q));
    }
}

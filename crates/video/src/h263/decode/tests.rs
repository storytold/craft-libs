//! Unit tests for the parts of the H.263 decoder that the ffmpeg oracle cannot reach alone.

use craft_media::bitstream::{BitReader, BitWriter};

use super::picture::{MCBPC_INTER, filter_edge, read_mcbpc_inter, read_table_d3};

/// Table D.3 as the spec builds it: the bits of `|v|` after the leading one, each followed by a
/// continuation bit, then the sign and a final zero.
fn write_d3(w: &mut BitWriter, v: i32) {
    if v == 0 {
        w.write_bit(true);
        return;
    }
    w.write_bit(false);
    let magnitude = v.unsigned_abs();
    let top = 31 - magnitude.leading_zeros();
    for bit in (0..top).rev() {
        w.write_bit(magnitude >> bit & 1 == 1);
        w.write_bit(true);
    }
    w.write_bit(v < 0);
    w.write_bit(false);
}

#[test]
fn table_d3_round_trips_every_difference_in_range() {
    for v in -4095..=4095 {
        let mut w = BitWriter::new();
        write_d3(&mut w, v);
        w.write_bits(0xA5, 8);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(read_table_d3(&mut r).expect("decode"), v);
        assert_eq!(r.read_bits(8).expect("what follows"), 0xA5, "the code ends where the spec says for {v}");
    }
}

#[test]
fn table_d3_refuses_a_code_that_never_ends() {
    let mut bytes = [0xFFu8; 8];
    bytes[0] = 0x7F; // a leading zero, then only continuation bits
    assert!(read_table_d3(&mut BitReader::new(&bytes)).is_err());
}

#[test]
fn p_picture_mcbpc_codes_decode_to_their_own_index_and_are_prefix_free() {
    for (index, &(code, len)) in MCBPC_INTER.iter().enumerate() {
        let mut w = BitWriter::new();
        w.write_bits(u32::from(code), u32::from(len));
        w.write_bits(0xFFFF, 16);
        let bytes = w.finish();
        let decoded = read_mcbpc_inter(&mut BitReader::new(&bytes)).expect("decode").expect("not stuffing");
        assert_eq!((decoded.0 * 4 + decoded.1) as usize, index);
        for (other, &(c, l)) in MCBPC_INTER.iter().enumerate().filter(|&(o, _)| o != index) {
            let shorter = l.min(len);
            assert!(u32::from(c) >> (l - shorter) != u32::from(code) >> (len - shorter), "code {other} is a prefix of code {index}");
        }
    }
}

#[test]
fn deblocking_smooths_small_steps_and_keeps_real_edges() {
    let strength = 4;
    // A small step across the edge is pulled together; a flat edge does not change.
    let mut small = [100u8, 100, 106, 106];
    filter_edge(&mut small, 2, 1, strength);
    assert!(small[2] - small[1] < 6, "{small:?}");
    let mut flat = [90u8; 4];
    filter_edge(&mut flat, 2, 1, strength);
    assert_eq!(flat, [90; 4]);
    // A step of more than twice the strength is a real edge.
    let mut real = [20u8, 20, 200, 200];
    filter_edge(&mut real, 2, 1, strength);
    assert_eq!(real, [20, 20, 200, 200]);
    // The result stays in range.
    let mut bright = [250u8, 255, 255, 250];
    filter_edge(&mut bright, 2, 1, 12);
    assert!(bright.iter().all(|&s| s >= 240));
}

//! Out-of-range arguments to the public constructors are errors or clamped, never arithmetic
//! overflow (a panic in a debug build).

use craft_video::apv::*;

#[test]
fn encoder_config_for_absurd_dimensions_does_not_overflow() {
    for (w, h) in [(u32::MAX, u32::MAX), (1 << 31, 1 << 31), (u32::MAX, 1), (0, 0)] {
        let cfg = EncoderConfig::new(Profile::P4444_12, w, h);
        assert!(Encoder::with_config(cfg).is_err(), "{w}x{h}");
    }
}

#[test]
fn frame_with_an_absurd_bit_depth_does_not_overflow() {
    for depth in [0, 7, 17, 31, 32, 40, 255] {
        let f = Frame::new(2, 2, ChromaFormat::Yuv4444, depth, true);
        assert_eq!((f.bit_depth, f.y.len(), f.alpha.map(|a| a.len())), (depth, 4, Some(4)));
    }
    assert_eq!(Frame::new(1, 1, ChromaFormat::Yuv422, 10, true).alpha, Some(vec![1023]));
}

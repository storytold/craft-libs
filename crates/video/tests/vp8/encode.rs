//! The key-frame encoder against the decoder beside it: every frame decodes, at the size given,
//! and the picture comes back closer the higher the quality.

use craft_video::vp8::Decoder;
use craft_video::vp8::encode::{MAX_DIMENSION, Params, Vp8Error, Yuv, encode_frame};

/// A test picture with gradients, edges and texture.
fn picture(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let edge = if (x / 7 + y / 5) % 2 == 0 { 40 } else { 0 };
            rgb.extend_from_slice(&[(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, ((x * 13 + y * 7) % 200) as u8 + edge]);
        }
    }
    rgb
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / a.len() as f64;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}

#[test]
fn frames_decode_and_quality_raises_fidelity() {
    for (w, h) in [(16, 16), (20, 13), (1, 1), (33, 47), (128, 72)] {
        let yuv = Yuv::from_rgb(&picture(w, h), w, h, 3).unwrap();
        let mut last = 0.0;
        for quality in [10u8, 50, 75, 90, 100] {
            let frame = encode_frame(&yuv, Params { quality }).unwrap();
            let pic = Decoder::new().decode(&frame, 7).unwrap().expect("a shown frame");
            assert_eq!((pic.width, pic.height, pic.key, pic.pts), (w, h, true, 7), "{w}x{h} q{quality}");
            assert_eq!((pic.y.len(), pic.u.len(), pic.v.len()), (yuv.y.len(), yuv.u.len(), yuv.v.len()));
            let p = psnr(&pic.y, &yuv.y);
            assert!(p + 0.5 >= last, "{w}x{h}: quality {quality} gave {p:.1} dB after {last:.1}");
            last = p;
        }
        assert!(last > 40.0, "{w}x{h}: quality 100 gave {last:.1} dB");
    }
}

#[test]
fn flat_pictures_skip_their_macroblocks_and_stay_small() {
    let rgb = vec![90u8; 64 * 64 * 3];
    let yuv = Yuv::from_rgb(&rgb, 64, 64, 3).unwrap();
    let frame = encode_frame(&yuv, Params { quality: 80 }).unwrap();
    assert!(frame.len() < 200, "{} bytes", frame.len());
    let pic = Decoder::new().decode(&frame, 0).unwrap().unwrap();
    assert!(pic.y.iter().all(|&v| v.abs_diff(yuv.y[0]) <= 1));
}

#[test]
fn the_largest_dimension_is_accepted_and_one_more_refused() {
    let thin = Yuv {
        width: MAX_DIMENSION,
        height: 1,
        y: vec![16; MAX_DIMENSION as usize],
        u: vec![128; MAX_DIMENSION.div_ceil(2) as usize],
        v: vec![128; MAX_DIMENSION.div_ceil(2) as usize],
    };
    let frame = encode_frame(&thin, Params { quality: 50 }).unwrap();
    assert_eq!(Decoder::new().decode(&frame, 0).unwrap().unwrap().width, MAX_DIMENSION);
    let wide = Yuv { width: MAX_DIMENSION + 1, height: 1, y: vec![16; MAX_DIMENSION as usize + 1], u: vec![128; 8192], v: vec![128; 8192] };
    assert_eq!(encode_frame(&wide, Params { quality: 50 }).err(), Some(Vp8Error::Dimensions));
}

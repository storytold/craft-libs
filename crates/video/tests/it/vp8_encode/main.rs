//! The VP8 encoder's frames read back by this crate's own decoder: the decode must reproduce the
//! encoder's reconstruction exactly (loop filter off) and read as a picture close to the source.

use craft_video::vp8::{Decoder, Picture, Vp8Frame, Vp8Params, encode_rgba};

/// A test picture with something for every mode: flat areas, gradients, hard edges, diagonal
/// stripes, fine noise, colour.
fn picture(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761) | 1;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for py in 0..h {
        for px in 0..w {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let noise = (x % 64) as i32 - 32;
            let (fx, fy) = (px as f32 / w as f32, py as f32 / h as f32);
            let (r, g, b) = if fx < 0.25 {
                (200, 40, 40) // flat
            } else if fx < 0.5 {
                ((fx * 400.0) as i32, (fy * 255.0) as i32, 128) // gradient
            } else if fx < 0.75 {
                let stripe = ((px + py) / 3) % 2 == 0; // diagonal stripes
                if stripe { (240, 240, 240) } else { (20, 30, 90) }
            } else {
                (128 + noise * 2, 100 + noise, 160 - noise) // noise
            };
            let c = |v: i32| v.clamp(0, 255) as u8;
            out.extend_from_slice(&[c(r), c(g), c(b), 255]);
        }
    }
    out
}

/// The decoder's reading of the frame.
fn decode(frame: &Vp8Frame) -> Picture {
    Decoder::new().decode(&frame.data, 0).expect("decode").expect("a key frame is a picture")
}

/// The encoder's reconstruction cropped to the picture, as the decoder's planes: Y, then U, then V.
fn reconstruction(frame: &Vp8Frame, w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = Vec::with_capacity(w * h + 2 * cw * ch);
    for y in 0..h {
        for x in 0..w {
            out.push(frame.y.get(x, y));
        }
    }
    for p in [&frame.u, &frame.v] {
        for y in 0..ch {
            for x in 0..cw {
                out.push(p.get(x, y));
            }
        }
    }
    out
}

/// With the loop filter off, the decoder reconstructs exactly what the encoder did.
#[test]
fn the_decoder_reconstructs_exactly_what_the_encoder_did() {
    for (w, h, quality) in [(64, 48, 90), (33, 17, 50), (100, 75, 10), (16, 16, 100), (130, 66, 75), (1, 1, 60)] {
        let rgba = picture(w, h, w * 31 + h);
        let frame = encode_rgba(&rgba, w, h, &Vp8Params { quality, loop_filter: Some(0) }).expect("encode");
        let name = format!("{w}x{h} q{quality}");
        let pic = decode(&frame);
        assert_eq!((pic.width, pic.height), (w, h), "{name}: picture size");
        let got: Vec<u8> = [&pic.y, &pic.u, &pic.v].into_iter().flatten().copied().collect();
        let want = reconstruction(&frame, w, h);
        assert_eq!(got.len(), want.len(), "{name}: plane sizes");
        if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            panic!("{name}: the decoder differs from the encoder's reconstruction at byte {i} of {} ({} vs {})", got.len(), got[i], want[i]);
        }
    }
}

/// The peak signal-to-noise ratio of `b` against `a`, in dB.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    let mse = se / a.len() as f64;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}

/// A higher quality decodes closer to the quality-100 picture.
#[test]
fn the_decoder_reads_it_back_close_to_the_source() {
    // Against the decode at quality 100, so the comparison is of the coding alone and not of
    // the colour 4:2:0 cannot hold (this picture's fine coloured stripes and noise).
    let (w, h) = (96, 64);
    let rgba = picture(w, h, 7);
    let planes = |quality| {
        let frame = encode_rgba(&rgba, w, h, &Vp8Params { quality, loop_filter: None }).expect("encode");
        let pic = decode(&frame);
        assert_eq!((pic.width, pic.height), (w, h));
        [&pic.y, &pic.u, &pic.v].into_iter().flatten().copied().collect::<Vec<u8>>()
    };
    let best = planes(100);
    let mut last = 0.0;
    for quality in [20u8, 60, 95] {
        let q = psnr(&planes(quality), &best);
        assert!(q >= last, "quality {quality} is no closer than a lower quality: {q:.1} dB after {last:.1}");
        last = q;
    }
    assert!(last > 38.0, "quality 95: {last:.1} dB");
}

/// A higher quality gives a larger frame.
#[test]
fn higher_quality_spends_more_bytes() {
    let (w, h) = (96, 64);
    let rgba = picture(w, h, 11);
    let size = |quality| encode_rgba(&rgba, w, h, &Vp8Params { quality, loop_filter: None }).expect("encode").data.len();
    let (low, mid, high) = (size(10), size(60), size(98));
    assert!(low < mid && mid < high, "{low} {mid} {high}");
}

/// Empty, short and oversized pictures are errors.
#[test]
fn bad_sizes_are_errors() {
    assert!(encode_rgba(&[], 0, 0, &Vp8Params::default()).is_err());
    assert!(encode_rgba(&[0; 12], 2, 2, &Vp8Params::default()).is_err());
    assert!(encode_rgba(&[0; 4], 16384, 1, &Vp8Params::default()).is_err());
}
